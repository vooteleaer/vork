//! The node's web dashboard, for a machine without a screen of its own (a
//! server), in a browser. `web = "0.0.0.0:8742"` in vork.toml turns it on.
//! The same page (`web/index.html`) is the window of vork-net on the desktop
//! (clients/net-desktop) and of the Android app, which carry its requests to
//! their node over the control socket; on a narrow screen its pages are in a
//! menu instead of the side.
//!
//! Every request carries the dashboard's token (from the link the node prints
//! at start, kept by the browser), so being on the same network isn't enough
//! to change the node. The API is the control protocol: `GET /api/status`,
//! and `POST /api/cmd` with one control request as JSON.
//!
//! **HTTPS** with a certificate the node makes once and keeps
//! (`state/web.crt`, `state/web.key`): self-signed, so the browser asks once;
//! `vork web-link` shows its SHA-256 fingerprint to compare. A plain `http://`
//! visit is sent to `https://`.

use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};

use crate::control::Request;

const PAGE: &str = include_str!("web/index.html");

/// The dashboard's token: made once, kept in the state directory.
pub fn token(dir: Option<&Path>) -> String {
    let file = dir.map(|d| d.join("web.token"));
    if let Some(t) = file.as_ref().and_then(|f| std::fs::read_to_string(f).ok()) {
        let t = t.trim().to_string();
        if t.len() >= 16 {
            return t;
        }
    }
    let mut b = [0u8; 16];
    let _ = getrandom::getrandom(&mut b);
    let t = hex::encode(b);
    if let Some(f) = file {
        let _ = std::fs::write(&f, &t);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o600));
        }
    }
    t
}

/// The dashboard's certificate: made once (for these names and addresses)
/// and kept. Returns the TLS setup and the certificate's SHA-256 fingerprint.
pub fn certificate(dir: Option<&Path>, names: &[String], ips: &[IpAddr], given: Option<(&Path, &Path)>) -> anyhow::Result<(Arc<rustls::ServerConfig>, String)> {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    // One the operator gives (Let's Encrypt's fullchain.pem and privkey.pem, say): used as it is.
    if let Some((c, k)) = given {
        let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(c)
            .map_err(|e| anyhow::anyhow!("web_cert {}: {e}", c.display()))?
            .collect::<Result<_, _>>()
            .map_err(|e| anyhow::anyhow!("web_cert {}: {e}", c.display()))?;
        let key = PrivateKeyDer::from_pem_file(k).map_err(|e| anyhow::anyhow!("web_key {}: {e}", k.display()))?;
        let fp = chain.first().map(|c| fingerprint(c)).ok_or_else(|| anyhow::anyhow!("web_cert {}: no certificate in it", c.display()))?;
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_single_cert(chain, key)?;
        return Ok((Arc::new(config), fp));
    }
    let files = dir.map(|d| (d.join("web.crt"), d.join("web.key")));
    let kept = files.as_ref().and_then(|(c, k)| Some((CertificateDer::from_pem_file(c).ok()?, PrivateKeyDer::from_pem_file(k).ok()?)));
    let (cert, key) = match kept {
        Some(ck) => ck,
        None => {
            let mut params = rcgen::CertificateParams::default();
            let mut dn = rcgen::DistinguishedName::new();
            dn.push(rcgen::DnType::CommonName, names.first().cloned().unwrap_or_else(|| "vork".into()));
            dn.push(rcgen::DnType::OrganizationName, "vØrk node");
            params.distinguished_name = dn;
            params.subject_alt_names = names
                .iter()
                .filter_map(|n| n.as_str().try_into().ok().map(rcgen::SanType::DnsName))
                .chain(ips.iter().map(|ip| rcgen::SanType::IpAddress(*ip)))
                .collect();
            let year = 1970 + (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) / 31_556_952) as i32;
            params.not_before = rcgen::date_time_ymd(year - 1, 1, 1);
            params.not_after = rcgen::date_time_ymd(year + 10, 1, 1);
            let key_pair = rcgen::KeyPair::generate()?;
            let cert = params.self_signed(&key_pair)?;
            if let Some((c, k)) = &files {
                std::fs::write(c, cert.pem())?;
                std::fs::write(k, key_pair.serialize_pem())?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(k, std::fs::Permissions::from_mode(0o600));
                }
            }
            (cert.der().clone(), PrivateKeyDer::Pkcs8(key_pair.serialize_der().into()))
        }
    };
    let fingerprint = fingerprint(&cert);
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)?;
    Ok((Arc::new(config), fingerprint))
}

/// SHA-256 of a certificate, as browsers show it (AB:CD:…).
pub fn fingerprint(der: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(der).iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
}

/// The kept certificate's fingerprint (for `vork web-link`).
pub fn kept_fingerprint(dir: &Path) -> Option<String> {
    use rustls::pki_types::pem::PemObject;
    let c = rustls::pki_types::CertificateDer::from_pem_file(dir.join("web.crt")).ok()?;
    Some(fingerprint(&c))
}

/// Serves the dashboard on `addr` over HTTPS; requests go to the node as control requests.
pub async fn serve(addr: SocketAddr, token: String, tls: Arc<rustls::ServerConfig>, to_node: mpsc::Sender<Request>) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let acceptor = tokio_rustls::TlsAcceptor::from(tls);
    tokio::spawn(async move {
        while let Ok((conn, _)) = listener.accept().await {
            let (token, to_node, acceptor) = (token.clone(), to_node.clone(), acceptor.clone());
            tokio::spawn(async move {
                // TLS begins with a handshake record (0x16); anything else is plain HTTP.
                let mut first = [0u8; 1];
                let tls = matches!(tokio::time::timeout(std::time::Duration::from_secs(10), conn.peek(&mut first)).await, Ok(Ok(1)) if first[0] == 0x16);
                if tls {
                    if let Ok(Ok(s)) = tokio::time::timeout(std::time::Duration::from_secs(10), acceptor.accept(conn)).await {
                        let _ = handle(s, Some(&token), to_node).await;
                    }
                } else {
                    let _ = to_https(conn).await;
                }
            });
        }
    });
    Ok(())
}

/// The dashboard behind Home Assistant's ingress (`web_ingress`): plain HTTP,
/// from one address only (the Supervisor's ingress proxy, `web_ingress_from`),
/// which has signed its user in already, so no token is asked for. The page
/// is served at a path of Home Assistant's, so it asks relative URLs, and in
/// Home Assistant's frame.
pub async fn serve_ingress(addr: SocketAddr, from: IpAddr, to_node: mpsc::Sender<Request>) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tokio::spawn(async move {
        while let Ok((conn, peer)) = listener.accept().await {
            if peer.ip().to_canonical() != from {
                continue; // dropped: only the ingress proxy is let in
            }
            let to_node = to_node.clone();
            tokio::spawn(async move {
                let _ = handle(conn, None, to_node).await;
            });
        }
    });
    Ok(())
}

/// A plain-HTTP visit: sent to the same place over HTTPS.
async fn to_https(mut conn: tokio::net::TcpStream) -> anyhow::Result<()> {
    let mut buf = vec![0u8; 4096];
    let n = tokio::time::timeout(std::time::Duration::from_secs(10), conn.read(&mut buf)).await??;
    let head = String::from_utf8_lossy(&buf[..n]);
    let host = head.lines().find_map(|l| l.strip_prefix("Host:").or_else(|| l.strip_prefix("host:")).map(str::trim)).unwrap_or_default();
    let path = head.lines().next().and_then(|l| l.split_whitespace().nth(1)).filter(|p| p.starts_with('/')).unwrap_or("/");
    // Only a plausible host goes into the header.
    let host: String = host.chars().filter(|c| c.is_ascii_alphanumeric() || "-.:[]".contains(*c)).collect();
    let r = format!("HTTP/1.1 301 Moved Permanently\r\nLocation: https://{host}{path}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    conn.write_all(r.as_bytes()).await?;
    Ok(())
}

/// One request. `token` None: behind the ingress proxy, which signed the user in.
async fn handle<S: AsyncRead + AsyncWrite + Unpin>(mut conn: S, token: Option<&str>, to_node: mpsc::Sender<Request>) -> anyhow::Result<()> {
    let ingress = token.is_none();
    // One request per connection (Connection: close), headers and a small body.
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    let (head_end, length) = loop {
        let n = tokio::time::timeout(std::time::Duration::from_secs(10), conn.read(&mut chunk)).await??;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > 64 * 1024 {
            return respond(&mut conn, 413, "text/plain", b"too large", ingress).await;
        }
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..i]).to_string();
            let len = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
            break (i + 4, len.min(32 * 1024));
        }
    };
    while buf.len() < head_end + length {
        let n = conn.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let body = buf[head_end..(head_end + length).min(buf.len())].to_vec();
    let mut first = head.lines().next().unwrap_or_default().split_whitespace();
    let (method, path) = (first.next().unwrap_or_default(), first.next().unwrap_or("/"));
    let path = path.split('?').next().unwrap_or("/");
    let given = head.lines().find_map(|l| l.strip_prefix("X-Vork-Token:").or_else(|| l.strip_prefix("x-vork-token:")).map(str::trim)).unwrap_or_default();
    let authorized = match token {
        Some(t) => !given.is_empty() && constant_eq(given.as_bytes(), t.as_bytes()),
        None => true,
    };

    match (method, path) {
        ("GET", "/") | ("GET", "/index.html") if ingress => {
            let page = PAGE.replacen("<head>", "<head>\n<script>window.VORK_INGRESS = true;</script>", 1);
            respond(&mut conn, 200, "text/html; charset=utf-8", page.as_bytes(), ingress).await
        }
        ("GET", "/") | ("GET", "/index.html") => respond(&mut conn, 200, "text/html; charset=utf-8", PAGE.as_bytes(), ingress).await,
        ("GET", "/api/status") | ("POST", "/api/cmd") if !authorized => respond(&mut conn, 401, "application/json", br#"{"error":"open the dashboard with the link the node printed (vork web-link)"}"#, ingress).await,
        ("GET", "/api/status") => {
            let answer = ask(&to_node, r#"{"cmd":"status"}"#.to_string()).await;
            respond(&mut conn, 200, "application/json", answer.as_bytes(), ingress).await
        }
        ("GET", "/api/metrics") | ("GET", "/api/log") if !authorized => respond(&mut conn, 401, "application/json", b"{}", ingress).await,
        ("GET", "/api/metrics") => {
            let answer = ask(&to_node, r#"{"cmd":"metrics"}"#.to_string()).await;
            respond(&mut conn, 200, "application/json", answer.as_bytes(), ingress).await
        }
        ("GET", "/api/log") => {
            let answer = ask(&to_node, r#"{"cmd":"log"}"#.to_string()).await;
            respond(&mut conn, 200, "application/json", answer.as_bytes(), ingress).await
        }
        ("POST", "/api/cmd") => {
            let Ok(v) = serde_json::from_slice::<serde_json::Value>(&body) else {
                return respond(&mut conn, 400, "application/json", br#"{"error":"bad request"}"#, ingress).await;
            };
            let answer = ask(&to_node, v.to_string()).await;
            respond(&mut conn, 200, "application/json", answer.as_bytes(), ingress).await
        }
        _ => respond(&mut conn, 404, "text/plain", b"not found", ingress).await,
    }
}

async fn ask(to_node: &mpsc::Sender<Request>, line: String) -> String {
    let (tx, rx) = oneshot::channel();
    if to_node.send((line, true, tx)).await.is_err() {
        return r#"{"error":"the node is stopping"}"#.into();
    }
    rx.await.unwrap_or_else(|_| r#"{"error":"no answer"}"#.into())
}

/// `framed`: shown in Home Assistant's frame (ingress), same origin only.
async fn respond<S: AsyncWrite + Unpin>(conn: &mut S, code: u16, kind: &str, body: &[u8], framed: bool) -> anyhow::Result<()> {
    let frame = if framed { "SAMEORIGIN" } else { "DENY" };
    let reason = match code {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: {frame}\r\nReferrer-Policy: no-referrer\r\nStrict-Transport-Security: max-age=31536000\r\nConnection: close\r\n\r\n",
        body.len()
    );
    conn.write_all(head.as_bytes()).await?;
    conn.write_all(body).await?;
    conn.flush().await?;
    let _ = conn.shutdown().await;
    Ok(())
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

//! Public nodes to start from (ADR-0031): a list kept in the repository,
//! `bootstrap.txt`, of nodes whose owners chose to offer them. A node that
//! knows no one yet dials them, so every node starts in the same network.
//!
//! The list is fetched from GitHub (at most once a day, only while the node
//! knows no one) and kept in the state directory; the copy built into this
//! binary is the fallback. Entries are only leads: a wrong one costs a key
//! query, and a node on the list can't read anyone's traffic.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// The list this release was built with.
const BUILT_IN: &str = include_str!("../bootstrap.txt");
const HOST: &str = "raw.githubusercontent.com";
const PATH: &str = "/vooteleaer/vork/main/bootstrap.txt";
/// The fetched copy, in the state directory.
const CACHE: &str = "bootstrap.txt";
/// How old the fetched copy may be before it is fetched again.
pub const REFRESH: Duration = Duration::from_secs(86400);
/// The most entries taken from a list (a list is a handful of servers).
const MAX_ENTRIES: usize = 64;
const MAX_BYTES: usize = 64 * 1024;

/// Entries of a list: "host:port" or "host:port@key", one per line, `#` comments.
pub fn parse(text: &str) -> Vec<String> {
    text.lines().map(|l| l.split('#').next().unwrap_or("").trim()).filter(|l| !l.is_empty()).take(MAX_ENTRIES).map(str::to_string).collect()
}

fn cache_path(state: Option<&Path>) -> Option<PathBuf> {
    state.map(|d| d.join(CACHE))
}

/// The entries to dial: the fetched copy's and the built-in ones, without repeats.
pub fn entries(state: Option<&Path>) -> Vec<String> {
    let fetched = cache_path(state).and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let mut all = parse(&fetched);
    for e in parse(BUILT_IN) {
        if !all.contains(&e) {
            all.push(e);
        }
    }
    all
}

/// Whether the fetched copy is missing or older than [`REFRESH`].
pub fn stale(state: Option<&Path>) -> bool {
    let Some(p) = cache_path(state) else { return false };
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_none_or(|age| age >= REFRESH)
}

/// Fetches the list in a thread of its own and keeps it in the state directory
/// (the node reads it at its next look); a failure leaves the old copy.
pub fn refresh(state: Option<&Path>) {
    let Some(path) = cache_path(state) else { return };
    std::thread::spawn(move || match fetch() {
        Ok(text) if !parse(&text).is_empty() => {
            let tmp = path.with_extension("txt.new");
            if std::fs::write(&tmp, &text).and_then(|_| std::fs::rename(&tmp, &path)).is_ok() {
                crate::node::log(format!("bootstrap list fetched: {} public node(s)", parse(&text).len()));
            }
        }
        Ok(_) => crate::node::log("bootstrap list fetched, but it lists no nodes: the built-in one is used"),
        Err(e) => crate::node::log(format!("bootstrap list not fetched ({e}): the built-in one is used")),
    });
}

/// `GET https://raw.githubusercontent.com/…/bootstrap.txt`, with rustls and
/// the Mozilla roots (no system certificate store needed, as on Android or Haiku).
fn fetch() -> Result<String, String> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_root_certificates(roots)
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from(HOST).map_err(|e| e.to_string())?;
    let conn = rustls::ClientConnection::new(Arc::new(config), name).map_err(|e| e.to_string())?;
    let addr: SocketAddr = (HOST, 443).to_socket_addrs().map_err(|e| e.to_string())?.next().ok_or("no address")?;
    let sock = TcpStream::connect_timeout(&addr, Duration::from_secs(10)).map_err(|e| e.to_string())?;
    sock.set_read_timeout(Some(Duration::from_secs(15))).map_err(|e| e.to_string())?;
    sock.set_write_timeout(Some(Duration::from_secs(15))).map_err(|e| e.to_string())?;
    let mut tls = rustls::StreamOwned::new(conn, sock);
    let req = format!("GET {PATH} HTTP/1.1\r\nHost: {HOST}\r\nUser-Agent: vork/{}\r\nAccept: text/plain\r\nConnection: close\r\n\r\n", env!("VORK_VERSION"));
    tls.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut answer = Vec::new();
    // A server closing without close_notify is still a whole answer once the body is in.
    let _ = Read::by_ref(&mut tls).take(MAX_BYTES as u64 + 4096).read_to_end(&mut answer);
    let text = String::from_utf8_lossy(&answer);
    let (head, body) = text.split_once("\r\n\r\n").ok_or("no answer")?;
    let status = head.lines().next().unwrap_or_default();
    if !status.split_whitespace().nth(1).is_some_and(|c| c == "200") {
        return Err(format!("the server said {status}"));
    }
    if head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
        return Err("a chunked answer".into());
    }
    Ok(body.chars().take(MAX_BYTES).collect())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_list_is_lines_without_comments() {
        let l = super::parse("# public nodes\n\n198.51.100.1:51820\n  example.org:4000@ab  # a server\n#203.0.113.9:1\n");
        assert_eq!(l, vec!["198.51.100.1:51820", "example.org:4000@ab"]);
    }

    #[test]
    fn the_built_in_list_names_a_node() {
        assert!(!super::parse(super::BUILT_IN).is_empty());
    }
}

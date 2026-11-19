//! The node's web dashboard, for a machine without a screen of its own (a
//! server): the same as the Android network app (Overview, My devices with
//! pairing codes, Settings), in a browser. `web = "0.0.0.0:8742"` in
//! vork.toml turns it on.
//!
//! Every request carries the dashboard's token (from the link the node prints
//! at start, kept by the browser), so being on the same network isn't enough
//! to change the node. The API is the control protocol: `GET /api/status`,
//! and `POST /api/cmd` with one control request as JSON.

use std::net::SocketAddr;
use std::path::Path;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
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

/// Serves the dashboard on `addr`; requests go to the node as control requests.
pub async fn serve(addr: SocketAddr, token: String, to_node: mpsc::Sender<Request>) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tokio::spawn(async move {
        while let Ok((conn, _)) = listener.accept().await {
            let (token, to_node) = (token.clone(), to_node.clone());
            tokio::spawn(async move {
                let _ = handle(conn, &token, to_node).await;
            });
        }
    });
    Ok(())
}

async fn handle(mut conn: TcpStream, token: &str, to_node: mpsc::Sender<Request>) -> anyhow::Result<()> {
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
            return respond(&mut conn, 413, "text/plain", b"too large").await;
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
    let authorized = !given.is_empty() && constant_eq(given.as_bytes(), token.as_bytes());

    match (method, path) {
        ("GET", "/") | ("GET", "/index.html") => respond(&mut conn, 200, "text/html; charset=utf-8", PAGE.as_bytes()).await,
        ("GET", "/api/status") | ("POST", "/api/cmd") if !authorized => respond(&mut conn, 401, "application/json", br#"{"error":"open the dashboard with the link the node printed (vork web-link)"}"#).await,
        ("GET", "/api/status") => {
            let answer = ask(&to_node, r#"{"cmd":"status"}"#.to_string()).await;
            respond(&mut conn, 200, "application/json", answer.as_bytes()).await
        }
        ("POST", "/api/cmd") => {
            let Ok(v) = serde_json::from_slice::<serde_json::Value>(&body) else {
                return respond(&mut conn, 400, "application/json", br#"{"error":"bad request"}"#).await;
            };
            let answer = ask(&to_node, v.to_string()).await;
            respond(&mut conn, 200, "application/json", answer.as_bytes()).await
        }
        _ => respond(&mut conn, 404, "text/plain", b"not found").await,
    }
}

async fn ask(to_node: &mpsc::Sender<Request>, line: String) -> String {
    let (tx, rx) = oneshot::channel();
    if to_node.send((line, true, tx)).await.is_err() {
        return r#"{"error":"the node is stopping"}"#.into();
    }
    rx.await.unwrap_or_else(|_| r#"{"error":"no answer"}"#.into())
}

async fn respond(conn: &mut TcpStream, code: u16, kind: &str, body: &[u8]) -> anyhow::Result<()> {
    let reason = match code {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
        body.len()
    );
    conn.write_all(head.as_bytes()).await?;
    conn.write_all(body).await?;
    Ok(())
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

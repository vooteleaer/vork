//! The local control endpoint: a Unix socket (or a Windows named pipe) that
//! takes one JSON request per line and answers with one JSON line. Requests
//! go to the node's event loop, which owns all state.
//!
//! M1 has only `{"cmd":"status"}`. The protocol grows into the shared-node
//! IPC of ADR-0020 (M3) and is specified with the wire format.

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

pub type Request = (String, oneshot::Sender<String>);

async fn serve_conn<S: AsyncRead + AsyncWrite + Unpin>(conn: S, to_node: mpsc::Sender<Request>) {
    let (r, mut w) = tokio::io::split(conn);
    let mut lines = BufReader::new(r).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let (tx, rx) = oneshot::channel();
        if to_node.send((line, tx)).await.is_err() {
            return;
        }
        let Ok(mut answer) = rx.await else { return };
        answer.push('\n');
        if w.write_all(answer.as_bytes()).await.is_err() {
            return;
        }
    }
}

/// Starts listening; requests arrive on the returned channel.
#[cfg(unix)]
pub fn listen(endpoint: &str) -> Result<mpsc::Receiver<Request>> {
    use std::os::unix::fs::PermissionsExt;
    let path = std::path::Path::new(endpoint);
    // A socket left by an earlier run would make bind fail.
    if path.exists() && std::os::unix::net::UnixStream::connect(path).is_err() {
        let _ = std::fs::remove_file(path);
    }
    let listener = tokio::net::UnixListener::bind(path).with_context(|| format!("control socket {endpoint}"))?;
    // Owner and group only (a `vork` group can be given access).
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o660));
    let (tx, rx) = mpsc::channel(16);
    tokio::spawn(async move {
        while let Ok((conn, _)) = listener.accept().await {
            tokio::spawn(serve_conn(conn, tx.clone()));
        }
    });
    Ok(rx)
}

#[cfg(windows)]
pub fn listen(endpoint: &str) -> Result<mpsc::Receiver<Request>> {
    use tokio::net::windows::named_pipe::ServerOptions;
    let name = endpoint.to_string();
    let mut server = ServerOptions::new().first_pipe_instance(true).create(&name).with_context(|| format!("control pipe {name}"))?;
    let (tx, rx) = mpsc::channel(16);
    tokio::spawn(async move {
        loop {
            if server.connect().await.is_err() {
                return;
            }
            let conn = server;
            server = match ServerOptions::new().create(&name) {
                Ok(s) => s,
                Err(_) => return,
            };
            tokio::spawn(serve_conn(conn, tx.clone()));
        }
    });
    Ok(rx)
}

async fn ask_conn<S: AsyncRead + AsyncWrite + Unpin>(conn: S, request: &str) -> Result<String> {
    let (r, mut w) = tokio::io::split(conn);
    w.write_all(format!("{request}\n").as_bytes()).await?;
    let mut line = String::new();
    BufReader::new(r).read_line(&mut line).await?;
    Ok(line)
}

/// Sends one request to a running node and returns its answer.
#[cfg(unix)]
pub async fn ask(endpoint: &str, request: &str) -> Result<String> {
    let conn = tokio::net::UnixStream::connect(endpoint)
        .await
        .with_context(|| format!("no node answers at {endpoint} (is it running?)"))?;
    ask_conn(conn, request).await
}

#[cfg(windows)]
pub async fn ask(endpoint: &str, request: &str) -> Result<String> {
    use tokio::net::windows::named_pipe::ClientOptions;
    let conn = ClientOptions::new().open(endpoint).with_context(|| format!("no node answers at {endpoint} (is it running?)"))?;
    ask_conn(conn, request).await
}

//! The node's side of the local API (crate `vork-api`): accepts application
//! connections and passes their frames to the node's event loop, which owns
//! all state and answers through each connection's sender.

use anyhow::{Context, Result};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use vork_api::{ToNode, read_frame, write_frame};

pub enum Event {
    Connected { client: u32, tx: mpsc::UnboundedSender<Vec<u8>> },
    Frame { client: u32, frame: ToNode },
    Gone { client: u32 },
}

async fn serve_conn<S: AsyncRead + AsyncWrite + Send + 'static>(conn: S, client: u32, to_node: mpsc::Sender<Event>) {
    let (mut r, mut w) = tokio::io::split(conn);
    let (tx, mut out) = mpsc::unbounded_channel::<Vec<u8>>();
    if to_node.send(Event::Connected { client, tx }).await.is_err() {
        return;
    }
    tokio::spawn(async move {
        while let Some(f) = out.recv().await {
            if write_frame(&mut w, &f).await.is_err() {
                return;
            }
        }
    });
    loop {
        let frame = match read_frame(&mut r).await.and_then(|f| ToNode::decode(&f)) {
            Ok(f) => f,
            Err(_) => break,
        };
        if to_node.send(Event::Frame { client, frame }).await.is_err() {
            return;
        }
    }
    let _ = to_node.send(Event::Gone { client }).await;
}

/// Starts listening; connections and their frames arrive on the returned channel.
#[cfg(unix)]
pub fn listen(endpoint: &str) -> Result<mpsc::Receiver<Event>> {
    use std::os::unix::fs::PermissionsExt;
    let path = std::path::Path::new(endpoint);
    if path.exists() && std::os::unix::net::UnixStream::connect(path).is_err() {
        let _ = std::fs::remove_file(path);
    }
    let listener = tokio::net::UnixListener::bind(path).with_context(|| format!("API socket {endpoint}"))?;
    // Any local program may use the network; registering a destination needs
    // its identity's secret, so nothing here lets one program act as another.
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666));
    let (tx, rx) = mpsc::channel(256);
    tokio::spawn(async move {
        let mut next = 1u32;
        while let Ok((conn, _)) = listener.accept().await {
            tokio::spawn(serve_conn(conn, next, tx.clone()));
            next += 1;
        }
    });
    Ok(rx)
}

#[cfg(windows)]
pub fn listen(endpoint: &str) -> Result<mpsc::Receiver<Event>> {
    use tokio::net::windows::named_pipe::ServerOptions;
    let name = endpoint.to_string();
    let mut server = ServerOptions::new().first_pipe_instance(true).create(&name).with_context(|| format!("API pipe {name}"))?;
    let (tx, rx) = mpsc::channel(256);
    tokio::spawn(async move {
        let mut next = 1u32;
        loop {
            if server.connect().await.is_err() {
                return;
            }
            let conn = server;
            server = match ServerOptions::new().create(&name) {
                Ok(s) => s,
                Err(_) => return,
            };
            tokio::spawn(serve_conn(conn, next, tx.clone()));
            next += 1;
        }
    });
    Ok(rx)
}

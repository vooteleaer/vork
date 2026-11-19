//! The local control endpoint: a Unix socket (or a Windows named pipe) that
//! takes one JSON request per line and answers with one JSON line. Requests
//! go to the node's event loop, which owns all state.
//!
//! M1 has only `{"cmd":"status"}`. The protocol grows into the shared-node
//! IPC of ADR-0020 (M3) and is specified with the wire format.

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

/// A request line, whether it may change the node (its owner or root asked),
/// and where the answer goes.
pub type Request = (String, bool, oneshot::Sender<String>);

/// Whether the other end of a Unix socket is the node's own user, root or
/// one of the operators: they may change settings; anyone else may only look.
#[cfg(unix)]
fn is_owner(conn: &tokio::net::UnixStream, operators: &[u32]) -> bool {
    match conn.peer_cred() {
        Ok(c) => c.uid() == 0 || c.uid() == unsafe { libc::geteuid() } || operators.contains(&c.uid()),
        Err(_) => false,
    }
}

/// A user's id from a name (or a number), through the system's user database.
#[cfg(unix)]
fn uid_of(user: &str) -> Option<u32> {
    if let Ok(n) = user.parse() {
        return Some(n);
    }
    let name = std::ffi::CString::new(user).ok()?;
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf = vec![0 as libc::c_char; 16 * 1024];
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    // Safe: every pointer is to a live local of the right size.
    let r = unsafe { libc::getpwnam_r(name.as_ptr(), &mut pw, buf.as_mut_ptr(), buf.len(), &mut found) };
    (r == 0 && !found.is_null()).then_some(pw.pw_uid)
}

async fn serve_conn<S: AsyncRead + AsyncWrite + Unpin>(conn: S, owner: bool, to_node: mpsc::Sender<Request>) {
    let (r, mut w) = tokio::io::split(conn);
    let mut lines = BufReader::new(r).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let (tx, rx) = oneshot::channel();
        if to_node.send((line, owner, tx)).await.is_err() {
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
pub fn listen(endpoint: &str, operators: &[String]) -> Result<mpsc::Receiver<Request>> {
    use std::os::unix::fs::PermissionsExt;
    let mut uids = Vec::new();
    for o in operators {
        match uid_of(o) {
            Some(u) => uids.push(u),
            None => crate::node::log(format!("operators: no user {o} on this machine (left out)")),
        }
    }
    let path = std::path::Path::new(endpoint);
    // A socket left by an earlier run would make bind fail.
    if path.exists() && std::os::unix::net::UnixStream::connect(path).is_err() {
        let _ = std::fs::remove_file(path);
    }
    let listener = tokio::net::UnixListener::bind(path).with_context(|| format!("control socket {endpoint}"))?;
    // Any local user: the status shows nothing secret, and the desktop
    // utility runs as the user, not as the service's owner.
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666));
    let (tx, rx) = mpsc::channel(16);
    tokio::spawn(async move {
        while let Ok((conn, _)) = listener.accept().await {
            let owner = is_owner(&conn, &uids);
            tokio::spawn(serve_conn(conn, owner, tx.clone()));
        }
    });
    Ok(rx)
}

/// On Windows the pipe's ACL decides (`create_pipe`): operators don't apply.
#[cfg(windows)]
pub fn listen(endpoint: &str, _operators: &[String]) -> Result<mpsc::Receiver<Request>> {
    let name = endpoint.to_string();
    let mut server = create_pipe(&name, true).with_context(|| format!("control pipe {name}"))?;
    let (tx, rx) = mpsc::channel(16);
    tokio::spawn(async move {
        loop {
            if server.connect().await.is_err() {
                return;
            }
            let conn = server;
            server = match create_pipe(&name, false) {
                Ok(s) => s,
                Err(_) => return,
            };
            // Who may connect is the pipe's ACL (create_pipe): those are the owner.
            tokio::spawn(serve_conn(conn, true, tx.clone()));
        }
    });
    Ok(rx)
}

/// A pipe instance that SYSTEM, administrators and the users signed in at
/// this computer (interactive) may use: the default ACL left out the user
/// whose app talks to a node running as a service.
#[cfg(windows)]
fn create_pipe(name: &str, first: bool) -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeServer> {
    use windows_sys::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
    use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
    let sddl: Vec<u16> = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)".encode_utf16().chain([0]).collect();
    let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // Safe: the SDDL string is NUL-terminated; the descriptor is freed below.
    if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut sd, std::ptr::null_mut()) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut sa = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd, bInheritHandle: 0 };
    let r = unsafe {
        tokio::net::windows::named_pipe::ServerOptions::new()
            .first_pipe_instance(first)
            .create_with_security_attributes_raw(name, &mut sa as *mut _ as *mut std::ffi::c_void)
    };
    unsafe { windows_sys::Win32::Foundation::LocalFree(sd as _) };
    r
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

#[cfg(all(test, unix))]
mod tests {
    #[test]
    fn operators_by_name_or_id() {
        assert_eq!(super::uid_of("root"), Some(0));
        assert_eq!(super::uid_of("1000"), Some(1000));
        assert_eq!(super::uid_of("no-such-user-here"), None);
    }
}

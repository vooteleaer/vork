//! The local API of a vØrk node (ADR-0020, ADR-0022): how applications on
//! the same machine use the machine's node instead of running their own.
//!
//! An application connects to the node's API socket (a Unix socket, or a
//! named pipe on Windows), registers its destinations (an identity and a
//! service name), announces them, finds others, opens sessions to them and
//! sends requests; requests to its own destinations arrive as events and it
//! answers them. Frames are `length[4] kind[1] body`, big-endian.
//!
//! This crate is both the protocol (used by the node) and an async client.

use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};

pub type Hash = [u8; 16];
pub const VERSION: u16 = 1;
/// Largest frame (a 32 MiB request or response with room for headers).
pub const MAX_FRAME: usize = (32 << 20) + 4096;

pub const STATUS_OK: u8 = 0;
pub const STATUS_NO_PATH: u8 = 1;
pub const STATUS_REFUSED: u8 = 2;

/// What an announce told about a destination.
#[derive(Clone, Debug, PartialEq)]
pub struct Known {
    /// The identity's public key, X25519 ‖ Ed25519 (the storage layout).
    pub public: [u8; 64],
    pub name_hash: [u8; 10],
    pub hops: u8,
    pub app_data: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ToNode {
    Hello { version: u16 },
    /// Hosts the destination `name` of the identity with this secret (X25519 ‖ Ed25519 seed).
    Register { req: u32, secret: [u8; 64], name: String },
    /// Announces a registered destination now and from now on with this app data.
    Announce { dest: Hash, app_data: Vec<u8> },
    /// What is known about a destination, without asking the network.
    Recall { req: u32, dest: Hash },
    /// Asks the network for a path and waits for it.
    Find { req: u32, dest: Hash, timeout_ms: u32 },
    Open { req: u32, dest: Hash },
    /// Proves to the far end of `session` that we are the identity of our destination `as_dest`.
    Identify { session: u32, as_dest: Hash },
    Request { req: u32, session: u32, path: String, body: Vec<u8> },
    Close { session: u32 },
    Respond { token: u64, status: u8, body: Vec<u8> },
    /// Every announce the node hears is passed on as `Heard`.
    Subscribe,
    /// The owner's other devices, as destinations or identities: sessions
    /// with them get direct links (ADR-0011). Replaces what this client said
    /// before; forgotten when it disconnects.
    Devices { devices: Vec<Hash> },
    /// Pages this application adds to the node's dashboard: `id` (a-z, 0-9,
    /// -), the menu's title, and (path, title) for each page. None: the
    /// application is forgotten. The node asks for a page's content with `Page`.
    Pages { id: String, title: String, pages: Vec<(String, String)> },
}

#[derive(Clone, Debug, PartialEq)]
pub enum FromNode {
    Hello { version: u16, address: Hash },
    Registered { req: u32, dest: Hash },
    Known { req: u32, known: Option<Known> },
    Opened { req: u32, session: u32, nonce: [u8; 32] },
    Response { req: u32, status: u8, body: Vec<u8> },
    Error { req: u32, message: String },
    Heard { dest: Hash, known: Known },
    /// A request to one of our destinations; answer with `Respond { token }`.
    Incoming { token: u64, dest: Hash, session: u32, nonce: [u8; 32], requester: Option<Hash>, path: String, body: Vec<u8> },
    Closed { session: u32 },
    /// The dashboard asks for one of this application's pages (or a call a
    /// page makes): answer with `Respond { token }`, status 0 and the content.
    Page { token: u64, id: String, path: String, body: Vec<u8> },
}

// ---- encoding --------------------------------------------------------------------

struct W(Vec<u8>);
impl W {
    fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn u64(&mut self, v: u64) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn raw(&mut self, v: &[u8]) -> &mut Self {
        self.0.extend_from_slice(v);
        self
    }
    fn str(&mut self, s: &str) -> &mut Self {
        let b = &s.as_bytes()[..s.len().min(255)];
        self.u8(b.len() as u8).raw(b)
    }
    fn known(&mut self, k: &Known) -> &mut Self {
        self.raw(&k.public).raw(&k.name_hash).u8(k.hops).u16(k.app_data.len() as u16).raw(&k.app_data)
    }
}

struct R<'a>(&'a [u8]);
impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        if self.0.len() < n {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too short"));
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn arr<const N: usize>(&mut self) -> io::Result<[u8; N]> {
        Ok(self.take(N)?.try_into().unwrap())
    }
    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> io::Result<u16> {
        Ok(u16::from_be_bytes(self.arr()?))
    }
    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_be_bytes(self.arr()?))
    }
    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_be_bytes(self.arr()?))
    }
    fn str(&mut self) -> io::Result<String> {
        let n = self.u8()? as usize;
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
    fn rest(&mut self) -> Vec<u8> {
        let v = self.0.to_vec();
        self.0 = &[];
        v
    }
    fn known(&mut self) -> io::Result<Known> {
        let public = self.arr()?;
        let name_hash = self.arr()?;
        let hops = self.u8()?;
        let n = self.u16()? as usize;
        Ok(Known { public, name_hash, hops, app_data: self.take(n)?.to_vec() })
    }
}

fn bad(kind: u8) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("unknown frame kind {kind}"))
}

impl ToNode {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W(Vec::new());
        match self {
            ToNode::Hello { version } => w.u8(1).u16(*version),
            ToNode::Register { req, secret, name } => w.u8(2).u32(*req).raw(secret).str(name),
            ToNode::Announce { dest, app_data } => w.u8(3).raw(dest).raw(app_data),
            ToNode::Recall { req, dest } => w.u8(4).u32(*req).raw(dest),
            ToNode::Find { req, dest, timeout_ms } => w.u8(5).u32(*req).raw(dest).u32(*timeout_ms),
            ToNode::Open { req, dest } => w.u8(6).u32(*req).raw(dest),
            ToNode::Identify { session, as_dest } => w.u8(7).u32(*session).raw(as_dest),
            ToNode::Request { req, session, path, body } => w.u8(8).u32(*req).u32(*session).str(path).raw(body),
            ToNode::Close { session } => w.u8(9).u32(*session),
            ToNode::Respond { token, status, body } => w.u8(10).u64(*token).u8(*status).raw(body),
            ToNode::Subscribe => w.u8(11),
            ToNode::Devices { devices } => {
                w.u8(12);
                for d in devices {
                    w.raw(d);
                }
                &mut w
            }
            ToNode::Pages { id, title, pages } => {
                w.u8(13).str(id).str(title).u8(pages.len().min(255) as u8);
                for (path, title) in pages.iter().take(255) {
                    w.str(path).str(title);
                }
                &mut w
            }
        };
        w.0
    }

    pub fn decode(b: &[u8]) -> io::Result<Self> {
        let mut r = R(b);
        Ok(match r.u8()? {
            1 => ToNode::Hello { version: r.u16()? },
            2 => ToNode::Register { req: r.u32()?, secret: r.arr()?, name: r.str()? },
            3 => ToNode::Announce { dest: r.arr()?, app_data: r.rest() },
            4 => ToNode::Recall { req: r.u32()?, dest: r.arr()? },
            5 => ToNode::Find { req: r.u32()?, dest: r.arr()?, timeout_ms: r.u32()? },
            6 => ToNode::Open { req: r.u32()?, dest: r.arr()? },
            7 => ToNode::Identify { session: r.u32()?, as_dest: r.arr()? },
            8 => ToNode::Request { req: r.u32()?, session: r.u32()?, path: r.str()?, body: r.rest() },
            9 => ToNode::Close { session: r.u32()? },
            10 => ToNode::Respond { token: r.u64()?, status: r.u8()?, body: r.rest() },
            11 => ToNode::Subscribe,
            12 => ToNode::Devices { devices: r.rest().chunks_exact(16).map(|c| c.try_into().unwrap()).collect() },
            13 => {
                let (id, title, n) = (r.str()?, r.str()?, r.u8()?);
                let mut pages = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    pages.push((r.str()?, r.str()?));
                }
                ToNode::Pages { id, title, pages }
            }
            k => return Err(bad(k)),
        })
    }
}

impl FromNode {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W(Vec::new());
        match self {
            FromNode::Hello { version, address } => w.u8(1).u16(*version).raw(address),
            FromNode::Registered { req, dest } => w.u8(2).u32(*req).raw(dest),
            FromNode::Known { req, known } => {
                w.u8(3).u32(*req);
                match known {
                    Some(k) => w.u8(1).known(k),
                    None => w.u8(0),
                }
            }
            FromNode::Opened { req, session, nonce } => w.u8(4).u32(*req).u32(*session).raw(nonce),
            FromNode::Response { req, status, body } => w.u8(5).u32(*req).u8(*status).raw(body),
            FromNode::Error { req, message } => w.u8(6).u32(*req).str(message),
            FromNode::Heard { dest, known } => w.u8(7).raw(dest).known(known),
            FromNode::Incoming { token, dest, session, nonce, requester, path, body } => {
                w.u8(8).u64(*token).raw(dest).u32(*session).raw(nonce);
                match requester {
                    Some(h) => w.u8(1).raw(h),
                    None => w.u8(0),
                };
                w.str(path).raw(body)
            }
            FromNode::Closed { session } => w.u8(9).u32(*session),
            FromNode::Page { token, id, path, body } => w.u8(10).u64(*token).str(id).str(path).raw(body),
        };
        w.0
    }

    pub fn decode(b: &[u8]) -> io::Result<Self> {
        let mut r = R(b);
        Ok(match r.u8()? {
            1 => FromNode::Hello { version: r.u16()?, address: r.arr()? },
            2 => FromNode::Registered { req: r.u32()?, dest: r.arr()? },
            3 => {
                let req = r.u32()?;
                let known = if r.u8()? == 1 { Some(r.known()?) } else { None };
                FromNode::Known { req, known }
            }
            4 => FromNode::Opened { req: r.u32()?, session: r.u32()?, nonce: r.arr()? },
            5 => FromNode::Response { req: r.u32()?, status: r.u8()?, body: r.rest() },
            6 => FromNode::Error { req: r.u32()?, message: r.str()? },
            7 => FromNode::Heard { dest: r.arr()?, known: r.known()? },
            8 => {
                let token = r.u64()?;
                let dest = r.arr()?;
                let session = r.u32()?;
                let nonce = r.arr()?;
                let requester = if r.u8()? == 1 { Some(r.arr()?) } else { None };
                FromNode::Incoming { token, dest, session, nonce, requester, path: r.str()?, body: r.rest() }
            }
            9 => FromNode::Closed { session: r.u32()? },
            10 => FromNode::Page { token: r.u64()?, id: r.str()?, path: r.str()?, body: r.rest() },
            k => return Err(bad(k)),
        })
    }
}

pub async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, body: &[u8]) -> io::Result<()> {
    w.write_all(&(body.len() as u32).to_be_bytes()).await?;
    w.write_all(body).await
}

pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).await?;
    let n = u32::from_be_bytes(len) as usize;
    if n == 0 || n > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("frame of {n} bytes")));
    }
    let mut b = vec![0u8; n];
    r.read_exact(&mut b).await?;
    Ok(b)
}

// ---- where the node listens ------------------------------------------------------

/// The API endpoint next to a node's state directory (Unix), or for its UDP
/// port (Windows named pipe).
pub fn endpoint_for(state: Option<&std::path::Path>, port: u16) -> Option<String> {
    #[cfg(windows)]
    {
        let _ = state;
        Some(format!(r"\\.\pipe\vork-api-{port}"))
    }
    #[cfg(unix)]
    {
        let _ = port;
        state.map(|s| s.join("api.sock").to_string_lossy().into_owned())
    }
}

/// Where an application looks for the machine's node: `$VORK_API`, else the
/// system node's endpoint, else the user's.
pub fn default_endpoint() -> String {
    if let Ok(e) = std::env::var("VORK_API") {
        return e;
    }
    #[cfg(windows)]
    {
        r"\\.\pipe\vork-api-51820".to_string()
    }
    #[cfg(unix)]
    {
        let system = "/var/lib/vork/api.sock";
        if std::path::Path::new(system).exists() {
            return system.to_string();
        }
        let state = std::env::var_os("XDG_STATE_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/state")))
            .unwrap_or_else(|| ".".into());
        state.join("vork/api.sock").to_string_lossy().into_owned()
    }
}

// ---- the control endpoint: status and settings, one JSON line each way --------------

/// The machine's node's control endpoint: `$VORK_CONTROL`, else the system
/// node's, else the user's (Unix); the named pipe for UDP port 51820 (Windows).
pub fn default_control_endpoint() -> String {
    if let Ok(e) = std::env::var("VORK_CONTROL") {
        return e;
    }
    #[cfg(windows)]
    {
        r"\\.\pipe\vork-51820".to_string()
    }
    #[cfg(unix)]
    {
        let system = "/var/lib/vork/control.sock";
        if std::path::Path::new(system).exists() {
            return system.to_string();
        }
        let state = std::env::var_os("XDG_STATE_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/state")))
            .unwrap_or_else(|| ".".into());
        state.join("vork/control.sock").to_string_lossy().into_owned()
    }
}

/// Sends one JSON request to the control endpoint and returns the answer
/// (blocking; for programs without an async runtime).
pub fn control_request(endpoint: &str, request: &str) -> io::Result<String> {
    use std::io::{BufRead, Write};
    #[cfg(unix)]
    let conn = {
        let c = std::os::unix::net::UnixStream::connect(endpoint)?;
        c.set_read_timeout(Some(Duration::from_secs(5)))?;
        c
    };
    #[cfg(windows)]
    let conn = std::fs::OpenOptions::new().read(true).write(true).open(endpoint)?;
    let mut w = conn.try_clone()?;
    w.write_all(format!("{request}\n").as_bytes())?;
    let mut line = String::new();
    std::io::BufReader::new(conn).read_line(&mut line)?;
    Ok(line)
}

// ---- the client ------------------------------------------------------------------

/// What arrives from the node unasked.
#[derive(Debug)]
pub enum Event {
    Heard { dest: Hash, known: Known },
    Incoming { token: u64, dest: Hash, session: u32, nonce: [u8; 32], requester: Option<Hash>, path: String, body: Vec<u8> },
    Closed { session: u32 },
    /// The dashboard asks for one of our pages (answer with `respond`).
    Page { token: u64, id: String, path: String, body: Vec<u8> },
    /// The connection to the node ended.
    Gone,
}

type Waiting = Arc<Mutex<HashMap<u32, oneshot::Sender<FromNode>>>>;

/// A connection to the node. Cheap to clone; all clones share it.
#[derive(Clone)]
pub struct Client {
    tx: mpsc::UnboundedSender<Vec<u8>>,
    waiting: Waiting,
    next: Arc<AtomicU32>,
    /// The node's own address.
    pub node: Hash,
}

fn err(m: impl Into<String>) -> io::Error {
    io::Error::other(m.into())
}

impl Client {
    /// Connects and greets the node; events arrive on the returned channel.
    pub async fn connect(endpoint: &str) -> io::Result<(Client, mpsc::UnboundedReceiver<Event>)> {
        #[cfg(unix)]
        let conn = tokio::net::UnixStream::connect(endpoint).await?;
        #[cfg(windows)]
        let conn = tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint)?;
        Self::over(conn).await
    }

    /// Over any byte stream to the node (tests, other transports).
    pub async fn over<S: AsyncRead + AsyncWrite + Send + 'static>(conn: S) -> io::Result<(Client, mpsc::UnboundedReceiver<Event>)> {
        let (mut r, mut w) = tokio::io::split(conn);
        write_frame(&mut w, &ToNode::Hello { version: VERSION }.encode()).await?;
        let node = match FromNode::decode(&read_frame(&mut r).await?)? {
            FromNode::Hello { address, .. } => address,
            other => return Err(err(format!("unexpected greeting {other:?}"))),
        };
        let (tx, mut out) = mpsc::unbounded_channel::<Vec<u8>>();
        tokio::spawn(async move {
            while let Some(f) = out.recv().await {
                if write_frame(&mut w, &f).await.is_err() {
                    return;
                }
            }
        });
        let waiting: Waiting = Arc::default();
        let (ev_tx, ev_rx) = mpsc::unbounded_channel();
        let w2 = waiting.clone();
        tokio::spawn(async move {
            loop {
                let frame = match read_frame(&mut r).await.and_then(|f| FromNode::decode(&f)) {
                    Ok(f) => f,
                    Err(_) => {
                        let _ = ev_tx.send(Event::Gone);
                        w2.lock().unwrap().clear();
                        return;
                    }
                };
                let req = match &frame {
                    FromNode::Registered { req, .. }
                    | FromNode::Known { req, .. }
                    | FromNode::Opened { req, .. }
                    | FromNode::Response { req, .. }
                    | FromNode::Error { req, .. } => Some(*req),
                    _ => None,
                };
                match (req, frame) {
                    (Some(req), f) => {
                        if let Some(tx) = w2.lock().unwrap().remove(&req) {
                            let _ = tx.send(f);
                        }
                    }
                    (None, FromNode::Heard { dest, known }) => {
                        let _ = ev_tx.send(Event::Heard { dest, known });
                    }
                    (None, FromNode::Incoming { token, dest, session, nonce, requester, path, body }) => {
                        let _ = ev_tx.send(Event::Incoming { token, dest, session, nonce, requester, path, body });
                    }
                    (None, FromNode::Closed { session }) => {
                        let _ = ev_tx.send(Event::Closed { session });
                    }
                    (None, FromNode::Page { token, id, path, body }) => {
                        let _ = ev_tx.send(Event::Page { token, id, path, body });
                    }
                    _ => {}
                }
            }
        });
        Ok((Client { tx, waiting, next: Arc::new(AtomicU32::new(1)), node }, ev_rx))
    }

    fn send(&self, f: ToNode) -> io::Result<()> {
        self.tx.send(f.encode()).map_err(|_| err("the node went away"))
    }

    async fn ask(&self, make: impl FnOnce(u32) -> ToNode) -> io::Result<FromNode> {
        let req = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.waiting.lock().unwrap().insert(req, tx);
        self.send(make(req))?;
        match rx.await {
            Ok(FromNode::Error { message, .. }) => Err(err(message)),
            Ok(f) => Ok(f),
            Err(_) => Err(err("the node went away")),
        }
    }

    /// Hosts destination `name` of the identity `secret`; returns its address.
    pub async fn register(&self, secret: [u8; 64], name: &str) -> io::Result<Hash> {
        match self.ask(|req| ToNode::Register { req, secret, name: name.to_string() }).await? {
            FromNode::Registered { dest, .. } => Ok(dest),
            f => Err(err(format!("unexpected {f:?}"))),
        }
    }

    pub fn announce(&self, dest: Hash, app_data: &[u8]) -> io::Result<()> {
        self.send(ToNode::Announce { dest, app_data: app_data.to_vec() })
    }

    pub fn subscribe(&self) -> io::Result<()> {
        self.send(ToNode::Subscribe)
    }

    /// Adds this application's pages to the node's dashboard (none: removes
    /// them). Requests for them arrive as `Event::Page`.
    pub fn pages(&self, id: &str, title: &str, pages: Vec<(String, String)>) -> io::Result<()> {
        self.send(ToNode::Pages { id: id.into(), title: title.into(), pages })
    }

    /// Tells the node the owner's other devices (destinations or identities).
    pub fn devices(&self, devices: Vec<Hash>) -> io::Result<()> {
        self.send(ToNode::Devices { devices })
    }

    pub async fn recall(&self, dest: Hash) -> io::Result<Option<Known>> {
        match self.ask(|req| ToNode::Recall { req, dest }).await? {
            FromNode::Known { known, .. } => Ok(known),
            f => Err(err(format!("unexpected {f:?}"))),
        }
    }

    /// Asks the network for a path to `dest` and waits up to `timeout`.
    pub async fn find(&self, dest: Hash, timeout: Duration) -> io::Result<Option<Known>> {
        let timeout_ms = timeout.as_millis().min(u32::MAX as u128) as u32;
        match self.ask(|req| ToNode::Find { req, dest, timeout_ms }).await? {
            FromNode::Known { known, .. } => Ok(known),
            f => Err(err(format!("unexpected {f:?}"))),
        }
    }

    /// Opens a session to `dest`: (session, nonce). We are anonymous in it until we identify.
    pub async fn open(&self, dest: Hash) -> io::Result<(u32, [u8; 32])> {
        match self.ask(|req| ToNode::Open { req, dest }).await? {
            FromNode::Opened { session, nonce, .. } => Ok((session, nonce)),
            f => Err(err(format!("unexpected {f:?}"))),
        }
    }

    pub fn identify(&self, session: u32, as_dest: Hash) -> io::Result<()> {
        self.send(ToNode::Identify { session, as_dest })
    }

    pub async fn request(&self, session: u32, path: &str, body: Vec<u8>) -> io::Result<(u8, Vec<u8>)> {
        match self.ask(|req| ToNode::Request { req, session, path: path.to_string(), body }).await? {
            FromNode::Response { status, body, .. } => Ok((status, body)),
            f => Err(err(format!("unexpected {f:?}"))),
        }
    }

    pub fn close(&self, session: u32) -> io::Result<()> {
        self.send(ToNode::Close { session })
    }

    pub fn respond(&self, token: u64, status: u8, body: Vec<u8>) -> io::Result<()> {
        self.send(ToNode::Respond { token, status, body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let k = Known { public: [1; 64], name_hash: [2; 10], hops: 3, app_data: vec![4, 5] };
        let to = vec![
            ToNode::Hello { version: 1 },
            ToNode::Register { req: 7, secret: [9; 64], name: "retifs.node".into() },
            ToNode::Announce { dest: [1; 16], app_data: vec![1, 2, 3] },
            ToNode::Recall { req: 1, dest: [2; 16] },
            ToNode::Find { req: 1, dest: [2; 16], timeout_ms: 500 },
            ToNode::Open { req: 1, dest: [3; 16] },
            ToNode::Identify { session: 4, as_dest: [5; 16] },
            ToNode::Request { req: 2, session: 4, path: "/vork/get".into(), body: vec![0; 100] },
            ToNode::Close { session: 4 },
            ToNode::Respond { token: 99, status: 0, body: vec![1] },
            ToNode::Subscribe,
            ToNode::Devices { devices: vec![[1; 16], [2; 16]] },
            ToNode::Devices { devices: vec![] },
            ToNode::Pages { id: "app".into(), title: "App".into(), pages: vec![("/".into(), "Overview".into()), ("/b".into(), "B".into())] },
            ToNode::Pages { id: "app".into(), title: "".into(), pages: vec![] },
        ];
        for f in to {
            assert_eq!(ToNode::decode(&f.encode()).unwrap(), f);
        }
        let from = vec![
            FromNode::Hello { version: 1, address: [1; 16] },
            FromNode::Registered { req: 1, dest: [2; 16] },
            FromNode::Known { req: 1, known: Some(k.clone()) },
            FromNode::Known { req: 1, known: None },
            FromNode::Opened { req: 1, session: 2, nonce: [3; 32] },
            FromNode::Response { req: 1, status: 0, body: vec![1, 2] },
            FromNode::Error { req: 1, message: "no".into() },
            FromNode::Heard { dest: [1; 16], known: k },
            FromNode::Incoming { token: 1, dest: [2; 16], session: 3, nonce: [4; 32], requester: Some([5; 16]), path: "/x".into(), body: vec![6] },
            FromNode::Incoming { token: 1, dest: [2; 16], session: 3, nonce: [4; 32], requester: None, path: "/x".into(), body: vec![] },
            FromNode::Closed { session: 3 },
            FromNode::Page { token: 1 << 63, id: "app".into(), path: "/".into(), body: b"{}".to_vec() },
        ];
        for f in from {
            assert_eq!(FromNode::decode(&f.encode()).unwrap(), f);
        }
    }
}

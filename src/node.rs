//! A node: one UDP socket, an outer WireGuard session per neighbor, an inner
//! (end-to-end) WireGuard session per remote peer, and a label switch.
//!
//! - A neighbor counts as up only while its handshake is fresh, so every link
//!   used for announces and paths is proven to work in both directions.
//! - Destinations are found by signed announces, flooded only over up links.
//!   A node keeps one route per neighbor it heard an announce from, so it
//!   knows several first hops toward a destination.
//! - A session runs over one or more paths. A path is opened by destination
//!   hash; every hop swaps the label for a fresh one on the next link, so no
//!   packet carries a source, only the opening message carries a destination,
//!   and colluding relays can't match packets by label. Relays hold no keys
//!   for the inner session and can't read it.
//! - The origin probes each path for round-trip time and loss, drops dead
//!   paths and opens new ones; packets are spread over the healthy paths.

use crate::identity::{self, Hash, Identity, LABEL_ANNOUNCE, LABEL_BIND, node_hash, short};
use crate::wire::{self, Announce, FLAG_RELAY, Msg, wg};
use boringtun::noise::handshake::parse_handshake_anon;
use boringtun::noise::{Packet, Tunn, TunnResult};
use boringtun::x25519::{PublicKey, StaticSecret};
use crate::stream::{self, Stream};
use crate::reach::{self, Reach};
use crate::state::{Remembered, State, unix_now};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::io::Write as _;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::net::UdpSocket;

/// Receive buffer (the largest UDP datagram).
const BUF: usize = 65536;
/// Output buffer for one WireGuard operation: our packets stay under the MTU.
const PKT_BUF: usize = 4096;
/// Bench payload per packet: with both WireGuard layers and headers this stays under a 1500-byte MTU.
const BENCH_PAYLOAD: usize = 1200;
const KEEPALIVE: u16 = 25;
/// A neighbor whose last handshake is older than this is down (WireGuard's REJECT_AFTER_TIME).
const LINK_FRESH: Duration = Duration::from_secs(180);
/// A live neighbor sends at least a keepalive every KEEPALIVE seconds; silence
/// longer than this means it is gone, long before its handshake expires.
const LINK_SILENT: Duration = Duration::from_secs(KEEPALIVE as u64 + 10);
/// After a path through a neighbor fails, wait this long (doubling) before trying it again.
const RETRY_MIN: Duration = Duration::from_secs(5);
const RETRY_MAX: Duration = Duration::from_secs(60);
const ANNOUNCE_EVERY: Duration = Duration::from_secs(30);
/// A route whose announce hasn't come again for this long is forgotten
/// (destinations announce every ANNOUNCE_EVERY): a node that left is gone
/// from the lists, and memory doesn't grow with everyone ever heard.
const ROUTE_TTL: Duration = Duration::from_secs(600);
/// Relay entries and far-end path state live this long without traffic (probes keep them alive).
const PATH_IDLE: Duration = Duration::from_secs(60);
const PROBE_EVERY: Duration = Duration::from_secs(1);
/// Unanswered probes in a row before a path counts as dead.
const PROBE_LOSS_DEAD: u32 = 3;
/// A responder uses the paths it heard from within this window. The origin
/// probes every healthy path each second, so a quiet path is a dead one.
const PATH_RECENT: Duration = Duration::from_secs(3);
const MAX_HOPS: u8 = 16;
// Session messages (inside WireGuard): datagrams 1–4, the stream 8–9.
const APP_SEG: u8 = 8;
const APP_ACK: u8 = 9;
/// Where link messages go (ADR-0024): a built-in path every node serves.
const LINK_PATH: &str = "/vork/link";
/// A link message not answered is sent again after this.
const LINK_AGAIN: Duration = Duration::from_secs(20);
/// A link code is good this long unless given otherwise.
const LINK_CODE_TTL: Duration = Duration::from_secs(600);

/// A link this node handed out (ADR-0024).
struct LinkCode {
    code: String,
    until: Instant,
    uses: Option<u32>,
    approve: bool,
}
/// An IPv6 packet between overlay addresses (M2), as a datagram.
const APP_IP: u8 = 5;
/// A direct-link offer: the node's link key, then its candidate addresses.
const APP_DIRECT: u8 = 6;
/// An IPv4 packet to or from the internet through an exit node (ADR-0026).
const APP_EXIT: u8 = 7;
/// "Who are you?": IP came in a session whose sender this node doesn't know
/// (it restarted, and the other end kept the session it had identified in).
const APP_WHO: u8 = 10;
// Messages on the stream.
const MSG_REQUEST: u8 = 1;
const MSG_RESPONSE: u8 = 2;
const MSG_IDENTIFY: u8 = 3;
pub const STATUS_OK: u8 = 0;
pub const STATUS_NO_PATH: u8 = 1;
pub const STATUS_REFUSED: u8 = 2;
const LABEL_IDENTIFY: &[u8] = b"vork/v0/identify";
const LABEL_SESSION: &[u8] = b"vork/v0/session";
/// A dial counts as cold, so its answer proves the neighbor reachable
/// (ADR-0023), when nothing came from it for this long: no NAT mapping of
/// its own toward us can still be open (UDP mappings last 30 s to 2 min).
const NAT_COLD: Duration = Duration::from_secs(150);
/// The handshake answer to a cold dial must come within this.
const COLD_ANSWER: Duration = Duration::from_secs(10);
/// Fewer relaying neighbors up than this, and the node dials reachable ones it learned of.
const MIN_UP: usize = 2;
const SEEK_EVERY: Duration = Duration::from_secs(30);
/// A learned node dialed and not up within this is let go.
const TRIAL: Duration = Duration::from_secs(20);
/// A learned node that didn't answer isn't dialed again for this long.
const TRIED_AGAIN: Duration = Duration::from_secs(600);
/// Neighbors are told what this node proved this often (and at link up).
const SHARE_EVERY: Duration = Duration::from_secs(600);
/// "What is your key?" `nonce[8]`, and the answer `nonce[8] x_pub[32] ed_pub[32]`.
/// Outside WireGuard (whose messages start with 1–4).
const KEY_ASK: &[u8] = b"VORK?";
const KEY_TELL: &[u8] = b"VORK!";

/// The machine's name: the default name of its node.
pub fn machine_name() -> String {
    #[cfg(unix)]
    {
        let mut b = [0u8; 256];
        if unsafe { libc::gethostname(b.as_mut_ptr().cast(), b.len()) } == 0 {
            let end = b.iter().position(|c| *c == 0).unwrap_or(b.len());
            let n = String::from_utf8_lossy(&b[..end]).trim().to_string();
            if !n.is_empty() && n != "localhost" {
                return n;
            }
        }
    }
    #[cfg(windows)]
    if let Ok(n) = std::env::var("COMPUTERNAME") {
        return n.to_lowercase();
    }
    "vork".to_string()
}

/// A node's name from its announce's app data (`{"name": …}`), at most 64 characters.
fn announced_name(app_data: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(app_data).ok()?;
    Some(v["name"].as_str()?.chars().take(64).collect())
}

/// A DNS server: an address, with port 53 unless given; link-local ones are
/// skipped (they need their interface).
fn parse_dns_server(s: &str) -> Option<SocketAddr> {
    let a = s.parse::<SocketAddr>().ok().or_else(|| s.trim_matches(|c| c == '[' || c == ']').parse::<std::net::IpAddr>().ok().map(|ip| SocketAddr::new(ip, 53)))?;
    match a.ip() {
        std::net::IpAddr::V6(v6) if v6.segments()[0] & 0xffc0 == 0xfe80 => None,
        _ => Some(a),
    }
}

/// `HOST:PORT` or `HOST:PORT@KEY`.
pub fn parse_peer_text(s: &str) -> Result<(SocketAddr, Option<[u8; 32]>), String> {
    use std::net::ToSocketAddrs;
    let (addr, key) = match s.split_once('@') {
        Some((a, k)) => (a, Some(k)),
        None => (s, None),
    };
    let addr = addr.trim().to_socket_addrs().map_err(|e| format!("{addr}: {e}"))?.next().ok_or(format!("{addr} doesn't resolve"))?;
    let key = match key {
        Some(k) => Some(hex::decode(k.trim()).ok().and_then(|b| <[u8; 32]>::try_from(b).ok()).ok_or("the key must be 64 hex digits")?),
        None => None,
    };
    Ok((addr, key))
}
const TTL: u8 = 32;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Spread {
    /// Weighted by round-trip time: more packets to faster paths.
    Spray,
    /// Every packet on every path: for loss and failover, at a bandwidth cost.
    Dup,
}

struct Neighbor {
    tunn: Tunn,
    x_pub: [u8; 32],
    addr: Option<SocketAddr>,
    hash: Option<Hash>,
    /// From its HELLO; `None` until then.
    flags: Option<u8>,
    /// The address this neighbor sees us at.
    observed_me: Option<SocketAddr>,
    /// When the last authenticated packet arrived.
    last_rx: Option<Instant>,
    /// Keep dialing while down, this often: bootstrap peers 5 s, remembered ones 60 s.
    retry: Option<Duration>,
    last_try: Instant,
    /// When we last sent this neighbor our HELLO.
    hello_sent: Option<Instant>,
    up: bool,
    /// A cold dial waiting for its answer: to where, when (ADR-0023).
    cold: Option<(SocketAddr, Instant)>,
    /// Dialed from what was learned of reachable nodes: let go if not up soon.
    trial: Option<Instant>,
}

/// A path as seen from one of its ends; keyed by (neighbor, label on that link).
struct PathEnd {
    /// We opened it (and probe it); otherwise we are its far end.
    origin: bool,
    /// At the far end: which of our destinations it was opened to.
    dest: Option<Hash>,
    sid: Option<u32>,
    acked: bool,
    srtt: Option<Duration>,
    probe: Option<(u64, Instant)>,
    lost: u32,
    last_seen: Instant,
    /// Packets received on this path (for the status line).
    rx: u64,
    /// Smooth weighted round robin state.
    credit: i64,
}

struct Session {
    tunn: Tunn,
    peer_x: [u8; 32],
    /// The destination we opened this session to (None: the far end opened it).
    peer_hash: Option<Hash>,
    /// The destination of ours the far end opened it to.
    local: Option<Hash>,
    /// Who the far end is: the destination's identity, or the opener once it identified.
    peer_identity: Option<Hash>,
    /// Known to both ends and to no other session: what proofs are bound to.
    nonce: [u8; 32],
    stream: Stream,
    /// A session between two destinations of this node: the other end's
    /// session, to which messages go directly, without WireGuard.
    looped: Option<u32>,
    /// Our requests waiting for an answer: id → (path, sent, the API client and its request id).
    waiting: HashMap<u32, (String, Instant, Option<(u32, u32)>)>,
    next_req: u32,
    paths: Vec<(u32, u64)>,
    /// First hops whose paths failed: (don't retry before, current backoff).
    failed: HashMap<u32, (Instant, Duration)>,
    /// Whether a direct link was considered since the far end's identity was last learned.
    direct_considered: bool,
    /// The far end's node link key, from its direct-link offer: a neighbor
    /// with it is the direct path (the session key may be a one-off, or an
    /// application's).
    direct_key: Option<[u8; 32]>,
}

struct Route {
    hops: u8,
    ann: Announce,
    /// When this neighbor last brought a newer announce.
    heard: Instant,
}

/// A destination this node answers for: of its own identity, or registered
/// by an application on this machine with that application's identity.
struct LocalDest {
    identity: Arc<Identity>,
    name: String,
    name_hash: [u8; identity::NAME_HASH_LEN],
    app_data: Vec<u8>,
    /// The API client it belongs to (None: the node's own).
    owner: Option<u32>,
}

/// An application connected to the local API.
struct ApiClient {
    tx: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    subscribed: bool,
    /// The owner's other devices it named (destinations or identities).
    devices: Vec<Hash>,
}

impl Session {
    fn new(tunn: Tunn, peer_x: [u8; 32], peer_hash: Option<Hash>, local: Option<Hash>, peer_identity: Option<Hash>, nonce: [u8; 32]) -> Self {
        Session {
            tunn,
            peer_x,
            peer_hash,
            local,
            peer_identity,
            nonce,
            stream: Stream::default(),
            looped: None,
            waiting: HashMap::new(),
            next_req: 0,
            paths: vec![],
            failed: HashMap::new(),
            direct_considered: false,
            direct_key: None,
        }
    }
}

/// SHA-256(label ‖ opener's session key ‖ destination's key): the opener's
/// key is fresh for every session, so no two sessions share it.
fn session_nonce(opener: &[u8; 32], dest: &[u8; 32]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(LABEL_SESSION);
    h.update(opener);
    h.update(dest);
    h.finalize().into()
}

/// A relay's entry: traffic from (neighbor, label) goes to (to, label).
struct Hop {
    to: u32,
    label: u64,
    used: Instant,
}

pub struct Config {
    pub listen: SocketAddr,
    pub peers: Vec<(SocketAddr, [u8; 32])>,
    /// Bootstrap neighbors known only by address: their key is asked for
    /// (trust on first use; a neighbor is never trusted for more than relaying).
    pub peers_by_address: Vec<SocketAddr>,
    pub announce: bool,
    pub ping: Option<Hash>,
    /// Lite: never relay, receive no floods, ask for paths (ESP32 and the like).
    pub lite: bool,
    /// Offer and accept direct links with the far end of a session. Reveals
    /// this node's addresses to that peer, so it's opt-in.
    pub direct: bool,
    /// Ask the router to open our port (UPnP, then PCP/NAT-PMP).
    pub map_port: bool,
    /// Find other nodes on the local network: mDNS (ADR-0023).
    pub lan: bool,
    /// The TUN interface to create (M2).
    pub tun: Option<String>,
    /// This node's name, in its announce.
    pub name: String,
    /// The `vork.toml` it was started with: changed by control commands.
    pub config_path: Option<std::path::PathBuf>,
    /// The web dashboard's address, if it's on.
    pub web: Option<SocketAddr>,
    /// Where names outside `.vork` are asked (when this node is the system's
    /// resolver, as on Android): the network's own servers.
    pub dns_upstream: Vec<SocketAddr>,
    /// Offers itself as an exit node to the owner's devices (ADR-0026).
    pub exit: bool,
    /// Its internet traffic goes out through this node.
    pub exit_via: Option<Hash>,
    /// The exit chosen by this node: the fastest of the owner's devices offering it.
    pub exit_auto: bool,
    /// The router for PCP/NAT-PMP; guessed as x.y.z.1 if not given.
    pub gateway: Option<std::net::Ipv4Addr>,
    /// How many paths (through different first hops) a session opens.
    pub paths: usize,
    pub spread: Spread,
    /// Named services of the node's identity, announced next to it.
    pub destinations: Vec<String>,
    /// The owner's other devices: direct links with them even without `direct`.
    pub devices: Vec<Hash>,
    /// Test: request this many bytes from a destination, again and again.
    pub fetch: Option<(Hash, usize)>,
    /// Test: identify in sessions we open.
    pub identify: bool,
    /// Test: drop this share of the session packets we send.
    pub drop: f64,
    /// Send bench traffic to this destination...
    pub bench: Option<Hash>,
    /// ...at this rate (Mbit/s).
    pub bench_rate: u64,
    /// Where the control endpoint listens (`None`: no control endpoint).
    pub control: Option<String>,
    /// The local API endpoint for applications (ADR-0020).
    pub api: Option<String>,
}

/// What a bench receiver has seen in the current second.
#[derive(Default)]
struct BenchRx {
    bytes: u64,
    packets: u64,
    first_seq: Option<u64>,
    max_seq: u64,
}

enum Out {
    Net(Vec<u8>),
    Payload(Vec<u8>),
    Err(String),
}

pub struct Node {
    me: Identity,
    my_hash: Hash,
    /// Destinations answered here besides the node's own address.
    local: HashMap<Hash, LocalDest>,
    me_arc: Arc<Identity>,
    clients: HashMap<u32, ApiClient>,
    /// `Find`s waiting for an announce: (client, request, destination, deadline).
    finds: Vec<(u32, u32, Hash, Instant)>,
    /// Neighbors we know by address only, asked for their key: nonce, when.
    asking: HashMap<SocketAddr, (u64, Instant)>,
    /// Receives through tokio; sends through the same socket's std handle,
    /// because tokio's `try_send_to` drops a datagram (WouldBlock) until the
    /// reactor has seen the socket writable, which loses the first handshake.
    sock: Arc<UdpSocket>,
    send_sock: std::net::UdpSocket,
    cfg: Config,
    neighbors: HashMap<u32, Neighbor>,
    neighbor_by_key: HashMap<[u8; 32], u32>,
    sessions: HashMap<u32, Session>,
    /// Sessions others opened to us, by the opener's (per-session) key.
    session_by_key: HashMap<[u8; 32], u32>,
    /// Sessions we opened, by destination.
    session_by_dest: HashMap<Hash, u32>,
    /// destination -> (first hop -> route through it)
    routes: HashMap<Hash, HashMap<u32, Route>>,
    switch: HashMap<(u32, u64), Hop>,
    ends: HashMap<(u32, u64), PathEnd>,
    seen: HashSet<(Hash, u64)>,
    pings: HashMap<u64, Instant>,
    relayed: u64,
    /// Neighbors waiting for an answer to a path request we passed on.
    pending: HashMap<Hash, (Vec<u32>, Instant)>,
    seen_req: HashSet<(Hash, u64)>,
    asked: HashMap<Hash, Instant>,
    /// Sessions in which our direct-link candidates were already sent.
    punched: HashSet<u32>,
    /// Our outside address from a router port mapping.
    mapped: Option<SocketAddr>,
    bench_seq: u64,
    bench_sent: u64,
    bench_rx: HashMap<u32, BenchRx>,
    state: State,
    started: Instant,
    /// Which nodes can be reached directly, learned from experience (ADR-0023).
    reach: Reach,
    /// Learned nodes dialed, by key: when.
    tried: HashMap<[u8; 32], Instant>,
    /// The TUN interface (M2).
    tun: Option<Arc<crate::tun::Tun>>,
    /// Sessions opened for IP in which we identified (the far end checks sources by identity).
    ip_identified: HashSet<u32>,
    /// Overlay destinations no node is known for: when last logged.
    ip_unknown: HashMap<[u8; 15], Instant>,
    /// The session IP last came in on from an identity: answers go back on it,
    /// rather than over a second session opened the other way.
    ip_peer: HashMap<Hash, u32>,
    /// Nodes found on the local network (mDNS), by WireGuard key: when.
    lan_seen: HashMap<[u8; 32], Instant>,
    /// Linking devices with both sides agreeing (ADR-0024).
    links: crate::links::Links,
    /// Link codes this node handed out (QR codes, links), as Resilio's: until
    /// when, how many uses are left (None: any), and whether the owner still
    /// approves each one.
    link_codes: Vec<LinkCode>,
    /// Outbox entries sent: when (sent again until answered).
    link_sent: HashMap<String, Instant>,
    /// Sessions opened for links in which we identified.
    link_identified: HashSet<u32>,
    /// Names asked lately (logged once a minute each).
    dns_seen: HashMap<(String, u16), Instant>,
    /// IPv4 aliases for nodes (ADR-0021), and the conversations programs
    /// here began over IPv4, whose answers go back translated.
    aliases: crate::alias::Aliases,
    v4_flows: HashMap<(Hash, crate::alias::Flow), Instant>,
    /// The devices using this node as their exit.
    exit_clients: crate::exit::Clients,
    /// Names asked through the exit, by the port they went from: who asked,
    /// when, and the network's servers to ask instead if no answer comes.
    exit_dns: HashMap<u16, (crate::dns::Query, Instant)>,
    /// Sessions asked who is on them, and when.
    who_asked: HashMap<u32, Instant>,
}

/// Socket buffers big enough to absorb bursts (the Linux default, ~200 KB,
/// holds only ~150 packets).
const SOCK_BUF: usize = 4 << 20;

fn bind_socket(addr: SocketAddr) -> anyhow::Result<std::net::UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let s = Socket::new(Domain::for_address(addr), Type::DGRAM, Some(Protocol::UDP))?;
    let _ = s.set_recv_buffer_size(SOCK_BUF);
    let _ = s.set_send_buffer_size(SOCK_BUF);
    #[cfg(target_os = "linux")]
    {
        // As root, go past net.core.rmem_max/wmem_max, like wireguard-go does.
        use std::os::fd::AsRawFd;
        let v = SOCK_BUF as libc::c_int;
        let len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        unsafe {
            let p = &v as *const libc::c_int as *const libc::c_void;
            libc::setsockopt(s.as_raw_fd(), libc::SOL_SOCKET, libc::SO_RCVBUFFORCE, p, len);
            libc::setsockopt(s.as_raw_fd(), libc::SOL_SOCKET, libc::SO_SNDBUFFORCE, p, len);
        }
    }
    s.set_nonblocking(true)?;
    s.bind(&addr.into())?;
    let rb = s.recv_buffer_size().unwrap_or(0);
    if rb < SOCK_BUF / 2 {
        log(format!("receive buffer is only {} KB (raise net.core.rmem_max, or run as root on Linux)", rb / 1024));
    }
    Ok(s.into())
}

fn rand_u64() -> u64 {
    let mut b = [0u8; 8];
    getrandom::getrandom(&mut b).expect("OS entropy");
    u64::from_le_bytes(b)
}

// ---- logging ---------------------------------------------------------------------

/// 0 warn, 1 info, 2 debug.
static LOG_LEVEL: AtomicU8 = AtomicU8::new(1);
static LOG_FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

/// Sets the level (`warn`, `info`, `debug`) and, optionally, a file to append
/// to instead of standard error (a Windows service has no console).
pub fn set_logging(level: &str, file: Option<&std::path::Path>) -> anyhow::Result<()> {
    let l = match level {
        "warn" => 0,
        "info" => 1,
        "debug" => 2,
        other => anyhow::bail!("log level must be warn, info or debug, not {other}"),
    };
    LOG_LEVEL.store(l, Ordering::Relaxed);
    if let Some(path) = file {
        let f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
        *LOG_FILE.lock().unwrap() = Some(f);
    }
    Ok(())
}

fn emit(level: u8, msg: &str) {
    if level > LOG_LEVEL.load(Ordering::Relaxed) {
        return;
    }
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let line = format!("[{}.{:03}] {}", t.as_secs() % 100_000, t.subsec_millis(), msg);
    match LOG_FILE.lock().unwrap().as_mut() {
        Some(f) => {
            let _ = writeln!(f, "{line}");
        }
        None => eprintln!("{line}"),
    }
}

pub fn log(msg: impl AsRef<str>) {
    emit(1, msg.as_ref());
}

fn debug(msg: impl AsRef<str>) {
    emit(2, msg.as_ref());
}

fn warn(msg: impl AsRef<str>) {
    emit(0, msg.as_ref());
}

fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1000.0)
}

/// Runs a datagram through a tunnel, draining queued packets (see boringtun's decapsulate).
fn decap(tunn: &mut Tunn, src: Option<std::net::IpAddr>, data: &[u8]) -> Vec<Out> {
    let mut buf = vec![0u8; PKT_BUF.max(data.len() + 64)];
    let mut outs = Vec::new();
    let mut first = true;
    loop {
        let r = if first { tunn.decapsulate(src, data, &mut buf) } else { tunn.decapsulate(None, &[], &mut buf) };
        first = false;
        match r {
            TunnResult::WriteToNetwork(p) => outs.push(Out::Net(p.to_vec())),
            TunnResult::WriteToTunnelV6(p, _) | TunnResult::WriteToTunnelV4(p, _) => {
                outs.push(Out::Payload(p.to_vec()));
                break;
            }
            TunnResult::Done => break,
            TunnResult::Err(e) => {
                outs.push(Out::Err(format!("{e:?}")));
                break;
            }
        }
    }
    outs
}

fn encap(tunn: &mut Tunn, payload: &[u8]) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; PKT_BUF.max(payload.len() + 64)];
    match tunn.encapsulate(payload, &mut buf) {
        TunnResult::WriteToNetwork(p) => Some(p.to_vec()),
        _ => None,
    }
}

fn timers(tunn: &mut Tunn) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; PKT_BUF];
    match tunn.update_timers(&mut buf) {
        TunnResult::WriteToNetwork(p) => Some(p.to_vec()),
        _ => None,
    }
}

impl Node {
    pub async fn new(me: Identity, cfg: Config, state: State) -> anyhow::Result<Node> {
        let aliases = crate::alias::Aliases::open(state.dir(), &me.hash());
        let std_sock = bind_socket(cfg.listen)?;
        let send_sock = std_sock.try_clone()?;
        let sock = Arc::new(UdpSocket::from_std(std_sock)?);
        let my_hash = me.hash();
        let links = crate::links::Links::open(state.dir());
        let mut node = Node {
            my_hash,
            local: HashMap::new(),
            me_arc: Arc::new(Identity::from_bytes(&me.to_bytes())),
            clients: HashMap::new(),
            finds: Vec::new(),
            asking: HashMap::new(),
            me,
            sock,
            send_sock,
            cfg,
            neighbors: HashMap::new(),
            neighbor_by_key: HashMap::new(),
            sessions: HashMap::new(),
            session_by_key: HashMap::new(),
            session_by_dest: HashMap::new(),
            routes: HashMap::new(),
            switch: HashMap::new(),
            ends: HashMap::new(),
            seen: HashSet::new(),
            pings: HashMap::new(),
            relayed: 0,
            pending: HashMap::new(),
            seen_req: HashSet::new(),
            asked: HashMap::new(),
            punched: HashSet::new(),
            mapped: None,
            bench_seq: 0,
            bench_sent: 0,
            bench_rx: HashMap::new(),
            reach: Reach::open(state.dir()),
            state,
            started: Instant::now(),
            tried: HashMap::new(),
            tun: None,
            ip_identified: HashSet::new(),
            ip_unknown: HashMap::new(),
            ip_peer: HashMap::new(),
            lan_seen: HashMap::new(),
            links,
            link_codes: Vec::new(),
            link_sent: HashMap::new(),
            link_identified: HashSet::new(),
            dns_seen: HashMap::new(),
            aliases,
            v4_flows: HashMap::new(),
            exit_clients: Default::default(),
            exit_dns: HashMap::new(),
            who_asked: HashMap::new(),
        };
        for name in node.cfg.destinations.clone() {
            let me = node.me_arc.clone();
            node.add_local(me, &name, None);
        }
        Ok(node)
    }

    /// A fresh 24-bit tunnel index, used by boringtun as the top bits of every
    /// session index (endpoints dispatch inner packets by it).
    fn new_id(&self) -> u32 {
        loop {
            let id = (rand_u64() as u32) & 0x00ff_ffff;
            if id != 0 && !self.neighbors.contains_key(&id) && !self.sessions.contains_key(&id) {
                return id;
            }
        }
    }

    fn session_tunn(secret: StaticSecret, peer: [u8; 32], id: u32) -> Tunn {
        Tunn::new(secret, PublicKey::from(peer), None, None, id, None)
    }

    fn new_tunn(&self, peer: [u8; 32], keepalive: Option<u16>, id: u32) -> Tunn {
        Tunn::new(self.me.dh.clone(), PublicKey::from(peer), None, keepalive, id, None)
    }

    fn add_neighbor(&mut self, x_pub: [u8; 32], addr: Option<SocketAddr>, retry: Option<Duration>) -> u32 {
        let id = self.new_id();
        let tunn = self.new_tunn(x_pub, Some(KEEPALIVE), id);
        self.neighbors.insert(
            id,
            Neighbor {
                tunn,
                x_pub,
                addr,
                hash: None,
                flags: None,
                observed_me: None,
                last_rx: None,
                retry,
                last_try: Instant::now(),
                hello_sent: None,
                up: false,
                cold: None,
                trial: None,
            },
        );
        self.neighbor_by_key.insert(x_pub, id);
        id
    }

    fn my_flags(&self) -> u8 {
        if self.cfg.lite { 0 } else { FLAG_RELAY }
    }

    /// A neighbor that said it doesn't relay.
    fn is_lite(&self, id: u32) -> bool {
        self.neighbors.get(&id).and_then(|n| n.flags).is_some_and(|f| f & FLAG_RELAY == 0)
    }

    fn is_up(&self, id: u32) -> bool {
        self.neighbors.get(&id).is_some_and(|n| n.up)
    }

    fn name(&self, id: u32) -> String {
        self.neighbors.get(&id).and_then(|n| n.hash).map(|h| short(&h)).unwrap_or_else(|| "?".into())
    }

    /// Runs until Ctrl-C or until `stop` changes (a Windows service stopping).
    pub async fn run(mut self, mut stop: Option<tokio::sync::watch::Receiver<bool>>) -> anyhow::Result<()> {
        log(format!(
            "node {} listening on {} (x25519 {}){}",
            hex::encode(self.my_hash),
            self.cfg.listen,
            hex::encode(self.me.x_pub().as_bytes()),
            if self.cfg.lite { " [lite]" } else { "" }
        ));
        for (addr, key) in self.cfg.peers.clone() {
            let id = self.add_neighbor(key, Some(addr), Some(Duration::from_secs(5)));
            self.initiate(id);
        }
        for addr in self.cfg.peers_by_address.clone() {
            self.ask_key(addr);
        }
        // Neighbors from earlier runs: reconnect without bootstrap peers.
        let remembered = self.state.neighbors();
        let mut dialed = 0;
        for r in remembered {
            let Ok(Ok(key)) = hex::decode(&r.key).map(<[u8; 32]>::try_from) else { continue };
            if self.neighbor_by_key.contains_key(&key) || key == *self.me.x_pub().as_bytes() {
                continue;
            }
            let id = self.add_neighbor(key, Some(r.addr), Some(Duration::from_secs(60)));
            self.initiate(id);
            dialed += 1;
        }
        if dialed > 0 {
            log(format!("dialing {dialed} remembered neighbors"));
        }
        let mut ctrl_rx = match &self.cfg.control {
            Some(ep) => match crate::control::listen(ep) {
                Ok(rx) => {
                    log(format!("control endpoint {ep}"));
                    Some(rx)
                }
                Err(e) => {
                    warn(format!("no control endpoint: {e:#}"));
                    None
                }
            },
            None => None,
        };
        let mut api_rx = match &self.cfg.api {
            Some(ep) => match crate::api::listen(ep) {
                Ok(rx) => {
                    log(format!("API endpoint {ep}"));
                    Some(rx)
                }
                Err(e) => {
                    warn(format!("no API endpoint: {e:#}"));
                    None
                }
            },
            None => None,
        };
        // Local network discovery (mDNS): this node registered, the others browsed for.
        let (lan_tx, mut lan_rx) = tokio::sync::mpsc::channel(64);
        let mdns = if self.cfg.lan {
            match crate::lan::start(&hex::encode(&self.my_hash[..8]), self.me.x_pub().as_bytes(), self.cfg.listen.port(), lan_tx) {
                Ok(d) => {
                    log(format!("looking for nodes on the local network (mDNS {})", crate::lan::SERVICE));
                    Some(d)
                }
                Err(e) => {
                    warn(format!("no local network discovery: {e:#}"));
                    None
                }
            }
        } else {
            drop(lan_tx);
            None
        };
        // An interface handed over by an app (Android's VpnService): `tun = "fd"`.
        let (tun_tx, mut tun_rx) = tokio::sync::mpsc::channel::<Arc<crate::tun::Tun>>(1);
        #[cfg(any(target_os = "linux", target_os = "android"))]
        if self.cfg.tun.as_deref() == Some("fd") {
            if let Some(dir) = self.state.dir() {
                let path = dir.join("tun.sock");
                log(format!("waiting for an interface at {}", path.display()));
                let rt = tokio::runtime::Handle::current();
                std::thread::spawn(move || loop {
                    let _in_runtime = rt.enter();
                    match crate::tun::receive_fd_at(&path).and_then(|fd| crate::tun::Tun::from_fd(fd, "vpn")) {
                        Ok(t) => {
                            if tun_tx.blocking_send(Arc::new(t)).is_err() {
                                return;
                            }
                        }
                        Err(e) => {
                            eprintln!("interface handover: {e:#}");
                            std::thread::sleep(Duration::from_secs(5));
                        }
                    }
                });
            }
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        drop(tun_tx);
        if let Some(name) = self.cfg.tun.clone().filter(|t| t != "fd") {
            let addr = crate::tun::overlay(&self.my_hash);
            match crate::tun::Tun::open(&name, addr, self.aliases.me, crate::tun::MTU) {
                Ok(t) => {
                    log(format!("interface {} up: this node is {addr}", t.name));
                    crate::dns::register(&t.name);
                    if self.cfg.exit {
                        match crate::exit::setup(&t.name) {
                            Ok(()) => log(format!("exit node: the owner's devices may send their internet traffic out through here ({}/{})", crate::exit::NET, crate::exit::PREFIX)),
                            Err(e) => warn(format!("exit node off: {e}")),
                        }
                    }
                    self.tun = Some(Arc::new(t));
                }
                Err(e) => warn(format!("no interface: {e:#}")),
            }
        }
        let mut tun = self.tun.clone();
        // The web dashboard: its requests are control requests from the owner.
        let (web_tx, mut web_rx) = tokio::sync::mpsc::channel::<crate::control::Request>(16);
        if let Some(addr) = self.cfg.web {
            let token = crate::web::token(self.state.dir());
            match crate::web::serve(addr, token.clone(), web_tx).await {
                Ok(()) => {
                    let host = if addr.ip().is_unspecified() { machine_name() } else { addr.ip().to_string() };
                    log(format!("dashboard at http://{host}:{}/#t={token}", addr.port()));
                }
                Err(e) => warn(format!("no dashboard on {addr}: {e:#}")),
            }
        } else {
            drop(web_tx);
        }
        let mut tun_buf = vec![0u8; 65536];
        let mut last_remember = Instant::now();
        let mut last_seek = Instant::now();
        let mut sought = false;
        let mut last_share = Instant::now();
        let mut last_links = Instant::now();
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let mut mapped_rx = None;
        let mut mapper = None;
        if self.cfg.map_port {
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            mapped_rx = Some(rx);
            mapper = Some(tokio::spawn(crate::portmap::run(self.cfg.listen, self.cfg.gateway, tx, stop_rx)));
        }
        let mut buf = vec![0u8; BUF];
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        let mut last_announce = Instant::now() - ANNOUNCE_EVERY;
        let mut last_ping = Instant::now();
        let mut last_probe = Instant::now();
        let mut last_status = Instant::now();
        let mut last_retry = Instant::now();
        let mut stream_tick = tokio::time::interval(Duration::from_millis(10));
        stream_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut bench_tick = tokio::time::interval(Duration::from_millis(1));
        bench_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut last_bench_report = Instant::now();
        let mut bench_due = 0.0f64;
        let mut bench_last = Instant::now();
        #[cfg(unix)]
        let mut sig_int = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        #[cfg(unix)]
        let mut sig_term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        #[cfg(windows)]
        let mut sig_int = tokio::signal::windows::ctrl_c()?;
        loop {
            // The interface was let go (interface_off): its descriptor closes here.
            if self.tun.is_none() && tun.is_some() {
                tun = None;
                log("interface let go");
            }
            tokio::select! {
                _ = bench_tick.tick(), if self.cfg.bench.is_some() => {
                    // Pace by elapsed time, so a late tick sends what it owes.
                    let bytes_per_s = self.cfg.bench_rate as f64 * 1_000_000.0 / 8.0;
                    bench_due += bench_last.elapsed().as_secs_f64() * bytes_per_s / BENCH_PAYLOAD as f64;
                    bench_last = Instant::now();
                    // Windows timers tick every ~15 ms, so a tick may owe hundreds of packets.
                    bench_due = bench_due.min(4096.0);
                    while bench_due >= 1.0 {
                        bench_due -= 1.0;
                        self.bench_send();
                    }
                }
                _ = stream_tick.tick() => {
                    let busy: Vec<u32> = self.sessions.iter().filter(|(_, s)| !s.stream.idle()).map(|(id, _)| *id).collect();
                    for sid in busy {
                        self.flush(sid);
                    }
                    if self.cfg.fetch.is_some() {
                        self.fetch();
                    }
                }
                r = self.sock.recv_from(&mut buf) => {
                    match r {
                        Ok((n, addr)) => {
                            let data = buf[..n].to_vec();
                            self.on_datagram(addr, &data);
                        }
                        // e.g. Windows reports an ICMP "port unreachable" from an earlier send this way
                        Err(e) => debug(format!("receive: {e}")),
                    }
                }
                r = async { match tun.as_ref() { Some(t) => t.recv(&mut tun_buf).await, None => std::future::pending().await } } => {
                    match r {
                        Ok(n) => {
                            let p = tun_buf[..n].to_vec();
                            self.on_tun_packet(&p);
                        }
                        Err(e) => debug(format!("interface: {e}")),
                    }
                }
                Some(t) = tun_rx.recv() => {
                    log(format!("interface handed over: this node is {}", crate::tun::overlay(&self.my_hash)));
                    tun = Some(t.clone());
                    self.tun = Some(t);
                }
                Some(found) = lan_rx.recv(), if mdns.is_some() => {
                    self.on_lan_found(found);
                }
                Some(ext) = async { match mapped_rx.as_mut() { Some(rx) => rx.recv().await, None => std::future::pending().await } } => {
                    self.mapped = Some(ext);
                }
                Some(ev) = async { match api_rx.as_mut() { Some(rx) => rx.recv().await, None => std::future::pending().await } } => {
                    self.on_api(ev);
                }
                Some((req, owner, reply)) = web_rx.recv() => {
                    let _ = reply.send(self.on_control(&req, owner));
                }
                Some((req, owner, reply)) = async { match ctrl_rx.as_mut() { Some(rx) => rx.recv().await, None => std::future::pending().await } } => {
                    let _ = reply.send(self.on_control(&req, owner));
                }
                _ = async { match stop.as_mut() { Some(s) => { let _ = s.changed().await; } None => std::future::pending().await } } => {
                    let _ = stop_tx.send(true);
                    if let Some(m) = mapper {
                        let _ = tokio::time::timeout(Duration::from_secs(3), m).await;
                    }
                    self.remember_neighbors();
                    self.reach.save();
                    if let Some(m) = &mdns {
                        let _ = m.shutdown();
                    }
                    log("stopped");
                    return Ok(());
                }
                // Listeners made once, before the loop: a fresh one each turn
                // missed a signal that came while a packet was being handled
                // (a busy node then ignored systemd's stop).
                _ = async {
                    #[cfg(unix)]
                    tokio::select! { _ = sig_int.recv() => {} _ = sig_term.recv() => {} }
                    #[cfg(windows)]
                    sig_int.recv().await;
                    #[cfg(not(any(unix, windows)))]
                    std::future::pending::<()>().await;
                } => {
                    self.remember_neighbors();
                    self.reach.save();
                    let _ = stop_tx.send(true);
                    if let Some(m) = mapper {
                        let _ = tokio::time::timeout(Duration::from_secs(3), m).await;
                    }
                    log("stopped");
                    return Ok(());
                }
                _ = tick.tick() => {
                    self.on_timers();
                    self.expire_finds();
                    self.ask_keys_again();
                    if self.cfg.announce && last_announce.elapsed() >= ANNOUNCE_EVERY {
                        last_announce = Instant::now();
                        self.announce_self();
                    }
                    if last_probe.elapsed() >= PROBE_EVERY {
                        last_probe = Instant::now();
                        self.probe_paths();
                        self.maintain_sessions();
                    }
                    if self.cfg.ping.is_some() && last_ping.elapsed() >= Duration::from_secs(1) {
                        last_ping = Instant::now();
                        self.ping();
                    }
                    if last_retry.elapsed() >= Duration::from_secs(1) {
                        last_retry = Instant::now();
                        let due: Vec<u32> = self.neighbors.iter()
                            .filter(|(_, n)| !n.up && n.retry.is_some_and(|r| n.last_try.elapsed() >= r))
                            .map(|(id, _)| *id).collect();
                        for id in due { self.initiate(id); }
                    }
                    if last_remember.elapsed() >= Duration::from_secs(60) {
                        last_remember = Instant::now();
                        self.remember_neighbors();
                        self.reach.prune(unix_now());
                        self.reach.save();
                        self.forget_old_routes();
                    }
                    // A few seconds for configured and remembered neighbors first.
                    if last_seek.elapsed() >= SEEK_EVERY || (!sought && self.started.elapsed() >= Duration::from_secs(8)) {
                        last_seek = Instant::now();
                        sought = true;
                        self.seek_neighbors();
                    }
                    if last_links.elapsed() >= Duration::from_secs(5) {
                        last_links = Instant::now();
                        self.send_links();
                        // A session to the exit node kept open, so whether it works is known before traffic needs it.
                        // (One this node opened: its paths are probed, so their state is known.)
                        // Choosing automatically: sessions to every candidate, measured the same way.
                        let watch: Vec<Hash> = if self.cfg.exit_auto { self.exit_candidates() } else { self.cfg.exit_via.into_iter().collect() };
                        for via in watch {
                            if let Some(sid) = self.session_to(via) {
                                if self.ip_identified.insert(sid) {
                                    let me = self.me_arc.clone();
                                    self.identify(sid, &me);
                                }
                            }
                        }
                        if self.cfg.exit_auto {
                            self.choose_exit();
                        }
                    }
                    self.exit_dns_late();
                    if last_share.elapsed() >= SHARE_EVERY {
                        last_share = Instant::now();
                        for id in self.up_neighbors() {
                            self.share_reach(id);
                        }
                    }
                    if last_bench_report.elapsed() >= Duration::from_secs(1) {
                        let secs = last_bench_report.elapsed().as_secs_f64();
                        last_bench_report = Instant::now();
                        self.bench_report(secs);
                    }
                    if last_status.elapsed() >= Duration::from_secs(10) {
                        last_status = Instant::now();
                        self.status();
                    }
                }
            }
        }
    }

    // ---- outer layer: neighbors ------------------------------------------------

    fn send_raw(&self, addr: SocketAddr, p: &[u8]) {
        if let Err(e) = self.send_sock.send_to(p, addr) {
            // Shown (at most every 10 s): a node that can't send is otherwise silent.
            static LAST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
            if now >= LAST.load(std::sync::atomic::Ordering::Relaxed) + 10 {
                LAST.store(now, std::sync::atomic::Ordering::Relaxed);
                warn(format!("can't send to {addr}: {e}"));
            } else {
                debug(format!("send to {addr}: {e}"));
            }
        }
    }

    fn initiate(&mut self, id: u32) {
        let Some(n) = self.neighbors.get_mut(&id) else { return };
        n.last_try = Instant::now();
        // Cold: its answer will show it can be reached from here unasked.
        if !n.up && n.last_rx.is_none_or(|t| t.elapsed() >= NAT_COLD) {
            n.cold = n.addr.map(|a| (a, Instant::now()));
        }
        let mut buf = vec![0u8; PKT_BUF];
        if let TunnResult::WriteToNetwork(p) = n.tunn.format_handshake_initiation(&mut buf, false) {
            let p = p.to_vec();
            if let Some(addr) = n.addr {
                self.send_raw(addr, &p);
            }
        }
    }

    fn send_msg(&mut self, id: u32, msg: &Msg) {
        let framed = wire::frame(&msg.encode());
        let Some(n) = self.neighbors.get_mut(&id) else { return };
        let Some(addr) = n.addr else { return };
        if let Some(p) = encap(&mut n.tunn, &framed) {
            self.send_raw(addr, &p);
        }
    }

    fn up_neighbors(&self) -> Vec<u32> {
        self.neighbors.iter().filter(|(_, n)| n.up).map(|(id, _)| *id).collect()
    }

    fn on_datagram(&mut self, addr: SocketAddr, data: &[u8]) {
        if data.starts_with(KEY_ASK) || data.starts_with(KEY_TELL) {
            return self.on_key_query(addr, data);
        }
        let answer = wg::kind(data) == Some(wg::RESP);
        let id = match wg::kind(data) {
            Some(wg::INIT) => {
                let Ok(Packet::HandshakeInit(init)) = Tunn::parse_incoming_packet(data) else { return };
                let Ok(half) = parse_handshake_anon(&self.me.dh, &self.me.x_pub(), &init) else { return };
                // An open network: anyone may become a neighbor. Being a
                // neighbor grants nothing but the right to exchange messages.
                match self.neighbor_by_key.get(&half.peer_static_public) {
                    Some(id) => *id,
                    None => {
                        log(format!("new neighbor at {addr}"));
                        self.add_neighbor(half.peer_static_public, Some(addr), None)
                    }
                }
            }
            Some(_) => {
                let Some(idx) = wg::receiver_idx(data) else { return };
                if !self.neighbors.contains_key(&(idx >> 8)) {
                    return;
                }
                idx >> 8
            }
            None => return,
        };
        let outs = {
            let n = self.neighbors.get_mut(&id).unwrap();
            decap(&mut n.tunn, Some(addr.ip()), data)
        };
        let mut authenticated = false;
        for out in outs {
            match out {
                Out::Net(p) => {
                    authenticated = true;
                    self.send_raw(addr, &p);
                }
                Out::Payload(p) => {
                    authenticated = true;
                    if let Some(body) = wire::unframe(&p).map(|b| b.to_vec()) {
                        self.on_neighbor_payload(id, &body);
                    }
                }
                Out::Err(e) => log(format!("neighbor {addr}: {e}")),
            }
        }
        // Roaming: the endpoint is wherever the last authenticated packet came from.
        if authenticated {
            let n = self.neighbors.get_mut(&id).unwrap();
            n.addr = Some(addr);
            n.last_rx = Some(Instant::now());
            // The answer to a cold dial, from where we dialed: reachable from here.
            if answer && n.cold.take().is_some_and(|(at, t)| at == addr && t.elapsed() < COLD_ANSWER) {
                let key = n.x_pub;
                debug(format!("reached {} directly at {addr}", hex::encode(&key[..4])));
                self.reach.proved(key, addr, unix_now());
            }
        }
        self.check_up(id);
    }

    fn check_up(&mut self, id: u32) {
        let Some(n) = self.neighbors.get_mut(&id) else { return };
        let fresh = matches!(n.tunn.time_since_last_handshake(), Some(t) if t < LINK_FRESH)
            && n.last_rx.is_some_and(|t| t.elapsed() < LINK_SILENT);
        if fresh && !n.up {
            n.up = true;
            let addr = n.addr;
            log(format!("link up: {}", addr.map(|a| a.to_string()).unwrap_or_default()));
            self.on_link_up(id, addr);
            self.remember_neighbors();
        } else if !fresh && n.up {
            n.up = false;
            log(format!("link down: {}", self.name(id)));
            self.on_link_down(id);
        }
    }

    fn on_link_up(&mut self, id: u32, addr: Option<SocketAddr>) {
        let flags = self.my_flags();
        let mut bound = self.me.x_pub().as_bytes().to_vec();
        bound.push(flags);
        let hello = Msg::Hello { ed_pub: self.me.ed_pub(), flags, sig: self.me.sign(LABEL_BIND, &bound) };
        self.send_msg(id, &hello);
        if let Some(n) = self.neighbors.get_mut(&id) {
            n.hello_sent = Some(Instant::now());
        }
        if let Some(addr) = addr {
            self.send_msg(id, &Msg::Observed { addr });
        }
        if let Some(n) = self.neighbors.get_mut(&id) {
            n.trial = None;
        }
        self.share_reach(id);
        if self.cfg.announce {
            for own in self.own_announces() {
                self.send_msg(id, &Msg::Announce(own));
            }
        }
    }

    /// Paths through a lost neighbor are broken: tell the other side of every
    /// relay entry, and drop our own path ends on it.
    fn on_link_down(&mut self, id: u32) {
        let broken: Vec<(u32, u64)> = self.switch.iter().filter(|(_, h)| h.to == id).map(|(k, _)| *k).collect();
        for (from, label) in broken {
            if let Some(h) = self.switch.remove(&(from, label)) {
                self.switch.remove(&(h.to, h.label));
            }
            self.send_msg(from, &Msg::PathErr { label });
        }
        let mine: Vec<(u32, u64)> = self.ends.keys().filter(|(n, _)| *n == id).copied().collect();
        for k in mine {
            self.drop_path(k, "its first hop went down");
        }
    }

    fn on_neighbor_payload(&mut self, from: u32, body: &[u8]) {
        let msg = match Msg::decode(body) {
            Ok(m) => m,
            Err(e) => return log(format!("bad message: {e}")),
        };
        match msg {
            Msg::Hello { ed_pub, flags, sig } => self.on_hello(from, ed_pub, flags, sig),
            Msg::Observed { addr } => {
                let n = self.neighbors.get_mut(&from).unwrap();
                if n.observed_me != Some(addr) {
                    n.observed_me = Some(addr);
                    log(format!("a neighbor sees us at {addr}"));
                }
            }
            Msg::Announce(a) => self.on_announce(from, a),
            Msg::PathReq { dest, nonce } => self.on_path_req(from, dest, nonce),
            Msg::PathOpen { label, dest, ttl } => self.on_path_open(from, label, dest, ttl),
            Msg::PathAck { label } => self.on_path_ack(from, label),
            Msg::PathErr { label } => self.on_path_err(from, label),
            Msg::Data { label, inner } => self.on_data(from, label, inner),
            Msg::Probe { label, nonce, reply } => self.on_probe(from, label, nonce, reply),
            Msg::Reach { entries } => self.on_reach(from, entries),
        }
    }

    fn on_hello(&mut self, from: u32, ed_pub: [u8; 32], flags: u8, sig: [u8; 64]) {
        let n = self.neighbors.get_mut(&from).unwrap();
        let mut bound = n.x_pub.to_vec();
        bound.push(flags);
        if !identity::verify(&ed_pub, LABEL_BIND, &bound, &sig) {
            return log("neighbor hello with a bad signature");
        }
        let h = node_hash(&ed_pub, &n.x_pub);
        let first = n.hash.is_none();
        // A HELLO we didn't just answer means the neighbor (re)started its side
        // of a link that was already up here: greet it again.
        let restarted = !first && n.hello_sent.is_none_or(|t| t.elapsed() > Duration::from_secs(1));
        n.hash = Some(h);
        n.flags = Some(flags);
        let key = n.x_pub;
        self.reach.set_relay(&key, flags & FLAG_RELAY != 0);
        let n = self.neighbors.get_mut(&from).unwrap();
        let addr = n.addr;
        if restarted {
            debug(format!("neighbor {} said hello again; answering", short(&h)));
            self.on_link_up(from, addr);
        } else if !first {
            return;
        }
        let lite = flags & FLAG_RELAY == 0;
        if first {
            log(format!("neighbor is {}{}", hex::encode(h), if lite { " [lite]" } else { "" }));
        }
        // Hand a full neighbor what we know, so it can reach destinations at
        // once. Lite nodes ask instead.
        if !self.cfg.lite && !lite {
            let known: Vec<Announce> = self.routes.keys().copied().filter_map(|d| self.best_announce(&d, Some(from))).collect();
            for a in known {
                self.send_msg(from, &Msg::Announce(a));
            }
        }
    }

    // ---- announces and routes ----------------------------------------------------

    /// A signed announce of `identity`'s destination `name_hash`.
    fn sign_announce(&mut self, identity: &Identity, name_hash: [u8; identity::NAME_HASH_LEN], app_data: Vec<u8>) -> Announce {
        let x_pub = *identity.x_pub().as_bytes();
        let seq = self.state.next_seq();
        let flags = self.my_flags();
        let mut a = Announce { ed_pub: identity.ed_pub(), x_pub, name_hash, seq, flags, app_data, sig: [0; 64], hops: 0 };
        a.sig = identity.sign(LABEL_ANNOUNCE, &a.signed());
        a
    }

    fn own_announce(&mut self) -> Announce {
        let me = self.me_arc.clone();
        let data = if self.cfg.exit { json!({ "name": self.cfg.name, "exit": true }) } else { json!({ "name": self.cfg.name }) }.to_string().into_bytes();
        self.sign_announce(&me, wire::NO_NAME, data)
    }

    /// The announce of a destination answered here.
    fn local_announce(&mut self, dest: &Hash) -> Option<Announce> {
        if *dest == self.my_hash {
            return Some(self.own_announce());
        }
        let d = self.local.get(dest)?;
        let (id, nh, data) = (d.identity.clone(), d.name_hash, d.app_data.clone());
        Some(self.sign_announce(&id, nh, data))
    }

    /// Announces of every local destination, the node's identity first.
    fn own_announces(&mut self) -> Vec<Announce> {
        let mut v = vec![self.own_announce()];
        let dests: Vec<Hash> = self.local.keys().copied().collect();
        for d in dests {
            v.extend(self.local_announce(&d));
        }
        v
    }

    fn local_dest(&self, dest: &Hash) -> Option<()> {
        (*dest == self.my_hash || self.local.contains_key(dest)).then_some(())
    }

    /// The identity that answers for a local destination.
    fn local_identity(&self, dest: Option<&Hash>) -> Arc<Identity> {
        dest.and_then(|d| self.local.get(d)).map(|d| d.identity.clone()).unwrap_or_else(|| self.me_arc.clone())
    }

    fn add_local(&mut self, identity: Arc<Identity>, name: &str, owner: Option<u32>) -> Hash {
        let dest = identity::destination_hash(&identity.hash(), name);
        self.local.insert(dest, LocalDest { identity, name: name.to_string(), name_hash: identity::name_hash(name), app_data: Vec::new(), owner });
        dest
    }

    fn announce_one(&mut self, dest: &Hash) {
        if let Some(a) = self.local_announce(dest) {
            for id in self.up_neighbors() {
                self.send_msg(id, &Msg::Announce(a.clone()));
            }
        }
    }

    fn announce_self(&mut self) {
        for a in self.own_announces() {
            for id in self.up_neighbors() {
                self.send_msg(id, &Msg::Announce(a.clone()));
            }
        }
    }

    fn on_announce(&mut self, from: u32, a: Announce) {
        if !self.is_up(from) {
            return; // only over links proven to work both ways
        }
        let hash = a.dest();
        if self.local_dest(&hash).is_some() || a.hops >= MAX_HOPS {
            return;
        }
        if !identity::verify(&a.ed_pub, LABEL_ANNOUNCE, &a.signed(), &a.sig) {
            return log("dropped an announce with a bad signature");
        }
        let hops = a.hops.saturating_add(1);
        let known = self.routes.contains_key(&hash);
        let by_via = self.routes.entry(hash).or_default();
        let newer = by_via.get(&from).is_none_or(|r| a.seq > r.ann.seq || (a.seq == r.ann.seq && hops < r.hops));
        if newer {
            by_via.insert(from, Route { hops, ann: a.clone(), heard: Instant::now() });
        }
        if newer {
            self.heard(hash);
        }
        if !known {
            log(format!("learned {} at {hops} hops", hex::encode(hash)));
        }
        let fwd = Announce { hops, ..a };
        // Answer neighbors that asked for this destination.
        if let Some((askers, _)) = self.pending.remove(&hash) {
            for id in askers {
                if id != from {
                    self.send_msg(id, &Msg::Announce(fwd.clone()));
                }
            }
        }
        // Lite nodes don't flood, and nobody floods to them.
        if !self.cfg.lite && self.seen.insert((hash, fwd.seq)) {
            for id in self.up_neighbors() {
                if id != from && !self.is_lite(id) {
                    self.send_msg(id, &Msg::Announce(fwd.clone()));
                }
            }
        }
    }

    /// Usable first hops toward `dest`, best first: up neighbors, never a Lite
    /// node unless it is the destination itself. The hop count is advertised,
    /// not proven; paths are judged by their measured round trip once open.
    fn first_hops(&self, dest: &Hash) -> Vec<u32> {
        let Some(by_via) = self.routes.get(dest) else { return vec![] };
        let mut v: Vec<(u8, u32)> = by_via
            .iter()
            .filter(|(via, _)| {
                let n = &self.neighbors[*via];
                n.up && (!self.is_lite(**via) || n.hash == Some(*dest) || n.hash == self.dest_identity(dest))
            })
            .map(|(via, r)| (r.hops, *via))
            .collect();
        v.sort();
        v.into_iter().map(|(_, via)| via).collect()
    }

    /// The newest announce for `dest`, with the smallest hop count, not learned from `except`.
    fn best_announce(&self, dest: &Hash, except: Option<u32>) -> Option<Announce> {
        self.routes
            .get(dest)?
            .iter()
            .filter(|(via, _)| Some(**via) != except && self.is_up(**via))
            .max_by_key(|(_, r)| (r.ann.seq, std::cmp::Reverse(r.hops)))
            .map(|(_, r)| Announce { hops: r.hops, ..r.ann.clone() })
    }

    fn request_path(&mut self, dest: Hash) {
        if self.asked.get(&dest).is_some_and(|t| t.elapsed() < Duration::from_secs(2)) {
            return;
        }
        self.asked.insert(dest, Instant::now());
        let nonce = rand_u64();
        self.seen_req.insert((dest, nonce));
        for id in self.up_neighbors() {
            if !self.is_lite(id) {
                self.send_msg(id, &Msg::PathReq { dest, nonce });
            }
        }
    }

    fn on_path_req(&mut self, from: u32, dest: Hash, nonce: u64) {
        if !self.is_up(from) || !self.seen_req.insert((dest, nonce)) {
            return;
        }
        if self.local_dest(&dest).is_some() {
            if self.cfg.announce || dest != self.my_hash {
                if let Some(a) = self.local_announce(&dest) {
                    self.send_msg(from, &Msg::Announce(a));
                }
            }
            return;
        }
        if let Some(a) = self.best_announce(&dest, Some(from)) {
            self.send_msg(from, &Msg::Announce(a));
            return;
        }
        if self.cfg.lite {
            return;
        }
        // Not known here: remember who asked and pass the question on.
        let entry = self.pending.entry(dest).or_insert_with(|| (Vec::new(), Instant::now()));
        if !entry.0.contains(&from) {
            entry.0.push(from);
        }
        for id in self.up_neighbors() {
            if id != from && !self.is_lite(id) {
                self.send_msg(id, &Msg::PathReq { dest, nonce });
            }
        }
    }

    // ---- paths: opening and switching (relay role) -------------------------------

    fn on_path_open(&mut self, from: u32, label: u64, dest: Hash, ttl: u8) {
        if !self.is_up(from) {
            return;
        }
        if self.local_dest(&dest).is_some() {
            // We are the far end. The session is known once traffic arrives.
            self.ends.insert(
                (from, label),
                PathEnd { origin: false, dest: Some(dest), sid: None, acked: true, srtt: None, probe: None, lost: 0, last_seen: Instant::now(), rx: 0, credit: 0 },
            );
            self.send_msg(from, &Msg::PathAck { label });
            return;
        }
        if self.cfg.lite || ttl == 0 {
            return self.send_msg(from, &Msg::PathErr { label }); // Lite nodes don't relay
        }
        let Some(via) = self.first_hops(&dest).into_iter().find(|v| *v != from) else {
            log(format!("no route to {}", short(&dest)));
            return self.send_msg(from, &Msg::PathErr { label });
        };
        let out = rand_u64();
        let now = Instant::now();
        self.switch.insert((from, label), Hop { to: via, label: out, used: now });
        self.switch.insert((via, out), Hop { to: from, label, used: now });
        self.send_msg(via, &Msg::PathOpen { label: out, dest, ttl: ttl - 1 });
    }

    /// Looks up a relay entry and refreshes it.
    fn switched(&mut self, from: u32, label: u64) -> Option<(u32, u64)> {
        let h = self.switch.get_mut(&(from, label))?;
        h.used = Instant::now();
        Some((h.to, h.label))
    }

    fn on_path_ack(&mut self, from: u32, label: u64) {
        if let Some((to, out)) = self.switched(from, label) {
            return self.send_msg(to, &Msg::PathAck { label: out });
        }
        if let Some(e) = self.ends.get_mut(&(from, label)) {
            if e.origin && !e.acked {
                e.acked = true;
                if let Some(s) = e.sid.and_then(|sid| self.sessions.get_mut(&sid)) {
                    s.failed.remove(&from);
                }
                log(format!("path open via {}", self.name(from)));
            }
        }
    }

    fn on_path_err(&mut self, from: u32, label: u64) {
        if let Some(h) = self.switch.remove(&(from, label)) {
            self.switch.remove(&(h.to, h.label));
            return self.send_msg(h.to, &Msg::PathErr { label: h.label });
        }
        if self.ends.contains_key(&(from, label)) {
            self.drop_path((from, label), "a hop reported it broken");
        }
    }

    fn on_data(&mut self, from: u32, label: u64, inner: Vec<u8>) {
        if let Some((to, out)) = self.switched(from, label) {
            self.relayed += 1;
            return self.send_msg(to, &Msg::Data { label: out, inner });
        }
        if let Some(e) = self.ends.get_mut(&(from, label)) {
            e.last_seen = Instant::now();
            e.rx += 1;
            return self.on_inner((from, label), &inner);
        }
        // Unknown label: tell the sender, so it stops using the path.
        self.send_msg(from, &Msg::PathErr { label });
    }

    fn on_probe(&mut self, from: u32, label: u64, nonce: u64, reply: bool) {
        if let Some((to, out)) = self.switched(from, label) {
            return self.send_msg(to, &Msg::Probe { label: out, nonce, reply });
        }
        let Some(e) = self.ends.get_mut(&(from, label)) else {
            return self.send_msg(from, &Msg::PathErr { label });
        };
        e.last_seen = Instant::now();
        if !reply {
            return self.send_msg(from, &Msg::Probe { label, nonce, reply: true });
        }
        if let Some((n, sent)) = e.probe {
            if n == nonce {
                let sample = sent.elapsed();
                e.srtt = Some(match e.srtt {
                    Some(s) => (s * 7 + sample) / 8,
                    None => sample,
                });
                e.probe = None;
                e.lost = 0;
                e.acked = true;
            }
        }
    }

    // ---- paths: the ends ----------------------------------------------------------

    fn open_path(&mut self, sid: u32, dest: Hash, via: u32) {
        let label = rand_u64();
        self.ends.insert(
            (via, label),
            PathEnd { origin: true, dest: None, sid: Some(sid), acked: false, srtt: None, probe: None, lost: 0, last_seen: Instant::now(), rx: 0, credit: 0 },
        );
        self.sessions.get_mut(&sid).unwrap().paths.push((via, label));
        log(format!("opening a path to {} via {}", short(&dest), self.name(via)));
        self.send_msg(via, &Msg::PathOpen { label, dest, ttl: TTL });
    }

    fn drop_path(&mut self, key: (u32, u64), why: &str) {
        let Some(e) = self.ends.remove(&key) else { return };
        if let Some(sid) = e.sid {
            if let Some(s) = self.sessions.get_mut(&sid) {
                s.paths.retain(|k| *k != key);
                if e.origin {
                    let backoff = s.failed.get(&key.0).map_or(RETRY_MIN, |(_, b)| (*b * 2).min(RETRY_MAX));
                    s.failed.insert(key.0, (Instant::now() + backoff, backoff));
                }
            }
            if e.origin {
                log(format!("path via {} dropped: {why}", self.name(key.0)));
            }
        }
    }

    fn is_direct(&self, sid: u32, key: (u32, u64)) -> bool {
        let Some(n) = self.neighbors.get(&key.0) else { return false };
        self.sessions.get(&sid).is_some_and(|s| n.x_pub == s.peer_x || Some(n.x_pub) == s.direct_key)
    }

    /// The identity a destination belongs to, from its announce.
    /// Whether a session's far end may learn our addresses: anyone with
    /// `direct`, else only the owner's devices (known by identity: the
    /// destination's, or the opener's once it identified).
    fn direct_ok(&self, sid: u32) -> bool {
        if self.cfg.direct {
            return true;
        }
        let Some(s) = self.sessions.get(&sid) else { return false };
        // Named in the config (identities), or by an application (its
        // destinations or identities): storage knows the owner's devices.
        let named = |h: &Hash| self.cfg.devices.contains(h) || self.clients.values().any(|c| c.devices.contains(h));
        if s.peer_hash.is_some_and(|d| named(&d)) {
            return true;
        }
        let who = s.peer_identity.or_else(|| s.peer_hash.and_then(|d| self.dest_identity(&d)));
        let Some(who) = who else { return false };
        named(&who)
            || self.clients.values().flat_map(|c| c.devices.iter()).any(|d| self.dest_identity(d) == Some(who))
    }

    /// Nodes known by their own announce (unnamed destination): name, overlay
    /// address, how far, and whether a neighbor or a direct session reaches it.
    // ---- names (ADR-0025) ---------------------------------------------------------

    fn on_dns(&mut self, q: crate::dns::Query) {
        use crate::dns::*;
        let Some(tun) = self.tun.clone() else { return };
        let Some((name, qtype, end)) = question(&q.msg) else { return };
        let ours = name == DOMAIN || name.ends_with(".vork");
        if ours {
            let label = name.strip_suffix(DOMAIN).unwrap_or("").trim_end_matches('.');
            let node = self.resolve(label);
            let rcode = if node.is_some() { NOERROR } else { NXDOMAIN };
            let found: Option<std::net::IpAddr> = node.and_then(|h| {
                if qtype == A {
                    let device = h == self.my_hash || self.is_device(&h);
                    self.aliases.of(&h, device).map(Into::into)
                } else {
                    Some(crate::tun::overlay(&h).into())
                }
            });
            let key = (name.clone(), qtype);
            if self.dns_seen.get(&key).is_none_or(|t| t.elapsed() > Duration::from_secs(60)) {
                if self.dns_seen.len() > 256 {
                    self.dns_seen.clear();
                }
                self.dns_seen.insert(key, Instant::now());
                let kind = match qtype { 28 => "AAAA".to_string(), 1 => "A".to_string(), 65 => "HTTPS".to_string(), t => format!("type {t}") };
                log(format!("name asked: {name} {kind} → {}", match (node, found) {
                    (None, _) => "no such name".to_string(),
                    (Some(_), Some(a)) if qtype == AAAA || qtype == A => a.to_string(),
                    _ => "no records of that type".to_string(),
                }));
            }
            return tun.send(&reply(&q, &answer(&q.msg, end, qtype, rcode, found)));
        }
        if self.exit_up() {
            return self.dns_via_exit(q);
        }
        let upstream = self.cfg.dns_upstream.clone();
        if upstream.is_empty() {
            return tun.send(&reply(&q, &answer(&q.msg, end, qtype, REFUSED, None)));
        }
        tokio::spawn(async move {
            let msg = match forward(upstream, q.msg.clone()).await {
                Some(m) => m,
                None => answer(&q.msg, end, qtype, SERVFAIL, None),
            };
            tun.send(&reply(&q, &msg));
        });
    }

    /// The owner's devices that offer themselves as exit nodes.
    fn exit_candidates(&self) -> Vec<Hash> {
        self.routes
            .iter()
            .filter(|(d, _)| self.is_device(d))
            .filter(|(_, by)| {
                by.values().any(|r| r.ann.name_hash == wire::NO_NAME && serde_json::from_slice::<serde_json::Value>(&r.ann.app_data).ok().and_then(|v| v["exit"].as_bool()) == Some(true))
            })
            .map(|(d, _)| *d)
            .collect()
    }

    /// The round trip to a node over its best answering path (None: none answers).
    fn rtt_to(&self, node: &Hash) -> Option<Duration> {
        let sid = self.session_by_dest.get(node)?;
        let s = self.sessions.get(sid)?;
        s.paths.iter().filter_map(|k| self.ends.get(k)).filter(|e| e.acked && e.last_seen.elapsed() < Duration::from_secs(10)).filter_map(|e| e.srtt).min()
    }

    /// The fastest answering exit, kept unless another is clearly faster
    /// (30 % and 10 ms) or it stops answering, so traffic doesn't flap.
    fn choose_exit(&mut self) {
        let mut timed: Vec<(Hash, Duration)> = self.exit_candidates().into_iter().filter_map(|c| Some((c, self.rtt_to(&c)?))).collect();
        timed.sort_by_key(|(_, t)| *t);
        let best = timed.first().copied();
        let now = self.cfg.exit_via.and_then(|c| timed.iter().find(|(h, _)| *h == c).copied());
        let pick = match (now, best) {
            (Some((c, t)), Some((b, bt))) if b != c && bt * 10 < t * 7 && t - bt > Duration::from_millis(10) => Some(b),
            (Some((c, _)), _) => Some(c),
            (None, b) => b.map(|(h, _)| h),
        };
        if pick != self.cfg.exit_via {
            match pick {
                Some(p) => log(format!("exit node chosen: {} ({} ms)", short(&p), timed.iter().find(|(h, _)| *h == p).map_or(0, |(_, t)| t.as_millis()))),
                None => log("no exit node answers: internet goes out directly"),
            }
            self.cfg.exit_via = pick;
        }
    }

    /// Whether the chosen exit node can be used now: a session to it with a
    /// path that answers.
    fn exit_up(&self) -> bool {
        let Some(via) = self.cfg.exit_via else { return false };
        // Either side may have opened the session: the opener's paths count once
        // answered, the far end's while their probes keep coming.
        [self.session_by_dest.get(&via), self.ip_peer.get(&via)].into_iter().flatten().filter_map(|sid| self.sessions.get(sid)).any(|s| {
            s.paths.iter().any(|k| self.ends.get(k).is_some_and(|e| (e.acked || !e.origin) && e.last_seen.elapsed() < Duration::from_secs(10)))
        })
    }

    /// A name outside `.vork` asked of public resolvers through the exit
    /// (ADR-0026), so the network the device is on doesn't see it.
    fn dns_via_exit(&mut self, q: crate::dns::Query) {
        let Some(via) = self.cfg.exit_via else { return };
        let port = loop {
            let p = 20000 + (rand_u64() % 40000) as u16;
            if !self.exit_dns.contains_key(&p) {
                break p;
            }
        };
        let Some(sid) = self.ip_session(via) else { return };
        for server in crate::exit::DNS {
            let mut body = vec![APP_EXIT];
            body.extend_from_slice(&crate::exit::udp4(self.aliases.me, port, server, 53, &q.msg));
            self.send_app(sid, &body);
        }
        self.exit_dns.insert(port, (q, Instant::now()));
    }

    /// Names the exit didn't answer in 2 s: asked of the network's servers.
    fn exit_dns_late(&mut self) {
        let late: Vec<u16> = self.exit_dns.iter().filter(|(_, (_, t))| t.elapsed() > Duration::from_secs(2)).map(|(p, _)| *p).collect();
        for p in late {
            let Some((q, _)) = self.exit_dns.remove(&p) else { continue };
            let (Some(tun), upstream) = (self.tun.clone(), self.cfg.dns_upstream.clone()) else { continue };
            if upstream.is_empty() {
                continue;
            }
            tokio::spawn(async move {
                if let Some(m) = crate::dns::forward(upstream, q.msg.clone()).await {
                    tun.send(&crate::dns::reply(&q, &m));
                }
            });
        }
    }

    /// The address a `.vork` label names: this node or one of the owner's
    /// devices by name (others could claim any name), or any node by its
    /// address (in full, or 8+ digits that only one known node starts with).
    fn resolve(&self, label: &str) -> Option<Hash> {
        if label.is_empty() || label.contains('.') {
            return None;
        }
        if label == crate::dns::label(&self.cfg.name) {
            return Some(self.my_hash);
        }
        if label.len() >= 8 && label.len() <= 32 && label.bytes().all(|b| b.is_ascii_hexdigit()) {
            if let Ok(h) = hex::decode(label) {
                if let Ok(full) = <Hash>::try_from(h.as_slice()) {
                    return Some(full);
                }
            }
            let mut hits = self.routes.keys().chain(std::iter::once(&self.my_hash)).filter(|d| hex::encode(d).starts_with(label));
            if let (Some(one), None) = (hits.next(), hits.next()) {
                return Some(*one);
            }
            return None;
        }
        let device = |d: &Hash| {
            self.cfg.devices.contains(d)
                || self.dest_identity(d).is_some_and(|i| self.cfg.devices.contains(&i))
                || self.clients.values().any(|c| c.devices.contains(d))
        };
        self.routes.iter().filter(|(d, _)| device(d)).find_map(|(d, by)| {
            let r = by.values().filter(|r| r.ann.name_hash == wire::NO_NAME).min_by_key(|r| r.hops)?;
            (crate::dns::label(&announced_name(&r.ann.app_data)?) == label).then_some(*d)
        })
    }

    fn known_nodes(&self) -> Vec<serde_json::Value> {
        let mut list: Vec<serde_json::Value> = self
            .routes
            .iter()
            .filter_map(|(dest, by)| {
                let best = by.values().filter(|r| r.ann.name_hash == wire::NO_NAME).min_by_key(|r| r.hops)?;
                let neighbor = self.neighbor_by_key.get(&best.ann.x_pub).is_some_and(|id| self.is_up(*id));
                let session = self.session_by_dest.get(dest).or_else(|| self.ip_peer.get(dest));
                let direct = session.and_then(|sid| self.sessions.get(sid).map(|s| (sid, s))).is_some_and(|(sid, s)| s.paths.iter().any(|k| self.is_direct(*sid, *k)));
                Some(json!({
                    "address": hex::encode(dest),
                    "name": announced_name(&best.ann.app_data),
                    "ip": crate::tun::overlay(dest).to_string(),
                    "ipv4": self.aliases.peek(dest, self.is_device(dest)).map(|a| a.to_string()),
                    "hops": best.hops,
                    "neighbor": neighbor,
                    "direct": direct || neighbor,
                    "lite": best.ann.flags & FLAG_RELAY == 0,
                    "device": self.cfg.devices.contains(dest),
                    "on_lan": self.lan_seen.get(&best.ann.x_pub).is_some_and(|t| t.elapsed() < ROUTE_TTL),
                    "exit": serde_json::from_slice::<serde_json::Value>(&best.ann.app_data).ok().and_then(|v| v["exit"].as_bool()).unwrap_or(false),
                }))
            })
            .collect();
        list.sort_by(|a, b| a["name"].as_str().unwrap_or("~").cmp(b["name"].as_str().unwrap_or("~")));
        list
    }

    fn dest_identity(&self, dest: &Hash) -> Option<Hash> {
        self.routes.get(dest)?.values().next().map(|r| r.ann.identity())
    }

    /// For sessions we opened: keep up to `--paths` paths through different
    /// first hops, plus a path over a direct link when there is one.
    fn maintain_sessions(&mut self) {
        let mine: Vec<(u32, Hash)> = self.sessions.iter().filter_map(|(sid, s)| s.peer_hash.map(|h| (*sid, h))).collect();
        for (sid, dest) in mine {
            let s = &self.sessions[&sid];
            let used: HashSet<u32> = s.paths.iter().map(|(n, _)| *n).collect();
            let resting: HashSet<u32> = s.failed.iter().filter(|(_, (until, _))| Instant::now() < *until).map(|(n, _)| *n).collect();
            let hops: Vec<u32> = self.first_hops(&dest).into_iter().filter(|v| !resting.contains(v)).collect();
            let owner = self.dest_identity(&dest);
            let direct_key = s.direct_key;
            let direct = hops.iter().copied().find(|v| (owner.is_some() && self.neighbors[v].hash == owner) || Some(self.neighbors[v].x_pub) == direct_key);
            if let Some(d) = direct {
                if !used.contains(&d) {
                    self.open_path(sid, dest, d);
                }
            }
            let relayed_used = used.iter().filter(|n| Some(**n) != direct).count();
            let free: Vec<u32> = hops.into_iter().filter(|v| !used.contains(v) && Some(*v) != direct).collect();
            for via in free.into_iter().take(self.cfg.paths.saturating_sub(relayed_used)) {
                self.open_path(sid, dest, via);
            }
        }
    }

    fn probe_paths(&mut self) {
        let mine: Vec<(u32, u64)> = self.ends.iter().filter(|(_, e)| e.origin).map(|(k, _)| *k).collect();
        for key in mine {
            let e = self.ends.get_mut(&key).unwrap();
            if e.probe.is_some() {
                e.lost += 1;
            }
            if e.lost >= PROBE_LOSS_DEAD {
                self.drop_path(key, "no answer to probes");
                continue;
            }
            let nonce = rand_u64();
            e.probe = Some((nonce, Instant::now()));
            self.send_msg(key.0, &Msg::Probe { label: key.1, nonce, reply: false });
        }
    }

    /// The paths a packet of this session goes out on.
    fn pick_paths(&mut self, sid: u32) -> Vec<(u32, u64)> {
        let Some(s) = self.sessions.get(&sid) else { return vec![] };
        let all: Vec<(u32, u64)> = s.paths.iter().copied().filter(|k| self.ends.contains_key(k) && self.is_up(k.0)).collect();
        let origin = s.peer_hash.is_some();
        // Healthy: for our own paths, confirmed and answering probes; for a
        // responder, heard from recently.
        let mut healthy: Vec<(u32, u64)> = all
            .iter()
            .copied()
            .filter(|k| {
                let e = &self.ends[k];
                if origin { e.acked && e.lost == 0 } else { e.last_seen.elapsed() < PATH_RECENT }
            })
            .collect();
        if healthy.is_empty() {
            healthy = all;
        }
        if healthy.len() <= 1 {
            return healthy;
        }
        // A direct link spares the relays: use it alone while it is healthy.
        if self.cfg.spread == Spread::Spray {
            if let Some(d) = healthy.iter().copied().find(|k| self.is_direct(sid, *k)) {
                return vec![d];
            }
        }
        match self.cfg.spread {
            Spread::Dup => healthy,
            Spread::Spray => {
                // Smooth weighted round robin, weight inversely proportional to the round trip.
                let weights: Vec<i64> = healthy
                    .iter()
                    .map(|k| {
                        let rtt = self.ends[k].srtt.unwrap_or(Duration::from_millis(50)).as_micros().max(100) as i64;
                        1_000_000_000 / rtt
                    })
                    .collect();
                let total: i64 = weights.iter().sum();
                let mut best = 0;
                for (i, k) in healthy.iter().enumerate() {
                    let e = self.ends.get_mut(k).unwrap();
                    e.credit += weights[i];
                    if e.credit > self.ends[&healthy[best]].credit {
                        best = i;
                    }
                }
                self.ends.get_mut(&healthy[best]).unwrap().credit -= total;
                vec![healthy[best]]
            }
        }
    }

    // ---- inner layer: end-to-end sessions ----------------------------------------

    /// An inner WireGuard packet arrived on one of our path ends.
    fn on_inner(&mut self, key: (u32, u64), inner: &[u8]) {
        let sid = match wg::kind(inner) {
            Some(wg::INIT) => {
                let Ok(Packet::HandshakeInit(init)) = Tunn::parse_incoming_packet(inner) else { return };
                let local = self.ends.get(&key).and_then(|e| e.dest);
                let ident = self.local_identity(local.as_ref());
                let Ok(half) = parse_handshake_anon(&ident.dh, &ident.x_pub(), &init) else { return };
                match self.session_by_key.get(&half.peer_static_public) {
                    Some(sid) => *sid,
                    None => {
                        // The opener is anonymous (a fresh key per session) until it identifies.
                        let sid = self.new_id();
                        let tunn = Self::session_tunn(ident.dh.clone(), half.peer_static_public, sid);
                        let nonce = session_nonce(&half.peer_static_public, ident.x_pub().as_bytes());
                        self.sessions.insert(sid, Session::new(tunn, half.peer_static_public, None, local, None, nonce));
                        self.session_by_key.insert(half.peer_static_public, sid);
                        log(format!("new end-to-end session {} to {}", hex::encode(&nonce[..4]), local.map(|d| short(&d)).unwrap_or_default()));
                        sid
                    }
                }
            }
            Some(_) => match wg::receiver_idx(inner) {
                Some(idx) if self.sessions.contains_key(&(idx >> 8)) => idx >> 8,
                _ => return,
            },
            None => return,
        };
        // The far end of a path learns its session from the traffic on it.
        let e = self.ends.get_mut(&key).unwrap();
        if e.sid.is_none() {
            e.sid = Some(sid);
        }
        let s = self.sessions.get_mut(&sid).unwrap();
        if !s.paths.contains(&key) {
            s.paths.push(key);
        }
        let outs = decap(&mut s.tunn, None, inner);
        for out in outs {
            match out {
                Out::Net(p) => self.send_inner(sid, p),
                Out::Payload(p) => {
                    if let Some(body) = wire::unframe(&p).map(|b| b.to_vec()) {
                        self.on_app(sid, key, &body);
                    }
                }
                // With --spread dup the second copy of every packet is a replay
                // to WireGuard, which is exactly right; don't log it.
                Out::Err(e) if e.contains("Duplicate") || e.contains("Tai64n") => {}
                Out::Err(e) => log(format!("session: {e}")),
            }
        }
    }

    fn send_inner(&mut self, sid: u32, p: Vec<u8>) {
        if self.cfg.drop > 0.0 && (rand_u64() as f64 / u64::MAX as f64) < self.cfg.drop {
            return;
        }
        let paths = self.pick_paths(sid);
        if paths.is_empty() {
            return;
        }
        for (n, label) in paths {
            self.send_msg(n, &Msg::Data { label, inner: p.clone() });
        }
    }

    fn send_app(&mut self, sid: u32, body: &[u8]) {
        let Some(s) = self.sessions.get_mut(&sid) else { return };
        if let Some(other) = s.looped {
            return self.on_app(other, (0, 0), body);
        }
        let framed = wire::frame(body);
        if let Some(p) = encap(&mut s.tunn, &framed) {
            self.send_inner(sid, p);
        }
    }

    /// A session to one of this node's own destinations: two ends joined in
    /// memory, with a nonce like any session's.
    fn loop_session(&mut self, dest: Hash) -> u32 {
        let ident = self.local_identity(Some(&dest));
        let mut seed = [0u8; 32];
        getrandom::getrandom(&mut seed).expect("OS entropy");
        let opener = StaticSecret::from(seed);
        let opener_pub = *PublicKey::from(&opener).as_bytes();
        let nonce = session_nonce(&opener_pub, ident.x_pub().as_bytes());
        let a = self.new_id();
        let mut b = self.new_id();
        while b == a {
            b = self.new_id();
        }
        let ta = Self::session_tunn(opener, *ident.x_pub().as_bytes(), a);
        let tb = Self::session_tunn(ident.dh.clone(), opener_pub, b);
        let mut sa = Session::new(ta, *ident.x_pub().as_bytes(), Some(dest), None, Some(ident.hash()), nonce);
        let mut sb = Session::new(tb, opener_pub, None, Some(dest), None, nonce);
        sa.looped = Some(b);
        sb.looped = Some(a);
        self.sessions.insert(a, sa);
        self.sessions.insert(b, sb);
        self.session_by_dest.insert(dest, a);
        log(format!("session {} to our own {}", hex::encode(&nonce[..4]), short(&dest)));
        a
    }

    // ---- a tiny application: ping ------------------------------------------------

    fn session_to(&mut self, dest: Hash) -> Option<u32> {
        if let Some(sid) = self.session_by_dest.get(&dest) {
            return Some(*sid);
        }
        if self.local_dest(&dest).is_some() {
            return Some(self.loop_session(dest));
        }
        let ann = self.routes.get(&dest)?.values().map(|r| r.ann.clone()).next()?;
        if self.first_hops(&dest).is_empty() {
            return None;
        }
        let sid = self.new_id();
        // A fresh key for every session: the far end can't tell who opens it.
        let mut seed = [0u8; 32];
        getrandom::getrandom(&mut seed).expect("OS entropy");
        let secret = StaticSecret::from(seed);
        let nonce = session_nonce(PublicKey::from(&secret).as_bytes(), &ann.x_pub);
        let tunn = Self::session_tunn(secret, ann.x_pub, sid);
        self.sessions.insert(sid, Session::new(tunn, ann.x_pub, Some(dest), None, Some(ann.identity()), nonce));
        self.session_by_dest.insert(dest, sid);
        log(format!("opening end-to-end session {} to {}", hex::encode(&nonce[..4]), short(&dest)));
        self.maintain_sessions();
        Some(sid)
    }

    fn ping(&mut self) {
        let dest = self.cfg.ping.unwrap();
        let Some(sid) = self.session_to(dest) else {
            self.request_path(dest);
            return log(format!("waiting for a route to {}", short(&dest)));
        };
        let nonce = rand_u64();
        self.pings.insert(nonce, Instant::now());
        let mut body = vec![1u8];
        body.extend_from_slice(&nonce.to_be_bytes());
        self.send_app(sid, &body);
    }

    // ---- IP over sessions (M2) ---------------------------------------------------------

    /// A packet from the interface: to the node whose identity the overlay
    /// destination names, over a session to it (opened, and identified in,
    /// on the first packet).
    fn on_tun_packet(&mut self, p: &[u8]) {
        if p.first().is_some_and(|b| b >> 4 == 4) {
            return self.on_tun_v4(p);
        }
        let Some((src, dst)) = crate::tun::v6_ends(p) else { return };
        if src != crate::tun::overlay(&self.my_hash).octets() {
            return; // only this node's own address goes out
        }
        if let Some(q) = crate::dns::query_in(p) {
            return self.on_dns(q);
        }
        if dst == crate::dns::ADDR.octets() {
            return; // not plain DNS (TLS on 853, TCP): the resolver doesn't do those
        }
        let Some(prefix) = crate::tun::prefix_of(&dst) else { return };
        let dest = self
            .routes
            .iter()
            .filter(|(d, r)| d[..15] == prefix && r.values().any(|x| x.ann.name_hash == wire::NO_NAME))
            .map(|(d, _)| *d)
            .next();
        let Some(dest) = dest else {
            if self.ip_unknown.get(&prefix).is_none_or(|t| t.elapsed() > Duration::from_secs(30)) {
                self.ip_unknown.insert(prefix, Instant::now());
                log(format!("no node known for {}", std::net::Ipv6Addr::from(dst)));
            }
            return;
        };
        let Some(sid) = self.ip_session(dest) else { return };
        // Sent over IPv6 now: its answers come back as IPv6, whatever went before over IPv4.
        if !self.v4_flows.is_empty() {
            if let Some(flow) = crate::alias::flow_v6_out(p) {
                self.v4_flows.remove(&(dest, flow));
            }
        }
        let mut body = Vec::with_capacity(p.len() + 1);
        body.push(APP_IP);
        body.extend_from_slice(p);
        self.send_app(sid, &body);
    }

    /// A packet from a session: onto the interface if its source is the far
    /// end's own overlay address and it is for ours.
    fn on_ip(&mut self, sid: u32, p: &[u8]) {
        let Some(tun) = self.tun.clone() else { return };
        let Some((src, dst)) = crate::tun::v6_ends(p) else { return };
        let Some(who) = self.sessions.get(&sid).and_then(|s| s.peer_identity) else {
            self.ask_who(sid);
            return debug("IP from a session that hasn't identified: dropped (asked who it is)");
        };
        if src != crate::tun::overlay(&who).octets() || dst != crate::tun::overlay(&self.my_hash).octets() {
            return debug(format!("IP with source {} from {}: dropped", std::net::Ipv6Addr::from(src), short(&who)));
        }
        self.ip_peer.insert(who, sid);
        // An answer in a conversation a program here began over IPv4: back as IPv4.
        if let Some(alias) = self.aliases.known(&who) {
            if let Some((v4, flow)) = crate::alias::to_v4(p, alias, self.aliases.me) {
                if let Some(t) = self.v4_flows.get_mut(&(who, flow)) {
                    *t = Instant::now();
                    return tun.send(&v4);
                }
            }
        }
        tun.send(p);
    }

    /// Asks the far end of a session who it is (at most once a second per session).
    fn ask_who(&mut self, sid: u32) {
        let now = Instant::now();
        if self.who_asked.get(&sid).is_some_and(|t| now.duration_since(*t) < Duration::from_secs(1)) {
            return;
        }
        if self.who_asked.len() > 1024 {
            self.who_asked.retain(|_, t| now.duration_since(*t) < Duration::from_secs(60));
        }
        self.who_asked.insert(sid, now);
        self.send_app(sid, &[APP_WHO]);
    }

    /// The session IP to a node goes in: the one it came in on, or one opened
    /// (and identified in, so the far end knows who sends).
    fn ip_session(&mut self, dest: Hash) -> Option<u32> {
        let came_in = self.ip_peer.get(&dest).copied().filter(|s| self.sessions.contains_key(s));
        let sid = came_in.or_else(|| self.session_to(dest))?;
        if came_in.is_none() && self.ip_identified.insert(sid) {
            let me = self.me_arc.clone();
            self.identify(sid, &me);
        }
        Some(sid)
    }

    /// Internet traffic through an exit node (ADR-0026). At the exit: from one
    /// of the owner's devices, out with its own address. At the device: from
    /// the exit it chose, onto the interface.
    fn on_exit_packet(&mut self, sid: u32, p: &[u8]) {
        let Some(tun) = self.tun.clone() else { return };
        let Some(who) = self.sessions.get(&sid).and_then(|s| s.peer_identity) else {
            return self.ask_who(sid);
        };
        if p.len() < 20 || p[0] >> 4 != 4 {
            return;
        }
        self.ip_peer.insert(who, sid);
        // A device sends from its own (derived) address; it's answered there.
        if self.cfg.exit && p[12..16] == crate::alias::derive(&who).octets() {
            if !self.is_device(&who) {
                return debug(format!("exit traffic from {}, not one of the owner's devices: dropped", short(&who)));
            }
            let Some(addr) = self.exit_clients.of(&who) else { return };
            if let Some(out) = crate::exit::readdress(p, Some(addr), None) {
                tun.send(&out);
            }
        } else if self.cfg.exit_via == Some(who) && p[16..20] == self.aliases.me.octets() {
            // An answer to a name this node asked through the exit: to whoever asked it (the first answer wins).
            if let Some((53, port, msg)) = crate::exit::udp4_parts(p) {
                if crate::exit::DNS.iter().any(|d| p[12..16] == d.octets()) {
                    if let Some((q, _)) = self.exit_dns.remove(&port) {
                        tun.send(&crate::dns::reply(&q, msg));
                    }
                    return;
                }
            }
            tun.send(p);
        }
    }

    /// An IPv4 packet from a program here: to an alias, it goes on as IPv6.
    fn on_tun_v4(&mut self, p: &[u8]) {
        if p.len() < 20 {
            return;
        }
        let dst = std::net::Ipv4Addr::new(p[16], p[17], p[18], p[19]);
        if p[12..16] != self.aliases.me.octets() {
            // At an exit node: an answer from the internet for a device using it.
            if self.cfg.exit && crate::exit::in_range(dst) {
                let Some(device) = self.exit_clients.node(dst) else { return };
                let Some(back) = crate::exit::readdress(p, None, Some(crate::alias::derive(&device))) else { return };
                if let Some(sid) = self.ip_session(device) {
                    let mut body = vec![APP_EXIT];
                    body.extend_from_slice(&back);
                    self.send_app(sid, &body);
                }
            }
            return;
        }
        if !crate::alias::in_range(dst) || crate::exit::in_range(dst) {
            // To the internet: through the exit node, if one is chosen.
            let Some(via) = self.cfg.exit_via else { return };
            if let Some(sid) = self.ip_session(via) {
                let mut body = vec![APP_EXIT];
                body.extend_from_slice(p);
                self.send_app(sid, &body);
            }
            return;
        }
        // A device's address used straight (copied from an app, no name asked): known by deriving.
        let node = self.aliases.node(dst).or_else(|| {
            let d = self.cfg.devices.iter().copied().find(|d| crate::alias::derive(d) == dst)?;
            (self.aliases.of(&d, true) == Some(dst)).then_some(d)
        });
        let Some(node) = node else { return };
        let me = crate::tun::overlay(&self.my_hash);
        let Some((v6, flow)) = crate::alias::to_v6(p, me, crate::tun::overlay(&node)) else { return };
        if self.v4_flows.len() > 4096 {
            self.v4_flows.retain(|_, t| t.elapsed() < Duration::from_secs(600));
        }
        // Sent first, then marked: the IPv6 path forgets IPv4 flows it carries itself.
        self.on_tun_packet(&v6);
        self.v4_flows.insert((node, flow), Instant::now());
    }

    fn on_app(&mut self, sid: u32, key: (u32, u64), body: &[u8]) {
        // The session works: offer a direct link, once, if this peer may have
        // one (considered again when it identifies).
        if let Some(s) = self.sessions.get_mut(&sid)
            && !s.direct_considered
        {
            s.direct_considered = true;
            if !matches!(body.first(), Some(&3) | Some(&APP_DIRECT)) && !self.punched.contains(&sid) && self.direct_ok(sid) {
                self.punched.insert(sid);
                self.send_punch(sid);
            }
        }
        match body.first() {
            Some(1) if body.len() == 9 => {
                let mut reply = vec![2u8];
                reply.extend_from_slice(&body[1..9]);
                self.send_app(sid, &reply);
            }
            Some(2) if body.len() == 9 => {
                let nonce = u64::from_be_bytes(body[1..9].try_into().unwrap());
                if let Some(t) = self.pings.remove(&nonce) {
                    let who = self.sessions[&sid].peer_hash.map(|h| short(&h)).unwrap_or_default();
                    let path = if self.is_direct(sid, key) { "direct".to_string() } else { format!("via {}", self.name(key.0)) };
                    log(format!("pong from {who}: {} ({path})", ms(t.elapsed())));
                }
                if self.direct_ok(sid) && self.punched.insert(sid) {
                    self.send_punch(sid);
                }
            }
            Some(&APP_IP) => self.on_ip(sid, &body[1..]),
            Some(&APP_EXIT) => self.on_exit_packet(sid, &body[1..]),
            // Asked who this node is: said again, in sessions it identified in before.
            Some(&APP_WHO) => {
                if self.ip_identified.contains(&sid) || self.link_identified.contains(&sid) {
                    let me = self.me_arc.clone();
                    self.identify(sid, &me);
                }
            }
            // The old offer without the node's key: dialing the session's key
            // instead (a one-off) disturbed the link already up. Ignored.
            Some(3) => debug("an old direct-link offer: ignored"),
            Some(&APP_DIRECT) if body.len() > 33 => {
                let key: [u8; 32] = body[1..33].try_into().unwrap();
                match wire::decode_addrs(&body[33..]) {
                    Ok(cands) => self.on_punch(sid, key, cands),
                    Err(e) => log(format!("bad direct-link offer: {e}")),
                }
            }
            Some(&APP_SEG) if body.len() >= 5 => {
                let seq = u32::from_be_bytes(body[1..5].try_into().unwrap());
                let msgs = {
                    let Some(s) = self.sessions.get_mut(&sid) else { return };
                    s.stream.on_seg(seq, &body[5..], Instant::now());
                    s.stream.messages()
                };
                match msgs {
                    Ok(msgs) => {
                        for m in msgs {
                            self.on_message(sid, &m);
                        }
                    }
                    Err(e) => warn(format!("session {}: {e}", sid)),
                }
                self.flush(sid);
            }
            Some(&APP_ACK) if body.len() >= 6 => {
                let next = u32::from_be_bytes(body[1..5].try_into().unwrap());
                let n = (body[5] as usize).min(stream::MAX_RANGES);
                let ranges: Vec<(u32, u32)> = body[6..]
                    .chunks_exact(8)
                    .take(n)
                    .map(|c| (u32::from_be_bytes(c[..4].try_into().unwrap()), u32::from_be_bytes(c[4..].try_into().unwrap())))
                    .collect();
                if let Some(s) = self.sessions.get_mut(&sid) {
                    s.stream.on_ack(next, &ranges, Instant::now());
                }
                self.flush(sid);
            }
            Some(4) if body.len() >= 9 => {
                let seq = u64::from_be_bytes(body[1..9].try_into().unwrap());
                let r = self.bench_rx.entry(sid).or_default();
                r.bytes += body.len() as u64;
                r.packets += 1;
                r.first_seq.get_or_insert(seq);
                r.max_seq = r.max_seq.max(seq);
            }
            _ => {}
        }
    }

    // ---- messages: identify, requests ----------------------------------------------

    /// Sends what the session's stream has ready: segments and acknowledgments.
    fn flush(&mut self, sid: u32) {
        let Some(s) = self.sessions.get_mut(&sid) else { return };
        let outs = s.stream.poll(Instant::now());
        for o in outs {
            let body = match o {
                stream::Out::Seg { seq, data } => {
                    let mut b = Vec::with_capacity(5 + data.len());
                    b.push(APP_SEG);
                    b.extend_from_slice(&seq.to_be_bytes());
                    b.extend_from_slice(&data);
                    b
                }
                stream::Out::Ack { next, ranges } => {
                    let mut b = vec![APP_ACK];
                    b.extend_from_slice(&next.to_be_bytes());
                    b.push(ranges.len() as u8);
                    for (start, end) in ranges {
                        b.extend_from_slice(&start.to_be_bytes());
                        b.extend_from_slice(&end.to_be_bytes());
                    }
                    b
                }
            };
            self.send_app(sid, &body);
        }
    }

    fn send_message(&mut self, sid: u32, msg: &[u8]) {
        if let Some(s) = self.sessions.get_mut(&sid) {
            s.stream.send(msg);
        }
        self.flush(sid);
    }

    /// Proves to the far end who we are, bound to this session.
    fn identify(&mut self, sid: u32, who: &Identity) {
        let Some(s) = self.sessions.get(&sid) else { return };
        let sig = who.sign(LABEL_IDENTIFY, &s.nonce);
        let mut m = vec![MSG_IDENTIFY];
        m.extend_from_slice(&who.ed_pub());
        m.extend_from_slice(who.x_pub().as_bytes());
        m.extend_from_slice(&sig);
        self.send_message(sid, &m);
    }

    /// Sends a request; the answer comes back as a RESPONSE message.
    fn request(&mut self, sid: u32, path: &str, body: &[u8], for_client: Option<(u32, u32)>) -> Option<u32> {
        let s = self.sessions.get_mut(&sid)?;
        let id = s.next_req;
        s.next_req = s.next_req.wrapping_add(1);
        s.waiting.insert(id, (path.to_string(), Instant::now(), for_client));
        let mut m = vec![MSG_REQUEST];
        m.extend_from_slice(&id.to_be_bytes());
        m.push(path.len().min(255) as u8);
        m.extend_from_slice(&path.as_bytes()[..path.len().min(255)]);
        m.extend_from_slice(body);
        self.send_message(sid, &m);
        Some(id)
    }

    fn on_message(&mut self, sid: u32, m: &[u8]) {
        match m.first() {
            Some(&MSG_IDENTIFY) if m.len() == 1 + 32 + 32 + 64 => {
                let ed: [u8; 32] = m[1..33].try_into().unwrap();
                let x: [u8; 32] = m[33..65].try_into().unwrap();
                let sig: [u8; 64] = m[65..129].try_into().unwrap();
                let Some(s) = self.sessions.get_mut(&sid) else { return };
                if identity::verify(&ed, LABEL_IDENTIFY, &s.nonce, &sig) {
                    let who = node_hash(&ed, &x);
                    s.peer_identity = Some(who);
                    s.direct_considered = false;
                    log(format!("session {} identified as {}", hex::encode(&s.nonce[..4]), short(&who)));
                } else {
                    warn("an identify with a bad signature");
                }
            }
            Some(&MSG_REQUEST) if m.len() >= 6 => {
                let id = u32::from_be_bytes(m[1..5].try_into().unwrap());
                let plen = m[5] as usize;
                if m.len() < 6 + plen {
                    return;
                }
                let path = String::from_utf8_lossy(&m[6..6 + plen]).into_owned();
                let s = &self.sessions[&sid];
                let owner = s.local.and_then(|d| self.local.get(&d)).and_then(|d| d.owner);
                if let Some(client) = owner {
                    let frame = vork_api::FromNode::Incoming {
                        token: ((sid as u64) << 32) | id as u64,
                        dest: s.local.unwrap(),
                        session: sid,
                        nonce: s.nonce,
                        requester: s.peer_identity,
                        path,
                        body: m[6 + plen..].to_vec(),
                    };
                    self.to_client(client, &frame);
                    return;
                }
                let (status, body) = self.serve(sid, &path, &m[6 + plen..]);
                self.respond(sid, id, status, &body);
            }
            Some(&MSG_RESPONSE) if m.len() >= 6 => {
                let id = u32::from_be_bytes(m[1..5].try_into().unwrap());
                let status = m[5];
                let Some(s) = self.sessions.get_mut(&sid) else { return };
                let Some((path, sent, for_client)) = s.waiting.remove(&id) else { return };
                // A link message arrived: off the outbox.
                if path == LINK_PATH && for_client.is_none() {
                    if let Some(dest) = s.peer_hash {
                        let a = hex::encode(dest);
                        if status == STATUS_OK {
                            self.links.delivered(&a);
                            self.link_sent.remove(&a);
                            self.links.save();
                        }
                    }
                    return;
                }
                if let Some((client, req)) = for_client {
                    let frame = vork_api::FromNode::Response { req, status, body: m[6..].to_vec() };
                    self.to_client(client, &frame);
                    return;
                }
                let n = m.len() - 6;
                let t = sent.elapsed();
                log(format!(
                    "{path}: status {status}, {n} bytes in {} ({:.1} Mbit/s)",
                    ms(t),
                    n as f64 * 8.0 / t.as_secs_f64().max(1e-6) / 1e6
                ));
            }
            _ => {}
        }
    }

    fn respond(&mut self, sid: u32, id: u32, status: u8, body: &[u8]) {
        let mut r = vec![MSG_RESPONSE];
        r.extend_from_slice(&id.to_be_bytes());
        r.push(status);
        r.extend_from_slice(body);
        self.send_message(sid, &r);
    }

    // ---- the local API: applications on this machine ----------------------------

    fn to_client(&self, client: u32, frame: &vork_api::FromNode) {
        if let Some(c) = self.clients.get(&client) {
            let _ = c.tx.send(frame.encode());
        }
    }

    fn known(&self, dest: &Hash) -> Option<vork_api::Known> {
        if let Some(d) = self.local.get(dest) {
            let mut public = [0u8; 64];
            public[..32].copy_from_slice(d.identity.x_pub().as_bytes());
            public[32..].copy_from_slice(&d.identity.ed_pub());
            return Some(vork_api::Known { public, name_hash: d.name_hash, hops: 0, app_data: d.app_data.clone() });
        }
        let r = self.routes.get(dest)?.values().min_by_key(|r| r.hops)?;
        let a = &r.ann;
        let mut public = [0u8; 64];
        public[..32].copy_from_slice(&a.x_pub);
        public[32..].copy_from_slice(&a.ed_pub);
        Some(vork_api::Known { public, name_hash: a.name_hash, hops: r.hops, app_data: a.app_data.clone() })
    }

    fn on_api(&mut self, ev: crate::api::Event) {
        use vork_api::{FromNode, ToNode};
        match ev {
            crate::api::Event::Connected { client, tx } => {
                self.clients.insert(client, ApiClient { tx, subscribed: false, devices: Vec::new() });
            }
            crate::api::Event::Gone { client } => {
                self.clients.remove(&client);
                let gone: Vec<Hash> = self.local.iter().filter(|(_, d)| d.owner == Some(client)).map(|(h, _)| *h).collect();
                for d in gone {
                    self.local.remove(&d);
                }
                self.finds.retain(|f| f.0 != client);
            }
            crate::api::Event::Frame { client, frame } => match frame {
                ToNode::Hello { .. } => self.to_client(client, &FromNode::Hello { version: vork_api::VERSION, address: self.my_hash }),
                ToNode::Register { req, secret, name } => {
                    let identity = Arc::new(Identity::from_bytes(&secret));
                    let dest = self.add_local(identity, &name, Some(client));
                    log(format!("an application hosts {} ({name})", short(&dest)));
                    self.to_client(client, &FromNode::Registered { req, dest });
                }
                ToNode::Announce { dest, app_data } => {
                    if let Some(d) = self.local.get_mut(&dest).filter(|d| d.owner == Some(client)) {
                        d.app_data = app_data;
                        d.app_data.truncate(wire::MAX_APP_DATA);
                        self.announce_one(&dest);
                    }
                }
                ToNode::Recall { req, dest } => {
                    let known = self.known(&dest);
                    self.to_client(client, &FromNode::Known { req, known });
                }
                ToNode::Find { req, dest, timeout_ms } => match self.known(&dest) {
                    Some(k) => self.to_client(client, &FromNode::Known { req, known: Some(k) }),
                    None => {
                        self.request_path(dest);
                        self.finds.push((client, req, dest, Instant::now() + Duration::from_millis(timeout_ms as u64)));
                    }
                },
                ToNode::Open { req, dest } => match self.session_to(dest) {
                    Some(sid) => {
                        let nonce = self.sessions[&sid].nonce;
                        self.to_client(client, &FromNode::Opened { req, session: sid, nonce });
                    }
                    None => {
                        self.request_path(dest);
                        self.to_client(client, &FromNode::Error { req, message: format!("no path to {}", hex::encode(dest)) });
                    }
                },
                ToNode::Identify { session, as_dest } => {
                    if let Some(d) = self.local.get(&as_dest).filter(|d| d.owner == Some(client)) {
                        let who = d.identity.clone();
                        self.identify(session, &who);
                    }
                }
                ToNode::Request { req, session, path, body } => {
                    if self.request(session, &path, &body, Some((client, req))).is_none() {
                        self.to_client(client, &FromNode::Error { req, message: "no such session".into() });
                    }
                }
                ToNode::Close { .. } => {}
                ToNode::Respond { token, status, body } => {
                    let (sid, id) = ((token >> 32) as u32, token as u32);
                    self.respond(sid, id, status, &body);
                }
                ToNode::Subscribe => {
                    if let Some(c) = self.clients.get_mut(&client) {
                        c.subscribed = true;
                    }
                }
                ToNode::Devices { devices } => {
                    if let Some(c) = self.clients.get_mut(&client) {
                        c.devices = devices;
                    }
                    // Sessions already open get another look.
                    for s in self.sessions.values_mut() {
                        s.direct_considered = false;
                    }
                }
            },
        }
    }

    /// Tells `Find`s and subscribers about an announce just accepted.
    fn heard(&mut self, dest: Hash) {
        let Some(known) = self.known(&dest) else { return };
        let done: Vec<(u32, u32)> = self.finds.iter().filter(|f| f.2 == dest).map(|f| (f.0, f.1)).collect();
        self.finds.retain(|f| f.2 != dest);
        for (client, req) in done {
            self.to_client(client, &vork_api::FromNode::Known { req, known: Some(known.clone()) });
        }
        let frame = vork_api::FromNode::Heard { dest, known };
        for c in self.clients.values().filter(|c| c.subscribed) {
            let _ = c.tx.send(frame.encode());
        }
    }

    fn expire_finds(&mut self) {
        let now = Instant::now();
        let late: Vec<(u32, u32)> = self.finds.iter().filter(|f| f.3 <= now).map(|f| (f.0, f.1)).collect();
        self.finds.retain(|f| f.3 > now);
        for (client, req) in late {
            self.to_client(client, &vork_api::FromNode::Known { req, known: None });
        }
    }

    /// Answers a request to one of our destinations. Built in for now:
    /// `/vork/echo` (the body back) and `/vork/blob` (N bytes, N a u32 body).
    fn serve(&mut self, _sid: u32, path: &str, body: &[u8]) -> (u8, Vec<u8>) {
        match path {
            "/vork/echo" => (STATUS_OK, body.to_vec()),
            "/vork/link" => self.on_link_message(_sid, body),
            "/vork/blob" if body.len() == 4 => {
                let n = (u32::from_be_bytes(body.try_into().unwrap()) as usize).min(stream::MAX_MESSAGE - 64);
                (STATUS_OK, (0..n).map(|i| (i % 251) as u8).collect())
            }
            _ => (STATUS_NO_PATH, Vec::new()),
        }
    }

    /// The `--fetch` test: one request at a time to the destination.
    fn fetch(&mut self) {
        let Some((dest, n)) = self.cfg.fetch else { return };
        let Some(sid) = self.session_to(dest) else {
            self.request_path(dest);
            return;
        };
        let s = &self.sessions[&sid];
        if !s.waiting.is_empty() {
            return;
        }
        if self.cfg.identify && s.next_req == 0 {
            let me = self.me_arc.clone();
            self.identify(sid, &me);
        }
        self.request(sid, "/vork/blob", &(n as u32).to_be_bytes(), None);
    }

    // ---- bench: throughput -------------------------------------------------------

    fn bench_send(&mut self) {
        let dest = self.cfg.bench.unwrap();
        if !self.routes.contains_key(&dest) {
            return self.request_path(dest);
        }
        let sid = match self.session_by_dest.get(&dest) {
            Some(sid) => *sid,
            None => match self.session_to(dest) {
                Some(sid) => sid,
                None => return,
            },
        };
        // Only once the session and a path are confirmed, so the first second isn't all loss.
        let ready = self.sessions[&sid].tunn.time_since_last_handshake().is_some()
            && self.sessions[&sid].paths.iter().any(|k| self.ends.get(k).is_some_and(|e| e.acked));
        if !ready {
            if self.bench_seq == 0 {
                self.bench_seq = 1;
                self.send_app(sid, &[0u8]); // starts the handshake
            }
            return;
        }
        let mut body = vec![0u8; BENCH_PAYLOAD];
        body[0] = 4;
        body[1..9].copy_from_slice(&self.bench_seq.to_be_bytes());
        self.bench_seq += 1;
        self.bench_sent += BENCH_PAYLOAD as u64;
        self.send_app(sid, &body);
    }

    fn bench_report(&mut self, secs: f64) {
        if self.cfg.bench.is_some() && self.bench_sent > 0 {
            log(format!("bench sent {:.1} Mbit/s", self.bench_sent as f64 * 8.0 / secs / 1e6));
            self.bench_sent = 0;
        }
        for (sid, r) in self.bench_rx.drain() {
            let expected = r.first_seq.map_or(0, |f| r.max_seq - f + 1);
            let loss = if expected > 0 { 100.0 * (1.0 - r.packets as f64 / expected as f64) } else { 0.0 };
            let who = self.sessions.get(&sid).map(|s| hex::encode(&s.peer_x[..4])).unwrap_or_default();
            log(format!("bench from {who}: {:.1} Mbit/s received, loss {:.1}%", r.bytes as f64 * 8.0 / secs / 1e6, loss.max(0.0)));
        }
    }

    // ---- direct links ----------------------------------------------------------

    /// Where others might reach us: the addresses our neighbors see us at, the
    /// router mapping if we have one, and our own listening address if it is a
    /// specific one (a LAN address).
    fn candidates(&self) -> Vec<SocketAddr> {
        let mut c: Vec<SocketAddr> = self.neighbors.values().filter_map(|n| n.observed_me).collect();
        c.extend(self.mapped);
        if !self.cfg.listen.ip().is_unspecified() {
            c.push(self.cfg.listen);
        }
        c.sort();
        c.dedup();
        c
    }

    /// Addresses for a link code: those other nodes proved they reach this
    /// one at (from outside), the router's mapping, and its LAN address.
    fn link_hints(&self) -> Vec<SocketAddr> {
        let week = 7 * 86400;
        let mut h: Vec<SocketAddr> = self.reach.about_me.values().filter(|(_, t)| unix_now().saturating_sub(*t) < week).map(|(a, _)| *a).collect();
        h.extend(self.mapped);
        let port = self.socket_port();
        let lan = std::net::UdpSocket::bind("0.0.0.0:0")
            .and_then(|s| s.connect("192.0.2.1:9").and_then(|_| s.local_addr()))
            .ok()
            .map(|a| a.ip())
            .filter(|ip| !ip.is_unspecified() && !ip.is_loopback());
        if let (Some(ip), Some(port)) = (lan, port) {
            h.push(SocketAddr::new(ip, port));
        }
        let mut seen = HashSet::new();
        h.retain(|a| seen.insert(*a));
        h.truncate(4);
        h
    }

    /// The port this node listens on.
    fn socket_port(&self) -> Option<u16> {
        self.send_sock.local_addr().ok().map(|a| a.port()).filter(|p| *p != 0)
    }

    fn send_punch(&mut self, sid: u32) {
        let cands = self.candidates();
        if cands.is_empty() {
            return;
        }
        log(format!("offering a direct link at {cands:?}"));
        // The node's own link key goes with the addresses: the session's key
        // on either end may be a one-off (the opener's) or an application's.
        let mut body = vec![APP_DIRECT];
        body.extend_from_slice(self.me.x_pub().as_bytes());
        body.extend_from_slice(&wire::encode_addrs(&cands));
        self.send_app(sid, &body);
    }

    /// The far end offered its addresses: answer with ours (once), then both
    /// sides send WireGuard handshakes to each other at the same time, which
    /// also opens the way through most NATs.
    fn on_punch(&mut self, sid: u32, peer: [u8; 32], cands: Vec<SocketAddr>) {
        if !self.direct_ok(sid) {
            return debug("peer offered a direct link; not with this peer here (direct, devices)");
        }
        if self.punched.insert(sid) {
            self.send_punch(sid);
        }
        if peer == *self.me.x_pub().as_bytes() {
            return;
        }
        if let Some(s) = self.sessions.get_mut(&sid) {
            s.direct_key = Some(peer);
        }
        if self.neighbor_by_key.get(&peer).is_some_and(|id| self.neighbors[id].up) {
            return;
        }
        let Some(first) = cands.first().copied() else { return };
        let id = match self.neighbor_by_key.get(&peer) {
            Some(id) => *id,
            None => self.add_neighbor(peer, Some(first), None),
        };
        let mut buf = vec![0u8; PKT_BUF];
        let n = self.neighbors.get_mut(&id).unwrap();
        if let TunnResult::WriteToNetwork(p) = n.tunn.format_handshake_initiation(&mut buf, true) {
            let p = p.to_vec();
            log(format!("dialing {} directly at {cands:?}", hex::encode(&peer[..4])));
            for c in &cands {
                self.send_raw(*c, &p);
            }
        }
    }

    // ---- timers -----------------------------------------------------------------

    fn on_timers(&mut self) {
        let ids: Vec<u32> = self.neighbors.keys().copied().collect();
        for id in ids {
            let n = self.neighbors.get_mut(&id).unwrap();
            let addr = n.addr;
            if let (Some(p), Some(addr)) = (timers(&mut n.tunn), addr) {
                self.send_raw(addr, &p);
            }
            self.check_up(id);
        }
        let sids: Vec<u32> = self.sessions.keys().copied().collect();
        for sid in sids {
            let s = self.sessions.get_mut(&sid).unwrap();
            if let Some(p) = timers(&mut s.tunn) {
                self.send_inner(sid, p);
            }
        }
        self.switch.retain(|_, h| h.used.elapsed() < PATH_IDLE);
        let idle: Vec<(u32, u64)> = self.ends.iter().filter(|(_, e)| !e.origin && e.last_seen.elapsed() >= PATH_IDLE).map(|(k, _)| *k).collect();
        for k in idle {
            self.drop_path(k, "idle");
        }
        self.pending.retain(|_, (_, t)| t.elapsed() < Duration::from_secs(30));
        if self.seen_req.len() > 10_000 {
            self.seen_req.clear();
        }
        self.pings.retain(|_, t| t.elapsed() < Duration::from_secs(10));
        // Learned nodes that didn't come up: let go.
        let failed: Vec<u32> = self.neighbors.iter().filter(|(_, n)| !n.up && n.trial.is_some_and(|t| t.elapsed() >= TRIAL)).map(|(id, _)| *id).collect();
        for id in failed {
            if let Some(n) = self.neighbors.remove(&id) {
                self.neighbor_by_key.remove(&n.x_pub);
                debug(format!("learned node {} didn't answer", n.addr.map(|a| a.to_string()).unwrap_or_default()));
            }
        }
    }

    /// Drops routes not announced again within ROUTE_TTL.
    fn forget_old_routes(&mut self) {
        let mut gone = 0;
        let in_use = &self.session_by_dest;
        self.routes.retain(|dest, by| {
            // A session's destination keeps its newest route (a Lite node
            // hears no floods; it asked once).
            if in_use.contains_key(dest) {
                let newest = by.iter().max_by_key(|(_, r)| r.heard).map(|(k, _)| *k);
                by.retain(|k, r| r.heard.elapsed() < ROUTE_TTL || Some(*k) == newest);
            } else {
                by.retain(|_, r| r.heard.elapsed() < ROUTE_TTL);
            }
            if by.is_empty() {
                gone += 1;
            }
            !by.is_empty()
        });
        if gone > 0 {
            debug(format!("forgot {gone} destination(s) not announced for {} min", ROUTE_TTL.as_secs() / 60));
        }
    }

    // ---- local network discovery ----------------------------------------------------

    /// A node on the local network: dial it at the address it registered
    /// (one our socket can reach), unless it's a neighbor already up.
    fn on_lan_found(&mut self, found: crate::lan::Found) {
        let key = found.key;
        if key == *self.me.x_pub().as_bytes() {
            return;
        }
        self.lan_seen.insert(key, Instant::now());
        // The addresses our socket can use; loopback only if it's all there is
        // (another node on this machine).
        let v4 = self.cfg.listen.is_ipv4();
        let usable: Vec<SocketAddr> = found.addrs.iter().copied().filter(|a| a.is_ipv4() || !v4).collect();
        let outside: Vec<SocketAddr> = usable.iter().copied().filter(|a| !a.ip().is_loopback()).collect();
        let cands = if outside.is_empty() { usable } else { outside };
        let Some(first) = cands.first().copied() else { return };
        let id = match self.neighbor_by_key.get(&key).copied() {
            Some(id) => {
                let Some(n) = self.neighbors.get_mut(&id) else { return };
                if n.up || n.last_try.elapsed() < Duration::from_secs(10) {
                    return;
                }
                id
            }
            None => {
                log(format!("found a node on the local network at {}", cands.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", ")));
                let id = self.add_neighbor(key, Some(first), None);
                if let Some(n) = self.neighbors.get_mut(&id) {
                    n.trial = Some(Instant::now());
                }
                id
            }
        };
        // One handshake to every address it gave: the one that answers is
        // where it is (the link takes the address the answer came from).
        let mut buf = vec![0u8; PKT_BUF];
        let n = self.neighbors.get_mut(&id).unwrap();
        n.last_try = Instant::now();
        if let TunnResult::WriteToNetwork(p) = n.tunn.format_handshake_initiation(&mut buf, false) {
            let p = p.to_vec();
            for c in &cands {
                self.send_raw(*c, &p);
            }
        }
    }

    // ---- reachability (ADR-0023) ---------------------------------------------------

    /// Tells a neighbor which nodes this one dialed cold and reached.
    fn share_reach(&mut self, id: u32) {
        let Some(n) = self.neighbors.get(&id) else { return };
        let Some(addr) = n.addr else { return };
        let entries = self.reach.to_share(reach::scope(addr.ip()), unix_now());
        if !entries.is_empty() {
            self.send_msg(id, &Msg::Reach { entries });
        }
    }

    /// A neighbor's report. About itself: ignored (no node vouches for
    /// itself). About us: how others reach us. Others: leads to dial.
    fn on_reach(&mut self, from: u32, entries: Vec<([u8; 32], SocketAddr, u32)>) {
        let Some(n) = self.neighbors.get(&from) else { return };
        let Some(reporter) = n.hash else { return };
        let reporter_key = n.x_pub;
        let mine = *self.me.x_pub().as_bytes();
        let now = unix_now();
        for (key, addr, age) in entries {
            if key == reporter_key {
                continue;
            }
            if key == mine {
                if self.reach.about_me.insert(reporter, (addr, now.saturating_sub(age as u64))).is_none() {
                    log(format!("{} reached us directly at {addr}", short(&reporter)));
                }
                continue;
            }
            self.reach.heard(&reporter, key, addr, age, now);
        }
    }

    /// Fewer relaying neighbors up than MIN_UP: dial the best learned node not tried lately.
    fn seek_neighbors(&mut self) {
        let up = self.neighbors.values().filter(|n| n.up && n.flags.is_none_or(|f| f & FLAG_RELAY != 0)).count();
        let trying = self.neighbors.values().any(|n| n.trial.is_some());
        if up >= MIN_UP || trying {
            return;
        }
        self.tried.retain(|_, t| t.elapsed() < TRIED_AGAIN);
        let mine = *self.me.x_pub().as_bytes();
        let skip = |k: &[u8; 32]| *k == mine || self.tried.contains_key(k) || self.neighbor_by_key.get(k).is_some_and(|id| self.is_up(*id));
        let Some((key, addr)) = self.reach.candidates(skip).into_iter().next() else { return };
        self.tried.insert(key, Instant::now());
        let id = match self.neighbor_by_key.get(&key) {
            // Known but down: try it at the learned address.
            Some(id) => {
                let id = *id;
                if let Some(n) = self.neighbors.get_mut(&id) {
                    n.addr = Some(addr);
                }
                id
            }
            None => {
                let id = self.add_neighbor(key, Some(addr), None);
                if let Some(n) = self.neighbors.get_mut(&id) {
                    n.trial = Some(Instant::now());
                }
                id
            }
        };
        log(format!("dialing {addr}, learned to be reachable"));
        self.initiate(id);
    }

    fn status(&self) {
        let up: Vec<String> = self.neighbors.iter().filter(|(_, n)| n.up).map(|(id, _)| self.name(*id)).collect();
        debug(format!(
            "status: neighbors up [{}], destinations {}, sessions {}, relay entries {}, relayed {} packets",
            up.join(" "),
            self.routes.len(),
            self.sessions.len(),
            self.switch.len() / 2,
            self.relayed
        ));
        for (sid, s) in &self.sessions {
            let paths: Vec<String> = s
                .paths
                .iter()
                .filter_map(|k| {
                    let e = self.ends.get(k)?;
                    let kind = if self.is_direct(*sid, *k) { "direct".to_string() } else { format!("via {}", self.name(k.0)) };
                    let rtt = e.srtt.map(ms).unwrap_or_else(|| "-".into());
                    Some(format!("{kind} rtt {rtt} rx {}", e.rx))
                })
                .collect();
            let who = s.peer_hash.map(|h| short(&h)).unwrap_or_else(|| hex::encode(&s.peer_x[..4]));
            debug(format!("  session {who}: [{}]", paths.join("; ")));
        }
    }

    // ---- state and control ------------------------------------------------------

    /// Stores the neighbors that are up now, so the next run can dial them.
    fn remember_neighbors(&self) {
        let now = unix_now();
        let up: Vec<Remembered> = self
            .neighbors
            .values()
            .filter(|n| n.up)
            .filter_map(|n| Some(Remembered { addr: n.addr?, key: hex::encode(n.x_pub), seen: now }))
            .collect();
        if !up.is_empty() {
            self.state.remember(&up);
        }
    }

    fn on_control(&mut self, request: &str, owner: bool) -> String {
        let v = serde_json::from_str::<serde_json::Value>(request).unwrap_or_default();
        let cmd = v.get("cmd").and_then(|c| c.as_str()).unwrap_or_default();
        if !matches!(cmd, "status") && !owner {
            return json!({ "error": "only the node's own user (or root) may change it" }).to_string();
        }
        match Some(cmd) {
            Some("status") => self.status_json().to_string(),
            // Lets go of the interface (Android's VPN turned off): no restart needed.
            // The network's own DNS servers (the Android app tells them, and again on each network change).
            Some("dns_upstream") => {
                let list: Vec<SocketAddr> = v["servers"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|s| s.as_str()).filter_map(parse_dns_server).collect())
                    .unwrap_or_default();
                if list != self.cfg.dns_upstream {
                    log(format!("names outside .vork go to {}", if list.is_empty() { "nobody".to_string() } else { list.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", ") }));
                }
                self.cfg.dns_upstream = list;
                json!({ "ok": true }).to_string()
            }
            Some("interface_off") => {
                self.tun = None;
                self.ip_peer.clear();
                self.ip_identified.clear();
                json!({ "ok": true }).to_string()
            }
            Some("link" | "link_allow" | "link_dismiss" | "unlink" | "link_code") => match self.on_link_command(cmd, &v) {
                Ok(answer) => answer.to_string(),
                Err(e) => json!({ "error": e }).to_string(),
            },
            Some("set" | "peer_add" | "peer_remove" | "device_add" | "device_remove") => match self.change_settings(cmd, &v) {
                Ok(restart) => json!({ "ok": true, "restart": restart }).to_string(),
                Err(e) => json!({ "error": e }).to_string(),
            },
            // Connect to a neighbor now: "HOST:PORT" (its key asked for) or "HOST:PORT@KEY".
            Some("peer") => {
                let Some(text) = v.get("addr").and_then(|a| a.as_str()) else { return json!({ "error": "peer needs addr" }).to_string() };
                match parse_peer_text(text) {
                    Ok((addr, Some(key))) => {
                        if !self.neighbor_by_key.contains_key(&key) {
                            let id = self.add_neighbor(key, Some(addr), Some(RETRY_MIN));
                            self.initiate(id);
                        }
                        json!({ "ok": true }).to_string()
                    }
                    Ok((addr, None)) => {
                        self.ask_key(addr);
                        json!({ "ok": true }).to_string()
                    }
                    Err(e) => json!({ "error": e }).to_string(),
                }
            }
            _ => json!({ "error": "unknown command; try {\"cmd\":\"status\"}" }).to_string(),
        }
    }

    // ---- linking devices (ADR-0024) ---------------------------------------------------

    fn is_device(&self, h: &Hash) -> bool {
        self.cfg.devices.contains(h)
    }

    /// Adds or removes a device in `vork.toml` (both sides agreed, or one unlinked).
    fn set_device(&mut self, address: &str, on: bool) {
        let cmd = if on { "device_add" } else { "device_remove" };
        if let Err(e) = self.change_settings(cmd, &json!({ "address": address })) {
            warn(format!("{cmd} {address}: {e}"));
        }
    }

    fn on_link_command(&mut self, cmd: &str, v: &serde_json::Value) -> Result<serde_json::Value, String> {
        if cmd == "link_code" {
            // A link (and its QR code), as Resilio's: valid for a while,
            // for some uses, linking at once or asking the owner each time.
            let mut b = [0u8; 8];
            getrandom::getrandom(&mut b).map_err(|e| e.to_string())?;
            let code = hex::encode(b);
            let valid = v.get("valid_s").and_then(|x| x.as_u64()).map(Duration::from_secs).unwrap_or(LINK_CODE_TTL).min(Duration::from_secs(30 * 86400));
            let uses = match v.get("uses").and_then(|x| x.as_u64()) {
                Some(0) => None,
                Some(n) => Some(n as u32),
                None => Some(1),
            };
            let approve = v.get("approve").and_then(|x| x.as_bool()).unwrap_or(false);
            self.link_codes.retain(|c| c.until > Instant::now());
            self.link_codes.push(LinkCode { code: code.clone(), until: Instant::now() + valid, uses, approve });
            let me = hex::encode(self.my_hash);
            let name: String = self.cfg.name.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
            // Where it can be dialed, so a scan alone connects (no address typed in).
            let hints = self.link_hints();
            let mut uri = format!("vork-link:{me}?c={code}&n={name}");
            if !hints.is_empty() {
                uri.push_str("&a=");
                uri.push_str(&hints.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(","));
            }
            // The QR code as rows of 0/1, for apps that draw it themselves.
            let qr: Vec<String> = qrcode::QrCode::new(uri.as_bytes())
                .map(|q| {
                    let w = q.width();
                    q.to_colors().chunks(w).map(|row| row.iter().map(|c| if *c == qrcode::Color::Dark { '1' } else { '0' }).collect()).collect()
                })
                .unwrap_or_default();
            return Ok(json!({ "address": me, "code": code, "uri": uri, "qr": qr, "valid_s": valid.as_secs(), "uses": uses, "approve": approve }));
        }
        let address = v.get("address").and_then(|a| a.as_str()).map(|a| a.trim().to_ascii_lowercase()).ok_or(format!("{cmd} needs address"))?;
        let hash = crate::links::parse(&address).ok_or("an address is 32 hex digits")?;
        if hash == self.my_hash {
            return Err("that's this node".into());
        }
        match cmd {
            "link" => {
                let code = v.get("code").and_then(|c| c.as_str()).unwrap_or_default().to_string();
                // Addresses the link came with: asked right away, so the two connect without other nodes.
                for a in v.get("addrs").and_then(|a| a.as_array()).into_iter().flatten().filter_map(|a| a.as_str()?.parse::<SocketAddr>().ok()) {
                    if !self.neighbors.values().any(|n| n.addr == Some(a)) && !self.asking.contains_key(&a) {
                        self.ask_key(a);
                    }
                }
                if self.is_device(&hash) {
                    return Ok(json!({ "ok": true, "already": true }));
                }
                // Asked by it already: this is the answer.
                if self.links.requests.iter().any(|r| r.address == address) {
                    self.links.forget(&address);
                    self.set_device(&address, true);
                    self.links.send(&address, "accept");
                } else {
                    if !self.links.is_pending(&address) {
                        self.links.pending.push(address.clone());
                    }
                    self.links.send(&address, if code.is_empty() { "request".to_string() } else { format!("request:{code}") }.as_str());
                }
            }
            "link_allow" => {
                if !self.links.requests.iter().any(|r| r.address == address) {
                    return Err("no request from that node".into());
                }
                self.links.forget(&address);
                self.set_device(&address, true);
                self.links.send(&address, "accept");
            }
            "link_dismiss" => {
                self.links.forget(&address);
                self.links.send(&address, "dismiss");
            }
            "unlink" => {
                self.links.forget(&address);
                if self.is_device(&hash) {
                    self.set_device(&address, false);
                }
                self.links.send(&address, "remove");
            }
            _ => unreachable!(),
        }
        self.links.save();
        self.link_sent.remove(&address);
        self.send_links();
        Ok(json!({ "ok": true }))
    }

    /// Sends what's in the outbox and not sent lately, over a session in
    /// which this node identifies (the receiver must know who asks).
    fn send_links(&mut self) {
        let due: Vec<(String, String)> = self
            .links
            .outbox
            .iter()
            .filter(|o| self.link_sent.get(&o.address).is_none_or(|t| t.elapsed() >= LINK_AGAIN))
            .map(|o| (o.address.clone(), o.op.clone()))
            .collect();
        for (address, op) in due {
            let Some(dest) = crate::links::parse(&address) else { continue };
            let Some(sid) = self.session_to(dest) else { continue }; // no route yet: later
            if self.link_identified.insert(sid) {
                let me = self.me_arc.clone();
                self.identify(sid, &me);
            }
            let (op, code) = op.split_once(':').map_or((op.as_str(), ""), |(o, c)| (o, c));
            let body = json!({ "op": op, "name": self.cfg.name, "code": code }).to_string();
            self.request(sid, LINK_PATH, body.as_bytes(), None);
            self.link_sent.insert(address, Instant::now());
        }
    }

    /// A link message from an identified node.
    fn on_link_message(&mut self, sid: u32, body: &[u8]) -> (u8, Vec<u8>) {
        let Some(who) = self.sessions.get(&sid).and_then(|s| s.peer_identity) else {
            return (STATUS_REFUSED, b"identify first".to_vec());
        };
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(body) else { return (STATUS_REFUSED, b"bad message".to_vec()) };
        let address = hex::encode(who);
        let name: String = v["name"].as_str().unwrap_or_default().chars().take(64).collect();
        match v["op"].as_str().unwrap_or_default() {
            "request" => {
                let code = v["code"].as_str().unwrap_or_default();
                let now = Instant::now();
                let valid = self.link_codes.iter().position(|c| !code.is_empty() && c.code == code && c.until > now && c.uses != Some(0));
                let by_code = valid.is_some_and(|i| !self.link_codes[i].approve);
                if let Some(i) = valid {
                    if let Some(n) = self.link_codes[i].uses.as_mut() {
                        *n -= 1;
                    }
                }
                if self.is_device(&who) {
                    self.links.send(&address, "accept"); // it lost track: tell it again
                } else if by_code || self.links.is_pending(&address) {
                    // A code this node gave out, or both asked: linked.
                    self.links.forget(&address);
                    self.set_device(&address, true);
                    self.links.send(&address, "accept");
                    log(format!("linked with {name} ({})", short(&who)));
                } else if !self.links.requests.iter().any(|r| r.address == address) {
                    self.links.requests.push(crate::links::Request { address: address.clone(), name: name.clone(), since: unix_now(), with_link: valid.is_some() });
                    log(format!("{name} ({}) asks to be linked as one of this owner's devices", short(&who)));
                }
            }
            "accept" => {
                if self.links.is_pending(&address) {
                    self.links.forget(&address);
                    self.set_device(&address, true);
                    log(format!("{name} ({}) accepted: linked", short(&who)));
                }
            }
            "dismiss" => {
                if self.links.is_pending(&address) {
                    self.links.forget(&address);
                    log(format!("{name} ({}) dismissed the link", short(&who)));
                }
            }
            "remove" => {
                self.links.forget(&address);
                if self.is_device(&who) {
                    self.set_device(&address, false);
                    log(format!("{name} ({}) unlinked", short(&who)));
                }
            }
            _ => return (STATUS_REFUSED, b"unknown op".to_vec()),
        }
        self.links.save();
        self.send_links();
        (STATUS_OK, Vec::new())
    }

    /// What the network app shows and changes: from `vork.toml` and as running.
    fn settings_json(&self) -> serde_json::Value {
        let text = self.cfg.config_path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
        json!({
            "file": self.cfg.config_path.as_ref().map(|p| p.display().to_string()),
            "peers": crate::settings::list(&text, "peers"),
            "peers_off": crate::settings::list(&text, "peers_off"),
            "devices": self.cfg.devices.iter().map(hex::encode).collect::<Vec<_>>(),
            "direct": self.cfg.direct,
            "lan": self.cfg.lan,
            "map_port": self.cfg.map_port,
            "announce": self.cfg.announce,
            "port": self.cfg.listen.port(),
            "exit": self.cfg.exit,
            "exit_via": self.cfg.exit_via.map(hex::encode),
            "exit_auto": self.cfg.exit_auto,
        })
    }

    /// Changes `vork.toml` and what can be applied at once. Ok(true): the rest
    /// takes effect at the next start.
    fn change_settings(&mut self, cmd: &str, v: &serde_json::Value) -> Result<bool, String> {
        let path = self.cfg.config_path.clone().ok_or("this node runs without a configuration file")?;
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let arg = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::trim).filter(|s| !s.is_empty()).ok_or(format!("{cmd} needs {k}"));
        let mut restart = false;
        let new = match cmd {
            "set" => {
                let key = arg("key")?;
                let value = v.get("value").ok_or("set needs value")?;
                match key {
                    "name" => {
                        let name = value.as_str().map(str::trim).filter(|s| !s.is_empty()).ok_or("a name is text")?;
                        let name: String = name.chars().take(64).collect();
                        self.cfg.name = name.clone();
                        self.announce_self();
                        crate::settings::set_value(&text, "name", &format!("\"{}\"", crate::settings::escape(&name)))
                    }
                    "direct" => {
                        let on = value.as_bool().ok_or("direct is true or false")?;
                        self.cfg.direct = on;
                        crate::settings::set_value(&text, "direct", &on.to_string())
                    }
                    "exit" => {
                        let on = value.as_bool().ok_or("exit is true or false")?;
                        restart = on != self.cfg.exit; // the system's forwarding is set up at start
                        crate::settings::set_value(&text, "exit", &on.to_string())
                    }
                    "exit_via" => {
                        let via = value.as_str().map(str::trim).unwrap_or_default();
                        self.cfg.exit_auto = via == "auto";
                        if via == "auto" {
                            self.cfg.exit_via = None;
                            log("the exit node is chosen automatically: the fastest of your devices offering it");
                            self.choose_exit();
                            crate::settings::set_value(&text, "exit_via", "\"auto\"")
                        } else if via.is_empty() {
                            self.cfg.exit_via = None;
                            log("internet traffic goes out directly again");
                            crate::settings::set_value(&text, "exit_via", "\"\"")
                        } else {
                            let h = crate::links::parse(via).ok_or("exit_via is an address (32 hex digits)")?;
                            if !self.is_device(&h) {
                                return Err("an exit node must be one of your devices".into());
                            }
                            self.cfg.exit_via = Some(h);
                            log(format!("internet traffic goes out through {}", short(&h)));
                            crate::settings::set_value(&text, "exit_via", &format!("\"{}\"", hex::encode(h)))
                        }
                    }
                    "lan" | "map_port" | "announce" => {
                        let on = value.as_bool().ok_or(format!("{key} is true or false"))?;
                        restart = true;
                        crate::settings::set_value(&text, key, &on.to_string())
                    }
                    _ => return Err(format!("{key} can't be set this way")),
                }
            }
            "peer_add" => {
                let addr = arg("addr")?;
                let (sock, key) = parse_peer_text(addr)?;
                match key {
                    Some(k) if !self.neighbor_by_key.contains_key(&k) => {
                        let id = self.add_neighbor(k, Some(sock), Some(RETRY_MIN));
                        self.initiate(id);
                    }
                    Some(_) => {}
                    None => self.ask_key(sock),
                }
                let mut peers = crate::settings::list(&text, "peers");
                if !peers.iter().any(|p| p == addr) {
                    peers.push(addr.to_string());
                }
                crate::settings::set_list(&text, "peers", &peers)
            }
            "peer_remove" => {
                let addr = arg("addr")?;
                let peers: Vec<String> = crate::settings::list(&text, "peers").into_iter().filter(|p| p != addr).collect();
                let off: Vec<String> = crate::settings::list(&text, "peers_off").into_iter().filter(|p| p != addr).collect();
                restart = true; // a neighbor already up stays until the next start
                crate::settings::set_list(&crate::settings::set_list(&text, "peers", &peers), "peers_off", &off)
            }
            "device_add" | "device_remove" => {
                let address = arg("address")?.to_ascii_lowercase();
                let hash: Hash = hex::decode(&address).ok().and_then(|b| b.try_into().ok()).ok_or("a device is a node address (32 hex digits)")?;
                let mut devices = crate::settings::list(&text, "devices");
                devices.retain(|d| !d.eq_ignore_ascii_case(&address));
                self.cfg.devices.retain(|d| *d != hash);
                if cmd == "device_add" {
                    devices.push(address.clone());
                    self.cfg.devices.push(hash);
                }
                for s in self.sessions.values_mut() {
                    s.direct_considered = false;
                }
                crate::settings::set_list(&text, "devices", &devices)
            }
            _ => unreachable!(),
        };
        // Through a temporary file: a crash leaves the old file or the new one.
        let tmp = path.with_extension("toml.new");
        std::fs::write(&tmp, &new).and_then(|()| std::fs::rename(&tmp, &path)).map_err(|e| format!("{}: {e}", path.display()))?;
        log(format!("settings changed ({cmd})"));
        Ok(restart)
    }

    // ---- neighbors known by address only ------------------------------------------

    /// Asks the node at `addr` for its key (answered unauthenticated: the key
    /// is public, and a neighbor is trusted with nothing but relaying).
    fn ask_key(&mut self, addr: SocketAddr) {
        if !self.asking.contains_key(&addr) {
            log(format!("asking {addr} for its key"));
        }
        let nonce = rand_u64();
        self.asking.insert(addr, (nonce, Instant::now()));
        let mut q = KEY_ASK.to_vec();
        q.extend_from_slice(&nonce.to_be_bytes());
        self.send_raw(addr, &q);
    }

    fn on_key_query(&mut self, addr: SocketAddr, data: &[u8]) {
        if data.starts_with(KEY_ASK) && data.len() == KEY_ASK.len() + 8 {
            let mut a = KEY_TELL.to_vec();
            a.extend_from_slice(&data[KEY_ASK.len()..]);
            a.extend_from_slice(self.me.x_pub().as_bytes());
            a.extend_from_slice(&self.me.ed_pub());
            return self.send_raw(addr, &a);
        }
        if data.starts_with(KEY_TELL) && data.len() == KEY_TELL.len() + 8 + 64 {
            let nonce = u64::from_be_bytes(data[KEY_TELL.len()..KEY_TELL.len() + 8].try_into().unwrap());
            if self.asking.get(&addr).map(|(n, _)| *n) != Some(nonce) {
                return;
            }
            self.asking.remove(&addr);
            let key: [u8; 32] = data[KEY_TELL.len() + 8..KEY_TELL.len() + 40].try_into().unwrap();
            if key == *self.me.x_pub().as_bytes() || self.neighbor_by_key.contains_key(&key) {
                return;
            }
            log(format!("{addr} gave its key {}", hex::encode(&key[..4])));
            let id = self.add_neighbor(key, Some(addr), Some(RETRY_MIN));
            self.initiate(id);
        }
    }

    /// Asks again those that haven't answered (every 5 s).
    fn ask_keys_again(&mut self) {
        let late: Vec<SocketAddr> = self.asking.iter().filter(|(_, (_, t))| t.elapsed() >= Duration::from_secs(5)).map(|(a, _)| *a).collect();
        for a in late {
            self.ask_key(a);
        }
    }

    fn status_json(&self) -> serde_json::Value {
        let neighbors: Vec<serde_json::Value> = self
            .neighbors
            .values()
            .map(|n| {
                json!({
                    "address": n.hash.map(hex::encode),
                    "endpoint": n.addr.map(|a| a.to_string()),
                    "up": n.up,
                    "lite": n.flags.map(|f| f & FLAG_RELAY == 0),
                    "last_rx_s": n.last_rx.map(|t| t.elapsed().as_secs()),
                })
            })
            .collect();
        let sessions: Vec<serde_json::Value> = self
            .sessions
            .iter()
            .map(|(sid, s)| {
                let paths: Vec<serde_json::Value> = s
                    .paths
                    .iter()
                    .filter_map(|k| {
                        let e = self.ends.get(k)?;
                        Some(json!({
                            "via": self.neighbors.get(&k.0).and_then(|n| n.hash).map(hex::encode),
                            "direct": self.is_direct(*sid, *k),
                            "rtt_ms": e.srtt.map(|d| d.as_secs_f64() * 1000.0),
                            "confirmed": e.acked,
                            "received": e.rx,
                        }))
                    })
                    .collect();
                json!({
                    "peer": s.peer_hash.map(hex::encode),
                    "peer_key": hex::encode(s.peer_x),
                    "to": s.local.map(hex::encode),
                    "identity": s.peer_identity.map(hex::encode),
                    "nonce": hex::encode(&s.nonce[..8]),
                    "stream_rtt_ms": s.stream.srtt().map(|d| d.as_secs_f64() * 1000.0),
                    "resent": s.stream.retransmits,
                    "paths": paths,
                })
            })
            .collect();
        json!({
            "address": hex::encode(self.my_hash),
            "local_destinations": self.local.iter().map(|(h, d)| serde_json::json!({ "name": d.name, "address": hex::encode(h), "application": d.owner.is_some() })).collect::<Vec<_>>(),
            "applications": self.clients.len(),
            "wg_key": hex::encode(self.me.x_pub().as_bytes()),
            "listen": self.cfg.listen.to_string(),
            "lite": self.cfg.lite,
            "uptime_s": self.started.elapsed().as_secs(),
            "outside": self.candidates().iter().map(|a| a.to_string()).collect::<Vec<_>>(),
            "neighbors": neighbors,
            "destinations": self.routes.len(),
            "sessions": sessions,
            "relay": { "paths": self.switch.len() / 2, "packets": self.relayed },
            "name": self.cfg.name,
            "ip": crate::tun::overlay(&self.my_hash).to_string(),
            "settings": self.settings_json(),
            "links": {
                "pending": self.links.pending.iter().map(|a| json!({ "address": a, "ip": crate::links::parse(a).map(|h| crate::tun::overlay(&h).to_string()) })).collect::<Vec<_>>(),
                "requests": self.links.requests.iter().map(|r| json!({ "address": r.address, "name": r.name, "since": r.since, "with_link": r.with_link, "ip": crate::links::parse(&r.address).map(|h| crate::tun::overlay(&h).to_string()) })).collect::<Vec<_>>(),
            },
            // Every node known by its announce, as Tailscale lists a tailnet's machines.
            "nodes": self.known_nodes(),
            // The interface for ordinary programs (M2), if there is one: this node's address on it.
            "interface": self.tun.as_ref().map(|t| json!({ "name": t.name, "address": crate::tun::overlay(&self.my_hash).to_string() })),
            "ipv4": self.aliases.me.to_string(),
            "exit": (self.cfg.exit_via.is_some() || self.cfg.exit_auto).then(|| json!({
                "via": self.cfg.exit_via.map(hex::encode),
                "up": self.exit_up(),
                "auto": self.cfg.exit_auto,
                "rtt_ms": self.cfg.exit_via.and_then(|v| self.rtt_to(&v)).map(|t| t.as_millis() as u64),
            })),
            "dns": json!({
                "server": crate::dns::ADDR.to_string(),
                "name": format!("{}.{}", crate::dns::label(&self.cfg.name), crate::dns::DOMAIN),
                "upstream": self.cfg.dns_upstream.iter().map(|a| a.to_string()).collect::<Vec<_>>(),
            }),
            "reach": {
                "known": self.reach.len().0,
                "proven_here": self.reach.len().1,
                // Who dialed us cold and got an answer: reachable from there.
                "reached_by": self.reach.about_me.iter().map(|(h, (a, t))| json!({ "node": hex::encode(h), "at": a.to_string(), "ago_s": unix_now().saturating_sub(*t) })).collect::<Vec<_>>(),
            },
        })
    }
}

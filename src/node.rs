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
use boringtun::x25519::PublicKey;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::net::UdpSocket;

const BUF: usize = 65536;
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
/// Relay entries and far-end path state live this long without traffic (probes keep them alive).
const PATH_IDLE: Duration = Duration::from_secs(60);
const PROBE_EVERY: Duration = Duration::from_secs(1);
/// Unanswered probes in a row before a path counts as dead.
const PROBE_LOSS_DEAD: u32 = 3;
/// A responder uses the paths it heard from within this window. The origin
/// probes every healthy path each second, so a quiet path is a dead one.
const PATH_RECENT: Duration = Duration::from_secs(3);
const MAX_HOPS: u8 = 16;
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
    bootstrap: bool,
    up: bool,
}

/// A path as seen from one of its ends; keyed by (neighbor, label on that link).
struct PathEnd {
    /// We opened it (and probe it); otherwise we are its far end.
    origin: bool,
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
    peer_hash: Option<Hash>,
    paths: Vec<(u32, u64)>,
    /// First hops whose paths failed: (don't retry before, current backoff).
    failed: HashMap<u32, (Instant, Duration)>,
}

struct Route {
    hops: u8,
    ann: Announce,
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
    pub announce: bool,
    pub ping: Option<Hash>,
    /// Lite: never relay, receive no floods, ask for paths (ESP32 and the like).
    pub lite: bool,
    /// Offer and accept direct links with the far end of a session. Reveals
    /// this node's addresses to that peer, so it's opt-in.
    pub direct: bool,
    /// Ask the router to open our port (UPnP, then PCP/NAT-PMP).
    pub map_port: bool,
    /// The router for PCP/NAT-PMP; guessed as x.y.z.1 if not given.
    pub gateway: Option<std::net::Ipv4Addr>,
    /// How many paths (through different first hops) a session opens.
    pub paths: usize,
    pub spread: Spread,
}

enum Out {
    Net(Vec<u8>),
    Payload(Vec<u8>),
    Err(String),
}

pub struct Node {
    me: Identity,
    my_hash: Hash,
    /// Receives through tokio; sends through the same socket's std handle,
    /// because tokio's `try_send_to` drops a datagram (WouldBlock) until the
    /// reactor has seen the socket writable, which loses the first handshake.
    sock: Arc<UdpSocket>,
    send_sock: std::net::UdpSocket,
    cfg: Config,
    neighbors: HashMap<u32, Neighbor>,
    neighbor_by_key: HashMap<[u8; 32], u32>,
    sessions: HashMap<u32, Session>,
    session_by_key: HashMap<[u8; 32], u32>,
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
}

fn rand_u64() -> u64 {
    let mut b = [0u8; 8];
    getrandom::getrandom(&mut b).expect("OS entropy");
    u64::from_le_bytes(b)
}

fn now_seq() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

fn log(msg: impl AsRef<str>) {
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    eprintln!("[{}.{:03}] {}", t.as_secs() % 100_000, t.subsec_millis(), msg.as_ref());
}

fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1000.0)
}

/// Runs a datagram through a tunnel, draining queued packets (see boringtun's decapsulate).
fn decap(tunn: &mut Tunn, src: Option<std::net::IpAddr>, data: &[u8]) -> Vec<Out> {
    let mut buf = vec![0u8; BUF];
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
    let mut buf = vec![0u8; BUF];
    match tunn.encapsulate(payload, &mut buf) {
        TunnResult::WriteToNetwork(p) => Some(p.to_vec()),
        _ => None,
    }
}

fn timers(tunn: &mut Tunn) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; BUF];
    match tunn.update_timers(&mut buf) {
        TunnResult::WriteToNetwork(p) => Some(p.to_vec()),
        _ => None,
    }
}

impl Node {
    pub async fn new(me: Identity, cfg: Config) -> anyhow::Result<Node> {
        let std_sock = std::net::UdpSocket::bind(cfg.listen)?;
        std_sock.set_nonblocking(true)?;
        let send_sock = std_sock.try_clone()?;
        let sock = Arc::new(UdpSocket::from_std(std_sock)?);
        let my_hash = me.hash();
        Ok(Node {
            my_hash,
            me,
            sock,
            send_sock,
            cfg,
            neighbors: HashMap::new(),
            neighbor_by_key: HashMap::new(),
            sessions: HashMap::new(),
            session_by_key: HashMap::new(),
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
        })
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

    fn new_tunn(&self, peer: [u8; 32], keepalive: Option<u16>, id: u32) -> Tunn {
        Tunn::new(self.me.dh.clone(), PublicKey::from(peer), None, keepalive, id, None)
    }

    fn add_neighbor(&mut self, x_pub: [u8; 32], addr: Option<SocketAddr>, bootstrap: bool) -> u32 {
        let id = self.new_id();
        let tunn = self.new_tunn(x_pub, Some(KEEPALIVE), id);
        self.neighbors.insert(id, Neighbor { tunn, x_pub, addr, hash: None, flags: None, observed_me: None, last_rx: None, bootstrap, up: false });
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

    pub async fn run(mut self) -> anyhow::Result<()> {
        log(format!(
            "node {} listening on {} (x25519 {}){}",
            hex::encode(self.my_hash),
            self.cfg.listen,
            hex::encode(self.me.x_pub().as_bytes()),
            if self.cfg.lite { " [lite]" } else { "" }
        ));
        for (addr, key) in self.cfg.peers.clone() {
            let id = self.add_neighbor(key, Some(addr), true);
            self.initiate(id);
        }
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
        loop {
            tokio::select! {
                r = self.sock.recv_from(&mut buf) => {
                    match r {
                        Ok((n, addr)) => {
                            let data = buf[..n].to_vec();
                            self.on_datagram(addr, &data);
                        }
                        // e.g. Windows reports an ICMP "port unreachable" from an earlier send this way
                        Err(e) => log(format!("receive: {e}")),
                    }
                }
                Some(ext) = async { match mapped_rx.as_mut() { Some(rx) => rx.recv().await, None => std::future::pending().await } } => {
                    self.mapped = Some(ext);
                }
                _ = tokio::signal::ctrl_c() => {
                    let _ = stop_tx.send(true);
                    if let Some(m) = mapper {
                        let _ = tokio::time::timeout(Duration::from_secs(3), m).await;
                    }
                    log("stopped");
                    return Ok(());
                }
                _ = tick.tick() => {
                    self.on_timers();
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
                    if last_retry.elapsed() >= Duration::from_secs(5) {
                        last_retry = Instant::now();
                        let down: Vec<u32> = self.neighbors.iter().filter(|(_, n)| n.bootstrap && !n.up).map(|(id, _)| *id).collect();
                        for id in down { self.initiate(id); }
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
            log(format!("send to {addr}: {e}"));
        }
    }

    fn initiate(&mut self, id: u32) {
        let Some(n) = self.neighbors.get_mut(&id) else { return };
        let mut buf = vec![0u8; BUF];
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
                        self.add_neighbor(half.peer_static_public, Some(addr), false)
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
        if let Some(addr) = addr {
            self.send_msg(id, &Msg::Observed { addr });
        }
        if self.cfg.announce {
            let own = self.own_announce();
            self.send_msg(id, &Msg::Announce(own));
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
        n.hash = Some(h);
        n.flags = Some(flags);
        if !first {
            return;
        }
        let lite = flags & FLAG_RELAY == 0;
        log(format!("neighbor is {}{}", hex::encode(h), if lite { " [lite]" } else { "" }));
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

    fn own_announce(&self) -> Announce {
        let x_pub = *self.me.x_pub().as_bytes();
        let seq = now_seq();
        let flags = self.my_flags();
        let sig = self.me.sign(LABEL_ANNOUNCE, &Announce::signed_part(&self.my_hash, &x_pub, seq, flags));
        Announce { ed_pub: self.me.ed_pub(), x_pub, seq, flags, sig, hops: 0 }
    }

    fn announce_self(&mut self) {
        let a = self.own_announce();
        for id in self.up_neighbors() {
            self.send_msg(id, &Msg::Announce(a.clone()));
        }
    }

    fn on_announce(&mut self, from: u32, a: Announce) {
        if !self.is_up(from) {
            return; // only over links proven to work both ways
        }
        let hash = node_hash(&a.ed_pub, &a.x_pub);
        if hash == self.my_hash || a.hops >= MAX_HOPS {
            return;
        }
        let signed = Announce::signed_part(&hash, &a.x_pub, a.seq, a.flags);
        if !identity::verify(&a.ed_pub, LABEL_ANNOUNCE, &signed, &a.sig) {
            return log("dropped an announce with a bad signature");
        }
        let hops = a.hops.saturating_add(1);
        let known = self.routes.contains_key(&hash);
        let by_via = self.routes.entry(hash).or_default();
        let newer = by_via.get(&from).is_none_or(|r| a.seq > r.ann.seq || (a.seq == r.ann.seq && hops < r.hops));
        if newer {
            by_via.insert(from, Route { hops, ann: a.clone() });
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
                n.up && (!self.is_lite(**via) || n.hash == Some(*dest))
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
        if dest == self.my_hash {
            if self.cfg.announce {
                let a = self.own_announce();
                self.send_msg(from, &Msg::Announce(a));
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
        if dest == self.my_hash {
            // We are the far end. The session is known once traffic arrives.
            self.ends.insert(
                (from, label),
                PathEnd { origin: false, sid: None, acked: true, srtt: None, probe: None, lost: 0, last_seen: Instant::now(), rx: 0, credit: 0 },
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
            PathEnd { origin: true, sid: Some(sid), acked: false, srtt: None, probe: None, lost: 0, last_seen: Instant::now(), rx: 0, credit: 0 },
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
        self.neighbors.get(&key.0).map(|n| n.x_pub) == self.sessions.get(&sid).map(|s| s.peer_x)
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
            let direct = hops.iter().copied().find(|v| self.neighbors[v].hash == Some(dest));
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
                let Ok(half) = parse_handshake_anon(&self.me.dh, &self.me.x_pub(), &init) else { return };
                match self.session_by_key.get(&half.peer_static_public) {
                    Some(sid) => *sid,
                    None => {
                        // The initiator may be anonymous: we learn its key, not where it is.
                        let sid = self.new_id();
                        let tunn = self.new_tunn(half.peer_static_public, None, sid);
                        self.sessions.insert(sid, Session { tunn, peer_x: half.peer_static_public, peer_hash: None, paths: vec![], failed: HashMap::new() });
                        self.session_by_key.insert(half.peer_static_public, sid);
                        log(format!("new end-to-end session from {}", hex::encode(&half.peer_static_public[..4])));
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
        let framed = wire::frame(body);
        if let Some(p) = encap(&mut s.tunn, &framed) {
            self.send_inner(sid, p);
        }
    }

    // ---- a tiny application: ping ------------------------------------------------

    fn session_to(&mut self, dest: Hash) -> Option<u32> {
        let x_pub = self.routes.get(&dest)?.values().map(|r| r.ann.x_pub).next()?;
        if let Some(sid) = self.session_by_key.get(&x_pub) {
            return Some(*sid);
        }
        if self.first_hops(&dest).is_empty() {
            return None;
        }
        let sid = self.new_id();
        let tunn = self.new_tunn(x_pub, None, sid);
        self.sessions.insert(sid, Session { tunn, peer_x: x_pub, peer_hash: Some(dest), paths: vec![], failed: HashMap::new() });
        self.session_by_key.insert(x_pub, sid);
        log(format!("opening end-to-end session to {}", short(&dest)));
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

    fn on_app(&mut self, sid: u32, key: (u32, u64), body: &[u8]) {
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
                if self.cfg.direct && self.punched.insert(sid) {
                    self.send_punch(sid);
                }
            }
            Some(3) => match wire::decode_addrs(&body[1..]) {
                Ok(cands) => self.on_punch(sid, cands),
                Err(e) => log(format!("bad direct-link offer: {e}")),
            },
            _ => {}
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

    fn send_punch(&mut self, sid: u32) {
        let cands = self.candidates();
        if cands.is_empty() {
            return;
        }
        log(format!("offering a direct link at {cands:?}"));
        let mut body = vec![3u8];
        body.extend_from_slice(&wire::encode_addrs(&cands));
        self.send_app(sid, &body);
    }

    /// The far end offered its addresses: answer with ours (once), then both
    /// sides send WireGuard handshakes to each other at the same time, which
    /// also opens the way through most NATs.
    fn on_punch(&mut self, sid: u32, cands: Vec<SocketAddr>) {
        if !self.cfg.direct {
            return log("peer offered a direct link; not enabled here (--direct)");
        }
        if self.punched.insert(sid) {
            self.send_punch(sid);
        }
        let peer = self.sessions[&sid].peer_x;
        if self.neighbor_by_key.get(&peer).is_some_and(|id| self.neighbors[id].up) {
            return;
        }
        let Some(first) = cands.first().copied() else { return };
        let id = match self.neighbor_by_key.get(&peer) {
            Some(id) => *id,
            None => self.add_neighbor(peer, Some(first), false),
        };
        let mut buf = vec![0u8; BUF];
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
    }

    fn status(&self) {
        let up: Vec<String> = self.neighbors.iter().filter(|(_, n)| n.up).map(|(id, _)| self.name(*id)).collect();
        log(format!(
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
            log(format!("  session {who}: [{}]", paths.join("; ")));
        }
    }
}

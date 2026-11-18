//! A node: one UDP socket, an outer WireGuard session per neighbor, an inner
//! (end-to-end) WireGuard session per remote destination, and a switch table.
//!
//! - A neighbor counts as up only while its handshake is fresh, so every link
//!   used for announces and forwarding is proven to work in both directions.
//! - Destinations are found by signed announces, flooded only over up links.
//! - An inner handshake initiation is forwarded by destination hash; each
//!   relay remembers where it came from, keyed by the initiator's WireGuard
//!   index. Everything after it is switched by receiver index alone, so
//!   relays see neither a source nor (after the first packet) a destination,
//!   and can't read anything: they hold no keys for the inner session.

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
const ANNOUNCE_EVERY: Duration = Duration::from_secs(30);
const SWITCH_IDLE: Duration = Duration::from_secs(300);
const MAX_HOPS: u8 = 16;
const TTL: u8 = 32;

struct Neighbor {
    tunn: Tunn,
    x_pub: [u8; 32],
    addr: Option<SocketAddr>,
    hash: Option<Hash>,
    /// From its HELLO; `None` until then.
    flags: Option<u8>,
    /// The address this neighbor sees us at.
    observed_me: Option<SocketAddr>,
    bootstrap: bool,
    up: bool,
}

struct Session {
    tunn: Tunn,
    peer_x: [u8; 32],
    peer_hash: Option<Hash>,
    via: u32,
}

struct Route {
    via: u32,
    hops: u8,
    ann: Announce,
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
    routes: HashMap<Hash, Route>,
    /// (from neighbor, inner receiver index) -> (to neighbor, last used)
    switch: HashMap<(u32, u32), (u32, Instant)>,
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
    /// session index. Random, so indices from different nodes rarely collide in
    /// a relay's switch table.
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
        self.neighbors.insert(id, Neighbor { tunn, x_pub, addr, hash: None, flags: None, observed_me: None, bootstrap, up: false });
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
            self.neighbors.get_mut(&id).unwrap().addr = Some(addr);
        }
        self.check_up(id);
    }

    fn check_up(&mut self, id: u32) {
        let Some(n) = self.neighbors.get_mut(&id) else { return };
        let fresh = matches!(n.tunn.time_since_last_handshake(), Some(t) if t < LINK_FRESH);
        if fresh && !n.up {
            n.up = true;
            let addr = n.addr;
            log(format!("link up: {}", addr.map(|a| a.to_string()).unwrap_or_default()));
            self.on_link_up(id, addr);
        } else if !fresh && n.up {
            n.up = false;
            log(format!("link down: {}", n.hash.map(|h| short(&h)).unwrap_or_else(|| "?".into())));
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

    fn on_neighbor_payload(&mut self, from: u32, body: &[u8]) {
        let msg = match Msg::decode(body) {
            Ok(m) => m,
            Err(e) => return log(format!("bad message: {e}")),
        };
        match msg {
            Msg::Hello { ed_pub, flags, sig } => {
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
                if first {
                    let lite = flags & FLAG_RELAY == 0;
                    log(format!("neighbor is {}{}", hex::encode(h), if lite { " [lite]" } else { "" }));
                    // Hand a full neighbor what we know, so it can reach
                    // destinations at once. Lite nodes ask instead.
                    if !self.cfg.lite && !lite {
                        let known: Vec<Announce> = self
                            .routes
                            .values()
                            .filter(|r| r.via != from)
                            .map(|r| Announce { hops: r.hops, ..r.ann.clone() })
                            .collect();
                        for a in known {
                            self.send_msg(from, &Msg::Announce(a));
                        }
                    }
                }
            }
            Msg::Observed { addr } => {
                let n = self.neighbors.get_mut(&from).unwrap();
                if n.observed_me != Some(addr) {
                    n.observed_me = Some(addr);
                    log(format!("a neighbor sees us at {addr}"));
                }
            }
            Msg::Announce(a) => self.on_announce(from, a),
            Msg::Fwd { ttl, dest, inner } => self.on_fwd(from, ttl, dest, inner),
            Msg::PathReq { dest, nonce } => self.on_path_req(from, dest, nonce),
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
        if !self.neighbors.get(&from).is_some_and(|n| n.up) {
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
        let better = match self.routes.get(&hash) {
            None => true,
            Some(r) => {
                let via_down = !self.neighbors.get(&r.via).is_some_and(|n| n.up);
                a.seq > r.ann.seq || (a.seq == r.ann.seq && hops < r.hops) || via_down
            }
        };
        if better {
            if !self.routes.contains_key(&hash) {
                log(format!("learned {} at {hops} hops", hex::encode(hash)));
            }
            self.routes.insert(hash, Route { via: from, hops, ann: a.clone() });
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

    /// The next hop toward `dest`: an up neighbor, and never a Lite node
    /// unless it is the destination itself.
    fn route_via(&self, dest: &Hash) -> Option<u32> {
        let r = self.routes.get(dest)?;
        let n = self.neighbors.get(&r.via)?;
        (n.up && (!self.is_lite(r.via) || n.hash == Some(*dest))).then_some(r.via)
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
        if !self.neighbors.get(&from).is_some_and(|n| n.up) || !self.seen_req.insert((dest, nonce)) {
            return;
        }
        if dest == self.my_hash {
            if self.cfg.announce {
                let a = self.own_announce();
                self.send_msg(from, &Msg::Announce(a));
            }
            return;
        }
        if let Some(r) = self.routes.get(&dest) {
            let a = Announce { hops: r.hops, ..r.ann.clone() };
            if r.via != from {
                self.send_msg(from, &Msg::Announce(a));
            }
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

    // ---- forwarding (relay role) -------------------------------------------------

    fn on_fwd(&mut self, from: u32, ttl: u8, dest: Option<Hash>, inner: Vec<u8>) {
        if ttl == 0 {
            return;
        }
        match wg::kind(&inner) {
            Some(wg::INIT) => {
                let Some(dest) = dest else { return };
                if dest == self.my_hash {
                    return self.on_inner(from, &inner);
                }
                if self.cfg.lite {
                    return; // Lite nodes don't relay
                }
                let Some(via) = self.route_via(&dest) else {
                    return log(format!("no route to {}", short(&dest)));
                };
                if via == from {
                    return;
                }
                let sender = wg::sender_idx(&inner).unwrap();
                // Replies come back from `via` addressed to the initiator's index.
                self.switch.insert((via, sender), (from, Instant::now()));
                self.relay(via, Msg::Fwd { ttl: ttl - 1, dest: Some(dest), inner });
            }
            Some(_) => {
                let Some(ridx) = wg::receiver_idx(&inner) else { return };
                if self.cfg.lite {
                    if self.sessions.contains_key(&(ridx >> 8)) {
                        self.on_inner(from, &inner);
                    }
                    return;
                }
                if let Some((to, used)) = self.switch.get_mut(&(from, ridx)) {
                    *used = Instant::now();
                    let to = *to;
                    if let Some(sender) = wg::sender_idx(&inner) {
                        // A handshake response: the initiator's later packets are
                        // addressed to the responder's index.
                        self.switch.insert((to, sender), (from, Instant::now()));
                    }
                    self.relay(to, Msg::Fwd { ttl: ttl - 1, dest: None, inner });
                } else if self.sessions.contains_key(&(ridx >> 8)) {
                    self.on_inner(from, &inner);
                }
            }
            None => {}
        }
    }

    fn relay(&mut self, to: u32, msg: Msg) {
        self.relayed += 1;
        self.send_msg(to, &msg);
    }

    // ---- inner layer: end-to-end sessions ----------------------------------------

    fn on_inner(&mut self, from: u32, inner: &[u8]) {
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
                        self.sessions.insert(sid, Session { tunn, peer_x: half.peer_static_public, peer_hash: None, via: from });
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
        let outs = {
            let s = self.sessions.get_mut(&sid).unwrap();
            s.via = from; // answer the way it came
            decap(&mut s.tunn, None, inner)
        };
        for out in outs {
            match out {
                Out::Net(p) => self.send_inner(sid, p),
                Out::Payload(p) => {
                    if let Some(body) = wire::unframe(&p).map(|b| b.to_vec()) {
                        self.on_app(sid, &body);
                    }
                }
                Out::Err(e) => log(format!("session: {e}")),
            }
        }
    }

    /// For a session we opened, follow the current best route: a direct link
    /// to the peer needs nothing more; a different relay path has no switch
    /// entries yet, so a new handshake lays them.
    fn refresh_via(&mut self, sid: u32) {
        let Some(s) = self.sessions.get(&sid) else { return };
        let Some(h) = s.peer_hash else { return };
        let Some(v) = self.route_via(&h) else { return };
        if v == s.via {
            return;
        }
        let direct = self.neighbors.get(&v).and_then(|n| n.hash) == Some(h);
        log(format!("session to {} now goes {}", short(&h), if direct { "direct" } else { "by another path" }));
        let s = self.sessions.get_mut(&sid).unwrap();
        s.via = v;
        if !direct {
            let mut buf = vec![0u8; BUF];
            if let TunnResult::WriteToNetwork(p) = s.tunn.format_handshake_initiation(&mut buf, true) {
                let p = p.to_vec();
                self.send_msg(v, &Msg::Fwd { ttl: TTL, dest: Some(h), inner: p });
            }
        }
    }

    fn send_inner(&mut self, sid: u32, p: Vec<u8>) {
        self.refresh_via(sid);
        let Some(s) = self.sessions.get(&sid) else { return };
        let dest = if wg::kind(&p) == Some(wg::INIT) {
            match s.peer_hash {
                Some(h) => Some(h),
                None => return log("can't start a handshake to an anonymous peer"),
            }
        } else {
            None
        };
        let via = s.via;
        self.send_msg(via, &Msg::Fwd { ttl: TTL, dest, inner: p });
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
        let r = self.routes.get(&dest)?;
        let x_pub = r.ann.x_pub;
        let via = self.route_via(&dest)?;
        if let Some(sid) = self.session_by_key.get(&x_pub) {
            return Some(*sid);
        }
        let sid = self.new_id();
        let tunn = self.new_tunn(x_pub, None, sid);
        self.sessions.insert(sid, Session { tunn, peer_x: x_pub, peer_hash: Some(dest), via });
        self.session_by_key.insert(x_pub, sid);
        log(format!("opening end-to-end session to {}", short(&dest)));
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

    fn on_app(&mut self, sid: u32, body: &[u8]) {
        match body.first() {
            Some(1) if body.len() == 9 => {
                let mut reply = vec![2u8];
                reply.extend_from_slice(&body[1..9]);
                self.send_app(sid, &reply);
            }
            Some(2) if body.len() == 9 => {
                let nonce = u64::from_be_bytes(body[1..9].try_into().unwrap());
                if let Some(t) = self.pings.remove(&nonce) {
                    let s = &self.sessions[&sid];
                    let who = s.peer_hash.map(|h| short(&h)).unwrap_or_default();
                    let path = if self.neighbors.get(&s.via).and_then(|n| n.hash) == s.peer_hash { "direct" } else { "relayed" };
                    log(format!("pong from {who}: {:.2} ms ({path})", t.elapsed().as_secs_f64() * 1000.0));
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
        self.switch.retain(|_, (_, used)| used.elapsed() < SWITCH_IDLE);
        self.pending.retain(|_, (_, t)| t.elapsed() < Duration::from_secs(30));
        if self.seen_req.len() > 10_000 {
            self.seen_req.clear();
        }
        self.pings.retain(|_, t| t.elapsed() < Duration::from_secs(10));
    }

    fn status(&self) {
        let up: Vec<String> = self
            .neighbors
            .values()
            .filter(|n| n.up)
            .map(|n| n.hash.map(|h| short(&h)).unwrap_or_else(|| "?".into()))
            .collect();
        log(format!(
            "status: neighbors up [{}], routes {}, sessions {}, switch entries {}, relayed {} packets",
            up.join(" "),
            self.routes.len(),
            self.sessions.len(),
            self.switch.len(),
            self.relayed
        ));
    }
}

//! IPv4 aliases (ADR-0021): local IPv4 addresses for vØrk nodes, handed out
//! by DNS (`A` queries) and translated to and from IPv6 at the node, so
//! programs and systems that only ask for IPv4 (Android's browser on a
//! network it thinks has no IPv6) reach `.vork` names too. Sessions carry
//! IPv6 only; an alias never leaves this machine.
//!
//! The range is `198.18.0.0/15` (reserved for testing, practically never in
//! use). **The owner's devices have the same address everywhere**, as in
//! Tailscale: derived from the node's address into `198.18.0.0/16` (each
//! node's own interface carries its derived address). Other nodes get local
//! aliases from `198.19.0.0`–`198.19.254.255`; `198.19.255.0/24` is the exit
//! clients' (ADR-0026). Two devices deriving the same address (unlikely: about
//! 0.3 % among 20) leave the second a local alias on that machine. The table
//! is kept in `<state>/aliases.json`, so a name keeps its alias across restarts.

use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;

use crate::identity::Hash;

/// The range: network and prefix length.
pub const NET: Ipv4Addr = Ipv4Addr::new(198, 18, 0, 0);
pub const PREFIX: u8 = 15;
/// A node's own IPv4 address, the same on every machine: `198.18.0.0/16` by
/// the first two bytes of its address (`198.18.0.x` left out).
pub fn derive(node: &Hash) -> Ipv4Addr {
    let v = u16::from_be_bytes([node[0], node[1]]);
    let v = if v < 256 { v + 256 } else { v };
    let [hi, lo] = v.to_be_bytes();
    Ipv4Addr::new(198, 18, hi, lo)
}

fn derived_range(a: Ipv4Addr) -> bool {
    a.octets()[..2] == [198, 18]
}
/// IPv4 packets become 20 bytes longer as IPv6: TCP segments are kept small
/// enough that the IPv6 packet still fits the interface's MTU.
const MSS: u16 = crate::tun::MTU - 40 - 20;

pub fn in_range(a: Ipv4Addr) -> bool {
    u32::from(a) >> (32 - PREFIX) == u32::from(NET) >> (32 - PREFIX)
}

pub struct Aliases {
    /// This node's own address (derived).
    pub me: Ipv4Addr,
    by_addr: HashMap<Ipv4Addr, Hash>,
    by_node: HashMap<Hash, Ipv4Addr>,
    file: Option<PathBuf>,
}

impl Aliases {
    pub fn open(dir: Option<&std::path::Path>, me: &Hash) -> Aliases {
        let file = dir.map(|d| d.join("aliases.json"));
        let mine = derive(me);
        let mut a = Aliases { me: mine, by_addr: HashMap::new(), by_node: HashMap::new(), file };
        a.by_addr.insert(mine, *me);
        a.by_node.insert(*me, mine);
        let saved: HashMap<String, String> =
            a.file.as_ref().and_then(|f| std::fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        for (ip, node) in saved {
            let (Ok(ip), Ok(Ok(h))) = (ip.parse::<Ipv4Addr>(), hex::decode(&node).map(|b| Hash::try_from(b.as_slice()))) else { continue };
            // Local aliases only: derived ones are worked out again (and older
            // tables, from before devices had derived addresses, give way).
            if in_range(ip) && !derived_range(ip) && !crate::exit::in_range(ip) && !a.by_node.contains_key(&h) {
                a.by_addr.insert(ip, h);
                a.by_node.insert(h, ip);
            }
        }
        a
    }

    /// The node's address: for one of the owner's devices its derived one
    /// (unless another node has it here), else a local alias, given now if it
    /// has none.
    pub fn of(&mut self, node: &Hash, device: bool) -> Option<Ipv4Addr> {
        if let Some(a) = self.by_node.get(node) {
            if !device || derived_range(*a) {
                return Some(*a);
            }
            // A local alias from before it became a device: its derived one now, if free.
            if self.by_addr.contains_key(&derive(node)) {
                return Some(*a);
            }
            let old = *a;
            self.by_addr.remove(&old);
            self.by_node.remove(node);
        }
        if device {
            let d = derive(node);
            if !self.by_addr.contains_key(&d) {
                self.by_addr.insert(d, *node);
                self.by_node.insert(*node, d);
                return Some(d);
            }
            crate::node::log(format!("{d} is another node's here already: {} gets a local IPv4 alias", hex::encode(&node[..4])));
        }
        let first = u32::from(Ipv4Addr::new(198, 19, 0, 1));
        let end = u32::from(crate::exit::NET); // the range's last /24 is for exit clients (ADR-0026)
        let ip = (first..end).map(Ipv4Addr::from).find(|ip| !self.by_addr.contains_key(ip))?;
        self.by_addr.insert(ip, *node);
        self.by_node.insert(*node, ip);
        self.save();
        Some(ip)
    }

    pub fn node(&self, ip: Ipv4Addr) -> Option<Hash> {
        self.by_addr.get(&ip).copied()
    }

    pub fn known(&self, node: &Hash) -> Option<Ipv4Addr> {
        self.by_node.get(node).copied()
    }

    /// The address a node would get, without giving it one (for showing).
    pub fn peek(&self, node: &Hash, device: bool) -> Option<Ipv4Addr> {
        match self.by_node.get(node) {
            Some(a) if !device || derived_range(*a) => Some(*a),
            _ if device && !self.by_addr.contains_key(&derive(node)) => Some(derive(node)),
            a => a.copied(),
        }
    }

    fn save(&self) {
        let Some(f) = &self.file else { return };
        let m: HashMap<String, String> = self.by_addr.iter().filter(|(ip, _)| !derived_range(**ip)).map(|(ip, h)| (ip.to_string(), hex::encode(h))).collect();
        if let Ok(b) = serde_json::to_vec_pretty(&m) {
            let tmp = f.with_extension("json.tmp");
            if std::fs::write(&tmp, b).is_ok() {
                let _ = std::fs::rename(tmp, f);
            }
        }
    }
}

/// The flow label a translated packet carries, so the far end gives it to its
/// programs as IPv4 again: a program there listening on IPv4 only (Docker's
/// published ports, many servers) is reached at its IPv4 address too. A node
/// that doesn't know the mark gives it IPv6, as before; a native IPv6 packet
/// that happens to carry the label (one flow in a million) is given as IPv4
/// and its answers go back as IPv6 all the same.
pub const FROM_V4: u32 = 0xf4f4f;

/// Whether an IPv6 packet was translated from IPv4 by the node that sent it.
pub fn from_v4(p: &[u8]) -> bool {
    p.len() >= 40 && p[0] >> 4 == 6 && (u32::from(p[1] & 15) << 16 | u32::from(p[2]) << 8 | u32::from(p[3])) == FROM_V4
}

/// Whether a packet given here as IPv4 opens a conversation (a TCP SYN, any
/// UDP): kept a moment, to be given as IPv6 instead if IPv4 is refused.
pub fn opens(v4: &[u8]) -> bool {
    let ihl = (v4[0] & 15) as usize * 4;
    match v4[9] {
        6 => v4.get(ihl + 13).is_some_and(|f| f & 0x12 == 0x02),
        17 => true,
        _ => false,
    }
}

/// A refusal a program here sends over IPv4 (a TCP reset, or ICMP port
/// unreachable for UDP), and the flow it refuses, as `to_v4` names it: a
/// server listening on one IPv4 address only (a NAS's web page on its LAN
/// address) refuses its vØrk one, and the conversation is given again as IPv6.
pub fn refusal_v4(p: &[u8]) -> Option<Flow> {
    if p.len() < 20 || p[0] >> 4 != 4 {
        return None;
    }
    let b = p.get((p[0] & 15) as usize * 4..)?;
    match p[9] {
        6 if b.len() >= 20 && b[13] & 0x04 != 0 => Some((6, port(b, 0), port(b, 2))),
        1 if b.len() >= 28 && b[0] == 3 && b[1] == 3 => {
            let inner = &b[8..];
            if inner[9] != 17 {
                return None;
            }
            let u = inner.get((inner[0] & 15) as usize * 4..)?.get(..4)?;
            Some((17, port(u, 2), port(u, 0)))
        }
        _ => None,
    }
}

/// What tells a translated conversation's replies apart: protocol (IPv6
/// numbering), this end's port (or echo id), the far end's port.
pub type Flow = (u8, u16, u16);

/// An IPv4 packet from this machine as IPv6 (`src`, `dst` its new ends), and
/// its flow. None: not something translated (fragments, other protocols).
pub fn to_v6(p: &[u8], src: Ipv6Addr, dst: Ipv6Addr) -> Option<(Vec<u8>, Flow)> {
    if p.len() < 20 || p[0] >> 4 != 4 {
        return None;
    }
    let ihl = (p[0] & 15) as usize * 4;
    let total = u16::from_be_bytes([p[2], p[3]]) as usize;
    let frag = u16::from_be_bytes([p[6], p[7]]);
    if ihl < 20 || total > p.len() || total < ihl || frag & 0x3fff != 0 {
        return None; // fragments aren't translated (they don't occur under this MTU)
    }
    let proto = p[9];
    let mut body = p[ihl..total].to_vec();
    let (next, flow) = match proto {
        6 if body.len() >= 20 => {
            clamp_mss(&mut body);
            (6, (6, port(&body, 0), port(&body, 2)))
        }
        17 if body.len() >= 8 => (17, (17, port(&body, 0), port(&body, 2))),
        1 if body.len() >= 8 && (body[0] == 8 || body[0] == 0) => {
            body[0] = if body[0] == 8 { 128 } else { 129 };
            (58, (58, port(&body, 4), 0))
        }
        _ => return None,
    };
    let mut out = Vec::with_capacity(40 + body.len());
    out.extend_from_slice(&[0x60 | (p[1] >> 4), p[1] << 4 | (FROM_V4 >> 16) as u8, (FROM_V4 >> 8) as u8, FROM_V4 as u8]);
    out.extend_from_slice(&(body.len() as u16).to_be_bytes());
    out.push(next);
    out.push(p[8].max(1));
    out.extend_from_slice(&src.octets());
    out.extend_from_slice(&dst.octets());
    out.extend_from_slice(&body);
    checksum_v6(&mut out);
    Some((out, flow))
}

/// An IPv6 packet for this machine as IPv4 (`src`, `dst` its new ends), and
/// its flow as `to_v6` named it (this end's port first).
pub fn to_v4(p: &[u8], src: Ipv4Addr, dst: Ipv4Addr) -> Option<(Vec<u8>, Flow)> {
    if p.len() < 40 || p[0] >> 4 != 6 {
        return None;
    }
    let len = u16::from_be_bytes([p[4], p[5]]) as usize;
    if p.len() < 40 + len {
        return None;
    }
    let mut body = p[40..40 + len].to_vec();
    let (proto, flow) = match p[6] {
        6 if body.len() >= 20 => (6, (6, port(&body, 2), port(&body, 0))),
        17 if body.len() >= 8 => (17, (17, port(&body, 2), port(&body, 0))),
        58 if body.len() >= 8 && (body[0] == 128 || body[0] == 129) => {
            body[0] = if body[0] == 128 { 8 } else { 0 };
            (1, (58, port(&body, 4), 0))
        }
        _ => return None,
    };
    let total = 20 + body.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&[0x45, (p[0] << 4) | (p[1] >> 4)]);
    out.extend_from_slice(&(total as u16).to_be_bytes());
    out.extend_from_slice(&[0, 0, 0x40, 0]); // id 0, don't fragment
    out.push(p[7].max(1));
    out.push(proto);
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&src.octets());
    out.extend_from_slice(&dst.octets());
    let c = !fold(sum(&out[..20]));
    out[10..12].copy_from_slice(&c.to_be_bytes());
    out.extend_from_slice(&body);
    checksum_v4(&mut out);
    Some((out, flow))
}

/// The flow of an IPv6 packet a program here sends (as `to_v6` names a
/// translated one's), so the same conversation begun over IPv6 isn't taken
/// for an IPv4 one (Windows pings both with the same echo id).
pub fn flow_v6_out(p: &[u8]) -> Option<Flow> {
    if p.len() < 48 || p[0] >> 4 != 6 {
        return None;
    }
    let b = &p[40..];
    match p[6] {
        6 | 17 => Some((p[6], port(b, 0), port(b, 2))),
        58 if b[0] == 128 => Some((58, port(b, 4), 0)),
        _ => None,
    }
}

fn port(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

/// A SYN's MSS option lowered to what fits after translation.
fn clamp_mss(tcp: &mut [u8]) {
    if tcp[13] & 0x02 == 0 {
        return;
    }
    let end = ((tcp[12] >> 4) as usize * 4).min(tcp.len());
    let mut i = 20;
    while i < end {
        match tcp[i] {
            0 => break,
            1 => i += 1,
            kind => {
                let Some(&n) = tcp.get(i + 1) else { break };
                if n < 2 || i + n as usize > end {
                    break;
                }
                if kind == 2 && n == 4 && port(tcp, i + 2) > MSS {
                    tcp[i + 2..i + 4].copy_from_slice(&MSS.to_be_bytes());
                }
                i += n as usize;
            }
        }
    }
}

fn sum(b: &[u8]) -> u32 {
    b.chunks(2).map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)]) as u32).sum()
}

fn fold(mut s: u32) -> u16 {
    while s >> 16 != 0 {
        s = (s & 0xffff) + (s >> 16);
    }
    s as u16
}

/// Where the transport checksum sits, by protocol (IPv6 numbering for ICMP).
fn checksum_at(proto: u8) -> Option<usize> {
    match proto {
        6 => Some(16),
        17 => Some(6),
        58 | 1 => Some(2),
        _ => None,
    }
}

/// The transport checksum of an IPv6 packet, over its pseudo-header.
fn checksum_v6(p: &mut [u8]) {
    let Some(at) = checksum_at(p[6]) else { return };
    let len = p.len() - 40;
    p[40 + at..40 + at + 2].copy_from_slice(&[0, 0]);
    let s = sum(&p[8..40]) + len as u32 + p[6] as u32 + sum(&p[40..]);
    let mut c = !fold(s);
    if c == 0 && p[6] == 17 {
        c = 0xffff;
    }
    p[40 + at..40 + at + 2].copy_from_slice(&c.to_be_bytes());
}

/// The transport checksum of an IPv4 packet (ICMP has no pseudo-header).
fn checksum_v4(p: &mut [u8]) {
    let Some(at) = checksum_at(p[9]) else { return };
    let len = p.len() - 20;
    p[20 + at..20 + at + 2].copy_from_slice(&[0, 0]);
    let pseudo = if p[9] == 1 { 0 } else { sum(&p[12..20]) + len as u32 + p[9] as u32 };
    let mut c = !fold(pseudo + sum(&p[20..]));
    if c == 0 && p[9] == 17 {
        c = 0xffff;
    }
    p[20 + at..20 + at + 2].copy_from_slice(&c.to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v4_tcp_syn() -> Vec<u8> {
        let mut tcp = vec![0u8; 24];
        tcp[0..2].copy_from_slice(&40000u16.to_be_bytes());
        tcp[2..4].copy_from_slice(&8742u16.to_be_bytes());
        tcp[12] = 6 << 4;
        tcp[13] = 0x02;
        tcp[20..24].copy_from_slice(&[2, 4, 0x05, 0xb4]); // MSS 1460
        let mut p = vec![0x45, 0, 0, 44, 0, 1, 0x40, 0, 64, 6, 0, 0, 198, 18, 0, 1, 198, 18, 0, 2];
        p[2..4].copy_from_slice(&((20 + tcp.len()) as u16).to_be_bytes());
        p.extend_from_slice(&tcp);
        checksum_v4(&mut p);
        p
    }

    fn valid_v6(p: &[u8]) -> bool {
        fold(sum(&p[8..40]) + (p.len() - 40) as u32 + p[6] as u32 + sum(&p[40..])) == 0xffff
    }

    fn valid_v4(p: &[u8]) -> bool {
        let pseudo = if p[9] == 1 { 0 } else { sum(&p[12..20]) + (p.len() - 20) as u32 + p[9] as u32 };
        fold(sum(&p[..20])) == 0xffff && fold(pseudo + sum(&p[20..])) == 0xffff
    }

    #[test]
    fn tcp_both_ways() {
        let me6: Ipv6Addr = "fd7e::1".parse().unwrap();
        let far6: Ipv6Addr = "fd5f::1".parse().unwrap();
        let (v6, flow) = to_v6(&v4_tcp_syn(), me6, far6).unwrap();
        assert_eq!(flow, (6, 40000, 8742));
        assert!(valid_v6(&v6));
        assert!(from_v4(&v6), "marked as translated");
        assert_eq!(port(&v6[40..], 22), MSS, "MSS lowered");
        // The answer, turned around: back to IPv4 with the same flow.
        let mut back = v6.clone();
        back[8..24].copy_from_slice(&far6.octets());
        back[24..40].copy_from_slice(&me6.octets());
        back[40..42].copy_from_slice(&8742u16.to_be_bytes());
        back[42..44].copy_from_slice(&40000u16.to_be_bytes());
        checksum_v6(&mut back);
        let (v4, flow) = to_v4(&back, Ipv4Addr::new(198, 18, 0, 2), Ipv4Addr::new(198, 18, 0, 1)).unwrap();
        assert_eq!(flow, (6, 40000, 8742));
        assert!(valid_v4(&v4));
        assert_eq!(&v4[12..16], &[198, 18, 0, 2]);
    }

    #[test]
    fn a_refusal_names_its_flow() {
        let syn = v4_tcp_syn();
        assert!(opens(&syn));
        assert_eq!(refusal_v4(&syn), None);
        // The server's reset for it: from its port to the opener's.
        let mut rst = syn.clone();
        rst[20..22].copy_from_slice(&8742u16.to_be_bytes());
        rst[22..24].copy_from_slice(&40000u16.to_be_bytes());
        rst[33] = 0x14; // RST, ACK
        assert!(!opens(&rst));
        assert_eq!(refusal_v4(&rst), Some((6, 8742, 40000)));
        // ICMP port unreachable for a UDP packet from 5353 to 53: the flow (17, 53, 5353).
        let mut icmp = vec![0x45, 0, 0, 56, 0, 0, 0x40, 0, 64, 1, 0, 0, 198, 18, 0, 2, 198, 18, 0, 1, 3, 3, 0, 0, 0, 0, 0, 0];
        icmp.extend_from_slice(&[0x45, 0, 0, 28, 0, 0, 0x40, 0, 64, 17, 0, 0, 198, 18, 0, 1, 198, 18, 0, 2]);
        icmp.extend_from_slice(&[0x14, 0xe9, 0, 53, 0, 8, 0, 0]);
        assert_eq!(refusal_v4(&icmp), Some((17, 53, 5353)));
    }

    #[test]
    fn ping_both_ways() {
        let mut icmp = vec![8, 0, 0, 0, 0x12, 0x34, 0, 1, b'h', b'i'];
        let mut p = vec![0x45, 0, 0, 0, 0, 1, 0x40, 0, 64, 1, 0, 0, 198, 18, 0, 1, 198, 18, 0, 2];
        p[2..4].copy_from_slice(&((20 + icmp.len()) as u16).to_be_bytes());
        p.append(&mut icmp);
        let (v6, flow) = to_v6(&p, "fd7e::1".parse().unwrap(), "fd5f::1".parse().unwrap()).unwrap();
        assert_eq!((v6[40], flow), (128, (58, 0x1234, 0)));
        assert!(valid_v6(&v6));
        let mut reply = v6.clone();
        reply[40] = 129;
        let (v4, flow) = to_v4(&reply, Ipv4Addr::new(198, 18, 0, 2), Ipv4Addr::new(198, 18, 0, 1)).unwrap();
        assert_eq!((v4[20], flow), (0, (58, 0x1234, 0)));
        assert!(valid_v4(&v4));
    }

    #[test]
    fn aliases_stay() {
        let dir = std::env::temp_dir().join(format!("vork-alias-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let me = [1u8; 16];
        let mut a = Aliases::open(Some(&dir), &me);
        assert_eq!(a.me, Ipv4Addr::new(198, 18, 1, 1));
        let n = [7u8; 16];
        let ip = a.of(&n, false).unwrap();
        assert_eq!(ip, Ipv4Addr::new(198, 19, 0, 1));
        assert_eq!(a.of(&[8u8; 16], false).unwrap(), Ipv4Addr::new(198, 19, 0, 2));
        // A device: its derived address, the same anywhere.
        assert_eq!(a.of(&[9u8; 16], true).unwrap(), derive(&[9u8; 16]));
        // One deriving the same as another device: a local alias instead.
        let mut twin = [9u8; 16];
        twin[15] = 0;
        assert_eq!(a.of(&twin, true).unwrap(), Ipv4Addr::new(198, 19, 0, 3));
        // Becoming a device moves a node from its local alias to its derived address.
        assert_eq!(a.of(&n, true).unwrap(), derive(&n));
        let b = Aliases::open(Some(&dir), &me);
        assert_eq!(b.node(Ipv4Addr::new(198, 19, 0, 2)), Some([8u8; 16]));
        std::fs::remove_dir_all(dir).unwrap();
    }
}

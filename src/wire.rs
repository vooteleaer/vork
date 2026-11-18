//! Messages between neighbors (inside the outer WireGuard session) and their
//! framing as plain UDP over IPv4.
//!
//! WireGuard carries IP packets, so every message travels inside the tunnel as
//! an ordinary UDP datagram to port [`L0_PORT`]. That keeps the layer portable:
//! a constrained device (ESP32 with ESPHome's WireGuard component: one peer,
//! IPv4 only) needs no WireGuard changes, only a UDP socket on the tunnel. The
//! addresses inside the tunnel are a fixed per-link convention and mean
//! nothing: identity lives in the messages, not in IP.

use crate::identity::{HASH_LEN, Hash};
use anyhow::{Result, bail};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// UDP port of the network layer inside a WireGuard tunnel.
pub const L0_PORT: u16 = 4799;
/// Link addresses inside a tunnel. A Lite node (e.g. ESPHome) is configured
/// with `address: 10.89.0.2` and `peer_allowed_ips: [10.89.0.1/32]`.
pub const LINK_UPSTREAM: [u8; 4] = [10, 89, 0, 1];
pub const LINK_LITE: [u8; 4] = [10, 89, 0, 2];
const IPV4_HDR: usize = 20;
const UDP_HDR: usize = 8;

fn ipv4_checksum(h: &[u8]) -> u16 {
    let mut sum: u32 = h.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]]) as u32).sum();
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// An IPv4/UDP datagram to [`L0_PORT`] (UDP checksum 0 is allowed over IPv4,
/// and the tunnel already authenticates every byte).
pub fn frame(payload: &[u8]) -> Vec<u8> {
    let total = IPV4_HDR + UDP_HDR + payload.len();
    let mut p = Vec::with_capacity(total);
    p.extend_from_slice(&[0x45, 0]);
    p.extend_from_slice(&(total as u16).to_be_bytes());
    p.extend_from_slice(&[0, 0, 0x40, 0, 64, 17, 0, 0]); // id, DF, ttl, UDP, checksum
    p.extend_from_slice(&LINK_UPSTREAM);
    p.extend_from_slice(&LINK_LITE);
    let c = ipv4_checksum(&p[..IPV4_HDR]);
    p[10..12].copy_from_slice(&c.to_be_bytes());
    p.extend_from_slice(&L0_PORT.to_be_bytes());
    p.extend_from_slice(&L0_PORT.to_be_bytes());
    p.extend_from_slice(&((UDP_HDR + payload.len()) as u16).to_be_bytes());
    p.extend_from_slice(&[0, 0]);
    p.extend_from_slice(payload);
    p
}

/// The payload of an IPv4/UDP datagram to [`L0_PORT`]; addresses are ignored.
pub fn unframe(packet: &[u8]) -> Option<&[u8]> {
    if packet.len() < IPV4_HDR + UDP_HDR || packet[0] >> 4 != 4 || packet[9] != 17 {
        return None;
    }
    let ihl = (packet[0] & 0x0f) as usize * 4;
    let udp = packet.get(ihl..)?;
    if udp.len() < UDP_HDR || u16::from_be_bytes([udp[2], udp[3]]) != L0_PORT {
        return None;
    }
    let len = u16::from_be_bytes([udp[4], udp[5]]) as usize;
    udp.get(UDP_HDR..len)
}

#[derive(Debug, Clone)]
pub enum Msg {
    /// Binds the neighbor's WireGuard key to its signing key, and says what
    /// the neighbor does ([`FLAG_RELAY`] unset: a Lite node).
    Hello { ed_pub: [u8; 32], flags: u8, sig: [u8; 64] },
    /// "This is the address I see you at" (STUN from peers).
    Observed { addr: SocketAddr },
    /// A destination's signed announce; `hops` is added by forwarders and is
    /// only a hint (it isn't signed and is never trusted on its own).
    Announce(Announce),
    /// Opens a path toward `dest`. Each hop picks a fresh label for the next
    /// link and remembers the pair, so a label means something only on one
    /// link and colluding relays can't match packets by it.
    PathOpen { label: u64, dest: Hash, ttl: u8 },
    /// The destination's answer, travelling back along the path.
    PathAck { label: u64 },
    /// The path is broken at some hop; sent toward the end that can act on it.
    PathErr { label: u64 },
    /// End-to-end traffic (inner WireGuard) on a path. No source, no destination.
    Data { label: u64, inner: Vec<u8> },
    /// Path health: the origin sends, the far end echoes (`reply`).
    Probe { label: u64, nonce: u64, reply: bool },
    /// "Where is `dest`?" Lite nodes don't receive floods; they ask. Answered
    /// with the destination's announce.
    PathReq { dest: Hash, nonce: u64 },
}

#[derive(Debug, Clone)]
pub struct Announce {
    pub ed_pub: [u8; 32],
    pub x_pub: [u8; 32],
    pub seq: u64,
    pub flags: u8,
    pub sig: [u8; 64],
    pub hops: u8,
}

pub const FLAG_RELAY: u8 = 1;

impl Announce {
    pub fn signed_part(hash: &Hash, x_pub: &[u8; 32], seq: u64, flags: u8) -> Vec<u8> {
        let mut m = hash.to_vec();
        m.extend_from_slice(x_pub);
        m.extend_from_slice(&seq.to_be_bytes());
        m.push(flags);
        m
    }
}

const T_HELLO: u8 = 1;
const T_OBSERVED: u8 = 2;
const T_ANNOUNCE: u8 = 3;
const T_PATH_REQ: u8 = 5;
const T_PATH_OPEN: u8 = 6;
const T_PATH_ACK: u8 = 7;
const T_PATH_ERR: u8 = 8;
const T_DATA: u8 = 9;
const T_PROBE: u8 = 10;

impl Msg {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::new();
        match self {
            Msg::Hello { ed_pub, flags, sig } => {
                b.push(T_HELLO);
                b.extend_from_slice(ed_pub);
                b.push(*flags);
                b.extend_from_slice(sig);
            }
            Msg::Observed { addr } => {
                b.push(T_OBSERVED);
                put_addr(&mut b, addr);
            }
            Msg::Announce(a) => {
                b.push(T_ANNOUNCE);
                b.extend_from_slice(&a.ed_pub);
                b.extend_from_slice(&a.x_pub);
                b.extend_from_slice(&a.seq.to_be_bytes());
                b.push(a.flags);
                b.extend_from_slice(&a.sig);
                b.push(a.hops);
            }
            Msg::PathOpen { label, dest, ttl } => {
                b.push(T_PATH_OPEN);
                b.extend_from_slice(&label.to_be_bytes());
                b.extend_from_slice(dest);
                b.push(*ttl);
            }
            Msg::PathAck { label } => {
                b.push(T_PATH_ACK);
                b.extend_from_slice(&label.to_be_bytes());
            }
            Msg::PathErr { label } => {
                b.push(T_PATH_ERR);
                b.extend_from_slice(&label.to_be_bytes());
            }
            Msg::Data { label, inner } => {
                b.push(T_DATA);
                b.extend_from_slice(&label.to_be_bytes());
                b.extend_from_slice(inner);
            }
            Msg::Probe { label, nonce, reply } => {
                b.push(T_PROBE);
                b.extend_from_slice(&label.to_be_bytes());
                b.extend_from_slice(&nonce.to_be_bytes());
                b.push(*reply as u8);
            }
            Msg::PathReq { dest, nonce } => {
                b.push(T_PATH_REQ);
                b.extend_from_slice(dest);
                b.extend_from_slice(&nonce.to_be_bytes());
            }
        }
        b
    }

    pub fn decode(b: &[u8]) -> Result<Msg> {
        let mut r = Reader { b, i: 0 };
        Ok(match r.u8()? {
            T_HELLO => Msg::Hello { ed_pub: r.arr()?, flags: r.u8()?, sig: r.arr()? },
            T_OBSERVED => Msg::Observed { addr: get_addr(&mut r)? },
            T_ANNOUNCE => Msg::Announce(Announce {
                ed_pub: r.arr()?,
                x_pub: r.arr()?,
                seq: u64::from_be_bytes(r.arr()?),
                flags: r.u8()?,
                sig: r.arr()?,
                hops: r.u8()?,
            }),
            T_PATH_OPEN => Msg::PathOpen { label: r.u64()?, dest: r.arr::<HASH_LEN>()?, ttl: r.u8()? },
            T_PATH_ACK => Msg::PathAck { label: r.u64()? },
            T_PATH_ERR => Msg::PathErr { label: r.u64()? },
            T_DATA => Msg::Data { label: r.u64()?, inner: r.rest().to_vec() },
            T_PROBE => Msg::Probe { label: r.u64()?, nonce: r.u64()?, reply: r.u8()? != 0 },
            T_PATH_REQ => Msg::PathReq { dest: r.arr()?, nonce: u64::from_be_bytes(r.arr()?) },
            t => bail!("unknown message type {t}"),
        })
    }
}

pub fn put_addr(b: &mut Vec<u8>, addr: &SocketAddr) {
    match addr.ip() {
        IpAddr::V4(ip) => {
            b.push(4);
            b.extend_from_slice(&ip.octets());
        }
        IpAddr::V6(ip) => {
            b.push(6);
            b.extend_from_slice(&ip.octets());
        }
    }
    b.extend_from_slice(&addr.port().to_be_bytes());
}

fn get_addr(r: &mut Reader) -> Result<SocketAddr> {
    let ip = match r.u8()? {
        4 => IpAddr::V4(Ipv4Addr::from(r.arr::<4>()?)),
        6 => IpAddr::V6(Ipv6Addr::from(r.arr::<16>()?)),
        _ => bail!("bad address family"),
    };
    Ok(SocketAddr::new(ip, u16::from_be_bytes(r.arr()?)))
}

/// A list of addresses (candidates for a direct link): count, then each address.
pub fn encode_addrs(addrs: &[SocketAddr]) -> Vec<u8> {
    let mut b = vec![addrs.len().min(255) as u8];
    for a in addrs.iter().take(255) {
        put_addr(&mut b, a);
    }
    b
}

pub fn decode_addrs(b: &[u8]) -> Result<Vec<SocketAddr>> {
    let mut r = Reader { b, i: 0 };
    let n = r.u8()?;
    (0..n).map(|_| get_addr(&mut r)).collect()
}

pub struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl Reader<'_> {
    fn u8(&mut self) -> Result<u8> {
        Ok(self.arr::<1>()?[0])
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.arr()?))
    }
    fn arr<const N: usize>(&mut self) -> Result<[u8; N]> {
        let Some(s) = self.b.get(self.i..self.i + N) else { bail!("message too short") };
        self.i += N;
        Ok(s.try_into().unwrap())
    }
    fn rest(&mut self) -> &[u8] {
        let s = &self.b[self.i..];
        self.i = self.b.len();
        s
    }
}

/// WireGuard message fields that endpoints read in the clear to find the
/// session a packet belongs to (type and receiver index; nothing encrypted).
pub mod wg {
    pub const INIT: u32 = 1;
    pub const RESP: u32 = 2;
    pub const COOKIE: u32 = 3;
    pub const DATA: u32 = 4;

    pub fn kind(p: &[u8]) -> Option<u32> {
        Some(u32::from_le_bytes(p.get(0..4)?.try_into().ok()?))
    }
    fn u32_at(p: &[u8], at: usize) -> Option<u32> {
        Some(u32::from_le_bytes(p.get(at..at + 4)?.try_into().ok()?))
    }
    /// The index of the session at the receiving end (all but initiation).
    pub fn receiver_idx(p: &[u8]) -> Option<u32> {
        match kind(p)? {
            RESP => u32_at(p, 8),
            COOKIE | DATA => u32_at(p, 4),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let msgs = [
            Msg::Hello { ed_pub: [1; 32], flags: 1, sig: [2; 64] },
            Msg::PathReq { dest: [7; 16], nonce: 42 },
            Msg::Observed { addr: "1.2.3.4:5678".parse().unwrap() },
            Msg::Observed { addr: "[2001:db8::1]:51820".parse().unwrap() },
            Msg::Announce(Announce { ed_pub: [3; 32], x_pub: [4; 32], seq: 7, flags: 1, sig: [5; 64], hops: 2 }),
            Msg::PathOpen { label: 1, dest: [6; 16], ttl: 9 },
            Msg::PathAck { label: 2 },
            Msg::PathErr { label: 3 },
            Msg::Data { label: 4, inner: vec![1, 2, 3] },
            Msg::Data { label: 5, inner: vec![] },
            Msg::Probe { label: 6, nonce: 7, reply: true },
        ];
        for m in msgs {
            let back = Msg::decode(&m.encode()).unwrap();
            assert_eq!(format!("{m:?}"), format!("{back:?}"));
        }
    }

    #[test]
    fn addrs() {
        let a: Vec<SocketAddr> = vec!["1.2.3.4:5".parse().unwrap(), "[::1]:6".parse().unwrap()];
        assert_eq!(decode_addrs(&encode_addrs(&a)).unwrap(), a);
    }

    #[test]
    fn framing() {
        let p = frame(b"hello");
        assert_eq!(unframe(&p), Some(&b"hello"[..]));
        assert_eq!(unframe(&p[..25]), None);
        // A valid header checksum sums to zero.
        assert_eq!(ipv4_checksum(&p[..20]), 0);
    }
}

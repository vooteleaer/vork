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
/// Link addresses inside a tunnel. A leaf device (e.g. ESPHome) is configured
/// with `address: 10.89.0.2` and `peer_allowed_ips: [10.89.0.1/32]`.
pub const LINK_UPSTREAM: [u8; 4] = [10, 89, 0, 1];
pub const LINK_LEAF: [u8; 4] = [10, 89, 0, 2];
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
    p.extend_from_slice(&LINK_LEAF);
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
    /// Binds the neighbor's WireGuard key to its signing key.
    Hello { ed_pub: [u8; 32], sig: [u8; 64] },
    /// "This is the address I see you at" (STUN from peers).
    Observed { addr: SocketAddr },
    /// A destination's signed announce; `hops` is added by forwarders and is
    /// only a hint (it isn't signed and is never trusted on its own).
    Announce(Announce),
    /// Something for a destination further away. `dest` is set only on an
    /// inner handshake initiation; everything after it is switched by the
    /// inner WireGuard receiver index, so relays don't see the destination
    /// in every packet, and no packet carries a source.
    Fwd { ttl: u8, dest: Option<Hash>, inner: Vec<u8> },
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
const T_FWD: u8 = 4;

impl Msg {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::new();
        match self {
            Msg::Hello { ed_pub, sig } => {
                b.push(T_HELLO);
                b.extend_from_slice(ed_pub);
                b.extend_from_slice(sig);
            }
            Msg::Observed { addr } => {
                b.push(T_OBSERVED);
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
            Msg::Announce(a) => {
                b.push(T_ANNOUNCE);
                b.extend_from_slice(&a.ed_pub);
                b.extend_from_slice(&a.x_pub);
                b.extend_from_slice(&a.seq.to_be_bytes());
                b.push(a.flags);
                b.extend_from_slice(&a.sig);
                b.push(a.hops);
            }
            Msg::Fwd { ttl, dest, inner } => {
                b.push(T_FWD);
                b.push(*ttl);
                match dest {
                    Some(d) => {
                        b.push(1);
                        b.extend_from_slice(d);
                    }
                    None => b.push(0),
                }
                b.extend_from_slice(inner);
            }
        }
        b
    }

    pub fn decode(b: &[u8]) -> Result<Msg> {
        let mut r = Reader { b, i: 0 };
        Ok(match r.u8()? {
            T_HELLO => Msg::Hello { ed_pub: r.arr()?, sig: r.arr()? },
            T_OBSERVED => {
                let ip = match r.u8()? {
                    4 => IpAddr::V4(Ipv4Addr::from(r.arr::<4>()?)),
                    6 => IpAddr::V6(Ipv6Addr::from(r.arr::<16>()?)),
                    _ => bail!("bad address family"),
                };
                let port = u16::from_be_bytes(r.arr()?);
                Msg::Observed { addr: SocketAddr::new(ip, port) }
            }
            T_ANNOUNCE => Msg::Announce(Announce {
                ed_pub: r.arr()?,
                x_pub: r.arr()?,
                seq: u64::from_be_bytes(r.arr()?),
                flags: r.u8()?,
                sig: r.arr()?,
                hops: r.u8()?,
            }),
            T_FWD => {
                let ttl = r.u8()?;
                let dest = match r.u8()? {
                    0 => None,
                    1 => Some(r.arr::<HASH_LEN>()?),
                    _ => bail!("bad dest flag"),
                };
                Msg::Fwd { ttl, dest, inner: r.rest().to_vec() }
            }
            t => bail!("unknown message type {t}"),
        })
    }
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl Reader<'_> {
    fn u8(&mut self) -> Result<u8> {
        Ok(self.arr::<1>()?[0])
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

/// WireGuard message fields that relays and endpoints read in the clear to
/// switch packets (type and indices; never anything encrypted).
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
    /// The sender's index (initiation and response).
    pub fn sender_idx(p: &[u8]) -> Option<u32> {
        match kind(p)? {
            INIT | RESP => u32_at(p, 4),
            _ => None,
        }
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
            Msg::Hello { ed_pub: [1; 32], sig: [2; 64] },
            Msg::Observed { addr: "1.2.3.4:5678".parse().unwrap() },
            Msg::Observed { addr: "[2001:db8::1]:51820".parse().unwrap() },
            Msg::Announce(Announce { ed_pub: [3; 32], x_pub: [4; 32], seq: 7, flags: 1, sig: [5; 64], hops: 2 }),
            Msg::Fwd { ttl: 9, dest: Some([6; 16]), inner: vec![1, 2, 3] },
            Msg::Fwd { ttl: 9, dest: None, inner: vec![] },
        ];
        for m in msgs {
            let back = Msg::decode(&m.encode()).unwrap();
            assert_eq!(format!("{m:?}"), format!("{back:?}"));
        }
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

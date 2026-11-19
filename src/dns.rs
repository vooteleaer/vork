//! Names for vØrk addresses (ADR-0025): `<name>.vork` for this node and the
//! owner's devices, `<address>.vork` for any node.
//!
//! The resolver lives at `fd00::53`, inside the `fd00::/8` route, so its
//! queries come to the node through the interface like any other packet: no
//! port to bind, no privilege. On Linux, systemd-resolved sends it only the
//! `.vork` domain; on Android the VPN makes it the phone's DNS server and the
//! node passes everything else on to the network's own servers.

use std::net::{Ipv6Addr, SocketAddr};
use std::time::Duration;

/// The resolver's address.
pub const ADDR: Ipv6Addr = Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 0x53);
/// The domain.
pub const DOMAIN: &str = "vork";
/// How long answers may be kept.
const TTL: u32 = 60;

pub const NOERROR: u8 = 0;
pub const SERVFAIL: u8 = 2;
pub const NXDOMAIN: u8 = 3;
pub const REFUSED: u8 = 5;
const AAAA: u16 = 28;

/// A node's name as a DNS label: lower case, runs of anything but letters,
/// digits and `-` made one `-`, at most 40 characters (as the apps show it).
pub fn label(name: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in name.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' {
            out.push(c);
            dash = false;
        } else if !dash {
            out.push('-');
            dash = true;
        }
    }
    out.trim_matches('-').chars().take(40).collect::<String>().trim_matches('-').to_string()
}

/// A DNS query that came through the interface.
pub struct Query {
    pub from: Ipv6Addr,
    pub port: u16,
    /// The DNS message.
    pub msg: Vec<u8>,
}

/// The query in an IPv6 packet for the resolver (UDP to `fd00::53` port 53).
pub fn query_in(p: &[u8]) -> Option<Query> {
    if p.len() < 48 || p[0] >> 4 != 6 || p[6] != 17 || p[24..40] != ADDR.octets() {
        return None;
    }
    let len = u16::from_be_bytes([p[44], p[45]]) as usize;
    if u16::from_be_bytes([p[42], p[43]]) != 53 || len < 8 || p.len() < 40 + len {
        return None;
    }
    let from = Ipv6Addr::from(<[u8; 16]>::try_from(&p[8..24]).unwrap());
    Some(Query { from, port: u16::from_be_bytes([p[40], p[41]]), msg: p[48..40 + len].to_vec() })
}

/// The question of a DNS message: its name (lower case, no trailing dot),
/// type, and where the question ends.
pub fn question(msg: &[u8]) -> Option<(String, u16, usize)> {
    if msg.len() < 12 || msg[2] & 0x80 != 0 || u16::from_be_bytes([msg[4], msg[5]]) != 1 {
        return None;
    }
    let mut i = 12;
    let mut name = String::new();
    loop {
        let n = *msg.get(i)? as usize;
        i += 1;
        if n == 0 {
            break;
        }
        if n > 63 || name.len() > 253 {
            return None;
        }
        let part = msg.get(i..i + n)?;
        if !name.is_empty() {
            name.push('.');
        }
        name.push_str(&String::from_utf8_lossy(part).to_lowercase());
        i += n;
    }
    let qtype = u16::from_be_bytes([*msg.get(i)?, *msg.get(i + 1)?]);
    Some((name, qtype, i + 4))
}

/// The answer to `msg`: `rcode`, and the address when there is one and it was
/// asked for (AAAA). Only the question is kept from the query.
pub fn answer(msg: &[u8], end: usize, qtype: u16, rcode: u8, addr: Option<Ipv6Addr>) -> Vec<u8> {
    let give = rcode == NOERROR && qtype == AAAA && addr.is_some();
    let mut out = Vec::with_capacity(end + 28);
    out.extend_from_slice(&msg[..2]);
    out.push(0x84 | (msg[2] & 0x79)); // response, authoritative, opcode and RD kept
    out.push(0x80 | rcode); // recursion available
    out.extend_from_slice(&[0, 1, 0, give as u8, 0, 0, 0, 0]);
    out.extend_from_slice(&msg[12..end]);
    if let (true, Some(a)) = (give, addr) {
        out.extend_from_slice(&[0xc0, 12]);
        out.extend_from_slice(&AAAA.to_be_bytes());
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&TTL.to_be_bytes());
        out.extend_from_slice(&16u16.to_be_bytes());
        out.extend_from_slice(&a.octets());
    }
    out
}

/// A reply packet: UDP from the resolver to whoever asked.
pub fn reply(to: &Query, msg: &[u8]) -> Vec<u8> {
    let len = (8 + msg.len()) as u16;
    let mut p = Vec::with_capacity(40 + len as usize);
    p.extend_from_slice(&[0x60, 0, 0, 0]);
    p.extend_from_slice(&len.to_be_bytes());
    p.extend_from_slice(&[17, 64]);
    p.extend_from_slice(&ADDR.octets());
    p.extend_from_slice(&to.from.octets());
    p.extend_from_slice(&53u16.to_be_bytes());
    p.extend_from_slice(&to.port.to_be_bytes());
    p.extend_from_slice(&len.to_be_bytes());
    p.extend_from_slice(&[0, 0]);
    p.extend_from_slice(msg);
    // The checksum, over the pseudo-header and the datagram (required in IPv6).
    let mut sum: u32 = 0;
    let mut add = |b: &[u8]| {
        for c in b.chunks(2) {
            sum += u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)]) as u32;
        }
    };
    add(&p[8..40]);
    add(&(len as u32).to_be_bytes());
    add(&[0, 0, 0, 17]);
    add(&p[40..]);
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    let c = !(sum as u16);
    p[46..48].copy_from_slice(&(if c == 0 { 0xffff } else { c }).to_be_bytes());
    p
}

/// Asks the network's own servers (names outside `.vork`), each in turn.
pub async fn forward(upstream: Vec<SocketAddr>, msg: Vec<u8>) -> Option<Vec<u8>> {
    for server in upstream {
        let bind: SocketAddr = if server.is_ipv4() { "0.0.0.0:0".parse().unwrap() } else { "[::]:0".parse().unwrap() };
        let Ok(s) = tokio::net::UdpSocket::bind(bind).await else { continue };
        if s.send_to(&msg, server).await.is_err() {
            continue;
        }
        let mut buf = vec![0u8; 4096];
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while let Ok(Ok((n, from))) = tokio::time::timeout_at(deadline, s.recv_from(&mut buf)).await {
            if from.ip() == server.ip() && n >= 2 && buf[..2] == msg[..2] {
                return Some(buf[..n].to_vec());
            }
        }
    }
    None
}

/// Tells the system's resolver to ask this node about `.vork` (Linux:
/// systemd-resolved; Windows: a name resolution policy rule).
pub fn register(interface: &str) {
    #[cfg(target_os = "linux")]
    {
        let run = |args: &[&str]| std::process::Command::new("resolvectl").args(args).output();
        match run(&["dns", interface, &ADDR.to_string()]).and_then(|o| {
            if o.status.success() { run(&["domain", interface, "~vork"]) } else { Ok(o) }
        }) {
            Ok(o) if o.status.success() => crate::node::log(format!("names: .vork asked of {ADDR} (systemd-resolved)")),
            Ok(o) => crate::node::log(format!(
                "names: systemd-resolved didn't take .vork ({}); point .vork at {ADDR} yourself",
                String::from_utf8_lossy(&o.stderr).trim()
            )),
            Err(_) => crate::node::log(format!("names: no systemd-resolved here; point .vork at {ADDR} to use them")),
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = interface;
        let script = format!(
            "if (-not (Get-DnsClientNrptRule | Where-Object {{ $_.Namespace -contains '.vork' }})) {{ Add-DnsClientNrptRule -Namespace '.vork' -NameServers '{ADDR}' }}"
        );
        let ok = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .creation_flags(0x0800_0000)
            .status()
            .is_ok_and(|s| s.success());
        crate::node::log(if ok { format!("names: .vork asked of {ADDR}") } else { "names: couldn't add the .vork rule".to_string() });
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    let _ = interface;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query_packet(name: &str, qtype: u16) -> Vec<u8> {
        let mut msg = vec![0xab, 0xcd, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
        for part in name.split('.') {
            msg.push(part.len() as u8);
            msg.extend_from_slice(part.as_bytes());
        }
        msg.extend_from_slice(&[0]);
        msg.extend_from_slice(&qtype.to_be_bytes());
        msg.extend_from_slice(&[0, 1]);
        let me: Ipv6Addr = "fd5f:7a9a:c63d:c07c:1176:bff9:51d6:a264".parse().unwrap();
        let q = Query { from: me, port: 40000, msg: msg.clone() };
        // A query is a reply turned around: reuse the builder, then swap the ends
        // (the checksum stays right: the sum doesn't depend on their order).
        let mut p = reply(&q, &msg);
        p[8..24].copy_from_slice(&me.octets());
        p[24..40].copy_from_slice(&ADDR.octets());
        p[40..42].copy_from_slice(&40000u16.to_be_bytes());
        p[42..44].copy_from_slice(&53u16.to_be_bytes());
        p
    }

    #[test]
    fn labels_match_the_apps() {
        assert_eq!(label("cafa-cloud"), "cafa-cloud");
        assert_eq!(label("Vootele's Pixel 8"), "vootele-s-pixel-8");
        assert_eq!(label("  M2101K6G "), "m2101k6g");
        assert_eq!(label("ÄÖ"), "");
    }

    #[test]
    fn answers_aaaa() {
        let p = query_packet("cafa-cloud.vork", AAAA);
        let q = query_in(&p).unwrap();
        assert_eq!(q.port, 40000);
        let (name, qtype, end) = question(&q.msg).unwrap();
        assert_eq!((name.as_str(), qtype), ("cafa-cloud.vork", AAAA));
        let a: Ipv6Addr = "fd5f::1".parse().unwrap();
        let r = answer(&q.msg, end, qtype, NOERROR, Some(a));
        assert_eq!(&r[..2], &[0xab, 0xcd]);
        assert_eq!(r[3] & 15, NOERROR);
        assert_eq!(u16::from_be_bytes([r[6], r[7]]), 1);
        assert_eq!(&r[r.len() - 16..], &a.octets());
        // Asked for A: no records, no error.
        let r = answer(&q.msg, end, 1, NOERROR, Some(a));
        assert_eq!(u16::from_be_bytes([r[6], r[7]]), 0);
        let pkt = reply(&q, &r);
        assert_eq!(&pkt[24..40], &q.from.octets());
    }

    #[test]
    fn checksum_verifies() {
        let p = query_packet("x.vork", AAAA);
        let len = u16::from_be_bytes([p[4], p[5]]) as u32;
        let mut sum: u32 = 0;
        for c in p[8..40].chunks(2).chain(p[40..].chunks(2)) {
            sum += u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)]) as u32;
        }
        sum += len + 17;
        while sum >> 16 != 0 {
            sum = (sum & 0xffff) + (sum >> 16);
        }
        assert_eq!(sum, 0xffff);
    }
}

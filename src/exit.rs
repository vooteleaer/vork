//! Exit nodes (ADR-0026): a device's internet traffic (IPv4) goes out through
//! another of the owner's nodes.
//!
//! The device sends its IPv4 packets, source `198.18.0.1` (its own alias
//! address, ADR-0021), to the exit node in its identified session. The exit
//! node gives each device an address in `198.19.255.0/24`, writes the packets
//! onto its interface with that source, and the system forwards them out with
//! NAT; answers come back to the interface, to the device's address, and go
//! back with the destination `198.18.0.1`. Only the owner's devices may use it.
//!
//! Where the system can't do that (Windows, Android, a Linux without
//! iptables), and for IPv6 everywhere, the node's own stack carries it
//! instead (`netstack`): a device's packets keep its own address and come
//! back to it.

use std::net::Ipv4Addr;

use crate::identity::Hash;

/// The exit's addresses for the devices using it.
pub const NET: Ipv4Addr = Ipv4Addr::new(198, 19, 255, 0);
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const PREFIX: u8 = 24;

pub fn in_range(a: Ipv4Addr) -> bool {
    a.octets()[..3] == NET.octets()[..3]
}

/// The devices using this node as their exit, by address.
#[derive(Default)]
pub struct Clients {
    by_addr: std::collections::HashMap<Ipv4Addr, Hash>,
    by_node: std::collections::HashMap<Hash, Ipv4Addr>,
}

impl Clients {
    pub fn of(&mut self, node: &Hash) -> Option<Ipv4Addr> {
        if let Some(a) = self.by_node.get(node) {
            return Some(*a);
        }
        let ip = (2..255u8).map(|i| Ipv4Addr::new(198, 19, 255, i)).find(|ip| !self.by_addr.contains_key(ip))?;
        self.by_addr.insert(ip, *node);
        self.by_node.insert(*node, ip);
        Some(ip)
    }

    pub fn node(&self, ip: Ipv4Addr) -> Option<Hash> {
        self.by_addr.get(&ip).copied()
    }
}

/// Whether an exit forwards to `a` without `exit_lan`: the internet only, not
/// the exit's own networks (private, shared, link-local, loopback, multicast,
/// and the vØrk ranges themselves).
pub fn internet(a: Ipv4Addr) -> bool {
    let o = a.octets();
    !(a.is_private()
        || a.is_loopback()
        || a.is_link_local()
        || a.is_multicast()
        || a.is_broadcast()
        || a.is_unspecified()
        || (o[0] == 100 && (o[1] & 0xc0) == 64) // 100.64.0.0/10 (carrier NAT, Tailscale)
        || (o[0] == 198 && (o[1] & 0xfe) == 18) // 198.18.0.0/15 (vØrk's own)
        || o[0] >= 240)
}

/// IPv6's counterpart of [`internet`]: global unicast only (2000::/3), so
/// not the exit's own networks (unique local, link-local), loopback,
/// multicast, IPv4-mapped or documentation addresses.
pub fn internet6(a: std::net::Ipv6Addr) -> bool {
    crate::netstack::global6(a)
}

/// Public resolvers asked through the exit (Quad9, Cloudflare): the network
/// the device is on doesn't see its names (the device's own network's
/// servers may be private addresses the exit can't reach).
pub const DNS: [Ipv4Addr; 2] = [Ipv4Addr::new(9, 9, 9, 9), Ipv4Addr::new(1, 1, 1, 1)];

/// The resolvers this machine uses: an exit asks them its clients' names
/// (ADR-0026). Unix: `nameserver` in /etc/resolv.conf. Windows: those of
/// the connected adapters with a gateway (the machine's own networks, not
/// vØrk's interface), the lowest metric first. Android has neither: the
/// node's `dns_upstream` (the phone's network's) is asked instead.
#[cfg(not(windows))]
pub fn system_resolvers() -> Vec<std::net::SocketAddr> {
    resolvers_in(&std::fs::read_to_string("/etc/resolv.conf").unwrap_or_default())
}

#[cfg(windows)]
pub fn system_resolvers() -> Vec<std::net::SocketAddr> {
    use windows_sys::Win32::NetworkManagement::IpHelper::*;
    use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6, AF_UNSPEC, SOCKADDR_IN, SOCKADDR_IN6};
    const INCLUDE_GATEWAYS: u32 = 0x80;
    let mut size: u32 = 32 * 1024;
    let mut buf: Vec<u64> = Vec::new();
    for _ in 0..3 {
        buf = vec![0u64; (size as usize).div_ceil(8)];
        let r = unsafe { GetAdaptersAddresses(AF_UNSPEC as u32, GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | INCLUDE_GATEWAYS, std::ptr::null(), buf.as_mut_ptr().cast(), &mut size) };
        if r == 0 {
            break;
        }
        if r != 111 {
            return Vec::new(); // not ERROR_BUFFER_OVERFLOW
        }
    }
    let mut found: Vec<(u32, std::net::SocketAddr)> = Vec::new();
    unsafe {
        let mut a = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
        while !a.is_null() {
            let ad = &*a;
            // Up (1), and with a gateway: a network of the machine's own.
            if ad.OperStatus == 1 && !ad.FirstGatewayAddress.is_null() {
                let mut d = ad.FirstDnsServerAddress;
                while !d.is_null() {
                    let sa = (*d).Address.lpSockaddr;
                    if !sa.is_null() {
                        let addr: Option<std::net::IpAddr> = match (*sa).sa_family {
                            AF_INET => Some(std::net::Ipv4Addr::from(u32::from_be((*(sa as *const SOCKADDR_IN)).sin_addr.S_un.S_addr)).into()),
                            AF_INET6 => Some(std::net::Ipv6Addr::from((*(sa as *const SOCKADDR_IN6)).sin6_addr.u.Byte).into()),
                            _ => None,
                        };
                        // fec0:0:0:ffff::/48: Windows' placeholders where none is set.
                        if let Some(ip) = addr.filter(|ip| !matches!(ip, std::net::IpAddr::V6(v6) if v6.segments()[0] == 0xfec0 || v6.segments()[0] & 0xffc0 == 0xfe80)) {
                            found.push((ad.Ipv4Metric, std::net::SocketAddr::new(ip, 53)));
                        }
                    }
                    d = (*d).Next;
                }
            }
            a = ad.Next;
        }
    }
    found.sort_by_key(|(m, a)| (*m, a.is_ipv6()));
    let mut out: Vec<std::net::SocketAddr> = Vec::new();
    for (_, a) in found {
        if !out.contains(&a) {
            out.push(a);
        }
    }
    out.truncate(3);
    out
}

#[cfg_attr(windows, allow(dead_code))]
fn resolvers_in(text: &str) -> Vec<std::net::SocketAddr> {
    text.lines()
        .filter_map(|l| l.trim().strip_prefix("nameserver"))
        .filter_map(|a| a.trim().parse::<std::net::IpAddr>().ok())
        .map(|ip| std::net::SocketAddr::new(ip, 53))
        .take(3)
        .collect()
}

/// A UDP packet over IPv4, checksums made.
pub fn udp4(src: Ipv4Addr, sport: u16, dst: Ipv4Addr, dport: u16, payload: &[u8]) -> Vec<u8> {
    let len = 8 + payload.len();
    let total = 20 + len;
    let mut p = Vec::with_capacity(total);
    p.extend_from_slice(&[0x45, 0]);
    p.extend_from_slice(&(total as u16).to_be_bytes());
    p.extend_from_slice(&[0, 0, 0x40, 0, 64, 17, 0, 0]);
    p.extend_from_slice(&src.octets());
    p.extend_from_slice(&dst.octets());
    let h = !fold(sum16(&p[..20]));
    p[10..12].copy_from_slice(&h.to_be_bytes());
    p.extend_from_slice(&sport.to_be_bytes());
    p.extend_from_slice(&dport.to_be_bytes());
    p.extend_from_slice(&(len as u16).to_be_bytes());
    p.extend_from_slice(&[0, 0]);
    p.extend_from_slice(payload);
    let mut c = !fold(sum16(&p[12..20]) + len as u32 + 17 + sum16(&p[20..]));
    if c == 0 {
        c = 0xffff;
    }
    p[26..28].copy_from_slice(&c.to_be_bytes());
    p
}

/// A UDP packet's source port, destination port and payload (IPv4).
pub fn udp4_parts(p: &[u8]) -> Option<(u16, u16, &[u8])> {
    if p.len() < 28 || p[0] >> 4 != 4 || p[9] != 17 {
        return None;
    }
    let ihl = (p[0] & 15) as usize * 4;
    let total = (u16::from_be_bytes([p[2], p[3]]) as usize).min(p.len());
    let u = p.get(ihl..total)?;
    if u.len() < 8 {
        return None;
    }
    Some((u16::from_be_bytes([u[0], u[1]]), u16::from_be_bytes([u[2], u[3]]), &u[8..]))
}

fn sum16(b: &[u8]) -> u32 {
    b.chunks(2).map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)]) as u32).sum()
}

fn fold(mut s: u32) -> u16 {
    while s >> 16 != 0 {
        s = (s & 0xffff) + (s >> 16);
    }
    s as u16
}

/// The packet with its source (`src`) or destination (`dst`) address changed,
/// checksums made right. None: not an IPv4 packet this handles.
pub fn readdress(p: &[u8], src: Option<Ipv4Addr>, dst: Option<Ipv4Addr>) -> Option<Vec<u8>> {
    if p.len() < 20 || p[0] >> 4 != 4 {
        return None;
    }
    let ihl = (p[0] & 15) as usize * 4;
    let total = u16::from_be_bytes([p[2], p[3]]) as usize;
    if ihl < 20 || total > p.len() || total < ihl {
        return None;
    }
    let mut out = p[..total].to_vec();
    let old: [u8; 8] = out[12..20].try_into().unwrap();
    if let Some(s) = src {
        out[12..16].copy_from_slice(&s.octets());
    }
    if let Some(d) = dst {
        out[16..20].copy_from_slice(&d.octets());
    }
    // The header checksum, and the transport one where it covers the addresses
    // (TCP, UDP; not ICMP), adjusted for the change (RFC 1624), so fragments
    // and partial transport headers need nothing more.
    let new = out[12..20].to_vec();
    adjust(&mut out[10..12], &old, &new);
    let frag_offset = u16::from_be_bytes([out[6], out[7]]) & 0x1fff;
    if frag_offset == 0 {
        let at = match out[9] {
            6 => Some(16),
            17 => Some(6),
            _ => None,
        };
        if let Some(at) = at {
            let i = ihl + at;
            if out.len() >= i + 2 && !(out[9] == 17 && out[i..i + 2] == [0, 0]) {
                adjust(&mut out[i..i + 2], &old, &new);
                if out[9] == 17 && out[i..i + 2] == [0, 0] {
                    out[i..i + 2].copy_from_slice(&[0xff, 0xff]);
                }
            }
        }
    }
    Some(out)
}

/// A checksum adjusted for bytes that changed from `old` to `new` (RFC 1624: HC' = ~(~HC + ~m + m')).
fn adjust(field: &mut [u8], old: &[u8], new: &[u8]) {
    let mut sum = !u16::from_be_bytes([field[0], field[1]]) as u32;
    for c in old.chunks(2) {
        sum += !u16::from_be_bytes([c[0], c[1]]) as u32;
    }
    for c in new.chunks(2) {
        sum += u16::from_be_bytes([c[0], c[1]]) as u32;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    field.copy_from_slice(&(!(sum as u16)).to_be_bytes());
}

/// The system set up to forward the devices' IPv4 out with NAT (Linux):
/// forwarding on, and in iptables, the devices' range let through and
/// masqueraded. Each rule is added only if missing. An error: the node's
/// own stack carries it instead.
#[cfg(target_os = "linux")]
pub fn setup(interface: &str) -> Result<(), String> {
    let range = format!("{NET}/{PREFIX}");
    if std::fs::read_to_string("/proc/sys/net/ipv4/ip_forward").map(|s| s.trim() != "1").unwrap_or(true)
        && std::fs::write("/proc/sys/net/ipv4/ip_forward", "1").is_err()
    {
        return Err("IPv4 forwarding is off and can't be turned on here (sysctl net.ipv4.ip_forward=1)".into());
    }
    let rules: [(&str, Vec<&str>); 3] = [
        ("nat", vec!["POSTROUTING", "-s", &range, "!", "-o", interface, "-j", "MASQUERADE"]),
        ("filter", vec!["FORWARD", "-i", interface, "-s", &range, "-j", "ACCEPT"]),
        ("filter", vec!["FORWARD", "-o", interface, "-d", &range, "-m", "conntrack", "--ctstate", "RELATED,ESTABLISHED", "-j", "ACCEPT"]),
    ];
    for (table, rule) in rules {
        let run = |op: &str| {
            let mut args = vec!["-w", "-t", table, op];
            args.extend(rule.iter().copied());
            std::process::Command::new("iptables").args(&args).output()
        };
        let present = run("-C").map(|o| o.status.success()).unwrap_or(false);
        if !present {
            // FORWARD rules first in the chain: Docker and others drop by default.
            let op = if table == "filter" { "-I" } else { "-A" };
            let o = run(op).map_err(|e| format!("iptables: {e}"))?;
            if !o.status.success() {
                return Err(format!("iptables {}: {}", rule.join(" "), String::from_utf8_lossy(&o.stderr).trim()));
            }
        }
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn setup(_interface: &str) -> Result<(), String> {
    Err("no NAT of the system's here".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolvers_from_resolv_conf() {
        let r = resolvers_in("# comment\nnameserver 100.100.100.100\nnameserver fd7a:115c:a1e0::53\nnameserver fe80::1%eth0\nsearch example.org\n");
        assert_eq!(r, vec!["100.100.100.100:53".parse().unwrap(), "[fd7a:115c:a1e0::53]:53".parse().unwrap()]);
    }

    #[test]
    fn udp_built_and_read() {
        let p = udp4(Ipv4Addr::new(198, 18, 0, 1), 40000, DNS[0], 53, b"query");
        assert!(udp_ok(&p));
        assert_eq!(fold(sum16(&p[..20])), 0xffff);
        assert_eq!(udp4_parts(&p), Some((40000, 53, &b"query"[..])));
    }

    fn udp_ok(p: &[u8]) -> bool {
        let len = p.len() - 20;
        fold(sum16(&p[12..20]) + len as u32 + 17 + sum16(&p[20..])) == 0xffff
    }

    #[test]
    fn source_and_destination_change_with_checksums() {
        // A UDP packet 198.18.0.1:5000 → 1.1.1.1:53, checksums right.
        let mut p = vec![0x45, 0, 0, 32, 0, 0, 0x40, 0, 64, 17, 0, 0, 198, 18, 0, 1, 1, 1, 1, 1, 0x13, 0x88, 0, 53, 0, 12, 0, 0, b'h', b'e', b'l', b'o'];
        let h = !fold(sum16(&p[..20]));
        p[10..12].copy_from_slice(&h.to_be_bytes());
        let c = !fold(sum16(&p[12..20]) + 12 + 17 + sum16(&p[20..]));
        p[26..28].copy_from_slice(&c.to_be_bytes());
        assert!(udp_ok(&p));
        let out = readdress(&p, Some(Ipv4Addr::new(198, 19, 255, 2)), None).unwrap();
        assert_eq!(&out[12..16], &[198, 19, 255, 2]);
        assert_eq!(fold(sum16(&out[..20])), 0xffff, "header checksum");
        assert!(udp_ok(&out), "UDP checksum");
        let back = readdress(&out, None, Some(Ipv4Addr::new(9, 9, 9, 9))).unwrap();
        assert!(udp_ok(&back));
    }

    #[test]
    fn internet_only_v6() {
        assert!(internet6("2606:4700:4700::1111".parse().unwrap()));
        for a in ["fd00::53", "fe80::1", "::1", "ff02::1", "::ffff:10.0.0.1", "2001:db8::1", "::"] {
            assert!(!internet6(a.parse().unwrap()), "{a}");
        }
    }

    #[test]
    fn internet_only() {
        for a in ["1.1.1.1", "203.0.113.7", "9.9.9.9"] {
            assert!(internet(a.parse().unwrap()), "{a}");
        }
        for a in ["10.1.2.3", "192.168.1.1", "172.20.0.1", "100.100.100.100", "127.0.0.1", "169.254.1.1", "224.0.0.1", "198.18.95.122", "255.255.255.255"] {
            assert!(!internet(a.parse().unwrap()), "{a}");
        }
    }

    #[test]
    fn clients_get_their_own_addresses() {
        let mut c = Clients::default();
        let a = c.of(&[1; 16]).unwrap();
        let b = c.of(&[2; 16]).unwrap();
        assert_ne!(a, b);
        assert_eq!(c.of(&[1; 16]), Some(a));
        assert_eq!(c.node(b), Some([2; 16]));
        assert!(in_range(a) && in_range(b));
    }
}

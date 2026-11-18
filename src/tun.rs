//! The TUN interface (M2, ADR-0017, ADR-0021): ordinary programs reach other
//! nodes by IPv6. The node's identity has the address `fd` ‖ its address
//! [0..15]; the interface carries it, and `fd00::/8` is routed to it.
//!
//! Linux only for now (`/dev/net/tun`; configured with `ip`, so the node
//! needs CAP_NET_ADMIN). Windows (wintun) and others come later.

use std::net::Ipv6Addr;

use crate::identity::Hash;

/// Inner MTU: both WireGuard layers and their headers fit in 1500 (measured margin to come).
pub const MTU: u16 = 1280;

/// The overlay address of an identity: `fd` followed by its first 15 bytes.
pub fn overlay(identity: &Hash) -> Ipv6Addr {
    let mut b = [0u8; 16];
    b[0] = 0xfd;
    b[1..].copy_from_slice(&identity[..15]);
    Ipv6Addr::from(b)
}

/// The identity prefix an overlay address names (None: not one).
pub fn prefix_of(addr: &[u8; 16]) -> Option<[u8; 15]> {
    (addr[0] == 0xfd).then(|| addr[1..].try_into().unwrap())
}

/// An IPv6 packet's source and destination.
pub fn v6_ends(p: &[u8]) -> Option<([u8; 16], [u8; 16])> {
    if p.len() < 40 || p[0] >> 4 != 6 {
        return None;
    }
    Some((p[8..24].try_into().unwrap(), p[24..40].try_into().unwrap()))
}

#[cfg(target_os = "linux")]
pub use linux::Tun;

#[cfg(target_os = "linux")]
mod linux {
    use std::io;
    use std::net::Ipv6Addr;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use tokio::io::unix::AsyncFd;

    pub struct Tun {
        fd: AsyncFd<OwnedFd>,
        pub name: String,
    }

    const TUNSETIFF: libc::c_ulong = 0x4004_54ca;
    const IFF_TUN: libc::c_short = 0x0001;
    const IFF_NO_PI: libc::c_short = 0x1000;

    fn ip(args: &[&str]) -> anyhow::Result<()> {
        let out = std::process::Command::new("ip").args(args).output().map_err(|e| anyhow::anyhow!("ip {}: {e}", args.join(" ")))?;
        if !out.status.success() {
            anyhow::bail!("ip {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(())
    }

    impl Tun {
        /// Opens interface `name` with `addr`/128 and `fd00::/8` routed to it.
        pub fn open(name: &str, addr: Ipv6Addr, mtu: u16) -> anyhow::Result<Tun> {
            let raw = unsafe { libc::open(c"/dev/net/tun".as_ptr(), libc::O_RDWR | libc::O_NONBLOCK | libc::O_CLOEXEC) };
            if raw < 0 {
                anyhow::bail!("/dev/net/tun: {}", io::Error::last_os_error());
            }
            let fd = unsafe { OwnedFd::from_raw_fd(raw) };
            let mut req = [0u8; 40]; // struct ifreq
            let n = name.len().min(15);
            req[..n].copy_from_slice(&name.as_bytes()[..n]);
            req[16..18].copy_from_slice(&(IFF_TUN | IFF_NO_PI).to_ne_bytes());
            if unsafe { libc::ioctl(fd.as_raw_fd(), TUNSETIFF as _, req.as_mut_ptr()) } < 0 {
                anyhow::bail!("TUNSETIFF {name}: {} (the node needs CAP_NET_ADMIN)", io::Error::last_os_error());
            }
            let end = req[..16].iter().position(|b| *b == 0).unwrap_or(16);
            let name = String::from_utf8_lossy(&req[..end]).into_owned();
            ip(&["link", "set", "dev", &name, "mtu", &mtu.to_string(), "up"])?;
            ip(&["-6", "addr", "replace", &format!("{addr}/128"), "dev", &name])?;
            ip(&["-6", "route", "replace", "fd00::/8", "dev", &name])?;
            // Safe: the descriptor is ours alone and lives as long as the AsyncFd.
            let fd = unsafe { AsyncFd::register(fd) }.map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(Tun { fd, name })
        }

        pub async fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
            loop {
                let mut g = self.fd.readable().await?;
                match g.try_io(|f| {
                    let n = unsafe { libc::read(f.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
                    if n < 0 { Err(io::Error::last_os_error()) } else { Ok(n as usize) }
                }) {
                    Ok(r) => return r,
                    Err(_would_block) => continue,
                }
            }
        }

        /// Writes one packet; dropped if the interface can't take it now.
        pub fn send(&self, p: &[u8]) {
            unsafe { libc::write(self.fd.get_ref().as_raw_fd(), p.as_ptr().cast(), p.len()) };
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub struct Tun {
    pub name: String,
}

#[cfg(not(target_os = "linux"))]
impl Tun {
    pub fn open(_name: &str, _addr: Ipv6Addr, _mtu: u16) -> anyhow::Result<Tun> {
        anyhow::bail!("the TUN interface is Linux-only so far")
    }
    pub async fn recv(&self, _buf: &mut [u8]) -> std::io::Result<usize> {
        std::future::pending().await
    }
    pub fn send(&self, _p: &[u8]) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_addresses() {
        let id: Hash = [0x12; 16];
        let a = overlay(&id);
        assert_eq!(a.octets()[0], 0xfd);
        assert_eq!(prefix_of(&a.octets()), Some([0x12; 15]));
        assert_eq!(prefix_of(&[0x20; 16]), None);
        let mut p = vec![0u8; 40];
        p[0] = 0x60;
        p[8..24].copy_from_slice(&a.octets());
        assert_eq!(v6_ends(&p).unwrap().0, a.octets());
        p[0] = 0x45;
        assert!(v6_ends(&p).is_none());
    }
}

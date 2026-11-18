//! The TUN interface (M2, ADR-0017, ADR-0021): ordinary programs reach other
//! nodes by IPv6. The node's identity has the address `fd` ‖ its address
//! [0..15]; the interface carries it, and `fd00::/8` is routed to it.
//!
//! Linux: `/dev/net/tun`, configured with `ip` (the node needs CAP_NET_ADMIN).
//! Windows: wintun, configured with `netsh` (administrator: the service).
//! Android: the app's VpnService makes and configures the interface and
//! hands its descriptor to the node over `<state>/tun.sock` (`tun = "fd"`).

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

#[cfg(any(target_os = "linux", target_os = "android"))]
pub use linux::{Tun, receive_fd};

#[cfg(any(target_os = "linux", target_os = "android"))]
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

    /// One descriptor sent over `conn` with SCM_RIGHTS.
    pub fn receive_fd(conn: &std::os::unix::net::UnixStream) -> io::Result<OwnedFd> {
        let mut data = [0u8; 1];
        let mut iov = libc::iovec { iov_base: data.as_mut_ptr().cast(), iov_len: 1 };
        let mut cmsg = [0u8; 64];
        let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = cmsg.as_mut_ptr().cast();
        msg.msg_controllen = cmsg.len() as _;
        if unsafe { libc::recvmsg(conn.as_raw_fd(), &mut msg, 0) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let c = unsafe { libc::CMSG_FIRSTHDR(&msg) };
        if c.is_null() || unsafe { (*c).cmsg_level != libc::SOL_SOCKET || (*c).cmsg_type != libc::SCM_RIGHTS } {
            return Err(io::Error::other("no descriptor came"));
        }
        let fd: i32 = unsafe { std::ptr::read_unaligned(libc::CMSG_DATA(c).cast()) };
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
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

        /// An interface someone else made and configured (Android's
        /// VpnService), handed over as a descriptor.
        pub fn from_fd(fd: OwnedFd, name: &str) -> anyhow::Result<Tun> {
            unsafe {
                let flags = libc::fcntl(fd.as_raw_fd(), libc::F_GETFL);
                libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
            let fd = unsafe { AsyncFd::register(fd) }.map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(Tun { fd, name: name.to_string() })
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

/// Waits on the Unix socket `path` for one descriptor (SCM_RIGHTS), the
/// interface an app made (Android's VpnService). Blocking: run it on a thread.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn receive_fd_at(path: &std::path::Path) -> anyhow::Result<std::os::fd::OwnedFd> {
    let _ = std::fs::remove_file(path);
    let listener = std::os::unix::net::UnixListener::bind(path)?;
    loop {
        let (conn, _) = listener.accept()?;
        match receive_fd(&conn) {
            Ok(fd) => return Ok(fd),
            Err(e) => eprintln!("interface handover: {e}"),
        }
    }
}

#[cfg(windows)]
pub use windows_tun::Tun;

/// Windows: a wintun adapter (wintun.dll next to vork.exe; creating one
/// needs administrator rights, so the node runs as a service for it).
#[cfg(windows)]
mod windows_tun {
    use std::io;
    use std::net::Ipv6Addr;
    use std::os::windows::process::CommandExt;
    use std::sync::Arc;

    pub struct Tun {
        session: Arc<wintun::Session>,
        rx: tokio::sync::Mutex<tokio::sync::mpsc::Receiver<Vec<u8>>>,
        pub name: String,
    }

    fn netsh(args: &[&str]) -> anyhow::Result<()> {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let out = std::process::Command::new("netsh").args(args).creation_flags(CREATE_NO_WINDOW).output()?;
        if !out.status.success() {
            anyhow::bail!("netsh {}: {}", args.join(" "), String::from_utf8_lossy(&out.stdout).trim());
        }
        Ok(())
    }

    impl Tun {
        pub fn open(name: &str, addr: Ipv6Addr, mtu: u16) -> anyhow::Result<Tun> {
            let dll = std::env::current_exe()?.with_file_name("wintun.dll");
            let w = unsafe { wintun::load_from_path(&dll) }
                .map_err(|e| anyhow::anyhow!("{}: {e} (wintun.dll, from wintun.net, goes next to vork.exe)", dll.display()))?;
            let adapter = match wintun::Adapter::open(&w, name) {
                Ok(a) => a,
                Err(_) => wintun::Adapter::create(&w, name, "vork", None).map_err(|e| anyhow::anyhow!("creating adapter {name}: {e} (needs administrator rights: run the node as a service)"))?,
            };
            let session = Arc::new(adapter.start_session(wintun::MAX_RING_CAPACITY).map_err(|e| anyhow::anyhow!("adapter {name}: {e}"))?);
            let idx = adapter.get_adapter_index().map_err(|e| anyhow::anyhow!("adapter {name}: {e}"))?.to_string();
            let iface = format!("interface={idx}");
            netsh(&["interface", "ipv6", "set", "subinterface", &iface, &format!("mtu={mtu}"), "store=active"])?;
            // Already there from an earlier run: fine.
            let _ = netsh(&["interface", "ipv6", "add", "address", &iface, &format!("address={addr}/128"), "store=active"]);
            let _ = netsh(&["interface", "ipv6", "add", "route", "prefix=fd00::/8", &iface, "store=active"]);
            let (tx, rx) = tokio::sync::mpsc::channel(1024);
            let s = session.clone();
            std::thread::Builder::new().name("vork-wintun".into()).spawn(move || {
                while let Ok(p) = s.receive_blocking() {
                    if tx.blocking_send(p.bytes().to_vec()).is_err() {
                        return;
                    }
                }
            })?;
            Ok(Tun { session, rx: tokio::sync::Mutex::new(rx), name: name.to_string() })
        }

        pub async fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
            let p = self.rx.lock().await.recv().await.ok_or_else(|| io::Error::other("adapter closed"))?;
            let n = p.len().min(buf.len());
            buf[..n].copy_from_slice(&p[..n]);
            Ok(n)
        }

        /// Writes one packet; dropped if the ring is full.
        pub fn send(&self, p: &[u8]) {
            let Ok(len) = u16::try_from(p.len()) else { return };
            if let Ok(mut pk) = self.session.allocate_send_packet(len) {
                pk.bytes_mut().copy_from_slice(p);
                self.session.send_packet(pk);
            }
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android", windows)))]
pub struct Tun {
    pub name: String,
}

#[cfg(not(any(target_os = "linux", target_os = "android", windows)))]
impl Tun {
    pub fn open(_name: &str, _addr: Ipv6Addr, _mtu: u16) -> anyhow::Result<Tun> {
        anyhow::bail!("no interface on this system yet")
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

//! An exit node in userspace (ADR-0026, addendum of 2026-11-27): where the
//! system can't forward with NAT for the node (Windows, Android, a Linux
//! without iptables) and for IPv6 everywhere, the exit's clients' packets go
//! into a TCP/IP stack of the node's own (ipstack). Each TCP connection and
//! UDP flow a device opens is ended here and opened again from this machine
//! with an ordinary socket, so its traffic leaves as this machine's own; ping
//! is sent with the system's ping sockets. Answers come back out of the stack
//! as packets to the device's own address, which the node sends back to it.
//!
//! Nothing is configured on the system: no forwarding, no firewall rules.

use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// Flows open at a time, all clients together.
const MAX_FLOWS: usize = 4096;
/// How long a connection to the far end may take to open.
const CONNECT: Duration = Duration::from_secs(10);

/// The stack: packets from clients go in with [`Stack::send`], packets for
/// them come out of the channel given to [`Stack::start`].
pub struct Stack {
    tx: UnboundedSender<Vec<u8>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Stack {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The stack's side of the "interface": a packet per read and per write.
struct Device {
    rx: UnboundedReceiver<Vec<u8>>,
    out: UnboundedSender<Vec<u8>>,
}

impl AsyncRead for Device {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        match self.rx.poll_recv(cx) {
            Poll::Ready(Some(p)) => {
                let n = p.len().min(buf.remaining());
                buf.put_slice(&p[..n]);
                Poll::Ready(Ok(()))
            }
            // The node is gone: the stack's task is aborted with it.
            _ => Poll::Pending,
        }
    }
}

impl AsyncWrite for Device {
    fn poll_write(self: Pin<&mut Self>, _cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        let _ = self.out.send(buf.to_vec());
        Poll::Ready(Ok(buf.len()))
    }
    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

impl Stack {
    /// A stack whose packets for the clients go to `out`.
    pub fn start(out: UnboundedSender<Vec<u8>>) -> Stack {
        let (tx, rx) = unbounded_channel();
        let mut config = ipstack::IpStackConfig::default();
        config.mtu_unchecked(crate::tun::MTU).udp_timeout(Duration::from_secs(60));
        let mut stack = ipstack::IpStack::new(config, Device { rx, out });
        let flows = Arc::new(tokio::sync::Semaphore::new(MAX_FLOWS));
        let task = tokio::spawn(async move {
            while let Ok(s) = stack.accept().await {
                // Over the limit: the flow is dropped (TCP's is reset), the rest go on.
                let Ok(permit) = flows.clone().try_acquire_owned() else { continue };
                match s {
                    ipstack::IpStackStream::Tcp(t) => {
                        tokio::spawn(async move {
                            tcp(t).await;
                            drop(permit);
                        });
                    }
                    ipstack::IpStackStream::Udp(u) => {
                        tokio::spawn(async move {
                            udp(u).await;
                            drop(permit);
                        });
                    }
                    ipstack::IpStackStream::UnknownTransport(u) => {
                        tokio::spawn(async move {
                            echo(u).await;
                            drop(permit);
                        });
                    }
                    ipstack::IpStackStream::UnknownNetwork(_) => {}
                }
            }
        });
        Stack { tx, task }
    }

    /// A client's packet (IPv4 or IPv6), already checked: into the stack.
    pub fn send(&self, p: Vec<u8>) {
        let _ = self.tx.send(p);
    }
}

/// A TCP connection: opened to where the client asked, then both ways.
async fn tcp(mut t: ipstack::IpStackTcpStream) {
    let to = t.peer_addr();
    let Ok(Ok(mut far)) = tokio::time::timeout(CONNECT, tokio::net::TcpStream::connect(to)).await else {
        crate::node::debug(format!("exit: no connection to {to}"));
        return;
    };
    let _ = far.set_nodelay(true);
    let _ = tokio::io::copy_bidirectional(&mut t, &mut far).await;
}

/// A UDP flow: datagrams both ways over a socket of its own, until it's idle.
async fn udp(mut u: ipstack::IpStackUdpStream) {
    let to = u.peer_addr();
    let any: SocketAddr = if to.is_ipv4() { "0.0.0.0:0".parse().unwrap() } else { "[::]:0".parse().unwrap() };
    let Ok(sock) = tokio::net::UdpSocket::bind(any).await else { return };
    if sock.connect(to).await.is_err() {
        return;
    }
    let (mut a, mut b) = (vec![0u8; 65536], vec![0u8; 65536]);
    loop {
        tokio::select! {
            r = u.read(&mut a) => match r {
                Ok(n) if n > 0 => { let _ = sock.send(&a[..n]).await; }
                _ => return, // idle, or the stack is gone
            },
            r = sock.recv(&mut b) => match r {
                Ok(n) => { if u.write_all(&b[..n]).await.is_err() { return } }
                Err(_) => {} // an ICMP error for an earlier datagram: carry on
            },
        }
    }
}

/// A packet of another protocol: an echo request (ping) is sent from here
/// and, once answered, answered to the client; the rest is dropped.
async fn echo(u: ipstack::IpStackUnknownTransport) {
    let (v4, proto) = (u.dst_addr().is_ipv4(), u.ip_protocol().0);
    let req = u.payload().to_vec();
    let asked = match (v4, proto) {
        (true, 1) => req.first() == Some(&8),
        (false, 58) => req.first() == Some(&128),
        _ => false,
    };
    if !asked || req.len() < 8 {
        return;
    }
    let dst = u.dst_addr();
    let r = req.clone();
    let Ok(true) = tokio::task::spawn_blocking(move || ping(dst, &r)).await else { return };
    // The answer is the request turned around: type 0 (IPv4) or 129 (IPv6),
    // the same identifier, sequence and data.
    let mut reply = req;
    reply[0] = if v4 { 0 } else { 129 };
    reply[2..4].copy_from_slice(&[0, 0]);
    let sum = match (dst, u.src_addr()) {
        (IpAddr::V6(from), IpAddr::V6(to)) => {
            let mut pseudo = Vec::with_capacity(40 + reply.len());
            pseudo.extend_from_slice(&from.octets());
            pseudo.extend_from_slice(&to.octets());
            pseudo.extend_from_slice(&(reply.len() as u32).to_be_bytes());
            pseudo.extend_from_slice(&[0, 0, 0, 58]);
            pseudo.extend_from_slice(&reply);
            checksum(&pseudo)
        }
        _ => checksum(&reply),
    };
    reply[2..4].copy_from_slice(&sum.to_be_bytes());
    let _ = u.send(reply);
}

fn checksum(b: &[u8]) -> u16 {
    let mut s: u32 = b.chunks(2).map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)]) as u32).sum();
    while s >> 16 != 0 {
        s = (s & 0xffff) + (s >> 16);
    }
    !(s as u16)
}

/// Sends an echo request (`req`, the ICMP message) to `dst` with an
/// unprivileged ping socket; whether it was answered within 3 s.
#[cfg(unix)]
fn ping(dst: IpAddr, req: &[u8]) -> bool {
    use socket2::{Domain, Protocol, Socket, Type};
    let (domain, proto, answer) = if dst.is_ipv4() { (Domain::IPV4, Protocol::ICMPV4, 0u8) } else { (Domain::IPV6, Protocol::ICMPV6, 129u8) };
    let Ok(s) = Socket::new(domain, Type::DGRAM, Some(proto)) else { return false };
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    if s.send_to(req, &SocketAddr::new(dst, 0).into()).is_err() {
        return false;
    }
    let mut buf = [std::mem::MaybeUninit::<u8>::uninit(); 2048];
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() || s.set_read_timeout(Some(left)).is_err() {
            return false;
        }
        let Ok(n) = s.recv(&mut buf) else { return false };
        // Safe: recv wrote the first n bytes.
        let got: Vec<u8> = buf[..n].iter().map(|b| unsafe { b.assume_init() }).collect();
        // The kernel gives the ICMP message (no IP header) and sets the identifier itself.
        if got.len() >= 8 && got[0] == answer && got[6..8] == req[6..8] {
            return true;
        }
    }
}

/// Windows: IcmpSendEcho (IPv4; IPv6 echo isn't carried).
#[cfg(windows)]
fn ping(dst: IpAddr, req: &[u8]) -> bool {
    use windows_sys::Win32::NetworkManagement::IpHelper::*;
    let IpAddr::V4(v4) = dst else { return false };
    let data = &req[8..];
    unsafe {
        let h = IcmpCreateFile();
        if h.is_null() || h as isize == -1 {
            return false;
        }
        let mut reply = vec![0u8; std::mem::size_of::<ICMP_ECHO_REPLY>() + data.len() + 64];
        let n = IcmpSendEcho(h, u32::from_ne_bytes(v4.octets()), data.as_ptr().cast(), data.len() as u16, std::ptr::null(), reply.as_mut_ptr().cast(), reply.len() as u32, 3000);
        IcmpCloseHandle(h);
        // Status 0: IP_SUCCESS, an echo reply.
        n > 0 && std::ptr::read_unaligned(reply.as_ptr().cast::<ICMP_ECHO_REPLY>()).Status == 0
    }
}

#[cfg(not(any(unix, windows)))]
fn ping(_dst: IpAddr, _req: &[u8]) -> bool {
    false
}

/// Whether this machine reaches the IPv6 internet itself (an address of its
/// own on the way to a public resolver): an exit says so, and its clients
/// send their IPv6 through it only then.
pub fn has_ipv6() -> bool {
    std::net::UdpSocket::bind("[::]:0")
        .and_then(|s| s.connect("[2606:4700:4700::1111]:53").and_then(|_| s.local_addr()))
        .is_ok_and(|a| match a.ip() {
            IpAddr::V6(v6) => global6(v6),
            _ => false,
        })
}

/// A global unicast IPv6 address (2000::/3, documentation left out).
pub fn global6(a: std::net::Ipv6Addr) -> bool {
    let s = a.segments();
    (s[0] & 0xe000) == 0x2000 && !(s[0] == 0x2001 && s[1] == 0x0db8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_addresses() {
        assert!(global6("2a00:1450:4001::200e".parse().unwrap()));
        assert!(!global6("fd5f:7a9a::1".parse().unwrap()));
        assert!(!global6("fe80::1".parse().unwrap()));
        assert!(!global6("::1".parse().unwrap()));
        assert!(!global6("2001:db8::1".parse().unwrap()));
        assert!(!global6("ff02::1".parse().unwrap()));
    }

    #[test]
    fn checksums() {
        // An echo request with a correct checksum sums to zero.
        let mut m = vec![8, 0, 0, 0, 0x12, 0x34, 0, 1, b'h', b'i'];
        let c = checksum(&m);
        m[2..4].copy_from_slice(&c.to_be_bytes());
        assert_eq!(checksum(&m), 0);
    }
}

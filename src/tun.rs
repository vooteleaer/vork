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

/// Whether the interface carries this node's IPv4 address (ADR-0021): on
/// Linux once it is set, elsewhere always (Windows and Android set it with
/// the interface). Packets translated from IPv4 elsewhere arrive as IPv4 then.
pub static V4: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(!cfg!(target_os = "linux"));

/// Whether the interface takes IPv6: everywhere but Haiku, whose network has
/// IPv4 only; there everything for this node's programs is given as IPv4.
pub static V6: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

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
#[cfg(target_os = "linux")]
pub use linux::{exit_routes, mark_socket};

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

    /// IPv4 aliases (ADR-0021): the range on the interface, unless a route
    /// here already covers it (then IPv6 only, said in the log).
    fn ipv4_aliases(name: &str, v4: std::net::Ipv4Addr) {
        let me = v4.to_string();
        let range = format!("{}/{}", crate::alias::NET, crate::alias::PREFIX);
        let taken = std::process::Command::new("ip")
            .args(["-4", "route", "show", "match", &me])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().any(|l| !l.starts_with("default") && !l.contains(&format!("dev {name}"))))
            .unwrap_or(true);
        if taken {
            return crate::node::log(format!("IPv4 aliases off: something here already routes {range}"));
        }
        match ip(&["-4", "addr", "replace", &format!("{me}/32"), "dev", name]).and_then(|_| ip(&["-4", "route", "replace", &range, "dev", name])) {
            Ok(()) => {
                super::V4.store(true, std::sync::atomic::Ordering::Relaxed);
                crate::node::log(format!("IPv4 aliases in {range}: this node is {me}"))
            }
            Err(e) => crate::node::log(format!("IPv4 aliases off: {e:#}")),
        }
    }

    /// The mark on the node's own sockets (`SO_MARK`): what it sends to other
    /// nodes leaves by the machine's own routes while an exit carries the rest.
    #[cfg(target_os = "linux")]
    pub const MARK: u32 = 0x766b;
    /// The routing table that sends everything to the interface meanwhile.
    #[cfg(target_os = "linux")]
    const TABLE: &str = "7387";
    /// The rules, by priority: the node's own traffic, the private ranges
    /// (the LAN and other VLANs behind the router: an exit carries the
    /// internet only), every route the machine has but its default, then
    /// the table. Before the main table's own rule (32766).
    #[cfg(target_os = "linux")]
    const RULES: [u32; 4] = [5300, 5305, 5310, 5320];
    #[cfg(target_os = "linux")]
    const PRIVATE: [&str; 3] = ["10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16"];

    /// Marks a socket of the node's own (see [`MARK`]); needs CAP_NET_ADMIN,
    /// which the node has for its interface.
    #[cfg(target_os = "linux")]
    pub fn mark_socket(fd: std::os::fd::RawFd) {
        let v = MARK as libc::c_int;
        unsafe {
            libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_MARK, (&v as *const libc::c_int).cast(), std::mem::size_of::<libc::c_int>() as libc::socklen_t);
        }
    }

    /// Linux using an exit node (ADR-0026): all traffic to the interface
    /// `name` but the node's own and the machine's networks, and names
    /// through the node (systemd-resolved); or (false) all of it taken away
    /// again. What a run before left is taken away first, every time.
    #[cfg(target_os = "linux")]
    pub fn exit_routes(name: &str, on: bool) {
        for fam in ["-4", "-6"] {
            for pref in RULES {
                let pref = pref.to_string();
                for _ in 0..8 {
                    if ip(&[fam, "rule", "del", "pref", &pref]).is_err() {
                        break;
                    }
                }
            }
            let _ = ip(&[fam, "route", "flush", "table", TABLE]);
        }
        let resolvectl = |args: &[&str]| std::process::Command::new("resolvectl").args(args).output().is_ok_and(|o| o.status.success());
        if !on {
            resolvectl(&["domain", name, "~vork"]);
            resolvectl(&["default-route", name, "no"]);
            return;
        }
        let mark = format!("{MARK:#x}/0xffff");
        let mut fail = Vec::new();
        for fam in ["-4", "-6"] {
            let steps: [Vec<&str>; 4] = [
                vec![fam, "route", "replace", "default", "dev", name, "table", TABLE],
                vec![fam, "rule", "add", "pref", "5300", "fwmark", &mark, "lookup", "main"],
                vec![fam, "rule", "add", "pref", "5310", "lookup", "main", "suppress_prefixlength", "0"],
                vec![fam, "rule", "add", "pref", "5320", "lookup", TABLE],
            ];
            for s in steps {
                if let Err(e) = ip(&s) {
                    fail.push(format!("{e:#}"));
                }
            }
        }
        for net in PRIVATE {
            if let Err(e) = ip(&["-4", "rule", "add", "pref", "5305", "to", net, "lookup", "main"]) {
                fail.push(format!("{e:#}"));
            }
        }
        if !fail.is_empty() {
            crate::node::log(format!("exit: routes not all set: {}", fail.join("; ")));
        }
        if !(resolvectl(&["domain", name, "~vork", "~."]) && resolvectl(&["default-route", name, "yes"])) {
            crate::node::log("exit: systemd-resolved didn't take every name; names may be asked of this network's servers");
        }
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
        pub fn open(name: &str, addr: Ipv6Addr, v4: std::net::Ipv4Addr, mtu: u16) -> anyhow::Result<Tun> {
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
            ipv4_aliases(&name, v4);
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
pub use windows_tun::{default_route, exit_routes, pin_socket, Tun};

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
        /// The adapter's interface index.
        pub index: u32,
    }

    /// Routes that send everything to the interface while an exit node is
    /// used (ADR-0026): halves of each family, so they win over the default
    /// route without replacing it. IPv6 is held back by the node, not leaked.
    const EXIT_ROUTES: [&str; 4] = ["0.0.0.0/1", "128.0.0.0/1", "::/1", "8000::/1"];
    /// Private networks, kept on the machine's own gateway meanwhile: an
    /// exit carries the internet only, and the LAN (other VLANs behind the
    /// router too) stays reachable, as Tailscale's "allow LAN access".
    const LAN_ROUTES: [&str; 3] = ["10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16"];
    /// The metric the node's own routes carry, so they are known to take away.
    const METRIC: u32 = 7;
    /// The NRPT rule that sends every name to the node's resolver meanwhile.
    const EXIT_DNS_RULE: &str = "vork-exit";
    /// One change to routes at a time (they run on threads of their own).
    static EXIT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn powershell(script: &str) -> bool {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .is_ok_and(|s| s.success())
    }

    fn quoted(list: &[&str]) -> String {
        list.iter().map(|p| format!("'{p}'")).collect::<Vec<_>>().join(",")
    }

    /// All traffic through the interface `index` (the LAN through
    /// `physical`, the machine's own default route: interface and gateway),
    /// and names through the node; or (None) all of it taken away again.
    /// Slow (PowerShell): run off the node's loop.
    pub fn exit_routes(index: u32, physical: Option<(u32, std::net::Ipv4Addr)>) {
        let _held = EXIT_LOCK.lock();
        // What the node added before (by its metric) goes first, every time:
        // a new default route (another network) gets its LAN routes afresh.
        let mut script = format!(
            "$ErrorActionPreference = 'SilentlyContinue'; \
             Get-NetRoute -PolicyStore ActiveStore | Where-Object {{ ($_.RouteMetric -eq {METRIC} -or $_.InterfaceIndex -eq {index}) -and $_.DestinationPrefix -in @({}) }} | Remove-NetRoute -Confirm:$false; \
             Get-DnsClientNrptRule | Where-Object {{ $_.Comment -eq '{EXIT_DNS_RULE}' }} | Remove-DnsClientNrptRule -Force; ",
            quoted(&[&EXIT_ROUTES[..], &LAN_ROUTES[..]].concat())
        );
        if let Some((phys, gw)) = physical {
            script += &format!(
                "foreach ($p in @({})) {{ New-NetRoute -DestinationPrefix $p -InterfaceIndex {phys} -NextHop '{gw}' -RouteMetric {METRIC} -PolicyStore ActiveStore | Out-Null }}; \
                 foreach ($p in @({})) {{ New-NetRoute -DestinationPrefix $p -InterfaceIndex {index} -RouteMetric {METRIC} -PolicyStore ActiveStore | Out-Null }}; \
                 Add-DnsClientNrptRule -Namespace '.' -NameServers '{}' -Comment '{EXIT_DNS_RULE}'; ",
                if gw.is_unspecified() { String::new() } else { quoted(&LAN_ROUTES) },
                quoted(&EXIT_ROUTES),
                crate::dns::ADDR
            );
        }
        script += "Clear-DnsClientCache; exit 0";
        if !powershell(&script) {
            crate::node::log("exit: the routes couldn't be changed (PowerShell)");
        }
    }

    /// The machine's own default route (the lowest metric of a connected
    /// interface's 0.0.0.0/0, this adapter left out): interface and gateway.
    pub fn default_route(exclude: u32) -> Option<(u32, std::net::Ipv4Addr)> {
        use windows_sys::Win32::NetworkManagement::IpHelper::*;
        use windows_sys::Win32::Networking::WinSock::AF_INET;
        let mut best: Option<(u32, u32, std::net::Ipv4Addr)> = None;
        unsafe {
            let mut table: *mut MIB_IPFORWARD_TABLE2 = std::ptr::null_mut();
            if GetIpForwardTable2(AF_INET, &mut table) != 0 || table.is_null() {
                return None;
            }
            let rows = std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize);
            for r in rows {
                if r.DestinationPrefix.PrefixLength != 0 || r.InterfaceIndex == exclude {
                    continue;
                }
                let mut row: MIB_IPINTERFACE_ROW = std::mem::zeroed();
                InitializeIpInterfaceEntry(&mut row);
                row.Family = AF_INET;
                row.InterfaceIndex = r.InterfaceIndex;
                if GetIpInterfaceEntry(&mut row) != 0 || !row.Connected {
                    continue;
                }
                let metric = r.Metric.saturating_add(row.Metric);
                let gw = std::net::Ipv4Addr::from(u32::from_be(r.NextHop.Ipv4.sin_addr.S_un.S_addr));
                if best.is_none_or(|(m, _, _)| metric < m) {
                    best = Some((metric, r.InterfaceIndex, gw));
                }
            }
            FreeMibTable(table as *const _);
        }
        best.map(|(_, i, gw)| (i, gw))
    }

    /// The node's own UDP leaves by this interface whatever the routes say
    /// (`IP_UNICAST_IF`), so its packets don't go into the interface it
    /// carries; None lets the routes decide again.
    pub fn pin_socket(sock: &std::net::UdpSocket, index: Option<u32>) {
        use std::os::windows::io::AsRawSocket;
        use windows_sys::Win32::Networking::WinSock::*;
        let s = sock.as_raw_socket() as SOCKET;
        let i = index.unwrap_or(0);
        unsafe {
            if sock.local_addr().is_ok_and(|a| a.is_ipv6()) {
                let v = i;
                setsockopt(s, IPPROTO_IPV6, IPV6_UNICAST_IF, &v as *const u32 as *const u8, 4);
            }
            // IPv4's takes the index in network byte order.
            let v = i.to_be();
            setsockopt(s, IPPROTO_IP, IP_UNICAST_IF, &v as *const u32 as *const u8, 4);
        }
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
        pub fn open(name: &str, addr: Ipv6Addr, v4: std::net::Ipv4Addr, mtu: u16) -> anyhow::Result<Tun> {
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
            // IPv4 (ADR-0021): this node's address and the range.
            // (IPv4's netsh names the interface with name=, not interface=.)
            let _ = netsh(&["interface", "ipv4", "add", "address", &format!("name={idx}"), &format!("address={v4}"), "mask=255.255.255.255", "store=active"]);
            let _ = netsh(&["interface", "ipv4", "add", "route", &format!("prefix={}/{}", crate::alias::NET, crate::alias::PREFIX), &iface, "store=active"]);
            // IPv4 through an exit node fits the sessions too.
            let _ = netsh(&["interface", "ipv4", "set", "subinterface", &idx, &format!("mtu={mtu}"), "store=active"]);
            // An exit's routes and names left by a run that didn't end cleanly: gone.
            let index: u32 = idx.parse().unwrap_or(0);
            std::thread::spawn(move || exit_routes(index, None));
            let (tx, rx) = tokio::sync::mpsc::channel(1024);
            let s = session.clone();
            std::thread::Builder::new().name("vork-wintun".into()).spawn(move || {
                while let Ok(p) = s.receive_blocking() {
                    if tx.blocking_send(p.bytes().to_vec()).is_err() {
                        return;
                    }
                }
            })?;
            Ok(Tun { session, rx: tokio::sync::Mutex::new(rx), name: name.to_string(), index })
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

#[cfg(target_os = "haiku")]
pub use haiku_tun::Tun;

/// Haiku: its `tunnel` driver (`tun = "tun/0"`): `ifconfig tun/0 up` makes
/// `/dev/tun/0`, which reads and writes plain IP packets. Haiku's network has
/// IPv4 only, so the interface carries this node's IPv4 address and the
/// devices' range; what arrives over IPv6 is given to it as IPv4 (V6 false).
#[cfg(target_os = "haiku")]
mod haiku_tun {
    use std::io::{self, Read, Write};
    use std::net::Ipv6Addr;

    pub struct Tun {
        file: std::fs::File,
        rx: tokio::sync::Mutex<tokio::sync::mpsc::Receiver<Vec<u8>>>,
        pub name: String,
    }

    fn ifconfig(prog: &str, args: &[&str]) -> anyhow::Result<()> {
        let out = std::process::Command::new(prog).args(args).output().map_err(|e| anyhow::anyhow!("{prog} {}: {e}", args.join(" ")))?;
        if !out.status.success() {
            anyhow::bail!("{prog} {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(())
    }

    impl Tun {
        pub fn open(name: &str, _addr: Ipv6Addr, v4: std::net::Ipv4Addr, mtu: u16) -> anyhow::Result<Tun> {
            super::V6.store(false, std::sync::atomic::Ordering::Relaxed);
            ifconfig("ifconfig", &[name, "up"])?;
            let path = format!("/dev/{name}");
            let file = std::fs::OpenOptions::new().read(true).write(true).open(&path).map_err(|e| anyhow::anyhow!("{path}: {e}"))?;
            ifconfig("ifconfig", &[name, "inet", &v4.to_string(), "255.255.255.255", "mtu", &mtu.to_string(), "up"])?;
            // The devices' range (ADR-0021); already there from an earlier run: fine.
            let _ = ifconfig("route", &["add", name, "inet", &crate::alias::NET.to_string(), "netmask", "255.254.0.0"]);
            super::V4.store(true, std::sync::atomic::Ordering::Relaxed);
            crate::node::log(format!("IPv4 aliases in {}/{}: this node is {v4} (Haiku: IPv4 only)", crate::alias::NET, crate::alias::PREFIX));
            let (tx, rx) = tokio::sync::mpsc::channel(1024);
            let mut reader = file.try_clone()?;
            std::thread::Builder::new().name("vork-tun".into()).spawn(move || {
                let mut buf = vec![0u8; 65536];
                while let Ok(n) = reader.read(&mut buf) {
                    if n == 0 || tx.blocking_send(buf[..n].to_vec()).is_err() {
                        return;
                    }
                }
            })?;
            Ok(Tun { file, rx: tokio::sync::Mutex::new(rx), name: name.to_string() })
        }

        pub async fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
            let p = self.rx.lock().await.recv().await.ok_or_else(|| io::Error::other("interface closed"))?;
            let n = p.len().min(buf.len());
            buf[..n].copy_from_slice(&p[..n]);
            Ok(n)
        }

        /// Writes one IPv4 packet (Haiku takes no IPv6 here).
        pub fn send(&self, p: &[u8]) {
            if p.first().is_some_and(|b| b >> 4 == 4) {
                let _ = (&self.file).write(p);
            }
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android", windows, target_os = "haiku")))]
pub struct Tun {
    pub name: String,
}

#[cfg(not(any(target_os = "linux", target_os = "android", windows, target_os = "haiku")))]
impl Tun {
    pub fn open(_name: &str, _addr: Ipv6Addr, _v4: std::net::Ipv4Addr, _mtu: u16) -> anyhow::Result<Tun> {
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

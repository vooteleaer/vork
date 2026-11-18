//! Nodes on the local network find each other by mDNS / DNS-SD (ADR-0023):
//! each registers `_vork._udp.local.` with its port and its WireGuard key in
//! TXT (`k`), and browses for the others. What's found is only a lead: the
//! node dials it, and the WireGuard handshake decides.

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::net::{IpAddr, SocketAddr};

pub const SERVICE: &str = "_vork._udp.local.";

/// A node found on the local network.
pub struct Found {
    pub key: [u8; 32],
    pub addrs: Vec<SocketAddr>,
}

/// Registers this node and browses for others; what's found arrives on `tx`.
/// Keep the daemon while it should run (dropping it doesn't stop it: call `shutdown`).
pub fn start(name: &str, x_pub: &[u8; 32], port: u16, tx: tokio::sync::mpsc::Sender<Found>) -> anyhow::Result<ServiceDaemon> {
    let mdns = ServiceDaemon::new()?;
    let key = hex::encode(x_pub);
    let info = ServiceInfo::new(SERVICE, name, &format!("vork-{name}.local."), "", port, &[("k", key.as_str()), ("v", "0")][..])?.enable_addr_auto();
    mdns.register(info)?;
    let rx = mdns.browse(SERVICE)?;
    let mine = *x_pub;
    std::thread::Builder::new().name("vork-mdns".into()).spawn(move || {
        while let Ok(ev) = rx.recv() {
            let ServiceEvent::ServiceResolved(s) = ev else { continue };
            let Some(key) = s.get_property_val_str("k").and_then(|k| hex::decode(k).ok()).and_then(|k| <[u8; 32]>::try_from(k).ok()) else { continue };
            if key == mine {
                continue;
            }
            let port = s.get_port();
            let mut addrs: Vec<SocketAddr> = s
                .get_addresses()
                .iter()
                .map(|a| a.to_ip_addr())
                // Link-local IPv6 needs its interface: not dialed.
                .filter(|ip| !matches!(ip, IpAddr::V6(v) if v.segments()[0] & 0xffc0 == 0xfe80))
                .map(|ip| SocketAddr::new(ip, port))
                .collect();
            addrs.sort_by_key(|a| !a.is_ipv4());
            if !addrs.is_empty() && tx.blocking_send(Found { key, addrs }).is_err() {
                return;
            }
        }
    })?;
    Ok(mdns)
}

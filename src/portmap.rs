//! Asking the home router to open our UDP port, the way torrent clients do:
//! UPnP IGD first, then PCP and NAT-PMP. A node that gets a mapping becomes
//! reachable from outside, which makes direct links and relaying possible
//! without a hand-made port forward.
//!
//! Opening a port is safe here: WireGuard answers nothing to a packet that
//! isn't a valid handshake for our key. It's on by default (`map_port = false`
//! or `--no-map-port` to leave the router alone); the outside port needn't be
//! the node's own: when it's taken, the router picks one, and others learn
//! where they reached the node (ADR-0023).

use crab_nat::{GatewayAddress, InternetProtocol, PortMapping, PortMappingOptions, PortMappingType};
use igd_next::aio::tokio::search_gateway;
use igd_next::{PortMappingProtocol, SearchOptions};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::num::NonZeroU16;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

const LEASE: u32 = 3600;
const DESCRIPTION: &str = "vork";

fn log(msg: impl AsRef<str>) {
    eprintln!("[portmap] {}", msg.as_ref());
}

/// The address this machine uses toward the internet (no packet is sent).
fn local_ip() -> Option<Ipv4Addr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?; // TEST-NET-1: only picks a route
    match s.local_addr().ok()?.ip() {
        IpAddr::V4(ip) => Some(ip),
        IpAddr::V6(_) => None,
    }
}

/// An outer address that is itself private or carrier-grade NAT means there
/// is another NAT beyond the router; a mapping wouldn't reach us.
fn behind_another_nat(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_private() || v4.is_loopback() || v4.is_unspecified() || (o[0] == 100 && (o[1] & 0xc0) == 64)
        }
        IpAddr::V6(_) => false,
    }
}

enum Mapping {
    Upnp(igd_next::aio::Gateway<igd_next::aio::tokio::Tokio>, u16),
    NatPmp(PortMapping),
}

async fn try_upnp(local: SocketAddr) -> Result<(Mapping, SocketAddr), String> {
    let mut opts = SearchOptions::default();
    opts.timeout = Some(Duration::from_secs(3));
    let gw = search_gateway(opts).await.map_err(|e| format!("UPnP: no gateway ({e})"))?;
    let ext = gw.get_external_ip().await.map_err(|e| format!("UPnP: external address ({e})"))?;
    let port = match gw.add_port(PortMappingProtocol::UDP, local.port(), local, LEASE, DESCRIPTION).await {
        Ok(()) => local.port(),
        // The same port is taken (by another machine): let the router pick one.
        Err(_) => gw
            .add_any_port(PortMappingProtocol::UDP, local, LEASE, DESCRIPTION)
            .await
            .map_err(|e| format!("UPnP: mapping refused ({e})"))?,
    };
    Ok((Mapping::Upnp(gw, port), SocketAddr::new(ext, port)))
}

async fn try_pcp_natpmp(local: SocketAddr, gateway: Ipv4Addr) -> Result<(Mapping, SocketAddr), String> {
    let gw = GatewayAddress::IpV4(gateway);
    let opts = PortMappingOptions {
        external_port: NonZeroU16::new(local.port()),
        lifetime_seconds: Some(LEASE),
        timeout_config: None,
    };
    let m = PortMapping::new(gw, local.ip(), InternetProtocol::Udp, NonZeroU16::new(local.port()).unwrap(), opts)
        .await
        .map_err(|e| format!("PCP/NAT-PMP via {gateway}: {e}"))?;
    let ext_ip = match m.mapping_type() {
        PortMappingType::Pcp { external_ip, .. } => external_ip,
        PortMappingType::NatPmp => IpAddr::V4(
            crab_nat::natpmp::external_address(gw, None)
                .await
                .map_err(|e| format!("NAT-PMP: external address ({e})"))?,
        ),
    };
    let ext = SocketAddr::new(ext_ip, m.external_port().get());
    Ok((Mapping::NatPmp(m), ext))
}

async fn renew(m: &mut Mapping, local: SocketAddr) -> Result<(), String> {
    match m {
        Mapping::Upnp(gw, port) => gw
            .add_port(PortMappingProtocol::UDP, *port, local, LEASE, DESCRIPTION)
            .await
            .map_err(|e| e.to_string()),
        Mapping::NatPmp(pm) => pm.renew().await.map_err(|e| e.to_string()),
    }
}

async fn remove(m: Mapping) {
    match m {
        Mapping::Upnp(gw, port) => {
            let _ = gw.remove_port(PortMappingProtocol::UDP, port).await;
        }
        Mapping::NatPmp(pm) => {
            let _ = pm.try_drop().await;
        }
    }
}

/// Maps `port`, reports the outside address on `found`, renews the lease
/// until `stop` changes, then removes the mapping.
pub async fn run(
    listen: SocketAddr,
    gateway: Option<Ipv4Addr>,
    found: mpsc::Sender<SocketAddr>,
    mut stop: watch::Receiver<bool>,
) {
    let ip = match listen.ip() {
        IpAddr::V4(v4) if !v4.is_unspecified() => v4,
        _ => match local_ip() {
            Some(ip) => ip,
            None => return log("no local IPv4 address; skipping"),
        },
    };
    let local = SocketAddr::new(IpAddr::V4(ip), listen.port());
    let gateway = gateway.unwrap_or_else(|| {
        let o = ip.octets();
        Ipv4Addr::new(o[0], o[1], o[2], 1) // a guess; use --gateway if wrong
    });
    let (mut mapping, ext) = match try_upnp(local).await {
        Ok(r) => {
            log("mapped with UPnP");
            r
        }
        Err(e1) => match try_pcp_natpmp(local, gateway).await {
            Ok(r) => {
                log("mapped with PCP/NAT-PMP");
                r
            }
            Err(e2) => return log(format!("router mapping failed: {e1}; {e2}")),
        },
    };
    if behind_another_nat(ext.ip()) {
        log(format!("the router's outer address is {}: there is another NAT beyond it (CGNAT?), so the mapping won't make us reachable", ext.ip()));
        return remove(mapping).await;
    }
    log(format!("reachable from outside at {ext} (lease {LEASE} s)"));
    let _ = found.send(ext).await;
    let mut tick = tokio::time::interval(Duration::from_secs((LEASE / 2) as u64));
    tick.tick().await;
    loop {
        tokio::select! {
            _ = tick.tick() => {
                if let Err(e) = renew(&mut mapping, local).await {
                    log(format!("renewing the mapping failed: {e}"));
                }
            }
            _ = stop.changed() => {
                remove(mapping).await;
                return log("mapping removed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nat_beyond() {
        assert!(behind_another_nat("100.72.1.2".parse().unwrap()));
        assert!(behind_another_nat("192.168.1.1".parse().unwrap()));
        assert!(!behind_another_nat("90.191.2.3".parse().unwrap()));
        assert!(!behind_another_nat("100.128.0.1".parse().unwrap()));
    }
}

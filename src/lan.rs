//! Nodes on the local network find each other by mDNS / DNS-SD (ADR-0023):
//! each registers `_vork._udp.local.` with its port and its WireGuard key in
//! TXT (`k`), and browses for the others. What's found is only a lead: the
//! node dials it, and the WireGuard handshake decides.
//!
//! Not on Haiku yet: mdns-sd needs IP_PKTINFO, which Haiku doesn't have.
//! There the node finds others through its connections and links only.

use std::net::SocketAddr;

pub const SERVICE: &str = "_vork._udp.local.";

/// A node found on the local network.
pub struct Found {
    pub key: [u8; 32],
    pub addrs: Vec<SocketAddr>,
}

#[cfg(not(target_os = "haiku"))]
pub use mdns_sd::ServiceDaemon;

/// Registers this node and browses for others; what's found arrives on `tx`.
/// Keep the daemon while it should run (dropping it doesn't stop it: call `shutdown`).
#[cfg(not(target_os = "haiku"))]
pub fn start(name: &str, x_pub: &[u8; 32], port: u16, tx: tokio::sync::mpsc::Sender<Found>) -> anyhow::Result<ServiceDaemon> {
    use mdns_sd::{ServiceEvent, ServiceInfo};
    use std::net::IpAddr;
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

/// A service one of the owner's devices asks to be made known on this
/// node's local network for it (a vØrk Lite device's native API, so Home
/// Assistant here finds it as it finds a device on its own LAN): announced
/// at the device's vØrk address, which this node's interface reaches.
#[derive(Clone, PartialEq, Debug)]
pub struct Proxied {
    /// `_esphomelib._tcp` and the like.
    pub kind: String,
    pub name: String,
    pub port: u16,
    pub txt: Vec<(String, String)>,
}

impl Proxied {
    /// From a device's paths answer (`mdns`): at most four, of plain DNS-SD types.
    pub fn list(v: &serde_json::Value) -> Vec<Proxied> {
        let plain = |s: &str, max: usize| !s.is_empty() && s.len() <= max && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        v["mdns"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| {
                let kind = x["type"].as_str()?;
                let (service, proto) = kind.split_once('.')?;
                let name = x["name"].as_str()?;
                let port = u16::try_from(x["port"].as_u64()?).ok()?;
                if !service.starts_with('_') || !plain(&service[1..], 15) || !matches!(proto, "_tcp" | "_udp") || !plain(name, 63) {
                    return None;
                }
                let txt = x["txt"]
                    .as_object()
                    .map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).filter(|(k, v)| k.len() <= 32 && v.len() <= 200).take(16).collect())
                    .unwrap_or_default();
                Some(Proxied { kind: kind.to_string(), name: name.to_string(), port, txt })
            })
            .take(4)
            .collect()
    }
}

/// Instances of a type announced on the local network by the devices
/// themselves (not at a vØrk address), by name, when last seen: a device on
/// this node's own network needs no announcing for it.
#[cfg(not(target_os = "haiku"))]
static NATIVE: std::sync::Mutex<Option<std::collections::HashMap<String, std::collections::HashMap<String, std::time::Instant>>>> = std::sync::Mutex::new(None);

/// Whether `name` of `kind` was announced on the local network by itself
/// lately; watches the type from the first time it's asked.
#[cfg(not(target_os = "haiku"))]
pub fn native(mdns: &ServiceDaemon, kind: &str, name: &str) -> bool {
    let mut g = NATIVE.lock().unwrap();
    let all = g.get_or_insert_with(Default::default);
    if !all.contains_key(kind) {
        all.insert(kind.to_string(), Default::default());
        if let Ok(rx) = mdns.browse(&format!("{kind}.local.")) {
            let kind = kind.to_string();
            let _ = std::thread::Builder::new().name("vork-mdns-watch".into()).spawn(move || {
                while let Ok(ev) = rx.recv() {
                    let mdns_sd::ServiceEvent::ServiceResolved(s) = ev else { continue };
                    let own = s.get_addresses().iter().all(|a| matches!(a.to_ip_addr(), std::net::IpAddr::V4(v) if crate::alias::in_range(v)));
                    if own {
                        continue;
                    }
                    let name = s.get_fullname().split('.').next().unwrap_or_default().to_string();
                    if let Some(e) = NATIVE.lock().unwrap().as_mut().and_then(|a| a.get_mut(&kind)) {
                        e.insert(name, std::time::Instant::now());
                    }
                }
            });
        }
    }
    all.get(kind).and_then(|e| e.get(name)).is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(600))
}

#[cfg(target_os = "haiku")]
pub fn native(_mdns: &ServiceDaemon, _kind: &str, _name: &str) -> bool {
    false
}

/// Announces a service of one of the owner's devices; its full name, to withdraw it.
#[cfg(not(target_os = "haiku"))]
pub fn announce(mdns: &ServiceDaemon, s: &Proxied, ip: std::net::Ipv4Addr) -> Option<String> {
    let props: std::collections::HashMap<String, String> = s.txt.iter().cloned().collect();
    let info = mdns_sd::ServiceInfo::new(&format!("{}.local.", s.kind), &s.name, &format!("{}.local.", s.name), ip.to_string().as_str(), s.port, props).ok()?;
    let full = info.get_fullname().to_string();
    mdns.register(info).ok()?;
    Some(full)
}

#[cfg(not(target_os = "haiku"))]
pub fn withdraw(mdns: &ServiceDaemon, full: &str) {
    let _ = mdns.unregister(full);
}

#[cfg(target_os = "haiku")]
pub fn announce(_mdns: &ServiceDaemon, _s: &Proxied, _ip: std::net::Ipv4Addr) -> Option<String> {
    None
}

#[cfg(target_os = "haiku")]
pub fn withdraw(_mdns: &ServiceDaemon, _full: &str) {}

/// Haiku: no local network discovery yet.
#[cfg(target_os = "haiku")]
#[derive(Clone)]
pub struct ServiceDaemon;

#[cfg(target_os = "haiku")]
impl ServiceDaemon {
    pub fn shutdown(&self) -> Result<(), ()> {
        Ok(())
    }
}

#[cfg(target_os = "haiku")]
pub fn start(_name: &str, _x_pub: &[u8; 32], _port: u16, _tx: tokio::sync::mpsc::Sender<Found>) -> anyhow::Result<ServiceDaemon> {
    anyhow::bail!("not available on Haiku yet")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxied_from_a_paths_answer() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"paths":[],"mdns":[
                {"type":"_esphomelib._tcp","name":"esp32s3-vork","port":6053,"txt":{"mac":"aca7043c28d8","version":"2026.7.4"}},
                {"type":"_bad","name":"x","port":1},
                {"type":"_http._tcp","name":"no spaces","port":80}]}"#,
        )
        .unwrap();
        let l = Proxied::list(&v);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].kind, "_esphomelib._tcp");
        assert_eq!(l[0].port, 6053);
        assert!(l[0].txt.contains(&("mac".into(), "aca7043c28d8".into())));
    }
}

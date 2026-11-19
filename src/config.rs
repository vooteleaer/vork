//! The configuration file (`vork.toml`) and where things live by default.
//!
//! Relative paths in the file are relative to the file. Command-line flags
//! override the file.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

#[derive(Deserialize, Default, Debug)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    pub listen: Option<SocketAddr>,
    pub key: Option<PathBuf>,
    pub state: Option<PathBuf>,
    #[serde(default)]
    pub peers: Vec<String>,
    /// Neighbors switched off (kept for the desktop utility; not dialed).
    #[allow(dead_code)]
    #[serde(default)]
    pub peers_off: Vec<String>,
    pub announce: Option<bool>,
    pub lite: Option<bool>,
    pub direct: Option<bool>,
    pub map_port: Option<bool>,
    /// Find other nodes on the local network (multicast beacons).
    pub lan: Option<bool>,
    /// The TUN interface to create (M2): programs reach other nodes at `fd…` addresses.
    pub tun: Option<String>,
    /// This node's name, announced (default: the machine's name).
    pub name: Option<String>,
    /// The web dashboard's address (`0.0.0.0:8742`): off when missing.
    pub web: Option<String>,
    /// The dashboard's certificate chain and key (PEM), e.g. Let's Encrypt's;
    /// without them it makes a self-signed one.
    pub web_cert: Option<std::path::PathBuf>,
    pub web_key: Option<std::path::PathBuf>,
    /// Openings in the walled garden for nodes that aren't the owner's devices (ADR-0027).
    pub allow: Option<Vec<String>>,
    /// Offer this node as an exit node to the owner's devices.
    pub exit: Option<bool>,
    /// Send this device's internet traffic out through that node (its address).
    pub exit_via: Option<String>,
    pub gateway: Option<std::net::Ipv4Addr>,
    pub paths: Option<usize>,
    pub spread: Option<String>,
    /// Named services of this node's identity to announce (storage uses "retifs.node").
    #[serde(default)]
    pub destinations: Vec<String>,
    /// The owner's other devices (identity addresses, hex): direct links with
    /// them even when `direct` is off (ADR-0011).
    #[serde(default)]
    pub devices: Vec<String>,
    pub log: Option<String>,
    pub log_file: Option<PathBuf>,
    pub control: Option<String>,
    /// The local API endpoint for applications.
    pub api: Option<String>,
}

impl FileConfig {
    pub fn load(path: &Path) -> Result<FileConfig> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut c: FileConfig = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        let base = path.parent().unwrap_or(Path::new("."));
        let abs = |p: &mut Option<PathBuf>| {
            if let Some(x) = p.as_mut() {
                if x.is_relative() {
                    *x = base.join(&*x);
                }
            }
        };
        abs(&mut c.key);
        abs(&mut c.state);
        abs(&mut c.log_file);
        Ok(c)
    }
}

#[cfg(unix)]
fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

/// Where the configuration and the state live when nothing is said.
/// Linux as root: `/etc/vork/vork.toml` and `/var/lib/vork`; as a user:
/// `~/.config/vork/vork.toml` and `~/.local/state/vork`; Windows:
/// `%ProgramData%\vork\vork.toml` and `%ProgramData%\vork\state`.
pub fn default_paths() -> (PathBuf, PathBuf) {
    if let Ok(p) = std::env::var("VORK_CONFIG") {
        let p = PathBuf::from(p);
        let state = p.parent().unwrap_or(Path::new(".")).join("state");
        return (p, state);
    }
    #[cfg(windows)]
    {
        let base = PathBuf::from(std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".into())).join("vork");
        (base.join("vork.toml"), base.join("state"))
    }
    #[cfg(unix)]
    {
        if is_root() {
            return (PathBuf::from("/etc/vork/vork.toml"), PathBuf::from("/var/lib/vork"));
        }
        let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()));
        let config = std::env::var("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|_| home.join(".config"));
        let state = std::env::var("XDG_STATE_HOME").map(PathBuf::from).unwrap_or_else(|_| home.join(".local/state"));
        (config.join("vork/vork.toml"), state.join("vork"))
    }
}

/// The control endpoint: a Unix socket in the state directory, or a named
/// pipe per listening port on Windows.
pub fn control_endpoint(explicit: Option<&str>, state: Option<&Path>, listen: SocketAddr) -> Result<String> {
    if let Some(c) = explicit {
        return Ok(c.to_string());
    }
    #[cfg(windows)]
    {
        let _ = state;
        Ok(format!(r"\\.\pipe\vork-{}", listen.port()))
    }
    #[cfg(unix)]
    {
        let _ = listen;
        match state {
            Some(s) => Ok(s.join("control.sock").to_string_lossy().into_owned()),
            None => bail!("no control socket without a state directory (give --state or --control)"),
        }
    }
}

pub const SAMPLE: &str = r#"# vØrk node configuration. Relative paths are relative to this file.

# UDP address to listen on.
listen = "0.0.0.0:51820"

# Identity key; created on first run if missing. Keep it secret.
key = "KEY"

# Directory for what the node keeps between runs (sequence counter, neighbors).
state = "STATE"

# Bootstrap neighbors: "host:port" (the key is asked for at first contact) or
# "host:port@wireguard-key". After the first contact the node remembers its
# neighbors, so this can stay short.
peers = []

# Announce this node so others can reach it.
announce = true

# Lite: never relay, receive no floods, ask for paths (small devices).
lite = false

# Offer and accept direct links with anyone (reveals this node's address to the peer).
direct = false
# Your other devices (their addresses): direct links with them always.
devices = []

# An interface for ordinary programs (Linux; needs CAP_NET_ADMIN): this node
# is fd<its address> there, and every node it knows is fd<theirs>.
# tun = "vork0"

# Ask the router to open a port to this node (UPnP, then PCP/NAT-PMP). Any
# free outside port will do: others learn where they reached it.
map_port = true

# This node's name as others see it (default: the machine's name).
# name = "laptop"

# A web dashboard (pairing codes, devices, settings) for a machine without a
# screen: open the link `vork web-link` prints.
# web = "0.0.0.0:8742"
# It's HTTPS, with a certificate the node makes itself (the browser asks once;
# `vork web-link` shows its fingerprint), or one you give, e.g. Let's Encrypt's:
# web_cert = "/etc/letsencrypt/live/node.example.org/fullchain.pem"
# web_key = "/etc/letsencrypt/live/node.example.org/privkey.pem"

# Who reaches this node's own services over vØrk: your devices always; other
# nodes only what's opened here (relaying for others is never affected).
#   "tcp/80"                 port 80 for every vØrk node
#   "udp/5000-5010"          a range
#   "tcp/22 from <address>"  for one node (its full 32-digit address)
#   "any from <address>"     everything, for one node
#   "icmp"                   ping
allow = []

# Offer this node as an exit node: the owner's devices may send their internet
# traffic (IPv4) out through it (Linux, with the interface on).
# exit = true
# Send this node's internet traffic out through one of the owner's devices
# (its address), or "auto": the fastest of them that offers itself.
# exit_via = "auto"

# Find other vØrk nodes on the local network by themselves (mDNS,
# _vork._udp.local, which tells them this node's key and port).
lan = true
# gateway = "192.168.1.1"

# Paths per session through different first hops, and how packets use them.
paths = 2
spread = "spray"

# warn | info | debug
log = "info"
# log_file = "vork.log"
"#;

/// Writes a new configuration file with the given key and state paths.
pub fn write_sample(path: &Path, key: &Path, state: &Path) -> Result<()> {
    if path.exists() {
        bail!("{} exists", path.display());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = SAMPLE
        .replace("KEY", &key.to_string_lossy().replace('\\', "\\\\"))
        .replace("STATE", &state.to_string_lossy().replace('\\', "\\\\"));
    std::fs::write(path, text)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_parses_and_paths_are_relative_to_the_file() {
        let dir = std::env::temp_dir().join(format!("vork-cfg-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("vork.toml");
        let _ = std::fs::remove_file(&path);
        write_sample(&path, Path::new("node.key"), Path::new("state")).unwrap();
        let c = FileConfig::load(&path).unwrap();
        assert_eq!(c.key.unwrap(), dir.join("node.key"));
        assert_eq!(c.state.unwrap(), dir.join("state"));
        assert_eq!(c.paths, Some(2));
        assert!(toml::from_str::<FileConfig>("bogus = 1").is_err());
        let _ = std::fs::remove_file(&path);
    }
}

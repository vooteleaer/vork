//! This device: the vØrk node (the network vØrk storage runs on), the
//! services, and the node's settings. The node is asked through its control
//! endpoint (`vork status`'s): whether it runs, its neighbors, what it knows.
//! Its connections are the `peers` of its configuration file (`vork.toml`),
//! changed in place, and a new one is also dialed at once. The services are
//! systemd user units on Linux (the .deb's; from the Flatpak, the host's
//! through `flatpak-spawn --host`) and, on Windows, processes the
//! utility itself starts at login (the installer's); the settings are the
//! daemon's `vork.toml`, edited in place so its comments stay.

use std::path::{Path, PathBuf};
use std::process::Command;

/// One neighbor of the node.
#[derive(Debug, Clone, PartialEq)]
pub struct Interface {
    /// Where it is (`host:port`), or its address when that's unknown.
    pub name: String,
    /// "neighbor", or "lite" for a small device that doesn't relay.
    pub kind: String,
    pub up: bool,
    pub rx: u64,
    pub tx: u64,
}

impl Interface {
    /// Nothing of the node's own is listed as a neighbor.
    pub fn is_local(&self) -> bool {
        false
    }
}

/// The vØrk node on this machine.
#[derive(Debug, Clone, Default)]
pub struct Network {
    /// The node answered.
    pub running: bool,
    /// The node's address.
    pub address: Option<String>,
    pub uptime_s: Option<f64>,
    pub interfaces: Vec<Interface>,
    /// Destinations it knows a route to.
    pub destinations: u64,
    /// Packets it relayed for others.
    pub relayed: u64,
    /// Where others see it (its outside address).
    pub seen_at: Vec<String>,
    /// The node's address on its interface for ordinary programs (`fd…`), if it has one.
    pub ip: Option<String>,
    /// Others dialed it and got an answer (ADR-0023): reachable directly.
    pub reached_by: u64,
    /// This node's name and overlay address (whether or not it has an interface).
    pub name: Option<String>,
    pub node_ip: Option<String>,
    /// Every node known by its announce (the node's `nodes`, as it gave them).
    pub nodes: Vec<serde_json::Value>,
    pub error: Option<String>,
}

/// Parses the node's status (`vork status --json`).
pub fn parse_status(json: &str) -> Result<Network, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("vork status: {e}"))?;
    if let Some(e) = v["error"].as_str() {
        return Err(e.to_string());
    }
    let interfaces = v["neighbors"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|n| Interface {
                    name: n["endpoint"].as_str().or(n["address"].as_str()).unwrap_or("?").to_string(),
                    kind: if n["lite"].as_bool() == Some(true) { "lite" } else { "neighbor" }.to_string(),
                    up: n["up"].as_bool().unwrap_or(false),
                    rx: 0,
                    tx: 0,
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Network {
        running: true,
        address: v["address"].as_str().map(str::to_string),
        uptime_s: v["uptime_s"].as_f64(),
        interfaces,
        destinations: v["destinations"].as_u64().unwrap_or(0),
        relayed: v["relay"]["packets"].as_u64().unwrap_or(0),
        seen_at: v["outside"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default(),
        ip: v["interface"]["address"].as_str().map(str::to_string),
        reached_by: v["reach"]["reached_by"].as_array().map(|a| a.len() as u64).unwrap_or(0),
        name: v["name"].as_str().map(str::to_string),
        node_ip: v["ip"].as_str().map(str::to_string),
        nodes: v["nodes"].as_array().cloned().unwrap_or_default(),
        error: None,
    })
}

/// What's said when no node answers.
pub const NO_NODE: &str = "the vØrk node isn't running on this device";

/// Asks the node how it is.
pub fn network() -> Network {
    let endpoint = vork_api::default_control_endpoint();
    match vork_api::control_request(&endpoint, r#"{"cmd":"status"}"#) {
        Ok(answer) => parse_status(&answer).unwrap_or_else(|e| Network { error: Some(e), ..Default::default() }),
        Err(_) => Network { error: Some(NO_NODE.into()), ..Default::default() },
    }
}

/// Whether the node answers.
pub fn node_up() -> bool {
    vork_api::control_request(&vork_api::default_control_endpoint(), r#"{"cmd":"status"}"#).is_ok()
}

/// One line saying how the node is.
pub fn summary(r: &Network) -> String {
    if !r.running {
        return r.error.clone().unwrap_or_else(|| NO_NODE.into());
    }
    let mut parts = Vec::new();
    if let Some(a) = &r.address {
        parts.push(format!("node {}", &a[..a.len().min(8)]));
    }
    if let Some(s) = r.uptime_s {
        parts.push(if s > 86_400.0 { format!("up {:.0} d", s / 86_400.0) } else if s > 3600.0 { format!("up {:.0} h", s / 3600.0) } else { format!("up {:.0} min", s / 60.0) });
    }
    let up = r.interfaces.iter().filter(|i| i.up).count();
    parts.push(format!("{up} of {} neighbor(s) up", r.interfaces.len()));
    parts.push(format!("{} destination(s) known", r.destinations));
    if r.relayed > 0 {
        parts.push(format!("{} packet(s) relayed for others", r.relayed));
    }
    parts.push(format!("configuration {}", config_path().display()));
    parts.join(" · ")
}

/// The node's configuration file: `$VORK_CONFIG`; the system node's
/// (`/etc/vork/vork.toml`) when that one runs; else the user's
/// (`~/.config/vork/vork.toml`, `%LOCALAPPDATA%\vork\node\vork.toml`).
pub fn config_path() -> PathBuf {
    if let Some(p) = std::env::var_os("VORK_CONFIG").filter(|v| !v.is_empty()) {
        return PathBuf::from(p);
    }
    #[cfg(windows)]
    {
        let local = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("vork").join("node").join("vork.toml");
        let system = PathBuf::from(std::env::var_os("ProgramData").unwrap_or_default()).join("vork").join("vork.toml");
        if !local.is_file() && system.is_file() {
            return system;
        }
        local
    }
    #[cfg(not(windows))]
    {
        let system = PathBuf::from("/etc/vork/vork.toml");
        if system.is_file() && Path::new("/var/lib/vork/control.sock").exists() {
            return system;
        }
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config")).join("vork").join("vork.toml")
    }
}

/// The strings of a TOML array `key = [ ... ]` (one line or several).
fn toml_list(text: &str, key: &str) -> Vec<String> {
    let Some((start, end)) = list_span(text, key) else { return Vec::new() };
    let body = &text[start..end];
    body.split(['[', ']', ',', '\n'])
        .map(|p| p.trim())
        .filter(|p| p.starts_with('"'))
        .map(|p| p.trim_matches('"').to_string())
        .collect()
}

/// Byte span of `key = [ ... ]`, from the key to the closing bracket.
fn list_span(text: &str, key: &str) -> Option<(usize, usize)> {
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        let t = line.trim_start();
        if !t.starts_with('#') && t.split('=').next().map(str::trim) == Some(key) && t.contains('[') {
            let open = at + line.find('[')?;
            let close = open + text[open..].find(']')?;
            return Some((at, close + 1));
        }
        at += line.len();
    }
    None
}

/// The text with `key = [ ... ]` set to `items` (added at the end if missing).
fn set_toml_list(text: &str, key: &str, items: &[String]) -> String {
    let value = if items.is_empty() {
        format!("{key} = []")
    } else {
        format!("{key} = [\n{}\n]", items.iter().map(|i| format!("    \"{i}\",")).collect::<Vec<_>>().join("\n"))
    };
    match list_span(text, key) {
        Some((a, b)) => format!("{}{value}{}", &text[..a], &text[b..]),
        None => {
            let sep = if text.is_empty() || text.ends_with('\n') { "" } else { "\n" };
            format!("{text}{sep}{value}\n")
        }
    }
}

/// A configured connection: `HOST:PORT` (its key asked for) or `HOST:PORT@KEY`.
#[derive(Debug, Clone, PartialEq)]
pub struct Configured {
    pub name: String,
    pub kind: String,
    pub enabled: bool,
}

/// The connections in a node configuration: `peers` on, `peers_off` off.
pub fn parse_config_peers(text: &str) -> Vec<Configured> {
    let on = toml_list(text, "peers").into_iter().map(|p| Configured { name: p, kind: "neighbor".into(), enabled: true });
    let off = toml_list(text, "peers_off").into_iter().map(|p| Configured { name: p, kind: "neighbor".into(), enabled: false });
    on.chain(off).collect()
}

/// The node's configured connections, on or off.
pub fn configured() -> Result<Vec<Configured>, String> {
    Ok(parse_config_peers(&config_text()?))
}

fn edit_config(edit: impl FnOnce(&str) -> Result<String, String>) -> Result<(), String> {
    let path = config_path();
    let text = config_text()?;
    let new = edit(&text)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(&path, new).map_err(|e| format!("{}: {e} (the system node's configuration needs an administrator)", path.display()))
}

fn peer_text(host: &str, port: u16) -> String {
    if host.contains(':') && !host.contains(']') { format!("[{host}]:{port}") } else { format!("{host}:{port}") }
}

/// The configuration with connection `peer` added.
pub fn config_add_peer(text: &str, peer: &str) -> Result<String, String> {
    let mut on = toml_list(text, "peers");
    if on.iter().chain(toml_list(text, "peers_off").iter()).any(|p| p == peer) {
        return Err(format!("{peer} is there already"));
    }
    on.push(peer.to_string());
    Ok(set_toml_list(text, "peers", &on))
}

/// The configuration with connection `peer` switched on or off.
pub fn config_set_enabled(text: &str, peer: &str, enabled: bool) -> Result<String, String> {
    let (mut on, mut off) = (toml_list(text, "peers"), toml_list(text, "peers_off"));
    let (from, to) = if enabled { (&mut off, &mut on) } else { (&mut on, &mut off) };
    let i = from.iter().position(|p| p == peer).ok_or_else(|| format!("no connection {peer}"))?;
    to.push(from.remove(i));
    Ok(set_toml_list(&set_toml_list(text, "peers", &on), "peers_off", &off))
}

/// The configuration without connection `peer`.
pub fn config_remove(text: &str, peer: &str) -> Result<String, String> {
    let (mut on, mut off) = (toml_list(text, "peers"), toml_list(text, "peers_off"));
    let before = on.len() + off.len();
    on.retain(|p| p != peer);
    off.retain(|p| p != peer);
    if on.len() + off.len() == before {
        return Err(format!("no connection {peer}"));
    }
    Ok(set_toml_list(&set_toml_list(text, "peers", &on), "peers_off", &off))
}

/// Asks the running node to connect to `peer` now.
fn dial(peer: &str) {
    let req = serde_json::json!({ "cmd": "peer", "addr": peer }).to_string();
    let _ = vork_api::control_request(&vork_api::default_control_endpoint(), &req);
}

/// Adds a connection to another vØrk node (`name` is only a label; the
/// address is what counts), and dials it at once.
pub fn add_tcp(_name: &str, host: &str, port: u16) -> Result<(), String> {
    let peer = peer_text(host, port);
    edit_config(|t| config_add_peer(t, &peer))?;
    dial(&peer);
    Ok(())
}

pub fn set_interface(name: &str, enabled: bool) -> Result<(), String> {
    edit_config(|t| config_set_enabled(t, name, enabled))?;
    if enabled {
        dial(name);
    }
    Ok(())
}

pub fn remove_interface(name: &str) -> Result<(), String> {
    edit_config(|t| config_remove(t, name))
}

/// The node configuration file's text (empty if there's none yet).
pub fn config_text() -> Result<String, String> {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(t) => Ok(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Replaces the node configuration with `text` (the previous one kept beside
/// it as `vork.toml.previous`); the node takes it up when it next starts.
/// Returns whether it's in use already (never: a restart is needed).
pub fn save_config(text: &str) -> Result<bool, String> {
    let path = config_path();
    let old = config_text()?;
    if old == text {
        return Ok(true);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let previous = path.with_extension("toml.previous");
    std::fs::write(&previous, &old).map_err(|e| format!("{}: {e}", previous.display()))?;
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(false)
}

fn first_line(bytes: &[u8], or: &str) -> String {
    String::from_utf8_lossy(bytes).lines().find(|l| !l.trim().is_empty()).map(str::to_string).unwrap_or_else(|| or.to_string())
}

/// The user services the vØrk storage package installs (Linux, systemd).
#[cfg(not(any(windows, target_os = "haiku")))]
pub const SERVICES: &[(&str, &str)] = &[("vorkd.service", "vork daemon"), ("vork.service", "vØrk network (yours; a system node may run instead)"), ("vork-gateway.service", "Libraries as local websites")];
/// On Windows: the programs next to the utility, which it starts itself:
/// the daemon, the vØrk node it runs on, and the libraries in Explorer.
#[cfg(windows)]
pub const SERVICES: &[(&str, &str)] = &[("vorkd", "vork daemon"), ("vork", "vØrk network"), ("vork-cloudfiles", "Libraries in Explorer")];
/// On Haiku: launch_daemon jobs of the user's (`~/config/settings/launch/vork`,
/// or a package's `data/user_launch`), started at login.
#[cfg(target_os = "haiku")]
pub const SERVICES: &[(&str, &str)] = &[("x-vnd.vork-storage-daemon", "vork daemon"), ("x-vnd.vork-node", "vØrk network")];

/// The service vØrk storage itself runs as.
pub const VORKD: &str = SERVICES[0].0;

/// A user service's state ("active", "inactive", "failed", …), None where
/// there's no systemd.
#[cfg(not(any(windows, target_os = "haiku")))]
pub fn service_state(unit: &str) -> Option<String> {
    if in_flatpak() && !host_has_unit(unit) {
        return Some(NOT_INSTALLED.into());
    }
    let o = host("systemctl").args(["--user", "is-active", unit]).output().ok()?;
    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// `start`, `stop` or `restart` a user service.
#[cfg(not(any(windows, target_os = "haiku")))]
pub fn service(unit: &str, action: &str) -> Result<(), String> {
    if in_flatpak() && !host_has_unit(unit) {
        return Err(NOT_INSTALLED.into());
    }
    let o = host("systemctl").args(["--user", action, unit]).output().map_err(|e| e.to_string())?;
    if o.status.success() { Ok(()) } else { Err(first_line(&o.stderr, "systemctl failed")) }
}

/// Running as a Flatpak (FLATPAK_ID set): vØrk storage itself is the host's (the
/// vØrk storage .deb: the FUSE mount can't work in a sandbox), its services and
/// prnsd reached through `flatpak-spawn --host`.
pub fn in_flatpak() -> bool {
    cfg!(target_os = "linux") && std::env::var_os("FLATPAK_ID").is_some()
}

/// What the Flatpak says where the host has no vØrk storage.
pub const NOT_INSTALLED: &str = "vork isn't installed on this computer: install the vork .deb";

/// A program of this machine's: in the Flatpak, run on the host.
#[cfg_attr(windows, allow(dead_code))]
fn host(program: impl AsRef<std::ffi::OsStr>) -> Command {
    if in_flatpak() {
        let mut cmd = Command::new("flatpak-spawn");
        cmd.arg("--host").arg(program);
        cmd
    } else {
        Command::new(program)
    }
}

/// Whether the host has a user unit (the vØrk storage .deb installs them).
#[cfg(not(any(windows, target_os = "haiku")))]
fn host_has_unit(unit: &str) -> bool {
    host("systemctl").args(["--user", "cat", unit]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
}

/// Whether vØrk storage is missing on the host (the Flatpak); false elsewhere.
pub fn vork_missing() -> bool {
    #[cfg(not(any(windows, target_os = "haiku")))]
    if in_flatpak() {
        return !host_has_unit(VORKD);
    }
    false
}

/// A launch_daemon job's state: "active" while it runs, "inactive" when it
/// is known but not running. A job the launch_daemon doesn't know yet (a
/// package's jobs are read at login, so not until the next one after
/// installing) is its program, run by the utility itself.
#[cfg(target_os = "haiku")]
pub fn service_state(job: &str) -> Option<String> {
    let o = Command::new("launch_roster").args(["info", job]).output().ok()?;
    // The job's, or the program run here.
    let running = (roster_ok(&o) && job_running(&String::from_utf8_lossy(&o.stdout))) || !haiku_teams(job).is_empty();
    Some(if running { "active" } else { "inactive" }.to_string())
}

/// A job's program: its name, what starts it, and the program that then
/// runs (as `ps` shows it).
#[cfg(target_os = "haiku")]
fn job_program(job: &str) -> Option<(&'static str, PathBuf, PathBuf)> {
    match job {
        "x-vnd.vork-storage-daemon" => Some(("vorkd", install_dir().join("bin/vorkd"), install_dir().join("bin/vorkd"))),
        NODE_JOB => Some(("vork", install_dir().join("bin/vork"), install_dir().join("bin/vork"))),
        _ => None,
    }
}

/// The teams running a job's program.
#[cfg(target_os = "haiku")]
fn haiku_teams(job: &str) -> Vec<String> {
    let Some((_, _, runs)) = job_program(job) else { return Vec::new() };
    let path = runs.to_string_lossy();
    let Ok(o) = Command::new("ps").output() else { return Vec::new() };
    teams_running(&String::from_utf8_lossy(&o.stdout), &path)
}

/// Team IDs of `ps` lines running `path` (Haiku's `ps`: the command line,
/// then team ID, threads, group and user; the ID is the fourth field from
/// the end).
pub fn teams_running(ps: &str, path: &str) -> Vec<String> {
    ps.lines()
        .filter(|l| l.split_whitespace().next() == Some(path))
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            f.len().checked_sub(4).map(|i| f[i].to_string())
        })
        .collect()
}

/// Runs a job's program itself (the launch_daemon doesn't know the job yet),
/// its output in `~/config/var/log/vork/<program>.log`.
#[cfg(target_os = "haiku")]
fn run_program(job: &str, action: &str) -> Result<(), String> {
    let (prog, starts, _) = job_program(job).ok_or_else(|| format!("no job {job}"))?;
    if action == "stop" || action == "restart" {
        for team in haiku_teams(job) {
            let _ = Command::new("kill").arg(&team).output();
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    if action == "start" || action == "restart" {
        if !haiku_teams(job).is_empty() {
            return Ok(());
        }
        if prog == "vork" && node_up() {
            return Err("a vØrk node runs on this machine already".into());
        }
        // The node first: vorkd runs on it.
        if prog == "vorkd" && !node_up() {
            run_program(NODE_JOB, "start")?;
            for _ in 0..30 {
                if node_up() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        }
        let logs = PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("config/var/log/vork");
        std::fs::create_dir_all(&logs).map_err(|e| e.to_string())?;
        let log = std::fs::OpenOptions::new().create(true).append(true).open(logs.join(format!("{prog}.log"))).map_err(|e| e.to_string())?;
        Command::new(&starts)
            .args(if prog == "vork" { &["run"][..] } else { &[] })
            .stdin(std::process::Stdio::null())
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log)
            .spawn()
            .map_err(|e| format!("{prog}: {e}"))?;
    }
    Ok(())
}

/// Whether `launch_roster info` says the job runs (`running = bool(true)`).
pub fn job_running(info: &str) -> bool {
    info.lines().any(|l| {
        let l = l.trim();
        l.starts_with("running") && l.contains("bool(true)")
    })
}

/// `start`, `stop` or `restart` a launch_daemon job, or, while the
/// launch_daemon doesn't know it yet, its program.
#[cfg(target_os = "haiku")]
pub fn service(job: &str, action: &str) -> Result<(), String> {
    if action == "restart" {
        service(job, "stop")?;
        std::thread::sleep(std::time::Duration::from_secs(1));
        return service(job, "start");
    }
    let known = Command::new("launch_roster").args(["info", job]).output().is_ok_and(|o| roster_ok(&o));
    if !known {
        return run_program(job, action);
    }
    let o = Command::new("launch_roster").args([action, job]).output().map_err(|e| e.to_string())?;
    if !roster_ok(&o) {
        return Err(first_line(&o.stderr, "launch_roster failed"));
    }
    if action == "stop" {
        // And one run here, if any.
        return run_program(job, "stop");
    }
    // A job known from an older install (one that requires a job no longer
    // there) "starts" and doesn't run: then the program, run here.
    for _ in 0..10 {
        if service_state(job).as_deref() == Some("active") {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    run_program(job, "start")
}

/// Whether launch_roster did it: it exits with 0 also when it didn't (an
/// unknown job: "Name not found"), saying so only on stderr.
#[cfg(target_os = "haiku")]
fn roster_ok(o: &std::process::Output) -> bool {
    o.status.success() && o.stderr.iter().all(u8::is_ascii_whitespace)
}

static INSTALL_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Says where the utility is installed, when the program running isn't it
/// (Haiku: the Deskbar replicant runs inside the Deskbar). Only the first
/// call counts.
pub fn set_install_dir(dir: PathBuf) {
    let _ = INSTALL_DIR.set(dir);
}

/// Where the utility is installed, with the programs it starts (Windows, Haiku).
pub fn install_dir() -> PathBuf {
    if let Some(dir) = INSTALL_DIR.get() {
        return dir.clone();
    }
    std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(|| PathBuf::from("."))
}

/// The Haiku job running the vØrk node.
#[cfg(target_os = "haiku")]
const NODE_JOB: &str = "x-vnd.vork-node";

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(windows)]
fn running(exe: &str) -> bool {
    use std::os::windows::process::CommandExt;
    Command::new("tasklist")
        .args(["/FI", &format!("IMAGENAME eq {exe}"), "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_ascii_lowercase().contains(&exe.to_ascii_lowercase()))
        .unwrap_or(false)
}

/// A program's state: "active" while it runs.
#[cfg(windows)]
pub fn service_state(name: &str) -> Option<String> {
    Some(if running(&format!("{name}.exe")) { "active" } else { "inactive" }.to_string())
}

/// `start`, `stop` or `restart` one of the programs next to the utility,
/// hidden, its output in `%LOCALAPPDATA%\vork\logs\<name>.log`.
#[cfg(windows)]
pub fn service(name: &str, action: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let exe = format!("{name}.exe");
    if name == "vork-cloudfiles" {
        // Stopped on purpose: `keep` leaves it stopped until started again.
        CLOUDFILES_STOPPED.store(action == "stop", std::sync::atomic::Ordering::Relaxed);
    }
    if action == "stop" || action == "restart" {
        // vorkd is asked to stop, so it closes its store (killed, its index
        // is checked at the next start: minutes on a large store); killed
        // only if it hasn't stopped within 35 s.
        if name == "vorkd" && running(&exe) && crate::Daemon::new().stop().is_ok() {
            for _ in 0..70 {
                if !running(&exe) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        }
        if running(&exe) {
            let _ = Command::new("taskkill").args(["/IM", &exe, "/F"]).creation_flags(CREATE_NO_WINDOW).output();
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }
    if action == "start" || action == "restart" {
        let logs = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("vork").join("logs");
        std::fs::create_dir_all(&logs).map_err(|e| e.to_string())?;
        let log = std::fs::OpenOptions::new().create(true).append(true).open(logs.join(format!("{name}.log"))).map_err(|e| e.to_string())?;
        // The Cloud Files provider: every library, a folder each of
        // %USERPROFILE%\vork.
        let config = config_path();
        if name == "vork" && !config.is_file() {
            // First start: a configuration and an identity of this user's own.
            let o = Command::new(install_dir().join("vork.exe"))
                .args(["init", "--config"])
                .arg(&config)
                .creation_flags(CREATE_NO_WINDOW)
                .output()
                .map_err(|e| format!("vork init: {e}"))?;
            if !o.status.success() {
                return Err(first_line(&o.stderr, "vork init failed"));
            }
        }
        let config_arg = config.to_string_lossy().into_owned();
        let args: Vec<&str> = match name {
            "vork-cloudfiles" => vec!["--all"],
            "vork" => vec!["run", "--config", &config_arg],
            _ => vec![],
        };
        Command::new(install_dir().join(&exe))
            .args(args)
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log)
            // No console, and a process group of its own.
            .creation_flags(CREATE_NO_WINDOW | 0x0000_0200)
            .spawn()
            .map_err(|e| format!("{exe}: {e}"))?;
    }
    Ok(())
}

/// Starts this user's vØrk node (Windows) unless one answers already (the
/// system's service, or one started before): `vork run` with its own
/// configuration, made on first use. Says what it did.
#[cfg(windows)]
fn start_node(did: &mut Vec<String>) {
    if node_up() || running("vork.exe") || !install_dir().join("vork.exe").is_file() {
        return;
    }
    match service("vork", "start") {
        Ok(()) => did.push("started the vØrk node".to_string()),
        Err(e) => did.push(format!("vork: {e}")),
    }
    for _ in 0..40 {
        if node_up() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

/// At login (Windows): the vØrk node, vorkd (which waits for the node by
/// itself), and the libraries in Explorer (which wait for vorkd). Says what it did.
#[cfg(windows)]
pub fn supervise() -> Vec<String> {
    let mut did = Vec::new();
    start_node(&mut did);
    if !running("vorkd.exe") {
        match service("vorkd", "start") {
            Ok(()) => did.push("started vorkd".to_string()),
            Err(e) => did.push(format!("vorkd: {e}")),
        }
    }
    if install_dir().join("vork-cloudfiles.exe").is_file() && !running("vork-cloudfiles.exe") {
        match service("vork-cloudfiles", "start") {
            Ok(()) => did.push("libraries in Explorer".to_string()),
            Err(e) => did.push(format!("vork-cloudfiles: {e}")),
        }
    }
    did
}

/// Keeps vØrk running while the utility does (Windows), as the Android app
/// does: the node and vorkd started again if they stopped (vorkd finds the
/// node again by itself when it comes back). `away`: checks in a row the
/// node didn't answer. Says what it did.
#[cfg(windows)]
pub fn keep(away: &mut u32) -> Vec<String> {
    let mut did = Vec::new();
    // The libraries in Explorer: started again if they stopped by themselves
    // (a crash would otherwise leave Explorer's folders stale until the next login).
    if running("vorkd.exe")
        && !CLOUDFILES_STOPPED.load(std::sync::atomic::Ordering::Relaxed)
        && install_dir().join("vork-cloudfiles.exe").is_file()
        && !running("vork-cloudfiles.exe")
    {
        match service("vork-cloudfiles", "start") {
            Ok(()) => did.push("the libraries in Explorer had stopped: started them again".to_string()),
            Err(e) => did.push(format!("vork-cloudfiles: {e}")),
        }
    }
    let up = node_up();
    // Two missed checks in a row: the node is gone, not just busy.
    if !up && *away >= 1 {
        start_node(&mut did);
    }
    if !running("vorkd.exe") {
        match service("vorkd", "start") {
            Ok(()) => did.push("vorkd had stopped: started it again".to_string()),
            Err(e) => did.push(format!("vorkd: {e}")),
        }
    }
    *away = if up { 0 } else { *away + 1 };
    did
}

/// Set while the user has stopped the libraries in Explorer (Windows).
#[cfg(windows)]
static CLOUDFILES_STOPPED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The settings the utility offers, of `vork.toml`'s: (section, key, what it is).
pub const SETTINGS: [(&str, &str, &str); 7] = [
    ("node", "device", "This device's name"),
    ("node", "site", "Where it is (one copy of its files is kept at another site)"),
    ("store", "max_store", "Store limit"),
    ("store", "min_free", "Always leave this much free on the disk"),
    ("store", "no_storage", "Keep nothing of my own here (only a keyring)"),
    ("placement", "hold_for_others", "Hold others' encrypted copies up to (0: none)"),
    ("placement", "min_copies", "Copies of each of my files, this one included"),
];

pub fn settings_file(store: &Path) -> PathBuf {
    store.join("vork.toml")
}

/// A setting's value in the file (as written, e.g. `"50G"`, `3`, `true`),
/// None while it's commented out (the default).
pub fn get_setting(text: &str, section: &str, key: &str) -> Option<String> {
    let mut current = "";
    for line in text.lines() {
        let t = line.trim();
        if let Some(s) = t.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            current = s.trim();
        } else if current == section {
            if let Some((k, v)) = t.split_once('=') {
                if k.trim() == key {
                    return Some(strip_comment(v).trim().to_string());
                }
            }
        }
    }
    None
}

fn strip_comment(v: &str) -> &str {
    // A value never holds '#' outside quotes in these settings.
    let mut quoted = false;
    for (i, c) in v.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '#' if !quoted => return &v[..i],
            _ => {}
        }
    }
    v
}

/// Sets a setting (a TOML value: `"50G"`, `3`, `true`), or with None puts
/// it back to its default (commented out). The line's own comment stays; a
/// key missing from the file is added to its section (made if missing).
pub fn set_setting(text: &str, section: &str, key: &str, value: Option<&str>) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut done = false;
    let mut section_end: Option<usize> = None;
    for line in text.lines() {
        let t = line.trim();
        if let Some(s) = t.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            if current == section && section_end.is_none() {
                // After its last line that isn't blank.
                let mut at = out.len();
                while at > 0 && out[at - 1].trim().is_empty() {
                    at -= 1;
                }
                section_end = Some(at);
            }
            current = s.trim().to_string();
            out.push(line.to_string());
            continue;
        }
        if current == section && !done {
            let body = t.strip_prefix('#').map(str::trim_start).unwrap_or(t);
            if let Some((k, rest)) = body.split_once('=') {
                if k.trim() == key {
                    let old = strip_comment(rest);
                    let note = rest[old.len()..].trim();
                    let pad = |s: String| if note.is_empty() { s } else { format!("{s:<28} {note}") };
                    out.push(match value {
                        Some(v) => pad(format!("{key} = {v}")),
                        None if t.starts_with('#') => line.to_string(),
                        None => pad(format!("# {key} = {}", old.trim())),
                    });
                    done = true;
                    continue;
                }
            }
        }
        out.push(line.to_string());
    }
    if current == section && section_end.is_none() {
        section_end = Some(out.len());
    }
    if !done {
        if let Some(v) = value {
            match section_end {
                Some(at) => out.insert(at, format!("{key} = {v}")),
                None => {
                    out.push(String::new());
                    out.push(format!("[{section}]"));
                    out.push(format!("{key} = {v}"));
                }
            }
        }
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// A value typed by the user, as TOML: numbers and true/false as they are,
/// anything else quoted.
pub fn toml_value(typed: &str) -> String {
    let t = typed.trim();
    if t == "true" || t == "false" || t.parse::<i64>().is_ok() || t.parse::<f64>().is_ok() {
        t.to_string()
    } else {
        format!("\"{}\"", t.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// A value from the file, as the user reads it (quotes off).
pub fn shown_value(v: &str) -> String {
    v.trim().trim_matches('"').to_string()
}

/// The settings that are sizes: vorkd reads them as `250G`, `500M`
/// (binary units); the utility shows and takes `250 GB`, `500 MB`.
pub const SIZE_SETTINGS: [&str; 3] = ["max_store", "min_free", "hold_for_others"];

pub fn is_size(key: &str) -> bool {
    SIZE_SETTINGS.contains(&key)
}

const KIB: u64 = 1 << 10;
const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;
const TIB: u64 = 1 << 40;

/// A size as typed (`250 GB`, `250gb`, `250G`, `1,5 TB`, `500 MB`, `0`; a
/// bare number is in GB) as vorkd reads it (`250G`, `1.5T`, `500M`, `0`).
/// None if it isn't one.
pub fn size_typed(typed: &str) -> Option<String> {
    let t = typed.trim().replace(',', ".").to_ascii_uppercase();
    let split = t.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(t.len());
    let (num, unit) = (t[..split].trim(), t[split..].trim());
    let n: f64 = num.parse().ok().filter(|n: &f64| n.is_finite() && *n >= 0.0)?;
    let unit = match unit.trim_end_matches("IB").trim_end_matches('B') {
        "" if unit.is_empty() => "G",
        "" => "",
        "K" => "K",
        "M" => "M",
        "G" => "G",
        "T" => "T",
        _ => return None,
    };
    let num = if n.fract() == 0.0 { format!("{}", n as u64) } else { format!("{n}") };
    Some(if n == 0.0 { "0".into() } else { format!("{num}{unit}") })
}

/// A size as vorkd reads it (`250G`) in bytes, as vorkd counts them.
pub fn size_bytes(v: &str) -> Option<u64> {
    let v = v.trim().trim_matches('"').to_ascii_uppercase();
    let (num, mult) = match v.chars().last() {
        Some('K') => (&v[..v.len() - 1], KIB),
        Some('M') => (&v[..v.len() - 1], MIB),
        Some('G') => (&v[..v.len() - 1], GIB),
        Some('T') => (&v[..v.len() - 1], TIB),
        _ => (&v[..], 1),
    };
    num.parse::<f64>().ok().map(|n| (n * mult as f64) as u64)
}

/// A size from vorkd's form (`250G`) as the user reads it (`250 GB`).
pub fn size_shown(v: &str) -> String {
    let v = v.trim().trim_matches('"');
    match v.chars().last().map(|c| c.to_ascii_uppercase()) {
        Some(u @ ('K' | 'M' | 'G' | 'T')) => format!("{} {u}B", &v[..v.len() - 1]),
        _ if v == "0" => "0".into(),
        _ => match v.parse::<u64>() {
            Ok(n) => bytes_shown(n),
            Err(_) => v.to_string(),
        },
    }
}

/// Bytes as a round size to show, as the utility shows usage: TB with one
/// decimal from 1 TB, whole GB from 10 GB, GB with one decimal from 1 GB, MB
/// below.
pub fn bytes_shown(n: u64) -> String {
    let n = n as f64;
    if n >= TIB as f64 {
        format!("{:.1} TB", n / TIB as f64)
    } else if n >= 10.0 * GIB as f64 {
        format!("{:.0} GB", n / GIB as f64)
    } else if n >= GIB as f64 {
        format!("{:.1} GB", n / GIB as f64)
    } else {
        format!("{:.0} MB", n / MIB as f64)
    }
}

/// The machine's name as vorkd takes it for the device's (lower case).
fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        // Haiku has neither: its `hostname` says.
        .or_else(|| {
            let cmd = ["/boot/system/bin/hostname", "hostname"].into_iter().find(|c| !c.starts_with('/') || Path::new(c).exists())?;
            Command::new(cmd).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        })
        .map(|h| h.trim().to_lowercase())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "this-device".into())
}

/// The size of the disk the store is on (of its nearest existing folder).
fn disk_total(store: &Path) -> Option<u64> {
    let mut p = Some(store);
    while let Some(dir) = p {
        if dir.exists() {
            return fs4::total_space(dir).ok();
        }
        p = dir.parent();
    }
    None
}

/// What vorkd takes for a setting left at its default, here (as shown):
/// the store limit is half the disk, at least 10 % of it (5 GB at least) is
/// left free, others' copies are held up to the store limit, 3 copies, the
/// machine's name. None where there's no value (a site).
pub fn setting_default(store: &Path, key: &str) -> Option<String> {
    let total = disk_total(store);
    match key {
        "max_store" | "hold_for_others" => total.map(|t| bytes_shown(t / 2)),
        "min_free" => total.map(|t| bytes_shown((t / 10).max(5 * GIB))),
        "min_copies" => Some("3".into()),
        "device" => Some(hostname()),
        "no_storage" => Some("false".into()),
        _ => None,
    }
}

/// A setting as a front end shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingView {
    pub section: &'static str,
    pub key: &'static str,
    pub what: &'static str,
    /// What the field holds: the file's value, else the default this machine
    /// gets (so no field is empty where there is one); sizes with their unit.
    pub value: String,
    pub default: Option<String>,
    /// Whether the file sets it (else it's the default).
    pub set: bool,
    /// The unit to show by the field ("GB, MB or TB", "copies"), or "".
    pub unit: &'static str,
}

/// The settings of the store's `vork.toml`, as the front ends show them.
pub fn settings_view(store: &Path) -> Vec<SettingView> {
    let text = std::fs::read_to_string(settings_file(store)).unwrap_or_default();
    SETTINGS
        .iter()
        .map(|&(section, key, what)| {
            let set = get_setting(&text, section, key).map(|v| if is_size(key) { size_shown(&v) } else { shown_value(&v) });
            let default = setting_default(store, key);
            SettingView {
                section,
                key,
                what,
                value: set.clone().or_else(|| default.clone()).unwrap_or_default(),
                default,
                set: set.is_some(),
                unit: match key {
                    k if is_size(k) => "GB, MB or TB",
                    "min_copies" => "copies",
                    _ => "",
                },
            }
        })
        .collect()
}

/// Saves what the fields hold, `(section, key, typed)` (a switch: "true" or
/// ""): only what changed is written, and a field left at its default stays
/// the default. Sizes are taken with units (`250 GB`) and written as vorkd
/// reads them (`250G`). The caller restarts the daemon.
pub fn save_typed(store: &Path, typed: &[(&str, &str, String)]) -> Result<(), String> {
    let path = settings_file(store);
    let mut text = std::fs::read_to_string(&path).unwrap_or_default();
    for &(section, key, what) in SETTINGS.iter() {
        let Some((_, _, t)) = typed.iter().find(|(s, k, _)| *s == section && *k == key) else { continue };
        let default = setting_default(store, key);
        let now = typed_setting(key, what, t, default.as_deref())?;
        if get_setting(&text, section, key) != now {
            text = set_setting(&text, section, key, now.as_deref());
        }
    }
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// A typed setting as the file holds it, None when it is the default (the
/// file then leaves it commented out, and vorkd keeps deriving it); an
/// error for a size that isn't one.
pub fn typed_setting(key: &str, what: &str, typed: &str, default: Option<&str>) -> Result<Option<String>, String> {
    let typed = typed.trim();
    if typed.is_empty() {
        return Ok(None);
    }
    if is_size(key) {
        let v = size_typed(typed).ok_or_else(|| format!("{what}: \"{typed}\" isn't a size (like 250 GB, 500 MB or 1.5 TB)"))?;
        let same = default.and_then(size_typed).is_some_and(|d| d == v);
        return Ok((!same).then(|| toml_value(&v)));
    }
    if default.is_some_and(|d| d == typed) {
        return Ok(None);
    }
    Ok(Some(toml_value(typed)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The node's connections: `peers` and `peers_off` of its vork.toml.
    #[test]
    fn peers_read_and_changed() {
        let text = "# vØrk node configuration.
listen = \"0.0.0.0:51820\"

# Bootstrap neighbors.
peers = [\"10.0.0.1:51820\"]

log = \"info\"
";
        assert_eq!(parse_config_peers(text), [Configured { name: "10.0.0.1:51820".into(), kind: "neighbor".into(), enabled: true }]);
        let added = config_add_peer(text, "hub.example:51820").unwrap();
        assert_eq!(parse_config_peers(&added).len(), 2);
        assert!(config_add_peer(&added, "hub.example:51820").is_err());
        let off = config_set_enabled(&added, "10.0.0.1:51820", false).unwrap();
        assert_eq!(parse_config_peers(&off), [
            Configured { name: "hub.example:51820".into(), kind: "neighbor".into(), enabled: true },
            Configured { name: "10.0.0.1:51820".into(), kind: "neighbor".into(), enabled: false },
        ]);
        let gone = config_remove(&off, "10.0.0.1:51820").unwrap();
        assert_eq!(parse_config_peers(&gone).len(), 1);
        assert!(gone.contains("# Bootstrap neighbors.") && gone.contains("log = \"info\""));
        // None yet: the list is made.
        let fresh = config_add_peer("listen = \"0.0.0.0:51820\"\n", "a:1").unwrap();
        assert_eq!(parse_config_peers(&fresh)[0].name, "a:1");
        // The node reads what was written.
        assert!(fresh.contains("peers = [\n    \"a:1\",\n]"));
    }

    const FILE: &str = "# vØrk storage node settings.\n\n[store]\n# max_store = \"...\"          # store limit\n# no_storage = false         # keep nothing\n\n[node]\n# device = \"...\"             # this device's name\n";

    #[test]
    fn sizes_are_shown_and_typed_with_units() {
        assert_eq!(size_typed("250 GB").as_deref(), Some("250G"));
        assert_eq!(size_typed("250gb").as_deref(), Some("250G"));
        assert_eq!(size_typed("250G").as_deref(), Some("250G"));
        assert_eq!(size_typed("1,5 TB").as_deref(), Some("1.5T"));
        assert_eq!(size_typed("500 MiB").as_deref(), Some("500M"));
        assert_eq!(size_typed("250").as_deref(), Some("250G"), "a bare number is GB");
        assert_eq!(size_typed("0 GB").as_deref(), Some("0"));
        assert_eq!(size_typed("lots"), None);
        assert_eq!(size_typed("5 PB"), None);
        assert_eq!(size_shown("\"250G\""), "250 GB");
        assert_eq!(size_shown("1.5T"), "1.5 TB");
        assert_eq!(size_shown("0"), "0");
        assert_eq!(size_bytes("\"2G\""), Some(2 << 30));
        assert_eq!(bytes_shown(500 << 30), "500 GB");
        assert_eq!(bytes_shown((20 << 40) + (3 << 39)), "21.5 TB");
        assert_eq!(bytes_shown(300 << 20), "300 MB");
        assert_eq!(bytes_shown((20 << 30) - (40 << 20)), "20 GB", "as the usage line shows it");
        assert_eq!(bytes_shown(5 << 30), "5.0 GB");
        // A default shown and saved as it is stays the default.
        assert_eq!(size_typed("5.0 GB"), size_typed("5 GB"));
    }

    #[test]
    fn a_default_typed_stays_a_default() {
        let size = |t| typed_setting("max_store", "Store limit", t, Some("237 GB"));
        assert_eq!(size("237 GB"), Ok(None), "the default shown, saved: still the default");
        assert_eq!(size("237G"), Ok(None));
        assert_eq!(size("100 GB"), Ok(Some("\"100G\"".into())));
        assert_eq!(size(""), Ok(None));
        assert!(size("much").unwrap_err().contains("isn't a size"));
        assert_eq!(typed_setting("min_copies", "", "3", Some("3")), Ok(None));
        assert_eq!(typed_setting("min_copies", "", "2", Some("3")), Ok(Some("2".into())));
        assert_eq!(typed_setting("site", "", "kodu", None), Ok(Some("\"kodu\"".into())));
    }

    #[test]
    fn settings_are_set_and_put_back_with_their_comments() {
        let s = set_setting(FILE, "store", "max_store", Some("\"50G\""));
        assert!(s.contains("max_store = \"50G\"            # store limit"), "{s}");
        assert_eq!(get_setting(&s, "store", "max_store").as_deref(), Some("\"50G\""));
        assert_eq!(get_setting(&s, "node", "max_store"), None, "per section");
        let back = set_setting(&s, "store", "max_store", None);
        assert!(back.contains("# max_store = \"50G\""), "{back}");
        assert_eq!(get_setting(&back, "store", "max_store"), None);
        assert_eq!(set_setting(FILE, "node", "device", None), FILE, "a default stays as it was");
    }

    #[test]
    fn a_missing_key_or_section_is_added() {
        let s = set_setting(FILE, "store", "cache_max", Some("\"1G\""));
        assert!(s.contains("cache_max = \"1G\"\n\n[node]"), "{s}");
        let s = set_setting(FILE, "placement", "min_copies", Some("2"));
        assert!(s.ends_with("[placement]\nmin_copies = 2\n"), "{s}");
        assert_eq!(get_setting(&s, "placement", "min_copies").as_deref(), Some("2"));
    }

    #[test]
    fn typed_values_become_toml() {
        assert_eq!(toml_value("3"), "3");
        assert_eq!(toml_value("true"), "true");
        assert_eq!(toml_value("50G"), "\"50G\"");
        assert_eq!(toml_value("Prangli \"home\""), "\"Prangli \\\"home\\\"\"");
        assert_eq!(shown_value("\"50G\""), "50G");
    }

    #[test]
    fn haiku_ps_teams() {
        let ps = "Team                                                  Id #Threads  Gid  Uid \n\
                  kernel_team                                            1       51    0    0 \n\
                  /boot/system/apps/vork/bin/prnsd run                76       23    0    0 \n\
                  /boot/system/apps/vork/bin/vorkd                 105       17    0    0 \n\
                  /boot/system/apps/vork/bin/vorkd.old             107       17    0    0 \n";
        assert_eq!(teams_running(ps, "/boot/system/apps/vork/bin/vorkd"), ["105"]);
        assert_eq!(teams_running(ps, "/boot/system/apps/vork/bin/prnsd"), ["76"]);
        assert!(teams_running(ps, "/boot/home/vorkd").is_empty());
    }

    #[test]
    fn launch_roster_info() {
        let running = "Details: BMessage(0x0) {\n        name = string(\"x-vnd.vork-storage-daemon\", 20 bytes)\n        running = bool(true)\n        launched = bool(true)\n";
        assert!(job_running(running));
        assert!(!job_running(&running.replace("running = bool(true)", "running = bool(false)")));
    }

    #[test]
    fn node_status_json() {
        let r = parse_status(r#"{"address":"0123456789abcdef0123456789abcdef","uptime_s":42,"destinations":7,"relay":{"paths":0,"packets":12},"outside":["1.2.3.4:51820"],"neighbors":[{"address":"aa","endpoint":"10.1.2.3:51820","up":true,"lite":false},{"address":null,"endpoint":"10.0.0.9:51820","up":false,"lite":null}]}"#).unwrap();
        assert!(r.running);
        assert_eq!(r.address.as_deref(), Some("0123456789abcdef0123456789abcdef"));
        assert_eq!((r.destinations, r.relayed), (7, 12));
        assert_eq!(r.interfaces.len(), 2);
        assert!(r.interfaces[0].up && !r.interfaces[1].up);
        assert_eq!(r.interfaces[0].name, "10.1.2.3:51820");
        assert!(parse_status(r#"{"error":"unknown command"}"#).is_err());
    }
}

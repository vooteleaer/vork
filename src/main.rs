mod api;
mod apps;
mod config;
mod control;
mod identity;
mod node;
mod lan;
mod links;
mod portmap;
mod reach;
#[cfg(windows)]
mod service;
mod settings;
mod state;
mod stream;
mod tun;
mod dns;
mod alias;
mod bootstrap;
mod exit;
mod netstack;
mod firewall;
mod update;
mod web;
mod wire;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "vork",
    about = "vØrk: a serverless, identity-addressed network over WireGuard (prototype)",
    after_help = "Storage commands (put, get, query, library, mount, ...) are run by vork-storage; `vork help-storage` lists them."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Write a configuration file and create the node's identity.
    Init {
        /// Where to write it (default: /etc/vork/vork.toml as root, ~/.config/vork/vork.toml, or %ProgramData%\vork\vork.toml).
        #[arg(long)]
        config: Option<PathBuf>,
        /// Do nothing, successfully, when the file is already there (an installer's upgrade).
        #[arg(long)]
        if_missing: bool,
        /// Turn the interface on, with this name (`tun = "…"`).
        #[arg(long)]
        tun: Option<String>,
        /// The UDP port to listen on (default 51820), or `auto`: a free one
        /// chosen now (40000–59999), for a machine sharing its router with
        /// other nodes; the router is asked to open it (UPnP, PCP).
        #[arg(long)]
        port: Option<String>,
    },
    /// Create an identity (Ed25519 + X25519) and print its address and WireGuard key.
    Keygen {
        #[arg(long)]
        out: PathBuf,
    },
    /// Print an identity's address and WireGuard key.
    Show {
        #[arg(long)]
        key: Option<PathBuf>,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Run a node.
    Run(RunArgs),
    /// Ask a running node how it is doing.
    Status {
        #[arg(long)]
        config: Option<PathBuf>,
        /// Control socket or pipe (default: from the configuration).
        #[arg(long)]
        control: Option<String>,
        /// Print the raw JSON.
        #[arg(long)]
        json: bool,
    },
    /// Link another node as one of your devices (ADR-0024): it asks the other
    /// side, whose owner allows it (or a code from its QR code links at once).
    Link {
        /// Its address (32 hex digits), or a `vork:link/…` from its QR code.
        address: String,
        #[arg(long)]
        code: Option<String>,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// A link and its QR code, as Resilio's: a device that opens it is linked
    /// (at once, or once you allow it with --approve).
    LinkCode {
        /// How long it works: 10m, 3d… (default 10m, at most 30d).
        #[arg(long = "for", default_value = "10m")]
        valid: String,
        /// How many devices may use it (0: any) [default: 1].
        #[arg(long, default_value_t = 1)]
        uses: u64,
        /// Ask me for each device that uses it.
        #[arg(long)]
        approve: bool,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// This binary's build number and target.
    Version,
    /// Look for a newer release now (the running node asks its release node).
    Update {
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Put a downloaded, checked update in place of this binary (the service's
    /// start step on Linux, with the rights to replace it).
    UpdateInstall {
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Make release keys and signed releases (on the machine holding the key).
    Release {
        #[command(subcommand)]
        action: ReleaseAction,
    },
    /// The web dashboard's link (with its token): open it in a browser.
    WebLink {
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Who asks to be linked, and whom this node asked.
    Requests {
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Allow a node that asked to be linked.
    Allow {
        address: String,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Dismiss a node that asked to be linked.
    Dismiss {
        address: String,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// No longer one of your devices (the other side is told).
    Unlink {
        address: String,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Install or remove the Windows service.
    #[cfg(windows)]
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
    /// Entry point used by the Windows service manager.
    #[cfg(windows)]
    #[command(hide = true)]
    ServiceRun {
        #[arg(long)]
        config: PathBuf,
    },
}

#[derive(Subcommand)]
enum ReleaseAction {
    /// A new release key: its public half goes into nodes' `update_keys`.
    Keygen { path: PathBuf },
    /// A signed release in DIR from built binaries (TARGET=PATH, e.g. linux-x86_64=target/…/vork).
    Make {
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        dir: PathBuf,
        #[arg(long, default_value = "")]
        version: String,
        files: Vec<String>,
    },
}

#[cfg(windows)]
#[derive(Subcommand)]
enum ServiceAction {
    /// Register vØrk as an automatically started service (needs an administrator).
    Install {
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Stop and remove the service.
    Uninstall,
}

#[derive(Args, Default)]
pub struct RunArgs {
    /// Configuration file; flags below override it.
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    key: Option<PathBuf>,
    /// Directory for what the node keeps between runs.
    #[arg(long)]
    state: Option<PathBuf>,
    #[arg(long)]
    listen: Option<SocketAddr>,
    /// Bootstrap neighbor as HOST:PORT@X25519_HEX (repeatable; added to the configuration's).
    #[arg(long = "peer")]
    peers: Vec<String>,
    /// Don't announce this node (it can still reach others and relay).
    #[arg(long)]
    no_announce: bool,
    /// Ping a destination by its address (hex) once a second.
    #[arg(long)]
    ping: Option<String>,
    /// Lite: never relay, get no floods, ask for paths (small devices).
    #[arg(long)]
    lite: bool,
    /// Offer and accept direct links (reveals this node's addresses to the peer).
    #[arg(long)]
    direct: bool,
    /// Ask the router to open the port (UPnP, then PCP/NAT-PMP), like torrent
    /// clients do. The default; kept for older command lines.
    #[arg(long, hide = true)]
    map_port: bool,
    /// Don't ask the router to open the port.
    #[arg(long)]
    no_map_port: bool,
    /// Don't look for (or tell) other nodes on the local network.
    #[arg(long)]
    no_lan: bool,
    /// Router address for PCP/NAT-PMP (default: x.y.z.1 of this machine's address).
    #[arg(long)]
    gateway: Option<std::net::Ipv4Addr>,
    /// Paths per session, through different first hops (a direct link comes on top) [default: 2].
    #[arg(long)]
    paths: Option<usize>,
    /// How packets use the paths: spray (by round-trip time) or dup (every packet on every path) [default: spray].
    #[arg(long)]
    spread: Option<String>,
    /// Also announce this named service of the node's identity (repeatable).
    #[arg(long = "destination")]
    destinations: Vec<String>,
    /// Measure throughput: send to this address (hex); the receiver logs what arrives each second.
    #[arg(long)]
    bench: Option<String>,
    /// Test: request BYTES from a destination again and again (ADDRESS:BYTES).
    #[arg(long, hide = true)]
    fetch: Option<String>,
    /// Test: identify in the sessions this node opens.
    #[arg(long, hide = true)]
    identify: bool,
    /// Test: drop this share (0..1) of the session packets sent.
    #[arg(long, hide = true, default_value_t = 0.0)]
    drop: f64,
    /// Bench sending rate in Mbit/s.
    #[arg(long, default_value_t = 100)]
    bench_rate: u64,
    /// warn | info | debug [default: info].
    #[arg(long)]
    log: Option<String>,
    /// Append the log to this file instead of standard error.
    #[arg(long)]
    log_file: Option<PathBuf>,
    /// Control socket or pipe (default: in the state directory, or \\.\pipe\vork-<port>).
    #[arg(long)]
    control: Option<String>,
}

fn parse_peer(s: &str) -> Result<(SocketAddr, Option<[u8; 32]>)> {
    node::parse_peer_text(s).map_err(|e| anyhow::anyhow!("peer {s}: {e}"))
}

fn parse_address(h: Option<String>) -> Result<Option<[u8; 16]>> {
    match h {
        Some(h) => Ok(Some(hex::decode(&h)?.try_into().map_err(|_| anyhow::anyhow!("address must be 16 bytes"))?)),
        None => Ok(None),
    }
}

/// A UDP port in 40000–59999 that nothing here listens on (`init --port auto`).
fn free_port() -> u16 {
    for _ in 0..50 {
        let mut b = [0u8; 2];
        let _ = getrandom::getrandom(&mut b);
        let port = 40000 + u16::from_le_bytes(b) % 20000;
        if std::net::UdpSocket::bind(("0.0.0.0", port)).is_ok() {
            return port;
        }
    }
    51821
}

fn print_identity(id: &identity::Identity) {
    println!("address  {}", hex::encode(id.hash()));
    println!("wg key   {}", hex::encode(id.x_pub().as_bytes()));
}

/// The configuration file to use: the one given, or the default one if it exists.
fn find_config(given: Option<PathBuf>) -> Result<Option<(PathBuf, config::FileConfig)>> {
    match given {
        Some(p) => Ok(Some((p.clone(), config::FileConfig::load(&p)?))),
        None => {
            let (p, _) = config::default_paths();
            if p.exists() { Ok(Some((p.clone(), config::FileConfig::load(&p)?))) } else { Ok(None) }
        }
    }
}

/// Everything `run` needs, from the file and the flags.
pub struct Settings {
    pub key: PathBuf,
    pub state: Option<PathBuf>,
    pub cfg: node::Config,
    pub log: String,
    pub log_file: Option<PathBuf>,
}

pub fn settings(a: RunArgs) -> Result<Settings> {
    let found = find_config(a.config)?;
    let config_path = found.as_ref().map(|(p, _)| p.clone());
    let file = found.map(|(_, c)| c).unwrap_or_default();
    let state = a.state.or(file.state);
    let key = match a.key.or(file.key) {
        Some(k) => k,
        None => match &state {
            Some(s) => s.join("node.key"),
            None => bail!("no identity: give --config, --key or --state (or run `vork init`)"),
        },
    };
    let listen = a.listen.or(file.listen).unwrap_or_else(|| "0.0.0.0:51820".parse().unwrap());
    let all = file.peers.iter().chain(&a.peers).map(|p| parse_peer(p)).collect::<Result<Vec<_>>>()?;
    let peers: Vec<(SocketAddr, [u8; 32])> = all.iter().filter_map(|(a, k)| k.map(|k| (*a, k))).collect();
    let peers_by_address: Vec<SocketAddr> = all.iter().filter(|(_, k)| k.is_none()).map(|(a, _)| *a).collect();
    let spread = match a.spread.or(file.spread).as_deref().unwrap_or("spray") {
        "spray" => node::Spread::Spray,
        "dup" => node::Spread::Dup,
        s => bail!("spread must be spray or dup, not {s}"),
    };
    let api = file.api.clone().or_else(|| vork_api::endpoint_for(state.as_deref(), listen.port()));
    let control = Some(config::control_endpoint(a.control.as_deref().or(file.control.as_deref()), state.as_deref(), listen)).transpose().ok().flatten();
    let cfg = node::Config {
        listen,
        peers,
        peers_by_address,
        announce: !(a.no_announce || file.announce == Some(false)),
        ping: parse_address(a.ping)?,
        lite: a.lite || file.lite.unwrap_or(false),
        direct: a.direct || file.direct.unwrap_or(false),
        // Asked of the router by default (any free outside port will do: others learn it, ADR-0023).
        map_port: !a.no_map_port && (a.map_port || file.map_port.unwrap_or(true)),
        lan: !(a.no_lan || file.lan == Some(false)),
        bootstrap: file.bootstrap != Some(false),
        tun: file.tun.clone().filter(|t| !t.is_empty()),
        config_path,
        dns_upstream: Vec::new(),
        web_cert: match (&file.web_cert, &file.web_key) {
            (Some(c), Some(k)) => Some((c.clone(), k.clone())),
            (None, None) => None,
            _ => anyhow::bail!("web_cert and web_key go together"),
        },
        update_keys: file.update_keys.clone().unwrap_or_default().iter().map(|k| update::parse_key(k)).collect::<Result<_>>().context("update_keys")?,
        update_from: file.update_from.as_deref().filter(|s| !s.is_empty()).map(|s| crate::links::parse(s).with_context(|| format!("update_from: {s} isn't an address"))).transpose()?,
        update_dir: file.update_dir.clone(),
        exit: file.exit == Some(true),
        exit_lan: file.exit_lan == Some(true),
        allow: {
            let a = file.allow.clone().unwrap_or_default();
            crate::firewall::Firewall::new(&a).map_err(|e| anyhow::anyhow!("allow: {e}"))?;
            a
        },
        exit_auto: file.exit_via.as_deref() == Some("auto"),
        exit_via: file.exit_via.as_deref().filter(|s| !s.is_empty() && *s != "auto").map(|s| crate::links::parse(s).with_context(|| format!("exit_via: {s} isn't an address (or \"auto\")"))).transpose()?,
        web: file.web.as_deref().filter(|w| !w.is_empty()).map(|w| w.parse().with_context(|| format!("web: {w} isn't host:port"))).transpose()?,
        web_ingress: file.web_ingress.as_deref().filter(|w| !w.is_empty()).map(|w| w.parse().with_context(|| format!("web_ingress: {w} isn't host:port"))).transpose()?,
        web_ingress_from: file.web_ingress_from.unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::new(172, 30, 32, 2))),
        name: file.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(node::machine_name),
        gateway: a.gateway.or(file.gateway),
        paths: a.paths.or(file.paths).unwrap_or(2).max(1),
        spread,
        destinations: file.destinations.iter().chain(&a.destinations).cloned().collect(),
        devices: file
            .devices
            .iter()
            .map(|d| hex::decode(d.trim()).ok().and_then(|b| <[u8; 16]>::try_from(b).ok()).with_context(|| format!("devices: {d} isn't an address (32 hex digits)")))
            .collect::<Result<_>>()?,
        fetch: match a.fetch {
            Some(f) => {
                let (d, n) = f.split_once(':').context("--fetch ADDRESS:BYTES")?;
                Some((parse_address(Some(d.to_string()))?.context("--fetch address")?, n.parse().context("--fetch bytes")?))
            }
            None => None,
        },
        identify: a.identify,
        drop: a.drop,
        bench: parse_address(a.bench)?,
        bench_rate: a.bench_rate,
        control,
        operators: file.operators.clone(),
        api,
    };
    Ok(Settings { key, state, cfg, log: a.log.or(file.log).unwrap_or_else(|| "info".into()), log_file: a.log_file.or(file.log_file) })
}

/// Loads the identity, creating it on first run.
fn load_or_create_key(path: &Path) -> Result<identity::Identity> {
    if path.exists() {
        return identity::Identity::load(path);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let id = identity::Identity::generate()?;
    id.save(path)?;
    restrict(path);
    node::log(format!("created a new identity in {}", path.display()));
    Ok(id)
}

/// Owner-only permissions for secrets (on Windows the directory's ACL applies).
fn restrict(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

pub async fn run_node(s: Settings, stop: Option<tokio::sync::watch::Receiver<bool>>) -> Result<()> {
    node::set_logging(&s.log, s.log_file.as_deref())?;
    let me = load_or_create_key(&s.key)?;
    let state = state::State::open(s.state.as_deref())?;
    node::Node::new(me, s.cfg, state).await?.run(stop).await
}

/// `90s`, `10m`, `2h`, `3d` (a bare number is seconds).
fn parse_duration(s: &str) -> Result<u64> {
    let s = s.trim();
    let (n, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let n: u64 = n.parse().with_context(|| format!("not a duration: {s}"))?;
    Ok(n * match unit {
        "" | "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        u => bail!("unit {u}: s, m, h or d"),
    })
}

/// One control request to the running node; its answer, or its error as one.
async fn ask_node(config: Option<PathBuf>, request: serde_json::Value) -> Result<serde_json::Value> {
    let ep = settings(RunArgs { config, ..Default::default() })?.cfg.control.context("no control endpoint known: give --config")?;
    let answer = control::ask(&ep, &request.to_string()).await?;
    let v: serde_json::Value = serde_json::from_str(&answer).context("bad answer from the node")?;
    if let Some(e) = v["error"].as_str() {
        bail!("{e}");
    }
    Ok(v)
}

fn print_status(v: &serde_json::Value) {
    let g = |k: &str| v.get(k).cloned().unwrap_or_default();
    println!("address      {}", g("address").as_str().unwrap_or("?"));
    for d in g("local_destinations").as_array().cloned().unwrap_or_default() {
        println!("  {}  {}", d["address"].as_str().unwrap_or("?"), d["name"].as_str().unwrap_or("?"));
    }
    println!("listen       {}   uptime {} s{}", g("listen").as_str().unwrap_or("?"), g("uptime_s"), if g("lite").as_bool() == Some(true) { "   [lite]" } else { "" });
    let outside: Vec<String> = g("outside").as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
    if !outside.is_empty() {
        println!("seen at      {}", outside.join(", "));
    }
    println!("destinations {}   relaying {} paths, {} packets", g("destinations"), g("relay")["paths"], g("relay")["packets"]);
    let reach = g("reach");
    let by: Vec<String> = reach["reached_by"].as_array().cloned().unwrap_or_default().iter().filter_map(|r| r["at"].as_str().map(String::from)).collect();
    let mut at = by.clone();
    at.sort();
    at.dedup();
    let me = if by.is_empty() { "not shown yet (no node dialed us cold)".to_string() } else { format!("yes, by {} node(s), at {}", by.len(), at.join(", ")) };
    println!("reachable    {me}");
    println!("learned      {} reachable node(s), {} proven from here", reach["known"], reach["proven_here"]);
    println!("neighbors:");
    for n in g("neighbors").as_array().cloned().unwrap_or_default() {
        let addr = n["address"].as_str().map(|a| &a[..8.min(a.len())]).unwrap_or("?");
        let state = if n["up"].as_bool() == Some(true) { "up  " } else { "down" };
        let lite = if n["lite"].as_bool() == Some(true) { " lite" } else { "" };
        println!("  {state} {addr}  {}{lite}", n["endpoint"].as_str().unwrap_or("-"));
    }
    let sessions = g("sessions").as_array().cloned().unwrap_or_default();
    if !sessions.is_empty() {
        println!("sessions:");
    }
    for s in sessions {
        let peer = s["peer"].as_str().or(s["peer_key"].as_str()).map(|a| &a[..8.min(a.len())]).unwrap_or("?");
        let paths: Vec<String> = s["paths"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|p| {
                let via = if p["direct"].as_bool() == Some(true) { "direct".to_string() } else { format!("via {}", p["via"].as_str().map(|a| &a[..8.min(a.len())]).unwrap_or("?")) };
                let rtt = p["rtt_ms"].as_f64().map(|r| format!("{r:.1} ms")).unwrap_or_else(|| "-".into());
                format!("{via} {rtt}")
            })
            .collect();
        println!("  {peer}  [{}]", paths.join("; "));
    }
}

/// The network's own commands; everything else belongs to storage.
const NETWORK_COMMANDS: &[&str] = &[
    "init", "keygen", "show", "run", "status", "service", "service-run", "help", "-h", "--help", "-V", "--version",
    "link", "link-code", "requests", "allow", "dismiss", "unlink", "web-link", "version", "update", "update-install", "release",
];

/// Whether `vork ARGS` is a storage command. `status` is both: on its own
/// (with flags) it is the network node's, with a capability it is storage's.
fn is_storage_command(args: &[String]) -> bool {
    let Some(first) = args.first() else { return false };
    if first == "status" {
        let mut rest = args[1..].iter();
        while let Some(a) = rest.next() {
            match a.as_str() {
                "--config" | "--control" => {
                    rest.next();
                }
                a if a.starts_with('-') => {}
                _ => return true,
            }
        }
        return false;
    }
    !NETWORK_COMMANDS.contains(&first.as_str())
}

/// Hands the command to `vork-storage` (next to this program, else on PATH),
/// the way git and cargo run their external subcommands.
fn run_storage(args: &[String]) -> ! {
    let name = if cfg!(windows) { "vork-storage.exe" } else { "vork-storage" };
    let beside = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(name)));
    let program = beside.filter(|p| p.exists()).unwrap_or_else(|| PathBuf::from(name));
    let args: Vec<&String> = if args.first().map(String::as_str) == Some("help-storage") { vec![] } else { args.iter().collect() };
    let mut cmd = std::process::Command::new(&program);
    cmd.args(args);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = cmd.exec();
        eprintln!("vork: cannot run {}: {err}", program.display());
        std::process::exit(127);
    }
    #[cfg(not(unix))]
    match cmd.status() {
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(err) => {
            eprintln!("vork: cannot run {}: {err}", program.display());
            std::process::exit(127);
        }
    }
}

fn main() -> Result<()> {
    // Output piped into `head` and closed early: end quietly, as other tools do.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    if is_storage_command(&args) {
        run_storage(&args);
    }
    network_main()
}

#[tokio::main]
async fn network_main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Init { config: path, if_missing, tun, port } => {
            let (default_cfg, default_state) = config::default_paths();
            let path = path.unwrap_or(default_cfg.clone());
            if if_missing && path.exists() {
                println!("{} exists; left as it is", path.display());
                return Ok(());
            }
            // Next to a custom config file, keep everything together, written
            // relative to the file so the directory can be moved.
            let (state, written_state) = if path == default_cfg {
                (default_state.clone(), default_state)
            } else {
                (path.parent().unwrap_or(Path::new(".")).join("state"), PathBuf::from("state"))
            };
            config::write_sample(&path, &written_state.join("node.key"), &written_state)?;
            if let Some(name) = tun {
                let text = std::fs::read_to_string(&path)?;
                let line = format!("tun = {}", toml::Value::String(name));
                std::fs::write(&path, text.replacen("# tun = \"vork0\"", &line, 1))?;
            }
            if let Some(port) = port {
                let port: u16 = if port == "auto" { free_port() } else { port.parse().context("--port: a number or auto")? };
                let text = std::fs::read_to_string(&path)?;
                std::fs::write(&path, text.replacen("listen = \"0.0.0.0:51820\"", &format!("listen = \"0.0.0.0:{port}\""), 1))?;
            }
            std::fs::create_dir_all(&state)?;
            let id = load_or_create_key(&state.join("node.key"))?;
            println!("wrote {}", path.display());
            print_identity(&id);
        }
        Cmd::Keygen { out } => {
            if out.exists() {
                bail!("{} exists", out.display());
            }
            let id = identity::Identity::generate()?;
            id.save(&out)?;
            restrict(&out);
            print_identity(&id);
        }
        Cmd::Show { key, config } => {
            let s = settings(RunArgs { key, config, ..Default::default() })?;
            print_identity(&identity::Identity::load(&s.key)?);
        }
        Cmd::Run(a) => run_node(settings(a)?, None).await?,
        Cmd::Status { config, control, json } => {
            let ep = match control {
                Some(ep) => ep,
                None => settings(RunArgs { config, ..Default::default() })?.cfg.control.context("no control endpoint known: give --control or --config")?,
            };
            let answer = control::ask(&ep, r#"{"cmd":"status"}"#).await?;
            if json {
                print!("{answer}");
            } else {
                print_status(&serde_json::from_str(&answer).context("bad answer from the node")?);
            }
        }
        Cmd::Link { address, code, config } => {
            // A QR code's text: vork:link/ADDRESS?c=CODE&n=NAME
            let (address, code, addrs) = match address.strip_prefix("vork-link:").or_else(|| address.strip_prefix("vork:link/")) {
                Some(rest) => {
                    let (a, q) = rest.split_once('?').unwrap_or((rest, ""));
                    let c = q.split('&').find_map(|kv| kv.strip_prefix("c=")).map(str::to_string);
                    let addrs: Vec<String> = q.split('&').find_map(|kv| kv.strip_prefix("a=")).map(|l| l.split(',').map(str::to_string).collect()).unwrap_or_default();
                    (a.to_string(), code.or(c), addrs)
                }
                None => (address, code, Vec::new()),
            };
            let r = ask_node(config, serde_json::json!({ "cmd": "link", "address": address, "code": code.unwrap_or_default(), "addrs": addrs })).await?;
            if r["already"].as_bool() == Some(true) {
                println!("already one of your devices");
            } else if r["no_linking"].as_bool() == Some(true) {
                println!("linked {address}: a Lite device, told with its link's code");
            } else {
                println!("asked {address}: it's linked once its owner allows it (vork requests shows the state)");
            }
        }
        Cmd::LinkCode { valid, uses, approve, config } => {
            let secs = parse_duration(&valid)?;
            let r = ask_node(config, serde_json::json!({ "cmd": "link_code", "valid_s": secs, "uses": uses, "approve": approve })).await?;
            let uri = r["uri"].as_str().unwrap_or_default();
            if let Ok(q) = qrcode::QrCode::new(uri.as_bytes()) {
                println!("{}", q.render::<qrcode::render::unicode::Dense1x2>().quiet_zone(true).build());
            }
            println!("{uri}");
            println!("Scan it with the vØrk app, or on another node: vork link '{uri}'");
            let mins = r["valid_s"].as_u64().unwrap_or(600) / 60;
            let times = match r["uses"].as_u64() {
                None => "any number of devices".to_string(),
                Some(1) => "one device".to_string(),
                Some(n) => format!("{n} devices"),
            };
            println!("Good for {mins} minutes, for {times}{}.", if approve { ", each one allowed by you (vork requests)" } else { "" });
        }
        Cmd::Version => println!("vork {} build {} ({})", env!("VORK_VERSION"), update::build(), update::target()),
        Cmd::Update { config } => {
            let r = ask_node(config, serde_json::json!({ "cmd": "update_check" })).await?;
            println!("looking for a newer release than build {} (vork status shows what came of it)", r["build"]);
        }
        Cmd::UpdateInstall { config } => {
            let s = settings(RunArgs { config, ..Default::default() })?;
            let state = s.state.context("no state directory")?;
            let exe = std::env::current_exe()?;
            match update::install(&state, &s.cfg.update_keys, &exe)? {
                Some(b) => println!("build {b} put in place of {} (the old one kept as .previous)", exe.display()),
                None => println!("no update waiting"),
            }
        }
        Cmd::Release { action } => match action {
            ReleaseAction::Keygen { path } => {
                let public = update::keygen(&path)?;
                println!("release key written to {} (keep it safe; it signs what your nodes install)", path.display());
                println!("its public half, for update_keys:\n{public}");
            }
            ReleaseAction::Make { key, dir, version, files } => {
                let files: Vec<(String, PathBuf)> = files
                    .iter()
                    .map(|f| f.split_once('=').map(|(t, p)| (t.to_string(), PathBuf::from(p))).with_context(|| format!("{f}: TARGET=PATH")))
                    .collect::<Result<_>>()?;
                let m = update::make(&key, &dir, None, &version, &files)?;
                println!("release build {} in {}: {}", m.build, dir.display(), m.files.keys().cloned().collect::<Vec<_>>().join(", "));
            }
        },
        Cmd::WebLink { config } => {
            let s = settings(RunArgs { config, ..Default::default() })?;
            let web = s.cfg.web.context("the dashboard is off: set web = \"0.0.0.0:8742\" in vork.toml and restart the node")?;
            let token = std::fs::read_to_string(s.state.as_ref().context("no state directory")?.join("web.token")).context("no dashboard token yet: has the node run with web set?")?;
            let host = if web.ip().is_unspecified() { node::machine_name() } else { web.ip().to_string() };
            println!("https://{host}:{}/#t={}", web.port(), token.trim());
            if let Some(fp) = s.state.as_ref().and_then(|d| web::kept_fingerprint(d)) {
                println!("certificate SHA-256 {fp}");
                println!("(the browser asks once: compare this with the certificate it shows, then accept)");
            }
        }
        Cmd::Requests { config } => {
            let r = ask_node(config, serde_json::json!({ "cmd": "status" })).await?;
            let req = r["links"]["requests"].as_array().cloned().unwrap_or_default();
            let pend = r["links"]["pending"].as_array().cloned().unwrap_or_default();
            if req.is_empty() && pend.is_empty() {
                println!("no link requests");
            }
            for q in req {
                println!("asks to be linked: {} {}   vork allow {}", q["address"].as_str().unwrap_or("?"), q["name"].as_str().unwrap_or(""), q["address"].as_str().unwrap_or("?"));
            }
            for p in pend {
                println!("waiting for its owner: {}", p["address"].as_str().unwrap_or("?"));
            }
        }
        Cmd::Allow { address, config } => {
            ask_node(config, serde_json::json!({ "cmd": "link_allow", "address": address })).await?;
            println!("linked: {address} is one of your devices");
        }
        Cmd::Dismiss { address, config } => {
            ask_node(config, serde_json::json!({ "cmd": "link_dismiss", "address": address })).await?;
            println!("dismissed {address}");
        }
        Cmd::Unlink { address, config } => {
            ask_node(config, serde_json::json!({ "cmd": "unlink", "address": address })).await?;
            println!("unlinked {address} (it's told)");
        }
        #[cfg(windows)]
        Cmd::Service { action } => match action {
            ServiceAction::Install { config } => {
                let path = config.unwrap_or(config::default_paths().0);
                let path = std::fs::canonicalize(&path).with_context(|| format!("{} (run `vork init` first)", path.display()))?;
                service::install(&path)?;
                println!("installed and started the vork service with {}", path.display());
            }
            ServiceAction::Uninstall => {
                service::uninstall()?;
                println!("removed the vork service");
            }
        },
        #[cfg(windows)]
        Cmd::ServiceRun { config } => service::run_dispatcher(config)?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::is_storage_command;

    fn storage(line: &str) -> bool {
        is_storage_command(&line.split_whitespace().map(String::from).collect::<Vec<_>>())
    }

    #[test]
    fn network_and_storage_commands_are_told_apart() {
        assert!(!storage(""));
        assert!(!storage("run --peer 1.2.3.4:51820"));
        assert!(!storage("status"));
        assert!(!storage("status --config a.toml --json"));
        assert!(!storage("--help"));
        assert!(storage("status rc1abc"));
        assert!(storage("status --json rc1abc"));
        assert!(storage("put file.txt"));
        assert!(storage("--store /tmp/s ping"));
        assert!(storage("help-storage"));
    }
}

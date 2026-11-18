mod api;
mod config;
mod control;
mod identity;
mod node;
mod portmap;
#[cfg(windows)]
mod service;
mod state;
mod stream;
mod wire;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use std::net::{SocketAddr, ToSocketAddrs};
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
    /// Ask the router to open the port (UPnP, then PCP/NAT-PMP), like torrent clients do.
    #[arg(long)]
    map_port: bool,
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

fn parse_peer(s: &str) -> Result<(SocketAddr, [u8; 32])> {
    let (addr, key) = s.split_once('@').context("peer must be HOST:PORT@KEY")?;
    let addr = addr.to_socket_addrs()?.next().context("peer address doesn't resolve")?;
    let key: [u8; 32] = hex::decode(key)?.try_into().map_err(|_| anyhow::anyhow!("key must be 32 bytes"))?;
    Ok((addr, key))
}

fn parse_address(h: Option<String>) -> Result<Option<[u8; 16]>> {
    match h {
        Some(h) => Ok(Some(hex::decode(&h)?.try_into().map_err(|_| anyhow::anyhow!("address must be 16 bytes"))?)),
        None => Ok(None),
    }
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
    let file = find_config(a.config)?.map(|(_, c)| c).unwrap_or_default();
    let state = a.state.or(file.state);
    let key = match a.key.or(file.key) {
        Some(k) => k,
        None => match &state {
            Some(s) => s.join("node.key"),
            None => bail!("no identity: give --config, --key or --state (or run `vork init`)"),
        },
    };
    let listen = a.listen.or(file.listen).unwrap_or_else(|| "0.0.0.0:51820".parse().unwrap());
    let mut peers = file.peers.iter().map(|p| parse_peer(p)).collect::<Result<Vec<_>>>()?;
    peers.extend(a.peers.iter().map(|p| parse_peer(p)).collect::<Result<Vec<_>>>()?);
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
        announce: !(a.no_announce || file.announce == Some(false)),
        ping: parse_address(a.ping)?,
        lite: a.lite || file.lite.unwrap_or(false),
        direct: a.direct || file.direct.unwrap_or(false),
        map_port: a.map_port || file.map_port.unwrap_or(false),
        gateway: a.gateway.or(file.gateway),
        paths: a.paths.or(file.paths).unwrap_or(2).max(1),
        spread,
        destinations: file.destinations.iter().chain(&a.destinations).cloned().collect(),
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
    let args: Vec<String> = std::env::args().skip(1).collect();
    if is_storage_command(&args) {
        run_storage(&args);
    }
    network_main()
}

#[tokio::main]
async fn network_main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Init { config: path } => {
            let (default_cfg, default_state) = config::default_paths();
            let path = path.unwrap_or(default_cfg.clone());
            // Next to a custom config file, keep everything together, written
            // relative to the file so the directory can be moved.
            let (state, written_state) = if path == default_cfg {
                (default_state.clone(), default_state)
            } else {
                (path.parent().unwrap_or(Path::new(".")).join("state"), PathBuf::from("state"))
            };
            config::write_sample(&path, &written_state.join("node.key"), &written_state)?;
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

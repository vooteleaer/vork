mod identity;
mod node;
mod wire;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::PathBuf;

#[derive(Parser)]
#[command(about = "vork: a serverless, identity-addressed network over WireGuard (phase-1 prototype)")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create an identity (Ed25519 + X25519) and print its address and WireGuard key.
    Keygen {
        #[arg(long)]
        out: PathBuf,
    },
    /// Print an identity's address and WireGuard key.
    Show {
        #[arg(long)]
        key: PathBuf,
    },
    /// Run a node.
    Run {
        #[arg(long)]
        key: PathBuf,
        #[arg(long, default_value = "0.0.0.0:51820")]
        listen: SocketAddr,
        /// Bootstrap neighbor as HOST:PORT@X25519_HEX (repeatable).
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
    },
}

fn parse_peer(s: &str) -> Result<(SocketAddr, [u8; 32])> {
    let (addr, key) = s.split_once('@').context("peer must be HOST:PORT@KEY")?;
    let addr = addr.to_socket_addrs()?.next().context("peer address doesn't resolve")?;
    let key: [u8; 32] = hex::decode(key)?.try_into().map_err(|_| anyhow::anyhow!("key must be 32 bytes"))?;
    Ok((addr, key))
}

fn print_identity(id: &identity::Identity) {
    println!("address  {}", hex::encode(id.hash()));
    println!("wg key   {}", hex::encode(id.x_pub().as_bytes()));
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Keygen { out } => {
            if out.exists() {
                bail!("{} exists", out.display());
            }
            let id = identity::Identity::generate()?;
            id.save(&out)?;
            print_identity(&id);
        }
        Cmd::Show { key } => print_identity(&identity::Identity::load(&key)?),
        Cmd::Run { key, listen, peers, no_announce, ping, lite, direct } => {
            let me = identity::Identity::load(&key)?;
            let peers = peers.iter().map(|p| parse_peer(p)).collect::<Result<Vec<_>>>()?;
            let ping = match ping {
                Some(h) => Some(hex::decode(&h)?.try_into().map_err(|_| anyhow::anyhow!("address must be 16 bytes"))?),
                None => None,
            };
            let cfg = node::Config { listen, peers, announce: !no_announce, ping, lite, direct };
            node::Node::new(me, cfg).await?.run().await?;
        }
    }
    Ok(())
}

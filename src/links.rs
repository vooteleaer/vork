//! Linking devices into one owner's private network, with both sides
//! agreeing (ADR-0024). One node asks (`link`), the other's owner allows or
//! dismisses it in their app (or `vork allow` on a server); only then are
//! they each other's devices. Either may unlink, and the other learns it.
//!
//! The messages are requests to the built-in path `/vork/link` in a session
//! where the sender has identified, so the receiver knows who asks. What
//! hasn't been answered yet waits in an outbox and is sent again until it is.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::identity::Hash;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Request {
    /// Who asks: a node address, hex.
    pub address: String,
    /// The name it gave.
    #[serde(default)]
    pub name: String,
    /// Unix seconds.
    pub since: u64,
    /// It came with a link this node handed out (one that still asks the owner).
    #[serde(default)]
    pub with_link: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Outgoing {
    pub address: String,
    /// "request", "accept", "dismiss" or "remove".
    pub op: String,
}

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct Links {
    /// Nodes this one asked, not answered yet.
    #[serde(default)]
    pub pending: Vec<String>,
    /// Nodes asking this one, waiting for the owner.
    #[serde(default)]
    pub requests: Vec<Request>,
    /// Messages to deliver.
    #[serde(default)]
    pub outbox: Vec<Outgoing>,
    #[serde(skip)]
    file: Option<PathBuf>,
}

pub fn parse(address: &str) -> Option<Hash> {
    hex::decode(address.trim()).ok()?.try_into().ok()
}

impl Links {
    pub fn open(dir: Option<&Path>) -> Links {
        let file = dir.map(|d| d.join("links.json"));
        let mut l: Links = file.as_ref().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        l.file = file;
        l
    }

    pub fn save(&self) {
        let Some(f) = &self.file else { return };
        if let Ok(j) = serde_json::to_vec_pretty(self) {
            let tmp = f.with_extension("tmp");
            let _ = std::fs::write(&tmp, j).and_then(|()| std::fs::rename(&tmp, f));
        }
    }

    /// Queues `op` to `address`, replacing whatever was queued for it.
    pub fn send(&mut self, address: &str, op: &str) {
        self.outbox.retain(|o| o.address != address);
        self.outbox.push(Outgoing { address: address.to_string(), op: op.to_string() });
    }

    pub fn delivered(&mut self, address: &str) {
        self.outbox.retain(|o| o.address != address);
    }

    pub fn is_pending(&self, address: &str) -> bool {
        self.pending.iter().any(|p| p == address)
    }

    pub fn forget(&mut self, address: &str) {
        self.pending.retain(|p| p != address);
        self.requests.retain(|r| r.address != address);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kept_and_queued() {
        let dir = std::env::temp_dir().join(format!("vork-links-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let mut l = Links::open(Some(&dir));
        l.pending.push("aa".into());
        l.send("aa", "request");
        l.send("aa", "remove");
        assert_eq!(l.outbox.len(), 1);
        l.save();
        let mut l = Links::open(Some(&dir));
        assert!(l.is_pending("aa"));
        assert_eq!(l.outbox[0].op, "remove");
        l.delivered("aa");
        l.forget("aa");
        assert!(l.outbox.is_empty() && l.pending.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

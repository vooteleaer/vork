//! Linking devices into one owner's private network, with both sides
//! agreeing (ADR-0024). One node asks (`link`), the other's owner allows or
//! dismisses it in their app (or `vork allow` on a server); only then are
//! they each other's devices. Either may unlink, and the other learns it.
//!
//! The messages are requests to the built-in path `/vork/link` in a session
//! where the sender has identified, so the receiver knows who asks. What
//! hasn't been answered yet waits in an outbox and is sent again until it is.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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
    /// The owner's devices as this node knows them, with when each was
    /// linked or unlinked (ADR-0030): shared with the other devices, so a
    /// device linked to one is linked to all, and one unlinked anywhere is
    /// unlinked everywhere.
    #[serde(default)]
    pub group: BTreeMap<String, Member>,
    /// Counts changes to `group`: a device that has heard this one needn't hear it again.
    #[serde(default)]
    pub group_rev: u64,
    #[serde(skip)]
    file: Option<PathBuf>,
}

/// One of the owner's devices: when it was last linked and unlinked (Unix
/// seconds, 0: never). The later one holds; a tie is unlinked.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct Member {
    #[serde(default)]
    pub added: u64,
    #[serde(default)]
    pub removed: u64,
}

impl Member {
    pub fn present(&self) -> bool {
        self.added > self.removed
    }
}

/// The most entries taken from another device at once.
pub const MAX_GROUP: usize = 512;
/// How far ahead of this node's clock another's times are believed.
const SKEW: u64 = 300;

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

    /// Linked (`present`) or unlinked here, now. It beats what was known,
    /// even if another device's clock ran ahead.
    pub fn mark(&mut self, address: &str, present: bool, now: u64) {
        let m = self.group.entry(address.to_string()).or_default();
        if present {
            m.added = now.max(m.removed + 1).max(m.added);
        } else {
            m.removed = now.max(m.added).max(m.removed);
        }
        self.group_rev += 1;
    }

    /// Makes the group agree with the devices in `vork.toml` (edited by hand,
    /// or a node from before groups): what's listed there is linked, what
    /// isn't is not. Old devices count as linked long ago.
    pub fn reconcile(&mut self, devices: &[String], now: u64) {
        for d in devices {
            match self.group.get(d) {
                None => {
                    self.group.insert(d.clone(), Member { added: 1, removed: 0 });
                    self.group_rev += 1;
                }
                Some(m) if !m.present() => self.mark(d, true, now),
                _ => {}
            }
        }
        let gone: Vec<String> = self.group.iter().filter(|(a, m)| m.present() && !devices.contains(a)).map(|(a, _)| a.clone()).collect();
        for a in gone {
            self.mark(&a, false, now);
        }
    }

    /// Takes in what another device knows (never about `skip`: this node and
    /// the sender). Returns the devices that came or went: (address, present).
    pub fn merge(&mut self, theirs: &BTreeMap<String, Member>, skip: &[String], now: u64) -> Vec<(String, bool)> {
        let mut changed = Vec::new();
        for (a, m) in theirs.iter().take(MAX_GROUP) {
            if skip.contains(a) || parse(a).is_none() {
                continue;
            }
            let m = Member { added: m.added.min(now + SKEW), removed: m.removed.min(now + SKEW) };
            let e = self.group.entry(a.clone()).or_default();
            let before = e.present();
            let new = Member { added: e.added.max(m.added), removed: e.removed.max(m.removed) };
            if new != *e {
                *e = new;
                self.group_rev += 1;
            }
            if e.present() != before {
                changed.push((a.clone(), e.present()));
            }
        }
        changed
    }

    /// Out of the owner's group (unlinked by one of its devices): nothing known any more.
    pub fn leave_group(&mut self) {
        self.group.clear();
        self.group_rev += 1;
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

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const C: &str = "cccccccccccccccccccccccccccccccc";

    #[test]
    fn a_device_of_a_device_is_one_too() {
        // The PC linked with the server; the phone with the server.
        let mut server = Links::default();
        server.mark(A, true, 100); // the PC
        server.mark(B, true, 200); // the phone
        let mut pc = Links::default();
        pc.mark(C, true, 100); // the server
        let came = pc.merge(&server.group, &[A.into(), C.into()], 300);
        assert_eq!(came, vec![(B.to_string(), true)]);
        // Heard again: nothing new.
        assert!(pc.merge(&server.group, &[A.into(), C.into()], 300).is_empty());
    }

    #[test]
    fn unlinked_anywhere_is_unlinked_everywhere_and_stays_so() {
        let mut pc = Links::default();
        pc.mark(B, true, 100);
        let mut server = Links::default();
        server.mark(B, true, 100);
        // The phone is unlinked on the PC; the server hears it.
        pc.mark(B, false, 500);
        assert_eq!(server.merge(&pc.group, &[], 500), vec![(B.to_string(), false)]);
        // A device that hadn't heard yet doesn't bring it back.
        let mut stale = Links::default();
        stale.mark(B, true, 100);
        assert!(server.merge(&stale.group, &[], 600).is_empty());
        assert!(!server.group[B].present());
        // Linked again later: back.
        server.mark(B, true, 700);
        assert_eq!(pc.merge(&server.group, &[], 700), vec![(B.to_string(), true)]);
    }

    #[test]
    fn a_clock_far_ahead_is_held_back() {
        let mut far = Links::default();
        far.mark(B, true, 10_000_000);
        let mut me = Links::default();
        me.merge(&far.group, &[], 1000);
        assert_eq!(me.group[B].added, 1000 + SKEW);
        // Unlinking here still wins over it.
        me.mark(B, false, 1000);
        assert!(!me.group[B].present());
    }

    #[test]
    fn reconciled_with_the_configuration() {
        let mut l = Links::default();
        l.mark(A, true, 100);
        l.reconcile(&[B.to_string()], 200);
        assert!(!l.group[A].present());
        assert_eq!(l.group[B], Member { added: 1, removed: 0 });
    }
}

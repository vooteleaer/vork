//! Which nodes can be reached directly, learned from experience (ADR-0023).
//!
//! A node never says it is reachable; others find out. When this node dials
//! a neighbor cold (no packet from it for a while, so no NAT mapping of its
//! own can be letting us in) and the neighbor answers the handshake, this
//! node has *proven* that the neighbor is reachable at that address, from
//! here. Proofs are shared with neighbors as plain reports. A report is a
//! lead, not a fact: a node only dials it, and a lie costs one handshake.
//! What a node says about itself is ignored; reports about this node tell
//! it how others reach it.

use crate::identity::Hash;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

/// Entries not seen up (proven by us, or reported) for this long are dropped.
const KEEP_S: u64 = 7 * 86400;
/// The most entries kept: own proofs first, then reports by how many said so.
pub const MAX_ENTRIES: usize = 256;
/// The most entries in one report to a neighbor.
pub const MAX_SHARE: usize = 32;
/// The most observers remembered per entry.
const MAX_OBSERVERS: usize = 8;

/// Where an address can be reached from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Loopback,
    LinkLocal,
    /// RFC 1918, IPv6 ULA: one site's network.
    Private,
    /// 100.64/10 (carrier NAT, Tailscale): one provider's or overlay's network.
    Shared,
    Public,
}

pub fn scope(ip: IpAddr) -> Scope {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            if v.is_loopback() || v.is_unspecified() {
                Scope::Loopback
            } else if v.is_link_local() {
                Scope::LinkLocal
            } else if v.is_private() {
                Scope::Private
            } else if o[0] == 100 && (o[1] & 0xc0) == 64 {
                Scope::Shared
            } else {
                Scope::Public
            }
        }
        IpAddr::V6(v) => {
            if let Some(v4) = v.to_ipv4_mapped() {
                return scope(IpAddr::V4(v4));
            }
            let s = v.segments();
            if v.is_loopback() || v.is_unspecified() {
                Scope::Loopback
            } else if s[0] & 0xffc0 == 0xfe80 {
                Scope::LinkLocal
            } else if s[0] & 0xfe00 == 0xfc00 {
                Scope::Private
            } else {
                Scope::Public
            }
        }
    }
}

/// Whether an address of `entry` scope may be told to a neighbor we reach
/// at an address of `neighbor` scope: public ones to anyone, others only
/// within the same kind of network (it may be the same one).
pub fn shareable(entry: Scope, neighbor: Scope) -> bool {
    match entry {
        Scope::Public => true,
        Scope::Loopback | Scope::LinkLocal => false,
        s => s == neighbor,
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Entry {
    /// WireGuard key, hex.
    pub key: String,
    pub addr: SocketAddr,
    /// Unix seconds this node last proved it (0: only reported by others).
    #[serde(default)]
    pub proved: u64,
    /// Unix seconds it was last proven by anyone (us or a report).
    pub seen: u64,
    /// Who reported it (node hashes, hex), at most a few.
    #[serde(default)]
    pub observers: Vec<String>,
    /// It relays (from its HELLO, when it was our neighbor).
    #[serde(default)]
    pub relay: Option<bool>,
}

impl Entry {
    fn key_bytes(&self) -> Option<[u8; 32]> {
        hex::decode(&self.key).ok()?.try_into().ok()
    }
}

#[derive(Default)]
pub struct Reach {
    entries: HashMap<[u8; 32], Entry>,
    /// Neighbors that told us they reached us: node hash -> (address, unix seconds).
    pub about_me: HashMap<Hash, (SocketAddr, u64)>,
    file: Option<PathBuf>,
}

impl Reach {
    /// Loads what was learned before from `dir/reachable.json` (none: kept in memory only).
    pub fn open(dir: Option<&Path>) -> Reach {
        let file = dir.map(|d| d.join("reachable.json"));
        let mut r = Reach { file, ..Default::default() };
        if let Some(t) = r.file.as_ref().and_then(|f| std::fs::read_to_string(f).ok()) {
            let list: Vec<Entry> = serde_json::from_str(&t).unwrap_or_default();
            for e in list {
                if let Some(k) = e.key_bytes() {
                    r.entries.insert(k, e);
                }
            }
        }
        r
    }

    pub fn save(&self) {
        let Some(f) = &self.file else { return };
        let mut list: Vec<&Entry> = self.entries.values().collect();
        list.sort_by(|a, b| b.seen.cmp(&a.seen));
        if let Ok(j) = serde_json::to_vec_pretty(&list) {
            let tmp = f.with_extension("tmp");
            if std::fs::write(&tmp, j).and_then(|()| std::fs::rename(&tmp, f)).is_err() {
                eprintln!("can't store reachable nodes in {}", f.display());
            }
        }
    }

    pub fn len(&self) -> (usize, usize) {
        (self.entries.len(), self.entries.values().filter(|e| e.proved > 0).count())
    }

    /// This node dialed `key` cold at `addr` and it answered.
    pub fn proved(&mut self, key: [u8; 32], addr: SocketAddr, now: u64) {
        let e = self.entries.entry(key).or_insert_with(|| Entry { key: hex::encode(key), addr, proved: 0, seen: 0, observers: vec![], relay: None });
        if e.addr != addr {
            // Moved: earlier reports were about the old address.
            e.addr = addr;
            e.observers.clear();
        }
        e.proved = now;
        e.seen = now;
    }

    /// What its HELLO said about relaying.
    pub fn set_relay(&mut self, key: &[u8; 32], relay: bool) {
        if let Some(e) = self.entries.get_mut(key) {
            e.relay = Some(relay);
        }
    }

    /// A neighbor (`from`, node hash) reports it reached `key` at `addr`
    /// `age` seconds ago. Reports never override our own proof of another address.
    pub fn heard(&mut self, from: &Hash, key: [u8; 32], addr: SocketAddr, age: u32, now: u64) {
        if age as u64 > KEEP_S {
            return;
        }
        let when = now.saturating_sub(age as u64);
        let from = hex::encode(from);
        match self.entries.get_mut(&key) {
            Some(e) if e.addr == addr => {
                e.seen = e.seen.max(when);
                if !e.observers.contains(&from) {
                    e.observers.push(from);
                    if e.observers.len() > MAX_OBSERVERS {
                        e.observers.remove(0);
                    }
                }
            }
            // Our own recent proof wins over a report of another address.
            Some(e) if e.proved > 0 && e.proved >= when => {}
            _ => {
                self.entries.insert(key, Entry { key: hex::encode(key), addr, proved: 0, seen: when, observers: vec![from], relay: None });
            }
        }
        self.prune(now);
    }

    /// Drops old entries, then the weakest past the cap.
    pub fn prune(&mut self, now: u64) {
        self.entries.retain(|_, e| now.saturating_sub(e.seen) <= KEEP_S);
        if self.entries.len() > MAX_ENTRIES {
            let mut ranked: Vec<([u8; 32], (bool, usize, u64))> =
                self.entries.iter().map(|(k, e)| (*k, (e.proved > 0, e.observers.len(), e.seen))).collect();
            ranked.sort_by(|a, b| b.1.cmp(&a.1));
            for (k, _) in ranked.into_iter().skip(MAX_ENTRIES) {
                self.entries.remove(&k);
            }
        }
    }

    /// Our own proofs (of relaying nodes, or not known not to) for a neighbor
    /// we reach at an address of `to` scope: key, address, age in seconds.
    /// The neighbor's own entry stays in: that's how it learns it was reached.
    pub fn to_share(&self, to: Scope, now: u64) -> Vec<([u8; 32], SocketAddr, u32)> {
        let mut list: Vec<(&[u8; 32], &Entry)> = self
            .entries
            .iter()
            .filter(|(_, e)| e.proved > 0 && e.relay != Some(false) && shareable(scope(e.addr.ip()), to))
            .collect();
        list.sort_by(|a, b| b.1.proved.cmp(&a.1.proved));
        list.into_iter().take(MAX_SHARE).map(|(k, e)| (*k, e.addr, now.saturating_sub(e.proved).min(u32::MAX as u64) as u32)).collect()
    }

    /// Nodes to dial when this one needs neighbors, best first: our own
    /// proofs, then reports by how many said so, newer before older; public
    /// addresses before others.
    pub fn candidates(&self, skip: impl Fn(&[u8; 32]) -> bool) -> Vec<([u8; 32], SocketAddr)> {
        let mut list: Vec<(&[u8; 32], &Entry)> = self.entries.iter().filter(|(k, e)| e.relay != Some(false) && !skip(k)).collect();
        list.sort_by_key(|(_, e)| std::cmp::Reverse((scope(e.addr.ip()) == Scope::Public, e.proved > 0, e.observers.len().min(3), e.seen)));
        list.into_iter().map(|(k, e)| (*k, e.addr)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn scopes() {
        assert_eq!(scope("10.1.2.3".parse().unwrap()), Scope::Private);
        assert_eq!(scope("192.168.1.1".parse().unwrap()), Scope::Private);
        assert_eq!(scope("100.84.1.1".parse().unwrap()), Scope::Shared);
        assert_eq!(scope("100.200.1.1".parse().unwrap()), Scope::Public);
        assert_eq!(scope("203.0.113.7".parse().unwrap()), Scope::Public);
        assert_eq!(scope("127.0.0.1".parse().unwrap()), Scope::Loopback);
        assert_eq!(scope("fd00::1".parse().unwrap()), Scope::Private);
        assert_eq!(scope("fe80::1".parse().unwrap()), Scope::LinkLocal);
        assert_eq!(scope("2001:db8::1".parse().unwrap()), Scope::Public);
        assert!(shareable(Scope::Public, Scope::Private));
        assert!(shareable(Scope::Private, Scope::Private));
        assert!(!shareable(Scope::Private, Scope::Public));
        assert!(!shareable(Scope::Shared, Scope::Private));
    }

    #[test]
    fn proofs_are_shared_reports_are_not() {
        let mut r = Reach::default();
        let now = 1_000_000;
        r.proved([1; 32], a("1.2.3.4:51820"), now - 10);
        r.heard(&[9; 16], [2; 32], a("5.6.7.8:51820"), 5, now);
        let shared = r.to_share(Scope::Public, now);
        assert_eq!(shared, vec![([1; 32], a("1.2.3.4:51820"), 10)]);
        // Lite nodes aren't offered.
        r.set_relay(&[1; 32], false);
        assert!(r.to_share(Scope::Public, now).is_empty());
    }

    #[test]
    fn candidates_rank_proofs_and_agreement() {
        let mut r = Reach::default();
        let now = 1_000_000;
        r.heard(&[7; 16], [3; 32], a("9.9.9.9:1"), 0, now);
        r.heard(&[7; 16], [4; 32], a("8.8.8.8:1"), 0, now);
        r.heard(&[8; 16], [4; 32], a("8.8.8.8:1"), 0, now);
        r.proved([5; 32], a("7.7.7.7:1"), now - 100);
        r.heard(&[7; 16], [6; 32], a("192.168.1.5:1"), 0, now);
        let order: Vec<[u8; 32]> = r.candidates(|_| false).into_iter().map(|(k, _)| k).collect();
        assert_eq!(order, vec![[5; 32], [4; 32], [3; 32], [6; 32]]);
        let order: Vec<[u8; 32]> = r.candidates(|k| *k == [5; 32]).into_iter().map(|(k, _)| k).collect();
        assert_eq!(order[0], [4; 32]);
    }

    #[test]
    fn a_report_doesnt_move_a_fresh_proof() {
        let mut r = Reach::default();
        let now = 1_000_000;
        r.proved([1; 32], a("1.2.3.4:1"), now);
        r.heard(&[7; 16], [1; 32], a("6.6.6.6:1"), 60, now);
        assert_eq!(r.candidates(|_| false), vec![([1; 32], a("1.2.3.4:1"))]);
        // An old proof does give way to a newer report.
        r.proved([2; 32], a("1.2.3.5:1"), now - 3600);
        r.heard(&[7; 16], [2; 32], a("6.6.6.7:1"), 60, now);
        assert!(r.candidates(|_| false).contains(&([2; 32], a("6.6.6.7:1"))));
    }

    #[test]
    fn old_and_excess_entries_go() {
        let mut r = Reach::default();
        let now = 100 * 86400;
        r.heard(&[7; 16], [1; 32], a("1.1.1.1:1"), 8 * 86400, now);
        assert_eq!(r.len(), (0, 0));
        for i in 0..(MAX_ENTRIES + 10) {
            let mut k = [0u8; 32];
            k[..8].copy_from_slice(&(i as u64).to_be_bytes());
            r.heard(&[7; 16], k, SocketAddr::new([10, 0, (i / 256) as u8, (i % 256) as u8].into(), 1), 0, now);
        }
        assert_eq!(r.len().0, MAX_ENTRIES);
    }

    #[test]
    fn kept_across_runs() {
        let dir = std::env::temp_dir().join(format!("vork-reach-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let mut r = Reach::open(Some(&dir));
        r.proved([1; 32], a("1.2.3.4:1"), 1000);
        r.save();
        let r = Reach::open(Some(&dir));
        assert_eq!(r.len(), (1, 1));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

//! What a node keeps between runs, in its state directory: the announce
//! sequence counter (which must only grow) and the neighbors it has been
//! linked with (so it can reconnect without bootstrap peers).

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Remembered neighbors are kept this long since they were last up.
const NEIGHBOR_DAYS: u64 = 7;
pub const MAX_REMEMBERED: usize = 16;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Remembered {
    pub addr: SocketAddr,
    /// WireGuard key, hex.
    pub key: String,
    /// Unix seconds when the link was last up.
    pub seen: u64,
}

pub struct State {
    dir: Option<PathBuf>,
    seq: u64,
}

pub fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

/// Writes through a temporary file and a rename, so a crash leaves the old
/// or the new content, never half of it.
fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

impl State {
    /// `None`: keep nothing (the sequence still grows within this run).
    pub fn open(dir: Option<&Path>) -> Result<State> {
        let mut seq = 0;
        if let Some(d) = dir {
            std::fs::create_dir_all(d)?;
            if let Ok(t) = std::fs::read_to_string(d.join("seq")) {
                seq = t.trim().parse().unwrap_or(0);
            }
        }
        Ok(State { dir: dir.map(Path::to_path_buf), seq })
    }

    /// The state directory (`None`: nothing kept).
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// The next announce sequence number: strictly greater than any used
    /// before by this identity, and the clock in milliseconds when that is larger.
    pub fn next_seq(&mut self) -> u64 {
        let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
        self.seq = (self.seq + 1).max(now_ms);
        if let Some(d) = &self.dir {
            if let Err(e) = write_atomic(&d.join("seq"), self.seq.to_string().as_bytes()) {
                eprintln!("can't store the sequence counter: {e}");
            }
        }
        self.seq
    }

    pub fn neighbors(&self) -> Vec<Remembered> {
        let Some(d) = &self.dir else { return vec![] };
        let Ok(t) = std::fs::read_to_string(d.join("neighbors.json")) else { return vec![] };
        let list: Vec<Remembered> = serde_json::from_str(&t).unwrap_or_default();
        let cutoff = unix_now().saturating_sub(NEIGHBOR_DAYS * 86400);
        list.into_iter().filter(|r| r.seen >= cutoff).collect()
    }

    /// Merges `up` (neighbors up now) into the stored list, newest first.
    pub fn remember(&self, up: &[Remembered]) {
        let Some(d) = &self.dir else { return };
        let mut list = self.neighbors();
        for r in up {
            list.retain(|x| x.key != r.key);
            list.push(r.clone());
        }
        list.sort_by(|a, b| b.seen.cmp(&a.seen));
        list.truncate(MAX_REMEMBERED);
        if let Ok(j) = serde_json::to_vec_pretty(&list) {
            if let Err(e) = write_atomic(&d.join("neighbors.json"), &j) {
                eprintln!("can't store neighbors: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seq_only_grows_across_runs() {
        let dir = std::env::temp_dir().join(format!("vork-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = State::open(Some(&dir)).unwrap();
        let a = s.next_seq();
        let b = s.next_seq();
        assert!(b > a);
        // A counter ahead of the clock (a clock set back) still grows.
        std::fs::write(dir.join("seq"), (b + 1_000_000).to_string()).unwrap();
        let mut s = State::open(Some(&dir)).unwrap();
        assert_eq!(s.next_seq(), b + 1_000_001);
        let r = Remembered { addr: "1.2.3.4:5".parse().unwrap(), key: "ab".into(), seen: unix_now() };
        s.remember(&[r.clone()]);
        assert_eq!(s.neighbors(), vec![r]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

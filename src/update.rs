//! Self-updating (ADR-0028): a node takes a newer build, signed with a release
//! key it trusts, from one of the owner's nodes, and puts it in place itself.
//!
//! A release is a directory: `manifest.json` (build number, version, and per
//! target the binary's SHA-256 and size), `manifest.sig` (Ed25519 over the
//! manifest's exact bytes, hex) and the binaries, `vork-<target>[.exe]`. A
//! node with `update_dir` serves its release to the owner's devices over
//! `/vork/update`; one with `update_from` asks it. Nothing is taken unless the
//! signature is by a key in `update_keys`, the build is newer than its own,
//! and the downloaded file has the signed hash; the hash and signature are
//! checked again when the file is put in place.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::identity::Identity;

/// What a release signature is over (with the manifest's bytes).
pub const LABEL: &[u8] = b"vork release v1";
/// Where releases are asked for.
pub const PATH: &str = "/vork/update";
/// Bytes per piece of a download.
pub const CHUNK: u64 = 256 * 1024;

/// The build number as a marker in the binary, so a release's number is read
/// from the file it ships, not written by hand (ADR-0028).
#[used]
static BUILD_MARK: &str = concat!("vork-build-mark:", env!("VORK_BUILD"), ";");

/// This binary's build number (commits so far when it was built).
pub fn build() -> u32 {
    BUILD_MARK.trim_start_matches("vork-build-mark:").trim_end_matches(';').parse().unwrap_or(0)
}

/// The build number marked in a binary file.
pub fn build_of(file: &Path) -> Result<u32> {
    let bytes = std::fs::read(file).with_context(|| format!("{}", file.display()))?;
    let mark = b"vork-build-mark:";
    // The text also stands alone in the binary (in this code): the one with digits after it.
    bytes
        .windows(mark.len())
        .enumerate()
        .filter(|(_, w)| *w == mark)
        .find_map(|(at, _)| {
            let rest = &bytes[at + mark.len()..];
            let end = rest.iter().take(12).position(|b| *b == b';')?;
            std::str::from_utf8(&rest[..end]).ok().filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))?.parse().ok()
        })
        .ok_or_else(|| anyhow!("{}: not a vork binary (no build mark)", file.display()))
}

/// This binary's target, as releases name it.
pub fn target() -> &'static str {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "linux-x86_64"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "linux-aarch64"
    } else if cfg!(all(windows, target_arch = "x86_64")) {
        "windows-x86_64"
    } else if cfg!(all(target_os = "haiku", target_arch = "x86_64")) {
        "haiku-x86_64" // updated with its package, not by itself
    } else if cfg!(target_os = "android") {
        "android" // updated with its app, not by itself
    } else {
        "other"
    }
}

/// The vØrk window beside the Windows node (`vork-net.exe`): in a release as
/// a companion of the node's binary, put in place with it when it differs, so
/// the window doesn't stay behind an updated node. It has no build mark.
pub const WINDOW_TARGET: &str = "windows-x86_64-window";
pub const WINDOW_EXE: &str = "vork-net.exe";

/// A companion file (the window), not a node binary with a build mark.
fn companion(target: &str) -> bool {
    target.ends_with("-window")
}

/// The binary's file name in a release.
pub fn file_name(target: &str) -> String {
    if target.starts_with("windows") { format!("vork-{target}.exe") } else { format!("vork-{target}") }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FileInfo {
    pub sha256: String,
    pub size: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Manifest {
    pub build: u32,
    pub version: String,
    pub files: BTreeMap<String, FileInfo>,
}

/// The manifest, if signed by one of `keys`.
pub fn verify(manifest: &[u8], sig: &[u8; 64], keys: &[[u8; 32]]) -> Option<Manifest> {
    if !keys.iter().any(|k| crate::identity::verify(k, LABEL, manifest, sig)) {
        return None;
    }
    serde_json::from_slice(manifest).ok()
}

pub fn parse_key(s: &str) -> Result<[u8; 32]> {
    hex::decode(s.trim()).ok().and_then(|b| <[u8; 32]>::try_from(b).ok()).ok_or_else(|| anyhow!("{s}: a release key is 64 hex digits"))
}

/// A release directory's manifest and signature.
pub fn read_release(dir: &Path) -> Option<(Vec<u8>, [u8; 64])> {
    let m = std::fs::read(dir.join("manifest.json")).ok()?;
    let s = std::fs::read_to_string(dir.join("manifest.sig")).ok()?;
    let sig = hex::decode(s.trim()).ok().and_then(|b| <[u8; 64]>::try_from(b).ok())?;
    Some((m, sig))
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h)?;
    Ok(hex::encode(h.finalize()))
}

// ---- making releases (on the machine holding the release key) ---------------

/// A new release key, written to `path`; returns its public half (hex), for `update_keys`.
pub fn keygen(path: &Path) -> Result<String> {
    if path.exists() {
        bail!("{} exists already", path.display());
    }
    let id = Identity::generate()?;
    id.save(path)?;
    Ok(hex::encode(id.ed_pub()))
}

/// Writes a release to `dir`: the binaries copied in, the manifest, its
/// signature. The build number is the binaries' own (all must agree); `build`
/// only for tests.
pub fn make(key: &Path, dir: &Path, build: Option<u32>, version: &str, files: &[(String, PathBuf)]) -> Result<Manifest> {
    let id = Identity::load(key).with_context(|| format!("release key {}", key.display()))?;
    let build = match build {
        Some(b) => b,
        None => {
            let mut b = None;
            for (_, p) in files.iter().filter(|(t, _)| !companion(t)) {
                let n = build_of(p)?;
                if b.is_some_and(|x| x != n) {
                    bail!("the binaries are of different builds ({} and {n})", b.unwrap());
                }
                b = Some(n);
            }
            b.ok_or_else(|| anyhow!("no binaries"))?
        }
    };
    std::fs::create_dir_all(dir)?;
    let mut m = Manifest { build, version: version.to_string(), files: BTreeMap::new() };
    for (target, path) in files {
        if !target.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
            bail!("{target}: a target is letters, digits, - and _");
        }
        let to = dir.join(file_name(target));
        std::fs::copy(path, &to).with_context(|| format!("{}", path.display()))?;
        m.files.insert(target.clone(), FileInfo { sha256: sha256_file(&to)?, size: std::fs::metadata(&to)?.len() });
    }
    let bytes = serde_json::to_vec_pretty(&m)?;
    let sig = id.sign(LABEL, &bytes);
    std::fs::write(dir.join("manifest.json"), &bytes)?;
    std::fs::write(dir.join("manifest.sig"), hex::encode(sig))?;
    Ok(m)
}

// ---- serving a release (a node with update_dir) -------------------------------

/// The answer to a request on `/vork/update` from one of the owner's devices.
pub fn serve(dir: &Path, body: &[u8]) -> Result<Vec<u8>, String> {
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| "bad request".to_string())?;
    match v["op"].as_str() {
        Some("manifest") => {
            let (m, sig) = read_release(dir).ok_or("no release here")?;
            let mut out = sig.to_vec();
            out.extend_from_slice(&m);
            Ok(out)
        }
        Some("get") => {
            let target = v["target"].as_str().ok_or("get needs target")?;
            if !target.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
                return Err("bad target".into());
            }
            let offset = v["offset"].as_u64().ok_or("get needs offset")?;
            use std::io::{Read, Seek, SeekFrom};
            let mut f = std::fs::File::open(dir.join(file_name(target))).map_err(|_| "no such file".to_string())?;
            f.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
            let mut buf = Vec::with_capacity(CHUNK as usize);
            f.take(CHUNK).read_to_end(&mut buf).map_err(|e| e.to_string())?;
            Ok(buf)
        }
        _ => Err("unknown op".into()),
    }
}

// ---- downloading and putting in place --------------------------------------------

/// Where a downloaded update waits to be put in place.
pub fn staging(state: &Path) -> PathBuf {
    state.join("update")
}

/// A download in progress: the node's binary, then the window when its
/// release has one that isn't the one here.
pub struct Download {
    pub manifest: Vec<u8>,
    pub sig: [u8; 64],
    pub build: u32,
    pub info: FileInfo,
    pub offset: u64,
    hasher: Sha256,
    part: PathBuf,
    /// (target, what it is, the name it waits under), the one being fetched first.
    queue: Vec<(String, FileInfo, &'static str)>,
}

impl Download {
    pub fn start(state: &Path, manifest: Vec<u8>, sig: [u8; 64], m: &Manifest) -> Result<Download> {
        let info = m.files.get(target()).cloned().ok_or_else(|| anyhow!("no {} in release {}", target(), m.build))?;
        let mut queue = vec![(target().to_string(), info.clone(), "vork.new")];
        // The window, on Windows, when the release's isn't the one beside this binary.
        if target() == "windows-x86_64" {
            if let (Some(w), Ok(exe)) = (m.files.get(WINDOW_TARGET), std::env::current_exe()) {
                let here = exe.with_file_name(WINDOW_EXE);
                if here.exists() && sha256_file(&here).ok().as_deref() != Some(w.sha256.as_str()) {
                    queue.push((WINDOW_TARGET.to_string(), w.clone(), "vork-net.new"));
                }
            }
        }
        let dir = staging(state);
        std::fs::create_dir_all(&dir)?;
        let _ = std::fs::remove_file(dir.join("vork-net.new"));
        let part = dir.join("vork.part");
        std::fs::write(&part, b"")?;
        Ok(Download { manifest, sig, build: m.build, info, offset: 0, hasher: Sha256::new(), part, queue })
    }

    /// The release's target being fetched now.
    pub fn target(&self) -> &str {
        &self.queue[0].0
    }

    /// Adds a piece; Ok(true) when the whole file is in and its hash is right.
    pub fn add(&mut self, piece: &[u8]) -> Result<bool> {
        if piece.is_empty() || self.offset + piece.len() as u64 > self.info.size {
            bail!("the release sent {} bytes at {} of {}", piece.len(), self.offset, self.info.size);
        }
        use std::io::Write;
        std::fs::OpenOptions::new().append(true).open(&self.part)?.write_all(piece)?;
        self.hasher.update(piece);
        self.offset += piece.len() as u64;
        if self.offset < self.info.size {
            return Ok(false);
        }
        let got = hex::encode(std::mem::take(&mut self.hasher).finalize());
        if got != self.info.sha256 {
            let _ = std::fs::remove_file(&self.part);
            bail!("the downloaded build {} doesn't have the signed hash ({})", self.build, self.queue[0].0);
        }
        let dir = self.part.parent().unwrap().to_path_buf();
        let (_, _, name) = self.queue.remove(0);
        std::fs::rename(&self.part, dir.join(name))?;
        // The next file, if any.
        if let Some((_, info, _)) = self.queue.first() {
            self.info = info.clone();
            self.offset = 0;
            std::fs::write(&self.part, b"")?;
            return Ok(false);
        }
        std::fs::write(dir.join("manifest.json"), &self.manifest)?;
        std::fs::write(dir.join("manifest.sig"), hex::encode(self.sig))?;
        Ok(true)
    }
}

/// Puts a downloaded update in place of `exe`, after checking its signature
/// and hash again; the old one stays as `<exe>.previous`. Ok(None): nothing
/// waiting (or nothing newer than this binary).
pub fn install(state: &Path, keys: &[[u8; 32]], exe: &Path) -> Result<Option<u32>> {
    let dir = staging(state);
    let new = dir.join("vork.new");
    if !new.exists() {
        return Ok(None);
    }
    let (m, sig) = read_release(&dir).ok_or_else(|| anyhow!("{}: no manifest beside the update", dir.display()))?;
    let manifest = verify(&m, &sig, keys).ok_or_else(|| anyhow!("the waiting update isn't signed by a trusted release key"))?;
    if manifest.build <= build() {
        let _ = std::fs::remove_file(&new);
        return Ok(None);
    }
    let info = manifest.files.get(target()).ok_or_else(|| anyhow!("the waiting update has no {}", target()))?;
    if sha256_file(&new)? != info.sha256 {
        let _ = std::fs::remove_file(&new);
        bail!("the waiting update's hash isn't the signed one: removed");
    }
    let previous = with_suffix(exe, ".previous");
    #[cfg(windows)]
    {
        // A running .exe can't be overwritten but can be renamed.
        let _ = std::fs::remove_file(&previous);
        std::fs::rename(exe, &previous).with_context(|| format!("moving {} aside", exe.display()))?;
        if let Err(e) = std::fs::copy(&new, exe) {
            let _ = std::fs::rename(&previous, exe);
            return Err(e).with_context(|| format!("writing {}", exe.display()));
        }
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::copy(exe, &previous).with_context(|| format!("keeping {}", previous.display()))?;
        let tmp = with_suffix(exe, ".new");
        std::fs::copy(&new, &tmp).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        std::fs::rename(&tmp, exe).with_context(|| format!("replacing {}", exe.display()))?;
    }
    let _ = std::fs::remove_file(&new);
    let _ = std::fs::write(dir.join("installed"), manifest.build.to_string());
    install_window(&dir, &manifest, exe);
    Ok(Some(manifest.build))
}

/// The window that came with the update, beside the node's binary (Windows);
/// one running is moved aside (it goes on until it's started again). Best
/// effort: the node's own update doesn't depend on it.
fn install_window(dir: &Path, manifest: &Manifest, exe: &Path) {
    let new = dir.join("vork-net.new");
    let Some(info) = manifest.files.get(WINDOW_TARGET) else {
        let _ = std::fs::remove_file(&new);
        return;
    };
    if !new.exists() || sha256_file(&new).ok().as_deref() != Some(info.sha256.as_str()) {
        let _ = std::fs::remove_file(&new);
        return;
    }
    let to = exe.with_file_name(WINDOW_EXE);
    let previous = with_suffix(&to, ".previous");
    let _ = std::fs::remove_file(&previous);
    if to.exists() && std::fs::rename(&to, &previous).is_err() {
        return;
    }
    if std::fs::copy(&new, &to).is_err() {
        let _ = std::fs::rename(&previous, &to);
        return;
    }
    let _ = std::fs::remove_file(&new);
}

/// The build last put in place, if this binary is older (it didn't take: the
/// release's file isn't what its number says, or the old binary was started).
/// A newer binary was installed some other way (an installer, by hand): the
/// mark is forgotten, and self-updating goes on.
pub fn failed_install(state: &Path) -> Option<u32> {
    let mark = staging(state).join("installed");
    let b: u32 = std::fs::read_to_string(&mark).ok()?.trim().parse().ok()?;
    if b < build() {
        let _ = std::fs::remove_file(&mark);
        return None;
    }
    (b != build()).then_some(b)
}

fn with_suffix(p: &Path, s: &str) -> PathBuf {
    let mut o = p.as_os_str().to_owned();
    o.push(s);
    PathBuf::from(o)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_signed_downloaded_installed() {
        let root = std::env::temp_dir().join(format!("vork-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let key = root.join("release.key");
        let pubkey = parse_key(&keygen(&key).unwrap()).unwrap();
        let bin = root.join("built");
        std::fs::write(&bin, vec![7u8; 600_000]).unwrap();
        let rel = root.join("release");
        let m = make(&key, &rel, Some(build() + 1), "test", &[(target().to_string(), bin.clone())]).unwrap();
        // Served in pieces, taken in pieces.
        let answer = serve(&rel, br#"{"op":"manifest"}"#).unwrap();
        let sig: [u8; 64] = answer[..64].try_into().unwrap();
        let got = verify(&answer[64..], &sig, &[pubkey]).unwrap();
        assert_eq!(got.build, m.build);
        assert!(verify(&answer[64..], &sig, &[[9u8; 32]]).is_none(), "another key's");
        let state = root.join("state");
        let mut d = Download::start(&state, answer[64..].to_vec(), sig, &got).unwrap();
        loop {
            let piece = serve(&rel, format!(r#"{{"op":"get","target":"{}","offset":{}}}"#, target(), d.offset).as_bytes()).unwrap();
            if d.add(&piece).unwrap() {
                break;
            }
        }
        // Put in place, the old one kept.
        let exe = root.join("vork-exe");
        std::fs::write(&exe, b"old").unwrap();
        assert_eq!(install(&state, &[pubkey], &exe).unwrap(), Some(m.build));
        assert_eq!(std::fs::read(&exe).unwrap().len(), 600_000);
        assert_eq!(std::fs::read(with_suffix(&exe, ".previous")).unwrap(), b"old");
        // Not again, and not with an untrusted key.
        assert_eq!(install(&state, &[pubkey], &exe).unwrap(), None);
        // A release's number comes from the binary it ships.
        let me = std::env::current_exe().unwrap();
        assert_eq!(build_of(&me).unwrap(), build());
        assert!(make(&key, &root.join("r2"), None, "", &[(target().to_string(), bin.clone())]).is_err(), "no mark: refused");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_tampered_download_is_refused() {
        let root = std::env::temp_dir().join(format!("vork-update-t-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let key = root.join("release.key");
        let pubkey = parse_key(&keygen(&key).unwrap()).unwrap();
        let bin = root.join("built");
        std::fs::write(&bin, vec![1u8; 1000]).unwrap();
        let rel = root.join("release");
        make(&key, &rel, Some(build() + 1), "test", &[(target().to_string(), bin)]).unwrap();
        let (m, sig) = read_release(&rel).unwrap();
        let man = verify(&m, &sig, &[pubkey]).unwrap();
        let mut d = Download::start(&root.join("state"), m, sig, &man).unwrap();
        assert!(d.add(&vec![2u8; 1000]).is_err(), "other bytes than signed");
        let _ = std::fs::remove_dir_all(&root);
    }
}

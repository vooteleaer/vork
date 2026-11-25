//! On Linux, every library is a folder in `~/vork/<library>` (the FUSE
//! frontend, `vork-mount`): vorkd mounts each one it has, a library
//! joined later within seconds, and unmounts one it has left. Only for the
//! user's own store (vorkd started without `--store`, as the .deb's user
//! service is) and not with `--no-mounts`: a test node's store isn't
//! anyone's files.
//!
//! A mount is recorded under the store (`mounts/<library>`) as `vØrk storage
//! mount` records it, so `vØrk storage mount` lists it and `vØrk storage unmount` takes
//! it down; one taken down stays down until vorkd starts again. A frontend
//! that stopped (its record left behind) is started again.
//!
//! With `--mounts DIR` (a NAS's storage container) the folders are in DIR
//! instead, also for a given store, open to other users (another
//! container's apps, through a bind mount of DIR) and shown as DIR's
//! owner's files.

use std::collections::{HashMap, HashSet};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{visible_namespaces, Daemon};

/// How often the libraries are looked at again.
const EVERY: Duration = Duration::from_secs(10);
/// A mount that failed is tried again after this.
const RETRY: Duration = Duration::from_secs(300);
/// How long after its frontend exited a mount waits to be started again.
const EXIT_GRACE: Duration = Duration::from_secs(20);

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| "/".into())
}

/// Where the libraries' folders are, and whether they're for others too.
#[derive(Clone)]
struct Place {
    root: PathBuf,
    shared: bool,
}

impl Place {
    fn dir_of(&self, ns: &str) -> PathBuf {
        self.root.join(ns)
    }
}

fn record_of(store: &Path, ns: &str) -> PathBuf {
    store.join("mounts").join(ns)
}

/// Whether something is mounted at `dir`, from the kernel's mount table:
/// never a stat of the folder, which would be a FUSE request the frontend
/// answers by asking this daemon (a `vork unmount` hung behind one).
fn is_mounted(dir: &Path) -> bool {
    let want = dir.to_string_lossy();
    std::fs::read_to_string("/proc/self/mountinfo").is_ok_and(|t| t.lines().any(|l| l.split(' ').nth(4).is_some_and(|m| unescape(m) == want)))
}

/// A mount point as /proc/self/mountinfo writes it (space, tab, newline and
/// backslash as octal escapes).
fn unescape(s: &str) -> String {
    s.replace("\\040", " ").replace("\\011", "\t").replace("\\012", "\n").replace("\\134", "\\")
}

/// The FUSE frontend: next to vorkd (the .deb, cargo install), else on the PATH.
fn frontend() -> PathBuf {
    let here = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("vork-mount")));
    here.filter(|p| p.is_file()).unwrap_or_else(|| "vork-mount".into())
}

/// The frontend's pid from a mount's record, if it's still running.
fn recorded_pid(store: &Path, ns: &str) -> Option<u32> {
    let text = std::fs::read_to_string(record_of(store, ns)).ok()?;
    let pid: u32 = text.lines().next()?.trim().parse().ok()?;
    Path::new(&format!("/proc/{pid}")).exists().then_some(pid)
}

fn mount(place: &Place, store: &Path, ns: &str) -> Result<Child, String> {
    let dir = place.dir_of(ns);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let records = store.join("mounts");
    std::fs::create_dir_all(&records).map_err(|e| e.to_string())?;
    let log = std::fs::File::create(records.join(format!("{ns}.log"))).map_err(|e| e.to_string())?;
    let mut cmd = Command::new(frontend());
    if place.shared {
        use std::os::unix::fs::MetadataExt;
        let m = std::fs::metadata(&place.root).map_err(|e| format!("{}: {e}", place.root.display()))?;
        cmd.arg("--allow-other").arg("--owner").arg(format!("{}:{}", m.uid(), m.gid()));
    }
    let child = cmd
        .arg("--store")
        .arg(store)
        .arg(ns)
        .arg(&dir)
        .stdin(Stdio::null())
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log)
        .process_group(0)
        .spawn()
        .map_err(|e| format!("{}: {e}", frontend().display()))?;
    std::fs::write(record_of(store, ns), format!("{}\n{}\n\n", child.id(), dir.display())).map_err(|e| e.to_string())?;
    Ok(child)
}

/// A library left: its folder unmounted (the frontend then exits by itself).
/// fusermount3 for a user's own mounts; umount where there's no fusermount3
/// (a container, as root).
fn unmount(place: &Place, store: &Path, ns: &str) {
    let dir = place.dir_of(ns);
    let ran = |exe: &str, flag: &str| Command::new(exe).arg(flag).arg(&dir).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
    if is_mounted(&dir) && !ran("fusermount3", "-u") && !ran("fusermount3", "-uz") && !ran("umount", "-l") {
        eprintln!("mounts: couldn't unmount {}", dir.display());
        return;
    }
    let _ = std::fs::remove_file(record_of(store, ns));
    let _ = std::fs::remove_dir(&dir); // the empty mount point, if nothing else is in it
    println!("mounts: {ns} unmounted from {}", dir.display());
}

/// "Share with vork…" for the file managers that are here (ADR-0021):
/// Nautilus's scripts folder and Dolphin's service menus, in the user's own
/// folders. Written when missing or different; nothing for one not installed.
fn file_manager_menus() {
    let on_path = |exe: &str| std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(exe).is_file()));
    let share = home().join(".local/share");
    let menus: [(&str, PathBuf, &str); 2] = [
        ("nautilus", share.join("nautilus/scripts/Share with vork"), include_str!("share-nautilus.sh")),
        ("dolphin", share.join("kio/servicemenus/vork-share.desktop"), include_str!("vork-share.desktop")),
    ];
    for (exe, path, text) in menus {
        if !on_path(exe) || std::fs::read_to_string(&path).is_ok_and(|t| t == text) {
            continue;
        }
        let written = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&path, text)).and_then(|_| {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        });
        match written {
            Ok(()) => println!("mounts: Share with vork added to {exe} ({})", path.display()),
            Err(e) => eprintln!("mounts: {}: {e}", path.display()),
        }
    }
}

/// A shared folder's mounts left by an earlier run (its container gone, the
/// mounts still in the host's table, propagated back in and dead), and their
/// records: taken down, so every library is mounted afresh. Nothing in this
/// run has mounted anything yet.
fn clear_stale(place: &Place, store: &Path) {
    let under = format!("{}/", place.root.to_string_lossy().trim_end_matches('/'));
    let table = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    for line in table.lines() {
        // "… mountpoint … - fstype source options": vork-mount's are fuse
        // (fuse.vork where the subtype is kept) with source vork:<library>.
        let mut fs = line.split(" - ").nth(1).unwrap_or("").split(' ');
        let (Some(at), Some(fstype), Some(source)) = (line.split(' ').nth(4), fs.next(), fs.next()) else { continue };
        let at = unescape(at);
        if at.starts_with(&under) && fstype.starts_with("fuse") && source.starts_with("vork:") {
            let gone = Command::new("umount").arg("-l").arg(&at).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
            println!("mounts: an earlier run's mount at {at} {}", if gone { "taken down" } else { "couldn't be taken down" });
        }
    }
    if let Ok(records) = std::fs::read_dir(store.join("mounts")) {
        for r in records.flatten() {
            if r.path().extension().is_none() {
                let _ = std::fs::remove_file(r.path());
            }
        }
    }
}

pub async fn keeper(d: Arc<Daemon>, store: PathBuf, root: Option<PathBuf>) {
    let place = match root {
        Some(root) => Place { root, shared: true },
        None => {
            file_manager_menus();
            Place { root: home().join("vork"), shared: false }
        }
    };
    if !Path::new("/dev/fuse").exists() {
        eprintln!("mounts: no /dev/fuse here: libraries aren't mounted as folders");
        return;
    }
    if place.shared {
        clear_stale(&place, &store);
    }
    // Let the local API come up first: the frontends talk to it.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let mut children: HashMap<String, Child> = HashMap::new();
    // Mounted by this keeper since vorkd started: one whose record is gone
    // was taken down with `vork unmount`, and stays down.
    let mut mounted: HashSet<String> = HashSet::new();
    let mut failed: HashMap<String, Instant> = HashMap::new();
    // Frontends that exited, and when: `vork unmount` removes the record
    // only after its frontend is gone, so one is started again only if its
    // record is still there a while later (it stopped by itself).
    let mut exited: HashMap<String, Instant> = HashMap::new();
    loop {
        let names = visible_namespaces(&d).unwrap_or_default();
        // Frontends that have exited: reaped, so they don't linger as zombies.
        children.retain(|ns, c| {
            let running = matches!(c.try_wait(), Ok(None));
            if !running {
                exited.insert(ns.clone(), Instant::now());
            }
            running
        });
        for ns in &names {
            let dir = place.dir_of(ns);
            let recorded = record_of(&store, ns).exists();
            if is_mounted(&dir)
                || (mounted.contains(ns) && !recorded)
                || failed.get(ns).is_some_and(|at| at.elapsed() < RETRY)
                || exited.get(ns).is_some_and(|at| at.elapsed() < EXIT_GRACE)
            {
                continue;
            }
            // A frontend still starting (recorded, running, not mounted yet).
            if recorded && recorded_pid(&store, ns).is_some() {
                continue;
            }
            match mount(&place, &store, ns) {
                Ok(child) => {
                    println!("mounts: {ns} at {}", dir.display());
                    children.insert(ns.clone(), child);
                    mounted.insert(ns.clone());
                    failed.remove(ns);
                }
                Err(e) => {
                    eprintln!("mounts: {ns}: {e}");
                    failed.insert(ns.clone(), Instant::now());
                }
            }
        }
        // Left: no longer a library here.
        for ns in mounted.clone() {
            if !names.contains(&ns) {
                unmount(&place, &store, &ns);
                mounted.remove(&ns);
            }
        }
        tokio::time::sleep(EVERY).await;
    }
}

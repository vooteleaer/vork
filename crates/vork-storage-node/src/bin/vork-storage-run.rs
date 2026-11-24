#![cfg_attr(windows, windows_subsystem = "windows")]
//! vork-storage-run: keeps vØrk storage running for a user: vorkd, its pages
//! in the vØrk dashboard (vork-web --dashboard), the extractor that describes
//! new files (vork-extractd: types, dates, thumbnails) and, on Windows, the
//! libraries in Explorer (vork-cloudfiles --all). Started at login by the
//! installer; each program is started again if it ends, its output kept in
//! `<store>/logs`.
//!
//!     vork-storage-run [--store DIR]           (runs them; one per store)
//!     vork-storage-run [--store DIR] --stop    (asks the running one to stop them, and waits)
//!
//! The programs are looked for beside this one.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn exe(dir: &Path, name: &str) -> PathBuf {
    dir.join(if cfg!(windows) { format!("{name}.exe") } else { name.to_string() })
}

/// A program kept running: how to start it, the running one, when to try again.
struct Kept {
    name: &'static str,
    path: PathBuf,
    args: Vec<String>,
    child: Option<Child>,
    started: Instant,
    retry_at: Option<Instant>,
    backoff: Duration,
}

fn log_file(logs: &Path, name: &str) -> std::fs::File {
    let path = logs.join(format!("{name}.log"));
    // One older log kept beside it once it grows past 10 MB.
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 10 << 20) {
        let _ = std::fs::rename(&path, logs.join(format!("{name}.1.log")));
    }
    std::fs::OpenOptions::new().create(true).append(true).open(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn start(k: &mut Kept, logs: &Path) {
    let out = log_file(logs, k.name);
    let err = out.try_clone().expect("log file");
    let mut c = Command::new(&k.path);
    c.args(&k.args).stdin(Stdio::null()).stdout(out).stderr(err);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    match c.spawn() {
        Ok(child) => {
            k.child = Some(child);
            k.started = Instant::now();
            k.retry_at = None;
        }
        Err(e) => {
            eprintln!("{}: {e}", k.path.display());
            k.retry_at = Some(Instant::now() + k.backoff);
        }
    }
}

/// Asks vorkd to stop (it closes its store cleanly), then ends the rest.
fn stop_all(kept: &mut [Kept], store: &Path) {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
    let asked = rt.block_on(async {
        match libvork::Client::connect(store).await {
            Ok(mut c) => tokio::time::timeout(Duration::from_secs(5), c.stop()).await.is_ok(),
            Err(_) => false,
        }
    });
    let deadline = Instant::now() + Duration::from_secs(if asked { 30 } else { 0 });
    for k in kept.iter_mut().filter(|k| k.name == "vorkd") {
        if let Some(c) = k.child.as_mut() {
            while Instant::now() < deadline && matches!(c.try_wait(), Ok(None)) {
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }
    for k in kept.iter_mut() {
        if let Some(mut c) = k.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let given: Option<PathBuf> = args.iter().position(|a| a == "--store").and_then(|i| args.get(i + 1)).map(PathBuf::from);
    let store = given.clone().unwrap_or_else(libvork::default_store_dir);
    let stop_file = store.join("run.stop");
    if args.iter().any(|a| a == "--stop") {
        // The running one sees this and stops its programs; waited for by its lock.
        let _ = std::fs::write(&stop_file, b"");
        let lock = store.join("run.lock");
        for _ in 0..600 {
            match std::fs::OpenOptions::new().write(true).open(&lock) {
                Ok(f) if f.try_lock().is_ok() => break,
                Err(_) => break,
                _ => std::thread::sleep(Duration::from_millis(100)),
            }
        }
        let _ = std::fs::remove_file(&stop_file);
        return;
    }
    let logs = store.join("logs");
    std::fs::create_dir_all(&logs).unwrap_or_else(|e| panic!("{}: {e}", logs.display()));
    let _ = std::fs::remove_file(&stop_file);
    // One per store: held while it runs.
    let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(store.join("run.lock")).expect("lock file");
    if lock.try_lock().is_err() {
        return; // another runs them already
    }
    let here = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(|| ".".into());
    // The store named only when it was named here: vorkd without --store
    // is the user's own, whose libraries it shows as folders in ~/vork (Linux).
    let named: Vec<String> = given.iter().flat_map(|g| ["--store".to_string(), g.to_string_lossy().into_owned()]).collect();
    let with = |more: &[&str]| named.iter().cloned().chain(more.iter().map(|m| m.to_string())).collect::<Vec<String>>();
    let mut programs: Vec<(&'static str, Vec<String>)> = vec![("vorkd", with(&[])), ("vork-web", with(&["--dashboard"])), ("vork-extractd", with(&[]))];
    if cfg!(windows) {
        programs.push(("vork-cloudfiles", with(&["--all"])));
    }
    let mut kept: Vec<Kept> = programs
        .into_iter()
        .map(|(name, args)| Kept { name, path: exe(&here, name), args, child: None, started: Instant::now(), retry_at: None, backoff: Duration::from_secs(5) })
        .filter(|k| k.path.exists())
        .collect();
    for k in kept.iter_mut() {
        start(k, &logs);
        // vorkd first: the others talk to it.
        if k.name == "vorkd" {
            std::thread::sleep(Duration::from_secs(3));
        }
    }
    loop {
        std::thread::sleep(Duration::from_secs(2));
        if stop_file.exists() {
            stop_all(&mut kept, &store);
            let _ = std::fs::remove_file(&stop_file);
            return;
        }
        for k in kept.iter_mut() {
            if let Some(c) = k.child.as_mut() {
                if let Ok(Some(status)) = c.try_wait() {
                    k.child = None;
                    // Ended soon after starting: wait longer each time (at most a minute).
                    k.backoff = if k.started.elapsed() < Duration::from_secs(60) { (k.backoff * 2).min(Duration::from_secs(60)) } else { Duration::from_secs(5) };
                    k.retry_at = Some(Instant::now() + k.backoff);
                    eprintln!("{} ended ({status}); again in {} s", k.name, k.backoff.as_secs());
                }
            }
            if k.child.is_none() && k.retry_at.is_some_and(|t| Instant::now() >= t) {
                start(k, &logs);
            }
        }
    }
}

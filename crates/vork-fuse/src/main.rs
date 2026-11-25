//! `vork-mount NS MOUNTPOINT [--store DIR]`: a library as a directory
//! tree (ADR-0002), through `vorkd`'s local API. Linux (FUSE) only.

#[cfg(target_os = "linux")]
use vork_tree as tree;

#[cfg(target_os = "linux")]
mod fs;

fn usage() -> ! {
    eprintln!("vork-mount [--store DIR] [--wait SECONDS] [--via NODE] [--allow-other] [--owner UID:GID] NS MOUNTPOINT");
    eprintln!("  --wait: how long a read waits for content that isn't here yet (default 30)");
    eprintln!("  --via: the library is on another of your nodes (ADR-0019), not this one");
    eprintln!("  --allow-other: other users may use the mount too (root, or user_allow_other in /etc/fuse.conf)");
    eprintln!("  --owner: whose the files are shown as (default: yours)");
    std::process::exit(2)
}

fn main() {
    let mut store = libvork::default_store_dir();
    let mut wait = std::time::Duration::from_secs(30);
    let mut via = None;
    let mut allow_other = false;
    let mut owner = None;
    let mut rest = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--store" => store = args.next().unwrap_or_else(|| usage()).into(),
            "--wait" => wait = std::time::Duration::from_secs(args.next().and_then(|v| v.parse().ok()).unwrap_or_else(|| usage())),
            "--via" => via = Some(args.next().unwrap_or_else(|| usage())),
            "--allow-other" => allow_other = true,
            "--owner" => {
                let v = args.next().unwrap_or_else(|| usage());
                let (u, g) = v.split_once(':').unwrap_or((&v, &v));
                owner = Some((u.parse::<u32>().unwrap_or_else(|_| usage()), g.parse::<u32>().unwrap_or_else(|_| usage())));
            }
            "-h" | "--help" => usage(),
            _ => rest.push(a),
        }
    }
    let [ns, mountpoint] = <[String; 2]>::try_from(rest).unwrap_or_else(|_| usage());
    run(&store, &ns, std::path::Path::new(&mountpoint), wait, via, allow_other, owner);
}

#[cfg(target_os = "linux")]
fn run(store: &std::path::Path, ns: &str, mountpoint: &std::path::Path, wait: std::time::Duration, via: Option<String>, allow_other: bool, owner: Option<(u32, u32)>) {
    let on = via.as_deref().map(|v| format!(" on node {v}")).unwrap_or_default();
    eprintln!("vork-mount: library {ns}{on} at {} (unmount with fusermount3 -u or umount)", mountpoint.display());
    if let Err(e) = fs::mount(store, ns, mountpoint, wait, via, fs::Sharing { allow_other, owner }) {
        eprintln!("vork-mount: {e}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "linux"))]
fn run(_store: &std::path::Path, _ns: &str, _mountpoint: &std::path::Path, _wait: std::time::Duration, _via: Option<String>, _allow_other: bool, _owner: Option<(u32, u32)>) {
    eprintln!("vork-mount: FUSE mounts are Linux only for now (ADR-0002); Haiku's userlandfs frontend comes later");
    std::process::exit(1);
}

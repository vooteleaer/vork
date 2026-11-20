// The build number (commits so far): self-updating takes a release only if
// its number is higher (ADR-0028). And the version (ADR-0029): the crate's
// when this commit is tagged with it, else a build towards it,
// `0.1.0-dev.143`, which the dashboard compares with GitHub's releases.
fn main() {
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
    };
    let n = git(&["rev-list", "--count", "HEAD"]).and_then(|s| s.trim().parse::<u32>().ok()).unwrap_or(0);
    println!("cargo:rustc-env=VORK_BUILD={n}");
    let version = env!("CARGO_PKG_VERSION");
    let tagged = git(&["tag", "--points-at", "HEAD"]).is_some_and(|t| t.lines().any(|l| l.trim() == format!("v{version}")));
    let full = if tagged { version.to_string() } else { format!("{version}-dev.{n}") };
    println!("cargo:rustc-env=VORK_VERSION={full}");
    for f in [".git/HEAD", ".git/refs/heads", ".git/refs/tags", ".git/packed-refs"] {
        println!("cargo:rerun-if-changed={f}");
    }
}

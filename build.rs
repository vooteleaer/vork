// The build number (commits so far): self-updating takes a release only if
// its number is higher (ADR-0028).
fn main() {
    let n = std::process::Command::new("git")
        .args(["rev-list", "--count", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(0);
    println!("cargo:rustc-env=VORK_BUILD={n}");
    for f in [".git/HEAD", ".git/refs/heads", ".git/packed-refs"] {
        println!("cargo:rerun-if-changed={f}");
    }
}

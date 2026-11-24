# Versions

vØrk storage is released with vØrk and carries vØrk's version: a release is `v0.MINOR.PATCH`, a tag on the repository's commit, as in [vØrk ADR-0029](../adr/0029-releases.md). The version is the node crate's (`[package]` in the root `Cargo.toml`).

- **A build between releases** is the next release's number, `dev` and the commit count (`git rev-list --count HEAD`), which only grows, so newer builds always sort above older ones: `0.4.0-dev.180`.
- **A build is the release's** when its commit carries the tag `v` + the version; the build scripts look for it.

| | release `0.4.0` | build between releases |
|---|---|---|
| Windows | `vork-storage-0.4.0.msi` | `vork-storage-0.4.0-dev.180.msi` |
| Linux (`.deb`), Docker, Haiku, Android | coming | coming |

The MSI's own version is `MAJOR.MINOR.<commit count>`, so Windows sees every build as newer than the one before.

Until storage moved to vØrk (2026-11-18), storage (then retifs) and its apps had a version series of their own; vØrk's started again from 0.1.0.

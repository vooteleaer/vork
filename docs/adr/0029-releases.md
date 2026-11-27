# ADR-0029: Public releases

- **Status:** accepted (the format and the keys; v0.1.0 and v0.2.0 released 2026-11-20); proposed (how public nodes fetch updates)
- **Date:** 2026-11-19
- **Irreversible:** partly (the project key, once in installed nodes' `update_keys`, is what they trust until they are reinstalled)

## Context
The repository is public, and the README has nothing to download. Self-updating (ADR-0028) exists, but only for one owner's own nodes, from a node of their own, signed with their own key. Versions were muddled: the node's crate said 0.0.1, the workspace 0.6.0 (shared with storage, then a separate project), and the first self-update release said 0.6.0-dev.

## Decision

### Versions
- **vØrk the network has its own series, from 0.1.0**, separate from storage's old one. (Since 2026-11-23 vØrk storage is released with vØrk, under vØrk's version, with installers of its own. Since 2026-11-27 it's its own repository, [vork-storage](https://github.com/vooteleaer/vork-storage), with releases of its own; vØrk 0.6.0 is the first without it.) Its version is the node crate's (`[package]` in the root `Cargo.toml`).
- **A release is `v0.MINOR.PATCH`**, a tag on the commit. Before 1.0, a MINOR release may change the wire format (nodes of different MINOR versions may not understand each other), and a PATCH release never does. Releases are GitHub pre-releases until 1.0.
- **Builds between releases** are the next release's number, `dev` and the build number: `0.2.0-dev.143`. The build number is the commit count (ADR-0028), so it only grows.

### What a release holds
One GitHub release per version, every file named so the release works for self-updating as it is:

| File | What |
|---|---|
| `vork-linux-x86_64`, `vork-linux-aarch64` | the node, static (musl) |
| `vork-windows-x86_64.exe` | the node for Windows |
| `manifest.json`, `manifest.sig` | ADR-0028's signed manifest over the node binaries |
| `vork-<version>.msi` | Windows installer (WiX): the node as a service, `wintun.dll`, the `vork-net` window and tray, a Start menu entry |
| `vork_<version>_amd64.deb`, `vork_<version>_arm64.deb` | Debian/Ubuntu: the node as a systemd service, and `vork-net` (needs WebKitGTK) |
| `vork-net-<version>.apk` | Android (arm64) |
| `SHA256SUMS` | the SHA-256 of every file above |

Release notes are written for the README's readers: what's new, whether it is compatible with the previous version (MINOR or PATCH), and which file to take.

### Keys
- **A project release key**, separate from the owner's personal one, signs public releases. The personal key keeps signing builds for the owner's own nodes, so a test build never reaches anyone else.
- **Public installs update themselves by default:** the installers and `vork init` put the project key's public half in `update_keys`. A user can remove it to stop updates, or replace it with a key of their own.
- The project key's private half stays offline (not on a server, not in CI), like the personal one.

### How public nodes fetch updates (proposed)
ADR-0028 delivers releases over vØrk, from an owner's node to that owner's devices. Nodes of strangers have no such node. Two ways, to be decided when it is built:
- **From GitHub over HTTPS:** `releases/latest/download/manifest.json` and the binary, checked against the project key. The signature makes the transport untrusted, so GitHub only has to be reachable, not trusted. Needs an HTTP client in the node.
- **Over vØrk, from project release nodes** that serve the release to anyone. No new transport, but someone has to run those nodes, and every node learns their addresses.

## Consequences
- A stranger can download, install and stay up to date without building anything.
- The project key can make every default install run its code. Its loss is the worst incident this project can have: keep it offline, and plan how to replace it (a release signed with both the old and the new key) before it's needed.
- The Linux packages own `/usr/bin/vork`; a self-update replaces it in place, so `dpkg --verify` reports it changed until the next package is installed. Android updates come with the APK, not by themselves.
- To do: the project key, a release script that builds every file above and writes `SHA256SUMS`, and the fetch path for public nodes.

## Addendum (2026-11-20): an MSI, and the dashboard asks GitHub
- **The Windows installer is an MSI** (`packaging/windows/vork.wxs`, built by `build-msi.ps1` with WiX 5, whose licence asks no fee), replacing the home-made `vork-setup.exe`: Windows' own way to install, silently too (`msiexec /i vork-….msi /qn`), with upgrades and removal from Apps & features. Its own version is `MAJOR.MINOR.BUILD`, as an MSI's must be numbers. It takes over a node installed by the old setup or by hand (the service is replaced; `%ProgramData%\vork` is kept, as on removal).
- **The binary knows its version:** `vork version` and the status show `0.1.0` for a tagged release and `0.1.0-dev.143` between releases.
- **The Updates page asks GitHub** for the latest release (pre-releases included, until 1.0), shows its notes, and when it is newer than the running version offers this platform's file to download (the MSI, the APK, the .deb, then the bare binary). The page asks, from the viewer's browser or app, when the page is opened, at most hourly: the node itself still doesn't talk to GitHub, and installing stays the user's step. Self-updating (ADR-0028) is shown below it only where it is set up.
- The first release, v0.1.0, has no `manifest.json`: the project key isn't made yet, and public nodes don't fetch updates by themselves.

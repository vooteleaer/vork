# ADR-0028: Self-updating

- **Status:** accepted
- **Date:** 2026-11-19
- **Irreversible:** no (the release key, once in nodes' `update_keys`, is what they trust)

## Context
Updating nodes by hand (copying a binary, a UAC prompt on Windows, adb on Android) doesn't scale beyond a few machines, and the threat model lists unsigned binaries as open. A node should take a newer build by itself, from the owner's own nodes, and only one the owner signed.

## Decision
- **Builds are numbered** by the commits so far, marked in the binary (`vork-build-mark:N;`); `vork version` shows it. `release make` reads the number from the binaries it ships, so a release can't claim another.
- **A release key** (Ed25519, `vork release keygen`) signs a manifest (build, version, per target the binary's SHA-256 and size): `manifest.json`, `manifest.sig`, `vork-<target>[.exe]` in a directory. The key stays with the owner; its public half goes into `update_keys`.
- **Releases travel over vØrk:** a node with `update_dir` serves its release on `/vork/update` to the owner's devices only; a node with `update_from` asks it 90 s after start and every 6 h (or at once: `vork update`), in pieces of 256 KiB over the session's stream. A node with `update_dir` takes the release itself too.
- **Taken only** if signed by a trusted key, newer than the running build, and the download has the signed hash; checked again when put in place. The old binary stays as `<binary>.previous`.
- **Put in place:** where the node can write its binary (Windows' service, a plain process), it does so (on Windows by renaming the running file aside) and exits with 75, to be started again (systemd's `Restart=on-failure`; on Windows the service sets its own recovery to restart). Under the Linux service's sandbox the binary isn't writable: the node exits, and the service's start step (`ExecStartPre=-+vork update-install`, outside the sandbox) checks and puts it in place.
- **No loops:** the build put in place is remembered; started again on another build, the node doesn't retry and says so.
- **Android** is updated with its app, not by itself.

## Consequences
- One signed release reaches all of the owner's nodes within hours, with no prompts.
- Whoever holds the release key can make every node run their code: keep it off servers.
- Not yet: a release channel for others' nodes (a project key), rollback on a failed start, and Android's app update.

## Addendum (2026-11-23): the window comes along
The vØrk window on Windows (`vork-net.exe`) came only with the MSI, so it stayed behind a node that updated itself, with an older dashboard built in. Two changes:
- The window shows the node's own dashboard (control command `page`); its built-in page only for a node older than it.
- A release may carry the window as a companion file (`windows-x86_64-window`, no build mark). A Windows node fetches it after its own binary when it isn't the one beside it, and puts it in place with its own update; a running window is moved aside and goes on until it's started again.

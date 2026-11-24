# ADR-0015: Storage (formerly retifs) is a vØrk application

- **Status:** accepted
- **Date:** 2026-11-18
- **Irreversible:** no

## Context
retifs (encrypted, content-addressed, replicated storage) was built on Reticulum through its `Transport` trait, with RNS identities (X25519 ‖ Ed25519, hash = SHA-256[0..16]) and RNS token encryption. Its blind storage nodes and vØrk's blind relays are the same idea, and both name endpoints by an identity hash. retifs has one user today, so its code and formats can change.

## Decision
- retifs becomes vØrk's storage layer and is an ordinary vØrk application (ADR-0017): a storage service has **its own vØrk identity**, is announced as its own destination and is reached through the native message API. A node may host it; the storage identity is not the node identity.
- **No Reticulum code.** The Prns dependency and the Reticulum transport are removed (ADR-0022).
- **The storage format stays** (amended 2026-11-18; changed by the addendum of 2026-11-23), so existing libraries move under vØrk unchanged: chunks, manifests, records, leases, channels, recovery kits, key wrapping, links and addresses keep their bytes. Names inside the format (`retifs/v1/…` labels, `retifs.*` record types, `retifs:` links, `retifs.node` destinations) are format constants, not branding, and keep their spelling. vØrk took over the storage identity instead: the same address hash, key file and named destinations (ADR-0004, ADR-0022).
- retifs's `Transport` trait stays as the seam; its one implementation is vØrk (`connect` opens a session to the storage identity, `link_nonce` comes from fresh randomness of both ends at session start, `request` is request/response in the session). Large transfers ride a reliable, congestion-controlled stream inside the session.
- Placement uses measured round trip and direct-vs-relayed instead of hop count.
- **One repository.** retifs's files were copied into this repository (2026-11-18, from retifs commit 10319c8) and renamed for vØrk; the retifs repository stays as it was. Crates: `vork-core`, `vork-store`, `vork-storage-net`, `vork-storage-node` (`vorkd`, the storage commands), `libvork` (local API client), `vork-tree`, `vork-fuse` (`vork-mount`), `vork-cloudfiles`, `vork-gateway`, `vork-extract`, `vork-haikufs`, `vork-card`. `vork <storage command>` runs `vork-storage`. The format's own names were not renamed; the test vectors are byte for byte retifs's. Documents are in `docs/storage/` with their own ADR numbering.
- vØrk must start and work without storage; the dependency points one way. Storage may later hold vØrk's own state (islands, petnames, remembered peers).

## Consequences
- Every node can contribute two blind resources: relaying and storage.
- One identity system: recovery kits, cards and leases in storage describe vØrk identities; YubiKey-held identities (ADR-0013) cover storage too.
- Existing libraries and identities carry over: a storage identity file is a vØrk key file, and every stored node or library address is a vØrk destination.
- No LoRa reach through storage any more; vØrk is IP-only (ADR-0001).
- Since ADR-0022 was built, storage runs on the machine's vØrk node; no Reticulum code is in the build.

## Addendum (2026-11-23): the format takes vØrk's names
The format's names were renamed after all (commit a5702a8): domain-separation labels are `vork-storage/v1/…` (apart from the network's `vork/v0/…`), links `vork:` (`vork:ns/…`, `vork:invite/…`, file references `vork:1:…`), channel addresses `vork-ch:1:…`, record types and destination names `vork.*` (`vork.node`, `vork.library`, `vork.keyring`…), attributes `vork:node` and `vork:writer`, extended attributes `user.vork.*`. The test vectors were generated again.
- This is a new format: a retifs library doesn't open under vØrk storage. `vork-import-retifs` copies libraries from a running retifs daemon (read only) into vØrk storage, encrypting their content again with vØrk storage's keys; `--export` writes a library out as plain files.
- The programs: `vorkd` (the daemon), `vork-storage` (its commands; `vork <storage command>` still passes them on), `vork-web` (its pages, in the vØrk dashboard with `--dashboard`, ADR-0032), `vork-cloudfiles` (Windows Explorer), `vork-storage-run` (keeps them running from login).
- The store has its own folder, apart from the node's: `%LOCALAPPDATA%\vork-storage` (Windows), `~/.local/share/vork-storage` (Linux, or `$XDG_DATA_HOME/vork-storage`), `~/config/var/vork-storage` (Haiku).
- Storage is installed apart from vØrk, by an installer of its own per platform, and needs vØrk installed first ([install](../install.md#installing-vørk-storage)).

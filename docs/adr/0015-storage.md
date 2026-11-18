# ADR-0015: Storage (formerly retifs) is a vØrk application

- **Status:** accepted
- **Date:** 2026-11-18
- **Irreversible:** no

## Context
retifs (encrypted, content-addressed, replicated storage) was built on Reticulum through its `Transport` trait, with RNS identities (X25519 ‖ Ed25519, hash = SHA-256[0..16]) and RNS token encryption. Its blind storage nodes and vØrk's blind relays are the same idea, and both name endpoints by an identity hash. retifs has one user today, so its code and formats can change.

## Decision
- retifs becomes vØrk's storage layer and is an ordinary vØrk application (ADR-0017): a storage service has **its own vØrk identity**, is announced as its own destination and is reached through the native message API. A node may host it; the storage identity is not the node identity.
- **No Reticulum code.** The Prns dependency and the Reticulum transport are removed (ADR-0022).
- **The storage format stays** (amended 2026-11-18), so existing libraries move under vØrk unchanged: chunks, manifests, records, leases, channels, recovery kits, key wrapping, links and addresses keep their bytes. Names inside the format (`retifs/v1/…` labels, `retifs.*` record types, `retifs:` links, `retifs.node` destinations) are format constants, not branding, and keep their spelling. vØrk took over the storage identity instead: the same address hash, key file and named destinations (ADR-0004, ADR-0022).
- retifs's `Transport` trait stays as the seam; its one implementation is vØrk (`connect` opens a session to the storage identity, `link_nonce` comes from fresh randomness of both ends at session start, `request` is request/response in the session). Large transfers ride a reliable, congestion-controlled stream inside the session.
- Placement uses measured round trip and direct-vs-relayed instead of hop count.
- **One repository.** retifs's files were copied into this repository (2026-11-18, from retifs commit 10319c8) and renamed for vØrk; the retifs repository stays as it was. Crates: `vork-core`, `vork-store`, `vork-storage-net`, `vork-storage-node` (`vorkd`, the storage commands), `libvork` (local API client), `vork-tree`, `vork-fuse` (`vork-mount`), `vork-cloudfiles`, `vork-gateway`, `vork-extract`, `vork-haikufs`, `vork-card`. `vork <storage command>` runs `vork-storage`. The format's own names were not renamed; the test vectors are byte for byte retifs's. Documents are in `docs/storage/` with their own ADR numbering.
- vØrk must start and work without storage; the dependency points one way. Storage may later hold vØrk's own state (islands, petnames, remembered peers).

## Consequences
- Every node can contribute two blind resources: relaying and storage.
- One identity system: recovery kits, cards and leases in storage describe vØrk identities; YubiKey-held identities (ADR-0013) cover storage too.
- Existing libraries and identities carry over: a storage identity file is a vØrk key file, and every stored node or library address is a vØrk destination.
- No LoRa reach through storage any more; vØrk is IP-only (ADR-0010).
- Since ADR-0022 was built, storage runs on the machine's vØrk node; no Reticulum code is in the build.

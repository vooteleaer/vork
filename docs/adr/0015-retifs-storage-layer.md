# ADR-0015: retifs is a vØrk application: the storage layer

- **Status:** accepted
- **Date:** 2026-11-18
- **Irreversible:** no

## Context
retifs (encrypted, content-addressed, replicated storage) was built on Reticulum through its `Transport` trait, with RNS identities (X25519 ‖ Ed25519, hash = SHA-256[0..16]) and RNS token encryption. Its blind storage nodes and vØrk's blind relays are the same idea, and both name endpoints by an identity hash. retifs has one user today, so its code and formats can change.

## Decision
- retifs becomes vØrk's storage layer and is an ordinary vØrk application (ADR-0017): a storage service has **its own vØrk identity**, is announced as its own destination and is reached through the native message API. A node may host it; the storage identity is not the node identity.
- **No Reticulum ties.** retifs adopts vØrk's identity format, address derivation and key files; the Prns dependency, the Reticulum transport and RNS-specific formats (identity hash, token encryption, destination naming) are removed and replaced by vØrk's.
- retifs's `Transport` trait stays as the seam; its one implementation is vØrk (`connect` opens a session to the storage identity, `link_nonce` comes from fresh randomness of both ends at session start, `request` is request/response in the session). Large transfers ride a reliable, congestion-controlled stream inside the session.
- Placement uses measured round trip and direct-vs-relayed instead of hop count.
- vØrk must start and work without retifs; the dependency points one way. retifs may later hold vØrk's own state (islands, petnames, remembered peers).

## Consequences
- Every node can contribute two blind resources: relaying and storage.
- One identity system: recovery kits, cards and leases in retifs describe vØrk identities; YubiKey-held identities (ADR-0013) cover storage too.
- retifs code changes: identity hash and public-key order, key wrapping (move from the RNS token to X25519 + HKDF + an AEAD), cards, recovery, leases, `retifs-net` and `retifs-node`. Data stored under RNS identities is migrated or recreated.
- No LoRa reach through retifs any more; vØrk is IP-only (ADR-0010).
- Open: one repository or two (open question 18), and when the work starts relative to M2/M3.

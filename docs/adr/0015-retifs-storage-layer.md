# ADR-0015: retifs as the storage layer

- **Status:** proposed
- **Date:** 2026-11-17
- **Irreversible:** no

## Context
retifs (encrypted, content-addressed, replicated storage) was built on Reticulum through its `Transport` trait. Its blind storage nodes and vØrk's blind relays are the same idea.

## Decision
vØrk is the network layer and retifs the storage layer on top. vØrk implements retifs's `Transport` (`connect`, `link_nonce`, `request`) and the server side; placement uses measured round trip and direct-vs-relayed. vØrk must start and work without retifs; retifs may later hold vØrk's own state (islands, petnames, remembered peers).

## Consequences
- Every node can contribute two blind resources: relaying and storage.
- retifs gets IP-speed links; LoRa reach stays with Reticulum via its other transport.

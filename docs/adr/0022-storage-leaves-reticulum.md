# ADR-0022: Storage leaves Reticulum: destinations, sessions, requests

- **Status:** proposed
- **Date:** 2026-11-18
- **Irreversible:** no (the storage format is untouched; this changes how storage reaches other nodes)

## Context
Storage (ADR-0015) reaches other nodes through Prns: announces with app data, path requests, links to destinations, link identification, request/response with resources up to 32 MB, and a shared instance that the daemon attaches to. Its stored data names nodes, libraries and keyrings by RNS destination hashes (`retifs.node`, `retifs.library`, `retifs.keyring` of an identity). The storage format must stay so existing libraries can move over (ADR-0015).

## Decision
vØrk gets what storage needs from Reticulum, with the same addresses, and storage drops Prns.
1. **Identities** are storage identities (ADR-0004): address = `SHA-256(x_pub ‖ ed_pub)[0..16]`, key file = X25519 secret ‖ Ed25519 seed. Done.
2. **Destinations.** A destination is a named service of an identity: `SHA-256(SHA-256(name)[0..10] ‖ identity address)[0..16]`. A node announces each local destination, signed by its identity, with the destination's app data (up to a few hundred bytes). Announces, path requests and paths work on destination hashes; a node's own address is its unnamed destination. Storage keeps its names, so every stored address is still right.
3. **Sessions to a destination.** The destination's identity answers (its X25519 key is the inner WireGuard key). The opening side is anonymous by default (a fresh key per session) and can **identify**: a signature over the session nonce by its identity, as RNS links do. The session nonce is known to both ends only, and is what storage binds proofs to (`link_nonce`).
4. **Requests.** `request(session, path, body) → response`, bodies up to 32 MiB, cut into packets and sent reliably with flow control over the session's paths. A destination registers the paths it serves.
5. **Applications attach to the machine's node** through the local API (ADR-0020), not by running their own: they register destinations (for now handing the node their identity secrets; later they can stay on a YubiKey), announce, open sessions, send and serve requests. `vorkd` is the first such application and storage's `Transport` is implemented over this API.
6. **Dropped:** the shared-instance and remote-control use of Prns, and NomadNet serving (a Reticulum application). The LXMF stamp construction stays, since lease stamps are part of the format.

## Consequences
- The local API (part of M3) and destinations come before IPv6 sessions and TUN (M2): storage is the first real application.
- Storage's placement can use measured round trip and direct-vs-relayed instead of hop count.
- vØrk announces carry app data, so they grow; announce rate limits (M4) matter sooner.
- Reticulum users lose nothing they relied on from vØrk storage except NomadNet pages and LoRa reach.

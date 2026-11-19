# vØrk documentation

| Document | What |
|---|---|
| [technical-overview](technical-overview.md) | how the prototype works, what was tested, throughput |
| [00-vision](00-vision.md) | what vØrk is for, principles, non-goals |
| [01-architecture](01-architecture.md) | how the parts fit: layers, paths, NAT traversal, node profiles, privacy classes, applications |
| [02-protocol](02-protocol.md) | wire format v0, byte by byte, as implemented |
| [03-threat-model](03-threat-model.md) | the hostile-node invariant, attacks and answers |
| [04-roadmap](04-roadmap.md) | milestones with scope, approach and acceptance tests |
| [05-open-questions](05-open-questions.md) | decisions still to make |
| [06-local-api](06-local-api.md) | how applications use the machine's node: frames, endpoint |
| [storage/](storage/README.md) | vØrk storage (experimental): its own documents and ADRs |

## Decisions (ADRs)

New decisions copy [adr/0000-template.md](adr/0000-template.md). *Accepted* means implemented or decided; *proposed* means agreed in direction, not built.

| ADR | Decision | Status |
|---|---|---|
| [0001](adr/0001-ip-only-own-layer.md) | IP only, with our own network layer (not on top of Reticulum) | accepted |
| [0002](adr/0002-no-roles.md) | Every node runs the same code; no roles | accepted |
| [0003](adr/0003-hostile-nodes-proof-not-trust.md) | Every node may be hostile; proof instead of trust | accepted |
| [0004](adr/0004-address-is-key-hash.md) | An address is the hash of keys; IP only between neighbors | accepted |
| [0005](adr/0005-wireguard-both-layers.md) | WireGuard for both layers | accepted |
| [0006](adr/0006-links-proven-both-ways.md) | A link counts only when proven to work both ways | accepted |
| [0007](adr/0007-layer-as-udp-in-tunnel.md) | The layer travels as UDP/IPv4 inside the tunnel | accepted |
| [0008](adr/0008-path-labels.md) | Paths with per-hop labels; no source address | accepted |
| [0009](adr/0009-several-paths.md) | Several paths per session | accepted |
| [0010](adr/0010-lite-nodes.md) | Lite nodes | accepted |
| [0011](adr/0011-direct-links-opt-in.md) | Direct links are opt-in | accepted |
| [0012](adr/0012-port-mapping-opt-in.md) | Router port mapping is opt-in | accepted |
| [0013](adr/0013-identities-on-hardware-keys.md) | Identities on hardware keys; devices are delegated | proposed |
| [0014](adr/0014-islands.md) | Islands: closed groups inside an open network | proposed |
| [0015](adr/0015-storage.md) | Storage (formerly retifs) is a vØrk application | accepted |
| [0016](adr/0016-privacy-classes.md) | Privacy classes; nested WireGuard for the strong class | proposed |
| [0017](adr/0017-applications.md) | Applications: IPv6 inside sessions, several ways in | proposed |
| [0018](adr/0018-dns-vork.md) | Names: the .vork DNS domain | accepted |
| [0019](adr/0019-open-relaying-owner-first.md) | Open relaying, the owner's traffic first | accepted |
| [0020](adr/0020-shared-node.md) | One shared node per machine | proposed |
| [0021](adr/0021-compatibility-addresses.md) | Compatibility addresses: IPv6 per identity, local IPv4 aliases | proposed |
| [0022](adr/0022-storage-leaves-reticulum.md) | Storage leaves Reticulum: destinations, sessions, requests | accepted |
| [0023](adr/0023-reachability-learned.md) | Reachability is learned from others, never claimed | accepted |
| [0024](adr/0024-linking-devices.md) | Devices are linked with both sides agreeing | accepted |

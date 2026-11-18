# vØrk: threat model

## The invariant

> **A hostile node can only delay, drop and observe metadata. It can't forge, read or impersonate.**

Any node may be hostile: neighbors, relays, the far end of a path that isn't the peer you meant. Trusted nodes (your own, your island's) make paths faster or more private; correctness never depends on them.

## Adversaries

| Adversary | Can | In scope |
|---|---|---|
| Hostile neighbor or relay | drop, delay, reorder, replay, observe metadata of what passes through it, lie in what it says | yes |
| Several colluding relays | correlate what passes through all of them | partly: labels prevent matching by content; timing correlation is not prevented |
| Sybil (many fake nodes) | flood announces, attract paths, surround a node | yes, with costs and diversity (planned) |
| Local network observer (ISP, Wi-Fi) | see your traffic to your neighbors | yes: everything is inside the outer WireGuard |
| Global passive observer | see all traffic everywhere | no (non-goal; see the anonymity trilemma) |
| Stolen device | use its keys | yes, with short-lived delegations (planned, ADR-0013) |

## Attacks and answers

| Attack | Answer | Status |
|---|---|---|
| Forge an announce for someone else's address | the address is the hash of the signing key; the announce is signed | implemented |
| Replay an old announce | a newer sequence number wins | implemented (sequence from the clock; to become a stored counter) |
| Tamper with a packet | WireGuard rejects it (both layers) | implemented |
| Read relayed traffic | the relay holds no keys for the inner session | implemented |
| Learn who is talking (source address) | no packet carries a source; labels change per link | implemented |
| Learn the destination of every packet | only `PATH_OPEN` carries it | implemented |
| One-way link used for routing | links count only with a fresh handshake and recent traffic | implemented |
| Sinkhole: advertise a short path, then drop | paths are probed; a path that stops answering is dropped in ~3 s and its first hop rests; other paths keep the session | implemented |
| Advertised hop count lies | it only orders first hops; measurement decides afterwards | partly; scoring by delivery planned |
| Flood `PATH_OPEN` to fill relay tables | per-neighbor limits, table caps | **planned (M4)** |
| Flood announces / path requests | rate limits per neighbor, proof-of-work stamps | **planned (M4)** |
| Lie in `OBSERVED` (wrong outside address) | only a candidate; take it only when several neighbors agree | **planned (M4)** |
| Eclipse: surround a new node | several bootstrap sources, diverse neighbors (AS, /16), long-lived anchors (own and island nodes) | **planned (M4)** |
| Sybil attracting paths | diversity rules in path choice, PoW cost on identities, island trust for strong paths | planned |
| Handshake flood | WireGuard's cookie mechanism (boringtun rate limiter) | implemented by boringtun |
| Traffic analysis by timing | multipath, optional cover traffic for the strong class | partly |
| Two colluding relays on one path | nested WireGuard for the strong class (no single relay knows both ends) | planned (ADR-0016) |
| Malicious peer harms a Lite node | Lite nodes never relay and accept only paths that end at them | implemented |

## Privacy notes

- **Your IP address** is seen by your direct neighbors, and by a peer you open a direct link with (`--direct`, opt-in). Through relays, a peer sees only the relay.
- **One IP links identities.** Separate identities per island don't help an observer who sees them all at one address; only different relays do.
- **Announces are public** (an address and its keys) but carry no location. Island announces are to be encrypted to the island (ADR-0014).
- **Neighbor lists aren't gossiped**, so the social graph isn't published.
- **An anonymous initiator** (one that doesn't announce) reveals only its WireGuard key to the peer it contacts. For anonymity toward that peer too, it can use a one-off key (planned).

## What a relay sees

| Visible | Not visible |
|---|---|
| its two neighbors on the path, and their IP addresses | the source or destination of `DATA` |
| the destination address in `PATH_OPEN` | content |
| packet sizes, counts and timing | which session a packet belongs to beyond its own two labels |

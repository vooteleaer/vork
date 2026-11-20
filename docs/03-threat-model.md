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
| Fill the neighbor table with fresh identities (identities are free) | at most 512 neighbors; a new anonymous one takes the place of the longest-silent one that dialed in, or is refused; 16 new anonymous neighbors per IP per minute; ones that dialed in and went silent are forgotten after 10 min | implemented |
| Open sessions until memory runs out | at most 4096 sessions; idle ones (15 min, nothing in flight or waiting) close | implemented |
| Use the key query as a reflector (spoofed source) | asks are padded to the answer's size; answers rationed (20 per IP per 10 s, 100 per s in all); old unpadded asks still answered within that ration | implemented (drop unpadded asks once old nodes are gone) |
| Steer a node's dials (lie in `REACH`) to scan or flood an address | dials only to fill a gap (fewer than 2 relaying neighbors), one at a time; each address at most once per 10 min, each key likewise; no port 0, multicast, broadcast or own address | implemented |
| Flood the owner with link requests | at most 16 waiting without a code; more refused | implemented |
| Junk from the network or from a neighbor crashes the node | memory-safe code; every parser reachable from a neighbor fuzzed (300 000 inputs in the tests); a node took 4 M junk packets in 20 s | tested; continuous fuzzing planned |
| Lie in `OBSERVED` (wrong outside address) | only a candidate; take it only when several neighbors agree | **planned (M4)** |
| Eclipse: surround a new node | several bootstrap sources, diverse neighbors (AS, /16), long-lived anchors (own and island nodes) | **planned (M4)** |
| Sybil attracting paths | diversity rules in path choice, PoW cost on identities, island trust for strong paths | planned |
| Strangers' traffic crowds out the owner's | relaying gets only leftover capacity and backs off on the owner's delay (ADR-0019) | **planned (M4)** |
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

## Services, names and the local surface

| Attack | Answer | Status |
|---|---|---|
| Reach a node's services (SSH, dashboards, Docker) from any node | the walled garden (ADR-0027): only the owner's devices, other nodes only what `allow` opens; relaying untouched | implemented |
| Grind a key to match an address prefix (2³² tries for 8 digits) | `allow` rules name nodes by full address; `<prefix>.vork` resolves only among the owner's devices | implemented |
| Take a name (`home-server.vork`) by announcing it | names resolve only for the owner's devices | implemented |
| Use someone's exit node | only the owner's devices (ADR-0026) | implemented |
| An exit client reaches the exit's LAN | the internet only by default (private, shared, link-local, loopback, multicast dropped); `exit_lan = true` to allow | implemented |
| Sniff the web dashboard's key on the LAN | HTTPS: a certificate the node makes and keeps (trust on first use, its SHA-256 shown by `vork web-link` to compare), or one given (`web_cert`, `web_key`: Let's Encrypt's, say); plain HTTP is redirected | implemented (a reload when a given certificate is renewed is to come: restart the node meanwhile) |
| Reach the dashboard behind Home Assistant's ingress without signing in | that listener (`web_ingress`, plain HTTP, no key) takes connections from one address only, the Supervisor's ingress proxy (`172.30.32.2`, `web_ingress_from`), which lets in only Home Assistant's signed-in users; anything else is dropped. It is off unless set (the add-on sets it) | implemented |
| Another user of a Windows PC changes the node | the control pipe lets interactive users in with full rights (a personal PC's trade-off) | accepted; an operator setting planned |
| A compromised device | a full member until unlinked, and since devices are one group (ADR-0030) it can link any node to all of the owner's devices; unlinking it on any device takes it off all of them | open: short-lived delegations (ADR-0013); a group change asking the owner on another device |
| A node claims to be in someone's group | only the owner's own devices are heard about the group, and what they say about themselves and the receiver is ignored; future times held back to 5 minutes | implemented |
| Learning which relays someone's devices use | each device tells how it reaches the others (directly, or through which neighbor and how many hops) for the dashboard's map, but only to the owner's devices, in a session where the asker identified (`/vork/paths`) | implemented |
| Tampered binaries, a malicious update | releases signed with the owner's release key (ADR-0028); a node takes only a newer build signed by a key in `update_keys`, with the signed hash, from the owner's own nodes; the old binary kept | implemented (a project key for others' nodes to come) |
| The release key stolen | whoever holds it runs code on every node that trusts it | keep it off servers; rotation by changing `update_keys` |

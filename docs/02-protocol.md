# vØrk: protocol (wire format v0)

> **Status: draft, as implemented in the prototype.** v0 changes without notice. It is frozen as v1 only with test vectors and review (milestone M10). Where this text and the code disagree, the code is right until the text is fixed.

Byte order: our own fields are **big-endian**. WireGuard's fields keep WireGuard's own encoding (little-endian indices and type).

## 1. Identity

- A node has an **Ed25519** signing key and an **X25519** key. The X25519 key is its WireGuard static key for both layers.
- **Address** = `SHA-256(ed_pub ‖ x_pub)[0..16]` (16 bytes; shown as 32 hex digits).
- Every signature covers `label ‖ message`, with a domain-separation label:

| Label | Signs |
|---|---|
| `vork/v0/bind` | `x_pub ‖ flags` in `HELLO` |
| `vork/v0/announce` | `address ‖ x_pub ‖ seq ‖ flags` in `ANNOUNCE` |

- **Overlay IPv6 address** (compatibility view): `fd` followed by the first 15 bytes of the address.
- **Flags:** bit 0 `RELAY` (set: full node; clear: Lite node). Other bits are zero.

## 2. Links (outer layer)

- WireGuard (Noise IK, as in the WireGuard paper), one session per neighbor, over UDP. Implementation: boringtun.
- A node accepts a handshake from **any** key (the network is open); being a neighbor grants only the right to exchange the messages below.
- Persistent keepalive: **25 s**.
- A neighbor is **up** while both hold: the last handshake is younger than **180 s**, and the last authenticated packet is younger than **35 s**. Announces, path requests and paths are accepted only from up neighbors.
- **Roaming:** a neighbor's endpoint is the source of its last authenticated packet.

### Framing inside the tunnel

WireGuard carries IP packets, so every message is the payload of an **IPv4/UDP datagram**:

| Field | Value |
|---|---|
| IPv4 | version 4, IHL 5, DF, TTL 64, protocol 17, valid header checksum |
| addresses | sender uses `10.89.0.1` → `10.89.0.2`; receivers ignore them |
| UDP | source and destination port **4799**; checksum 0 (allowed over IPv4; the tunnel authenticates every byte) |

The addresses are a per-link convention so that a platform WireGuard with fixed allowed IPs (ESPHome: `address: 10.89.0.2`, `peer_allowed_ips: [10.89.0.1/32]`) can carry the layer. They mean nothing to vØrk.

## 3. Messages

The UDP payload starts with a type byte.

| Type | Name | Layout after the type byte |
|---|---|---|
| 1 | `HELLO` | `ed_pub[32]` `flags[1]` `sig[64]` |
| 2 | `OBSERVED` | `family[1]` (4 or 6) `ip[4 or 16]` `port[2]` |
| 3 | `ANNOUNCE` | `ed_pub[32]` `x_pub[32]` `seq[8]` `flags[1]` `sig[64]` `hops[1]` |
| 4 | — | retired (`FWD` before path labels) |
| 5 | `PATH_REQ` | `dest[16]` `nonce[8]` |
| 6 | `PATH_OPEN` | `label[8]` `dest[16]` `ttl[1]` |
| 7 | `PATH_ACK` | `label[8]` |
| 8 | `PATH_ERR` | `label[8]` |
| 9 | `DATA` | `label[8]` `inner[…]` (to the end) |
| 10 | `PROBE` | `label[8]` `nonce[8]` `reply[1]` |

Unknown types are dropped.

### HELLO

Sent when a link comes up. Binds the neighbor's WireGuard key (known from the handshake) to its signing key and flags. The receiver checks the signature over `vork/v0/bind ‖ x_pub ‖ flags`, then knows the neighbor's address and whether it relays.

### OBSERVED

"I see you at this address." Sent when a link comes up. The receiver may use it as a candidate for direct links. It is a claim, not a proof.

### ANNOUNCE

- The address is computed from `ed_pub ‖ x_pub`; the signature is checked over `vork/v0/announce ‖ address ‖ x_pub ‖ seq ‖ flags`.
- `seq` must increase for each identity (v0: milliseconds since the Unix epoch; v1: any monotonic counter, see open questions).
- `hops` is not signed. A forwarder sends it on as `hops + 1`. Announces with `hops ≥ 16` are dropped.
- A node sends its own announce every **30 s** and when a link comes up.
- **Routes:** a node keeps, per destination, one route per neighbor it heard the announce from, replaced when that neighbor brings a higher `seq` (or the same `seq` with fewer hops).
- **Flooding:** a full node forwards the first copy of each `(address, seq)` to all up neighbors except the sender and except Lite neighbors. Lite nodes don't forward.
- When a full neighbor's `HELLO` arrives for the first time, a full node sends it its best announce for every destination it knows.

### PATH_REQ

"Where is `dest`?" Answered by the destination with its own announce, or by any node knowing a route with that announce. Otherwise a full node remembers who asked (30 s) and passes the request on to its full neighbors. Deduplicated by `(dest, nonce)`. A node asks at most once per 2 s per destination.

### PATH_OPEN, PATH_ACK, PATH_ERR

At a node receiving `PATH_OPEN(label L, dest D, ttl T)` from neighbor N:

1. If D is this node: remember the path end `(N, L)` and answer `PATH_ACK(L)` to N.
2. Else, if this is a Lite node, or `T = 0`, or no first hop toward D other than N exists: answer `PATH_ERR(L)` to N.
3. Else pick the best first hop V toward D (≠ N), draw a random label L′, store `(N, L) → (V, L′)` and `(V, L′) → (N, L)`, and send `PATH_OPEN(L′, D, T − 1)` to V.

`PATH_ACK` and `PATH_ERR` are switched back along the stored pairs. A relay that passes on a `PATH_ERR` removes both entries. The origin marks a path confirmed on `PATH_ACK` and drops it on `PATH_ERR`.

When a neighbor goes down, a relay sends `PATH_ERR` toward the other side of every entry through it and removes them.

The origin opens paths with `ttl = 32`. Labels are 64-bit random numbers, meaningful on one link only.

### DATA

Switched by `(neighbor, label)` at each relay. At a path end, `inner` is an inner WireGuard message (§4). A node receiving `DATA` with an unknown label answers `PATH_ERR(label)`.

### PROBE

The origin sends `PROBE(label, nonce, reply = 0)` on each of its paths every **1 s**; relays switch it like `DATA`; the far end answers `PROBE(label, nonce, reply = 1)`. A path that misses **3** probes in a row is dead.

Relay entries and far-end path state expire after **60 s** without traffic; probes keep them alive.

## 4. Sessions (inner layer)

- WireGuard between the two endpoints, carried in `DATA`. The initiator knows the responder's X25519 key from its announce; the responder learns the initiator's key from the handshake (the initiator need not announce).
- An endpoint finds the session of an incoming message by WireGuard's receiver index (`index >> 8` is the local session number in boringtun); a handshake initiation creates a session for its sender's key.
- The far end of a path learns which session the path belongs to from the first message on it.
- **Paths per session:** the origin keeps up to N paths through different first hops (default 2), plus one through a direct link when there is one. A dropped path's first hop rests 5 s, doubling up to 60 s, before it is tried again.
- **Which path a message takes:** only confirmed paths that answer probes (the origin) or that were heard from within 3 s (the far end); if none qualify, any. A healthy direct path is used alone. Otherwise *spray* (smooth weighted round robin, weight ∝ 1/round trip) or *dup* (all paths).

### Payload (v0, prototype)

The session carries IPv4/UDP datagrams to port 4799 framed as in §2, with an application byte first:

| Byte | Meaning |
|---|---|
| 0 | nothing (starts a handshake) |
| 1 | ping: `nonce[8]` |
| 2 | pong: `nonce[8]` |
| 3 | direct-link candidates: `count[1]` then addresses as in `OBSERVED` |
| 4 | bench: `seq[8]` then padding |

This is a stand-in. From milestone M2 the session carries **IPv6 packets between overlay addresses** (ADR-0017), and these functions move into a control service.

## 5. Direct links

On the first pong (or on receiving candidates), a node with `--direct` sends its candidates (addresses its neighbors observed, its router mapping, its listening address if specific) inside the session. Each side, on receiving the other's candidates, sends a WireGuard handshake initiation to all of them at once. When the link comes up, the two are neighbors; the origin opens a path over it (`PATH_OPEN` to the neighbor, which is the destination) and uses it alone.

## 6. Constants

| Name | Value |
|---|---|
| Address length | 16 bytes |
| Layer port | UDP 4799 (inside the tunnel) |
| Keepalive | 25 s |
| Link fresh / silent | 180 s / 35 s |
| Announce interval | 30 s |
| Max announce hops | 16 |
| Path TTL | 32 |
| Probe interval / dead after | 1 s / 3 missed |
| Far-end path recent | 3 s |
| Path idle expiry | 60 s |
| First-hop retry | 5 s doubling to 60 s |
| Path request interval | 2 s per destination |
| Path request memory | 30 s |
| Socket buffers | 4 MB (forced as root on Linux) |

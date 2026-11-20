# vØrk: architecture

This describes the design as a whole. What the prototype implements today is marked; the exact bytes are in [02-protocol.md](02-protocol.md).

## A node

One process, one UDP socket, the same code everywhere. Its state:

- **Identity** (implemented): an Ed25519 signing key and an X25519 key (the WireGuard static key). Address = `SHA-256(x_pub ‖ ed_pub)[:16]`; a named service of an identity has the address `SHA-256(SHA-256(name)[:10] ‖ address)[:16]` (a destination).
- **Neighbors** (implemented): an outer WireGuard session (boringtun, userspace) with each directly connected node.
- **Routes** (implemented): for each destination, one route per neighbor that announced it.
- **Relay table** (implemented): `(neighbor, label) → (neighbor, label)` for paths passing through.
- **Path ends** (implemented): paths this node opened or terminates.
- **Sessions** (implemented): an inner WireGuard session per remote peer, end to end.

## Two layers of WireGuard

```
[ outer WireGuard: neighbor <-> neighbor ]        proves the link works both ways; hides everything on the wire
  [ IPv4/UDP to port 4799 ]                       any WireGuard implementation can carry it (ESPHome too)
    [ HELLO | OBSERVED | ANNOUNCE | PATH_* | DATA | PROBE ]
      DATA: [ label ][ inner WireGuard: end <-> end ]   relays hold no keys for it
```

- The **outer** layer exists per link. A neighbor counts as *up* only while its handshake is fresh and it isn't silent, so every link the node uses has been proven to work in both directions. This is the fix for Reticulum's asymmetric-link problem: a path is never learned over a link that only works one way.
- The **inner** layer exists per pair of endpoints and is end-to-end. Relays only see a label and ciphertext.

## Finding destinations

- A node **announces** itself: its keys, a sequence number, flags (`relay` or Lite), signed. Announces carry **no IP address**.
- Announces are flooded over up links, deduplicated by `(address, seq)`. A node keeps a route per neighbor it heard the announce from, so it knows several first hops.
- **Lite nodes** receive no floods; they ask with `PATH_REQ` and get the announce back.
- The hop count in an announce is added by forwarders and isn't signed, so it is a hint only. Once a path exists, it is judged by measurement.

## Paths

```
C ──PATH_OPEN(label 7, dest B)──▶ A ──PATH_OPEN(label 91, dest B)──▶ B
C ◀────────PATH_ACK(7)────────── A ◀─────────PATH_ACK(91)────────── B
C ──DATA(7, …)──▶ A ──DATA(91, …)──▶ B
```

- A path is opened by destination address. Each hop picks a **fresh random label for the next link** and stores the pair in both directions.
- After that, `DATA` carries only a label. **No packet carries a source. Only the opening message carries a destination.** A label means something on one link only, so colluding relays can't match packets by label (timing remains).
- An endpoint finds the inner session of a `DATA` packet by the WireGuard receiver index inside it; the far end of a path learns which session it belongs to from the first packet. That also lets the far end answer and rekey toward an initiator that never announced itself.
- `PATH_ERR` tears a broken path down hop by hop.

## Several paths per session

- The origin opens up to N paths (`--paths`, default 2) through **different first hops**, plus one over a direct link when there is one.
- It probes each path every second (round trip, loss). A path that misses 3 probes is dropped and its first hop rests (5 s, doubling to 60 s).
- Packets go out by **spray** (one path each, weighted by measured round trip) or **dup** (every packet on every path).
- A healthy direct link is used alone, to spare the relays.
- The far end answers on the paths it heard from in the last 3 s.
- Planned with the TUN interface: **pin each flow** (TCP connection) to a path, because spraying one TCP connection over paths of different latency reorders it.

## NAT traversal

- **Observed addresses** (implemented): every neighbor tells a node the address it sees it at, so every node is a STUN server for its neighbors.
- **Router port mapping** (implemented, opt-in): UPnP, then PCP/NAT-PMP, like torrent clients. Gives up when the router is itself behind CGNAT.
- **Direct links** (implemented, opt-in): once a session works through relays, the two ends exchange their candidate addresses inside the session and both send WireGuard handshakes to each other at the same time (hole punching). On success, the session moves to the direct link.
- **Relays** for everything else: every full node relays for everyone, with its owner's traffic first (ADR-0019). Symmetric NAT on both ends never gets a direct link; the session stays relayed.

## Node profiles

| | Full node | Lite node |
|---|---|---|
| Relays for others | yes | never |
| Receives announce floods | yes | no; asks with `PATH_REQ` |
| Others route through it | yes | only when it is the destination |
| Outer WireGuard | userspace, many neighbors, accepts unknown peers | the platform's own (ESPHome: one peer, IPv4 only) |
| Typical | PC, server, Raspberry Pi | ESP32, battery devices, phones |

A Lite node isn't a different kind of node to the network; it announces that it doesn't relay.

## Privacy classes

| Class | Path | Who learns that A talks to C |
|---|---|---|
| Island (direct) | direct link | only A and C |
| Normal (default) | 1–2 relays, preferably your own or your island's | the relays, partly each |
| Strong (planned) | nested WireGuard over 2–3 relays; the receiver picks its inbound part | no single node |

The default rule: your IP address is never shown to anyone outside your islands unless you allow it (`--direct`).

## Above the network (planned)

- **Applications** (ADR-0017): sessions carry IPv6 packets between overlay addresses (`fd` + the first 15 bytes of an address). Applications attach through a TUN interface (every existing program), a local daemon API (status, peers, `whois`), a native message API (`connect(identity, "service")`), or an embedded userspace stack (no admin rights; lwIP on an ESP32). The overlay address is a compatibility view; the network never routes by it.
- **Compatibility addresses** (ADR-0021): every identity has its IPv6 overlay address on the machine's TUN interface; IPv4-only programs get local aliases handed out by DNS and translated at the node. Sessions always carry IPv6.
- **Shared node** (ADR-0020): one node per machine; the first program starts it and later ones attach over local IPC, each registering its own identities and services, like Reticulum's shared instance but with a specified IPC protocol.
- **Names** (ADR-0018): DNS for `.vork`: `<address>.vork` and petnames like `nas.vork` resolve to overlay addresses; only `.vork` queries go to the node.
- **Identity firewall:** rules say which identity may use which service, using `whois`; IP rules have nothing to hold on to.
- **Identities on hardware keys:** a person's identity on a YubiKey signs short-lived session keys for devices; servers have their own identities, delegated by their owner. See ADR-0013.
- **Islands:** closed groups (membership by signed invitation) inside the open network. See ADR-0014.

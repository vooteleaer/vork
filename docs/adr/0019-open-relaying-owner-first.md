# ADR-0019: Open relaying, the owner's traffic first

- **Status:** accepted (decided; not implemented yet, milestone M4)
- **Date:** 2026-11-18
- **Irreversible:** no

## Context
An open network needs nodes that relay for strangers, or it only works between people who already know each other. But nobody keeps running a node that slows down their own connection. The question was whether a node should relay for everyone by default, or only for its own and its islands' identities.

## Decision
**Every full node relays for everyone by default, and its owner's traffic always comes first.** Relayed traffic gets only the capacity the owner isn't using, like BitTorrent's LEDBAT/uTP background traffic.

- **Two classes in each node.** *Own*: paths that start or end at this node (its sessions, its applications). *Relayed*: everything it switches for others. Own traffic is sent first; relayed traffic waits.
- **Stay below the uplink.** Priority inside the node doesn't help once a queue builds up in the router or modem. So relayed traffic is rate-limited to the uplink capacity minus what the owner is using, minus headroom. The capacity is measured (and can be set by hand).
- **Back off on delay.** The node watches its own paths' round trip (probes, ADR-0009). When it rises above its baseline, the relayed rate drops at once and recovers slowly; it is the first traffic to give way.
- **Drop, don't queue.** Relayed packets over the budget are dropped. The endpoints' congestion control (TCP inside the sessions) slows down, and multipath (ADR-0009) moves their traffic to paths with room.
- **Fair among strangers.** Without source addresses, the relayed budget is shared per path label and per neighbor, so one stranger can't take all of it.
- **CPU too.** The same rule applies when the node is CPU-bound: relaying yields to the owner's work.
- An **island class** between the two (members before strangers) can be added with islands (ADR-0014).

## Consequences
- The network has relays from the start, and running a node costs its owner nothing they notice.
- A relay's capacity changes all the time; paths through busy owners get slow, and probing moves traffic elsewhere.
- Relaying needs a capacity estimate and delay measurement in every node (M4).
- A Lite node (ADR-0010) still never relays.

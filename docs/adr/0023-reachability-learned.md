# ADR-0023: Reachability is learned from others, never claimed

- **Status:** accepted (first part built 2026-11-18)
- **Date:** 2026-11-18
- **Irreversible:** no (a new message type; nodes that don't know it drop it)

## Context
A new node knows nobody: it needs one address to start (bootstrap), and the user has to type it. Nodes that can be reached directly from outside are the ones that make good first contacts and relays, but there are no roles (ADR-0002) and any node may lie (ADR-0003), so a node saying "I'm reachable" is worth nothing. What is worth something is another node's experience: it dialed this one and got an answer.

## Decision
1. **Proof by a cold dial.** When a node sends a handshake to a neighbor from which nothing has come for 150 s (no NAT mapping of the neighbor's can still be letting us in) and the answer comes from that address within 10 s, the node has proven the neighbor reachable at that address, from here. A link the neighbor opened proves nothing.
2. **Reports.** On link up and every 10 minutes, a node tells each neighbor what it proved (`REACH`: key, address, age), at most 32 entries, newest first, only of nodes that relay. Public addresses go to anyone; a private (RFC 1918, ULA) or shared (100.64/10: carrier NAT, Tailscale) address only to a neighbor reached at the same kind of address; loopback and link-local never.
3. **What a report counts for.** About the sender itself: ignored. About the receiver: how others reach it (shown in status: "reachable: yes, by N nodes"). About anyone else: a lead, kept with who reported it. A node only ever dials a lead; a lie costs one handshake. Its own proofs override reports of another address.
4. **Use: finding neighbors.** With fewer than 2 relaying neighbors up, a node dials the best lead every 30 s (8 s after start, after configured and remembered ones had their chance): public addresses first, then its own proofs, then by how many reported it, then by age. A lead not up within 20 s is dropped and not tried again for 10 minutes. Everything learned is kept in `reachable.json` (at most 256 entries, a week).
5. **The local network: mDNS.** Every node registers `_vork._udp.local.` (DNS-SD) with its port and its WireGuard key in TXT (`k`, hex; `v=0`), and browses for others. A node found is dialed with one handshake sent to every address it registered (loopback only if there's nothing else); the address the answer comes from is the link's. Standard tools (avahi-browse, Bonjour) see the nodes; Android needs a multicast lock, which the app holds while the node runs. `lan = false` turns it off.
6. **The router port.** A node asks its router for a port by default (UPnP, then PCP/NAT-PMP; `map_port = false` to leave the router alone). The outside port needn't be the node's own: when it's taken, the router picks one. Behind a NAT that keeps a node's outgoing mapping open to anyone (many home routers), the node is reachable without any mapping while a neighbor keeps it alive; cold dials find that out like anything else.
7. **Next:** a default list of leads shipped with installs (seeds), the same structure, replaced by experience; storage and path choice preferring nodes others proved reachable.

## Consequences
- On one network no address is needed at all; across networks one is enough to join: everything after that is learned, and a node that has been running once can start again without any configured peer.
- A node learns from others whether it can be reached, which tells the user whether the router port matters.
- Reports reveal which nodes a node has linked with (their keys and public addresses) to its neighbors. That's no more than announces already tell, but private addresses stay within their kind of network.
- Leads can make a node dial an address a liar chose: at most one handshake initiation (148 bytes) per 30 s per node, too little to be a weapon.

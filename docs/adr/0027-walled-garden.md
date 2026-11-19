# ADR-0027: The walled garden

- **Status:** accepted
- **Date:** 2026-11-19
- **Irreversible:** no

## Context
With the interface on (M2), a node's services (SSH, a dashboard, Docker's ports) are reachable at its overlay address. Sessions identify who sends, but until now any identified node could reach them. Every node should keep helping others connect (relaying is the network's strength), while the owner's own nodes stay a private garden, with doors opened deliberately for a bigger, public one.

## Decision
- **Relaying is untouched.** Forwarded traffic never reaches the interface and is never filtered here.
- **What reaches the interface** from a session (IP, ADR-0021) is checked against the sender's identity: the **owner's devices** (ADR-0024) reach everything; **other nodes** only what `allow` opens, plus **answers in conversations this node began** (TCP/UDP by ports, ping by echo id, idle for 10 min at most), plus ICMPv6 errors (connections need them).
- **Rules** (`allow = [...]`, editable from the dashboard and the control API, `allow_add`/`allow_remove`): `tcp/80`, `udp/5000-5010` (every node), `… from <address>` (one node, by its full address: a prefix can be matched by grinding keys), `any from <address>` (everything for a trusted node), `icmp` (ping). `any` for every node is refused: it would leave no garden.
- Blocked traffic is logged once a minute per node and port, with the rule that would let it in.
- The exit node (ADR-0026) stays the owner's devices only, whatever `allow` says.

## Consequences
- A node is closed to strangers by default, with no setup; a public service is one line.
- Rules match ports after the IPv6 header (no extension headers looked into); fragments and other protocols from non-devices are dropped.
- Not yet: rules by name, per-service identities (several identities on one node, ADR-0021), and rules in the Android app.

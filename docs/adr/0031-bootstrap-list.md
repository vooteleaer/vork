# ADR-0031: One network from the start: a list of public nodes, by their owners' choice

- **Status:** accepted (built 2026-11-21)
- **Date:** 2026-11-21
- **Irreversible:** no

## Context
A new node knows no one. Until now it was given a first contact by hand (`peers`, or a public node from the README), or it got one from a pairing link (`a=`). A link is enough for the owner's own devices, but without a common starting point every owner's devices would be an island of their own: they never meet other people's nodes, so no one else relays for them, and a device behind a router with no open port has no one to reach it through. vØrk is meant to be one public network (ADR-0027).

A list of addresses everyone starts from is the usual answer (Yggdrasil's public peers, Bitcoin's seeds). The worry is a list that collects people's home addresses: a public, lasting record of who runs a node where.

## Decision
1. **`bootstrap.txt` in the repository** lists public nodes to start from, one `host:port` (or `host:port@key`) per line.
2. **Only by the owner's choice.** A node is listed only when its owner asks for it (a pull request, docs/bootstrap.md), typically a server. Nodes never add themselves or others; what nodes learn of each other's addresses (REACH, ADR-0023) stays among nodes, for a week at most.
3. **Used only while a node knows no one:** 20 s after it starts, and at every look after that (30 s), while no neighbor is up, it dials the listed nodes (asking their keys if not given). Once a neighbor is up, the ones not answering are let go; remembered neighbors and REACH take over from there.
4. **Fetched, with a copy built in.** While it knows no one, a node fetches the list from GitHub over HTTPS (at most once an hour, and once a day when it has a copy), keeps it in its state directory, and also dials the copy built into its binary, so a release starts even where GitHub can't be reached.
5. **Off by choice:** `bootstrap = false` (Settings → Network: *Start from public nodes*) for a network of one's own, reached only through its `peers`.

## Consequences
- Every node starts in the same network, and adding a public node to the list reaches new installs at once, without a release.
- A node that knows no one tells GitHub its address when fetching the list. A node with neighbors never fetches it.
- A listed node learns the addresses of nodes starting from it, as any neighbor does. It can't read their traffic or reach their services (end-to-end sessions, the walled garden).
- A wrong or hostile entry costs a key query; a hostile listed node can refuse to relay, as any neighbor can. More listed nodes make that matter less.
- Today the list has one node; a DNS seed can be added next to it later, so the list doesn't depend on GitHub alone.

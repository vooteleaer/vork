# ADR-0025: The resolver at fd00::53; device names

- **Status:** accepted
- **Date:** 2026-11-19
- **Irreversible:** no (the resolver address is in every client's DNS settings once used)

## Context
ADR-0018 gave the `.vork` domain. Two things were left: where the resolver listens, and which names it may answer. Binding port 53 on the node's address needs a privilege the service doesn't otherwise have, and on Android the VPN can't send only one domain to a server: whatever it names becomes the phone's DNS server for everything. Names are announced by the nodes themselves, so any node can claim any name.

## Decision
- **The resolver is `fd00::53`**, inside the `fd00::/8` route: queries reach the node through the interface like any packet, and it writes the answers back into it. No port is bound and no privilege needed, on every platform alike. (An identity whose address starts with fifteen bytes `00…0053` would collide; not a real risk.)
- **What it answers:** `<name>.vork` for **this node and the owner's devices** (those linked, ADR-0024) by their announced name made a DNS label (lower case, other characters turned into `-`, at most 40, as the apps show it); `<address>.vork` for **any node**, in full; by 8 or more leading digits only for this node and the owner's devices (a stranger's prefix could be ground). Other names under `.vork` are NXDOMAIN; AAAA only (A is answered with no records).
- **Wired into the OS for `.vork` only** where it can be: Linux through systemd-resolved (`resolvectl dns <if> fd00::53`, `domain <if> ~vork`), Windows through an NRPT rule.
- **Android:** the VPN makes `fd00::53` the phone's DNS server and the node **forwards everything else** to the network's own servers, which the app tells it (`dns_upstream`, again on every change of network; public resolvers only when the network names none). The app is excluded from its own VPN, so the node's traffic, these questions included, never loops back into it.

## Consequences
- `ssh home-server.vork` works from the owner's own devices; a stranger announcing the same name gets nothing.
- On Android, DNS for the whole phone goes through the node while the VPN is on: a node that stops answers nothing, so the VPN must come down with it. Large answers that need TCP aren't forwarded yet (clients use EDNS over UDP).
- Petnames from a file, PTR, and IPv4 aliases (ADR-0021) are still to come.

## Addendum: IPv4 aliases now (ADR-0021)
On the first phone test the browser asked only for `A` (and `HTTPS`), never `AAAA`: Android and Chrome don't ask for IPv6 addresses on a network whose only IPv6 route is `fd00::/8`. So `A` queries are answered now with ADR-0021's local aliases from `198.18.0.0/15` (this node is `198.18.0.1`; the table in `<state>/aliases.json`), the range is on the interface (the Android VPN; on Linux unless a route already covers it), and the node translates: IPv4 from a program here to an alias becomes IPv6 to that node (TCP's MSS lowered by 20 so the result fits the MTU; ICMP echo as ICMPv6 echo; fragments and other protocols dropped), and answers in those conversations come back as IPv4. Conversations begun by the far end stay IPv6. Choosing among candidate ranges and translating ICMP errors are still to come.

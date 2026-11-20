# Offering a public node

New vØrk nodes start from the public nodes in [`bootstrap.txt`](../bootstrap.txt) (ADR-0031): a node that knows no one yet dials them, so everyone is in the same network. A public node helps nodes find each other and relays for them; it can't read anyone's traffic or reach their devices' services.

Every node on the list is there because its owner chose to offer it. Nothing is added by the nodes themselves.

## What a public node needs

- A **fixed address**: a static IP, or a DNS name that follows it.
- Its **UDP port open** from the internet (a server, or a forwarded port).
- To be **up most of the time**, and on a connection that can take some relaying.
- A recent vØrk release, updated now and then.

A server (a small VPS is enough) is the best fit. A node at home works too, but its address then becomes public.

## Adding it

1. Check it's reachable: from a node elsewhere, `vork peer add <host:port>` (or Settings → Connections) connects to it.
2. Open a pull request that adds a line to `bootstrap.txt`: `host:port`, or `host:port@<its wireguard key>` (`vork status` shows the key) so nodes don't have to ask for it.
3. To take it off the list, open a pull request that removes the line.

New nodes fetch the list from GitHub, so a change reaches them at once; releases also carry a copy.

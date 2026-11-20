# vØrk in Docker

For a NAS or server that runs Docker but takes no packages (TrueNAS SCALE:
Apps → Discover → ⋮ → Install via YAML). One small container downloads a
release's static node, checks its SHA-256, and runs it with the `vork0`
interface. Change the host folder in it first: it keeps the node's identity.
Then link it to your devices:

```sh
docker exec vork /var/lib/vork/bin/vork link --config /var/lib/vork/vork.toml 'vork-link:…'
```

with a link from another device (Devices → Show a code). For other nodes to
reach it from outside, the node asks the router for a port by itself (UPnP,
PCP); it listens on a free port chosen at its first start (in `vork.toml`,
`listen`), so it doesn't collide with other nodes behind the same router. A
router without UPnP needs that port forwarded by hand.

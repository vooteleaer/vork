# vØrk

*Technical name `vork` (crate, binary, folder), since `Ø` breaks shells, URLs and package registries.* A serverless, identity-addressed network over WireGuard: Reticulum's ideas on plain IP, fast, with no central servers. Any node can relay for others, and any node is assumed hostile.

> **Status: prototype (phase 1 done, phase 2 in progress).** Not for real use. The wire format will change. Tested only on one machine so far (localhost).

## Principles

- **Identity, not location.** A node's address is the hash of its keys. IP addresses only carry packets between neighbors; inside the network nothing is addressed by IP.
- **Proof, not trust.** Identities, announces and permissions are signatures. Where a proof is impossible (whether a relay really forwards), the node measures and routes around it, and never falls back to trusting a central party.
- **Every node may be hostile.** A hostile node can delay, drop and watch metadata. It can't forge, read or impersonate. Trusted nodes (your own, your island's) are an optimization, never an assumption.
- **No roles.** Every node runs the same code. Who relays, holds data or reflects addresses follows from measured properties (public address, uptime, bandwidth), not configuration.
- **Runs on small devices.** An ESP32 running ESPHome must be able to join (see [Lite nodes](#lite-nodes-esp32-esphome)).

## How phase 1 works

```
[ outer WireGuard: neighbor <-> neighbor ]   proves the link works both ways
  [ IPv4/UDP to port 4799 ]                  any WireGuard implementation can carry it
    [ HELLO | OBSERVED | ANNOUNCE | PATH_* | DATA | PROBE ]
      DATA: [ label ][ inner WireGuard: end <-> end ]   relays hold no keys for it
```

- **Identity:** Ed25519 (signing) + X25519 (the WireGuard key). `address = SHA-256(ed_pub ‖ x_pub)[:16]`.
- **Links:** a neighbor is *up* only while its WireGuard handshake is fresh (< 180 s), so every link used is proven to work in both directions. This avoids RNS's asymmetric-link problem.
- **Announces** are signed (`vork/v0/announce`, sequence number), flooded only over up links, and carry no IP address. The hop count is a hint, not trusted.
- **Paths:** a session runs over one or more paths. `PATH_OPEN` is routed by destination address; every hop swaps the label for a fresh random one on the next link and remembers the pair. After that, `DATA` carries only a label: **no packet carries a source, only the opening message carries a destination, and a label means something on one link only**, so colluding relays can't match packets by it.
- **Several paths per session** (`--paths N`, default 2): a node keeps a route per neighbor it heard an announce from, and opens paths through different first hops. The origin probes every path each second (round-trip time, loss); a path that misses 3 probes is dropped and that first hop rests (5 s, doubling to 60 s) before it is tried again. Packets go out by `--spread spray` (one path each, weighted by round-trip time) or `--spread dup` (every packet on every path; WireGuard drops the second copy). A healthy direct link is used alone, sparing the relays. The far end answers on the paths it heard from in the last 3 s, so it can also start a new handshake to an initiator that never announced itself.
- **Silent neighbors:** a neighbor that sends nothing for 35 s (it keepalives every 25 s) is down, and every path through it is torn down hop by hop with `PATH_ERR`.
- **Anonymous initiators:** a node needn't announce itself to reach others (`--no-announce`). The destination learns its key, not its address or location.
- **Roaming:** a neighbor's endpoint is wherever its last authenticated packet came from.
- **Lite nodes** (`--lite`) never relay, receive no floods and ask for the paths they need (`PATH_REQ`). Nobody routes through a Lite node unless it is the destination.
- **Direct links** (`--direct`, opt-in on both ends): once a session works through relays, the two ends send each other the addresses their neighbors see them at, then both send WireGuard handshakes to those addresses at once (hole punching). When the link comes up, the session moves to it and the relays drop out. This reveals each end's address to the other, so it's off by default.

### Tested (localhost)

| Test | Result |
|---|---|
| C → B through A (1 relay), C anonymous | ~0.8 ms ping; A relayed 21 packets with 0 sessions |
| C → D through A and B (2 relays), 193 s | 184 pings, 0 lost, survives the inner rekey at 120 s; each relay switched 356 packets with 0 sessions |
| Lite node L behind A pings B; C pings L | both work; L learned only the 2 destinations it asked for and relayed nothing |
| C and B with `--direct`, meeting through A | 1 ping relayed, then 20 direct; A stopped at 7 relayed packets |
| C → B over two relays A and E (`--paths 2`, spray), A killed after 12 s, 57 s run | first 11 pings split 6/5 between A and E; A's path dropped 3 s after the kill, then all traffic on E; 51 of about 55 pings answered; A retried after 5 s and 10 s, given up when its link went silent |
| Same with `--spread dup` | every ping on both paths, no errors; each relay carried about half the packets |

## Try it

Prebuilt static binaries are in `dist/` (Linux x86_64, Linux aarch64 for a Raspberry Pi, Windows x86_64), or `cargo build --release`.

```sh
V=dist/vork-linux-x86_64          # or target/release/vork
$V keygen --out a.key; $V keygen --out b.key; $V keygen --out c.key   # prints address and wg key

$V run --key a.key --listen 0.0.0.0:51820                             # e.g. cafa-cloud (public IP)
$V run --key b.key --listen 0.0.0.0:51821 --peer HOST:51820@<A wg key>  # home (behind NAT)
$V run --key c.key --listen 0.0.0.0:51822 --peer HOST:51820@<A wg key> \
       --no-announce --ping <B address>                                 # laptop, anonymous
```

Options: `--paths N` (paths per session through different first hops, default 2), `--spread spray|dup` (how packets use them), `--lite` (don't relay, small devices), `--direct` (try a direct link to the far end of a session; reveals your address to it), `--no-announce` (reach others without being findable), `--map-port` (ask the router to open the port, like torrent clients: UPnP, then PCP/NAT-PMP; `--gateway <ip>` if the router isn't x.y.z.1). Stop with Ctrl-C, which also removes the router mapping.

### Testing on real machines

1. **A public node** (e.g. cafa-cloud): `vork run --key a.key --listen 0.0.0.0:51820`, with UDP 51820 forwarded to it.
2. **Two nodes behind different NATs** (home, a laptop on a phone hotspot), each with `--peer <A public IP>:51820@<A wg key>`.
3. Ping one from the other by address: first through A, then with `--direct` on both to see whether the hole punch gets through. Symmetric NAT and CGNAT on both ends won't; the session then simply stays relayed.
4. A Lite node anywhere with `--lite`.

Watch the logs for `link up`, `a neighbor sees us at` (your public address as others see it), `dialing … directly`, `session … now goes direct`, and the `(relayed)` / `(direct)` tag on each pong.

## Lite nodes (ESP32, ESPHome)

ESPHome's WireGuard component has **one peer and IPv4 only**, and it can't accept unknown peers. So a small device runs **vork Lite**: the same protocol and identity, minus relaying. It isn't a different kind of node to the network, it just announces that it doesn't relay (`FLAG_RELAY` unset), and no one routes through it. The same build option suits battery devices and phones.

- **Outer layer unchanged.** The stock `wireguard:` component with a single upstream peer (a full node). That's why the layer travels as plain IPv4/UDP inside the tunnel: the Lite node needs only a UDP socket, no WireGuard changes.
  ```yaml
  wireguard:
    address: 10.89.0.2
    peer_allowed_ips: [10.89.0.1/32]
    peer_persistent_keepalive: 25s
  ```
- **No flooding to Lite nodes.** The upstream doesn't send a Lite node every announce. The Lite node asks for the path it needs (`PATH_REQ`, implemented) and uses the upstream as its default route.
- **End-to-end layer** needs a second WireGuard-compatible session engine on the device, running over `FWD` instead of a socket. To check: whether the Noise core of wireguard-lwip (which esp_wireguard is built on) can be used without its netif, or whether the fugo101/wireguard_lwip fork (16 peers, custom output for relays) fits.
- **Time:** WireGuard handshakes carry a timestamp the responder checks, so the device needs SNTP (ESPHome's `time:`). Announce sequence numbers must not depend on the clock (see below).
- **Identity:** the device's own (server-type) identity in encrypted flash, delegated by its owner. Ed25519 via libsodium (an ESP-IDF component).
- **Budget:** the Lite node side of L0 must stay small: fixed binary layouts, no CBOR/JSON in L0, a handful of neighbors and sessions.
- **Goal:** an ESPHome external component (`external_components:`), so that Home Assistant reaches a device by its identity, end to end, through any number of relays.

## Known limits and next steps

- **Sequence numbers** are wall-clock milliseconds here. The spec must say only "monotonic per identity" so devices without a clock can use a persisted counter.
- **First hops are chosen by advertised hop count**, which isn't proven; once open, paths are weighted by measured round trip and dropped when they stop answering, so a sinkhole costs a few seconds, not the session. Paths through different first hops may still share later hops (distance-vector routing can't see them).
- **Spraying one TCP connection over paths with different latency reorders it.** Pinning each flow to a path comes with the TUN interface (phase 3), where flows exist.
- **Several local uplinks at once** (Wi-Fi + 4G) aren't used yet: that needs a socket per interface, with the same path machinery on top.
- **Router port mapping** (`--map-port`) is untested against a real router. It detects a second NAT beyond the router (CGNAT) and gives up there, but it doesn't yet ask a neighbor to confirm the mapped address is reachable from outside.
- **Direct links are untested through real NATs.** Candidates are the addresses neighbors observe plus the listening address if it is a specific one; a node listening on `0.0.0.0` doesn't yet offer its LAN addresses, and observed addresses aren't yet cross-checked by several neighbors (a hostile neighbor could report a wrong one).
- **Eclipse resistance:** several bootstrap sources, diverse neighbors, long-lived anchors.
- **Sybil:** proof-of-work stamps on announces.
- **boringtun** 0.7.1 is used as-is; it needs payloads framed as IP packets. Check its maintenance status against the forks (defguard, nordisk) before relying on it.

## Phases

1. **Links, gossip, hash routing, forwarding** (this).
2. NAT: addresses observed by peers, hole punching, router port mapping, path requests, Lite profile; path labels and several paths per session. *(First version done; needs testing on real networks.)*
3. Overlay IPv6 as a compatibility view (TUN): SSH, SMB and browsers by identity.
4. retifs as the storage layer (its `Transport` trait over this network).
5. YubiKey identities, device delegation, islands.
6. Nested WireGuard for the strong privacy class.

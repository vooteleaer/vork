# vØrk

*Technical name `vork` (crate, binary, folder), since `Ø` breaks shells, URLs and package registries.* A serverless, identity-addressed network over WireGuard: Reticulum's ideas on plain IP, fast, with no central servers. Any node can relay for others, and any node is assumed hostile.

> **Status: prototype (milestone M1 of the [roadmap](docs/04-roadmap.md)).** Not for real use. The wire format will change. Tested on localhost and between two machines across a router.

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
vork init                         # writes vork.toml, creates the identity, prints address and wg key
                                  # (as root: /etc/vork/vork.toml + /var/lib/vork; Windows: %ProgramData%\vork)
vork run                          # uses the default vork.toml; or: vork run --config path/to/vork.toml
vork status                       # asks the running node (neighbors, destinations, sessions, paths)
```

`vork.toml` holds the listening address, bootstrap `peers` (`"host:port@wg key"`), and the options below; command-line flags override it. The node remembers its neighbors in its state directory, so after the first contact `peers` can be empty and it still reconnects after a restart. Everything also works without a file: `vork run --key node.key --peer host:port@key ...`.

As a service:

- **Linux:** `packaging/vork.service` (instructions inside): `vork init` as root, install the unit, `systemctl enable --now vork`.
- **Windows** (as administrator): `vork init`, then `vork service install` (starts at boot; logs to `vork.log` in the state directory), `vork service uninstall` to remove it.

Testing tools: `--ping <address>` once a second; `--bench <address> --bench-rate <Mbit/s>` sends paced traffic (1200-byte packets) and the receiver logs what arrives and the loss each second. There is no congestion control in the bench: above the ceiling it just loses packets.

Options (file or flags): `paths` (paths per session through different first hops, default 2), `spread` (`spray` or `dup`), `lite` (don't relay, small devices), `direct` (try a direct link to the far end of a session; reveals your address to it), `announce = false` / `--no-announce` (reach others without being findable), `map_port` (ask the router to open the port: UPnP, then PCP/NAT-PMP; `gateway` if the router isn't x.y.z.1), `log` (`warn`, `info`, `debug`), `log_file`. Stop with Ctrl-C, which also removes a router mapping.

### Testing on real machines

1. **A public node** (e.g. cafa-cloud): `vork run --key a.key --listen 0.0.0.0:51820`, with UDP 51820 forwarded to it.
2. **Two nodes behind different NATs** (home, a laptop on a phone hotspot), each with `--peer <A public IP>:51820@<A wg key>`.
3. Ping one from the other by address: first through A, then with `--direct` on both to see whether the hole punch gets through. Symmetric NAT and CGNAT on both ends won't; the session then simply stays relayed.
4. A Lite node anywhere with `--lite`.

Watch the logs for `link up`, `a neighbor sees us at` (your public address as others see it), `dialing … directly`, `session … now goes direct`, and the `(relayed)` / `(direct)` tag on each pong.

### Throughput (localhost, 2-core VM)

All nodes ran on one machine with two CPU cores, so every relay competes with the endpoints for CPU. These are CPU limits of this prototype, not network limits:

| Topology | Ceiling (received, low loss) |
|---|---|
| Direct (2 processes) | about 700–1000 Mbit/s |
| 1 relay (3 processes) | about 300–400 Mbit/s |
| 2 relays in a chain (4 processes) | about 300 Mbit/s |
| 2 parallel paths over 2 relays (4 processes) | about 400–470 Mbit/s |

One node is a single thread doing two layers of WireGuard in userspace, so it handles roughly 1 Gbit/s of traffic on one core of this VM. On separate machines each relay has its own CPU, and the limit is usually the slowest uplink on the path. Room to grow: batch socket calls (`sendmmsg`/`recvmmsg`, UDP GSO/GRO) — on older CPUs with Meltdown mitigations the per-packet system call is the main cost —, reuse buffers instead of allocating per packet, and spread work across threads.

### Real machines

A Windows PC (10.10.10.238) and cafa-cloud (Debian 13, Xeon E5-2650 from 2012, a busy production host; 10.10.30.2), in different subnets with a router between them, linked directly:

| Bench rate (Mbit/s) | Received | Loss |
|---|---|---|
| 100 | 100 | 0% |
| 250 | 250 | 0% |
| 500 | 430–497 | mostly 0–2%, spikes to 23% |
| 800–1000 | 280–460 | 20–64% |

The ping round trip was 2–4 ms. The ceiling, **about 450–500 Mbit/s**, is cafa-cloud's receiving side: an old CPU where Meltdown/MDS mitigations make every system call expensive, and vØrk makes one or more per packet. The PC sent up to ~830 Mbit/s. Before 4 MB socket buffers (`SO_RCVBUFFORCE` as root) and a larger burst allowance for Windows' ~15 ms timer, the same link lost 10% at 100 Mbit/s and topped out at ~130.

`scripts/bench-local.sh` runs a topology on one machine (several nodes on 127.0.0.1) to measure its CPU ceiling.

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

## Storage

vØrk storage (formerly [retifs](https://github.com/vooteleaer/retifs)) is the encrypted, replicated data layer: libraries of records and files, kept on your own devices and blind storage nodes. Its code was copied here and renamed (ADR-0015); it still runs on Reticulum through Prns until it moves onto vØrk (roadmap M9). Building `vork-storage-net` and `vork-storage-node` needs the Prns source at `../Prns-0.3.8`.

| Crate | What |
|---|---|
| `vork-core` | chunks, CIDs, manifests, records, leases, channels, recovery |
| `vork-store` | the local slab store |
| `vork-storage-net` | the network side (fetching, serving; Reticulum for now) |
| `vork-storage-node` | `vorkd`, the storage daemon, and `vork-storage`, its commands |
| `libvork` | client of the daemon's local API |
| `vork-tree`, `vork-fuse`, `vork-cloudfiles`, `vork-haikufs` | a library as a folder: FUSE (`vork-mount`), Windows Explorer, Haiku |
| `vork-gateway`, `vork-extract`, `vork-card` | libraries as local websites, attribute extraction, identities on a YubiKey |

`vork put FILE`, `vork query LIB …` and the other storage commands are passed to `vork-storage`; `vork help-storage` lists them. `vork status` on its own is the network node's; `vork status READ-CAP` is storage's. Storage documents: [docs/storage](docs/storage/README.md).

## Documentation

The design, the protocol, the threat model and the plan are in [`docs/`](docs/README.md): vision, architecture, wire format v0, threat model, roadmap (milestones M1–M12 with acceptance tests), open questions, and the decisions so far as ADRs.

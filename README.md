# vork

*Working name.* A serverless, identity-addressed network over WireGuard: Reticulum's ideas on plain IP, fast, with no central servers. Any node can relay for others, and any node is assumed hostile.

> **Status: phase-1 prototype.** Not for real use. The wire format will change.

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
    [ HELLO | OBSERVED | ANNOUNCE | FWD ]
      FWD: [ inner WireGuard: end <-> end ]  relays hold no keys for it
```

- **Identity:** Ed25519 (signing) + X25519 (the WireGuard key). `address = SHA-256(ed_pub ‖ x_pub)[:16]`.
- **Links:** a neighbor is *up* only while its WireGuard handshake is fresh (< 180 s), so every link used is proven to work in both directions. This avoids RNS's asymmetric-link problem.
- **Announces** are signed (`vork/v0/announce`, sequence number), flooded only over up links, and carry no IP address. The hop count is a hint, not trusted.
- **Forwarding:** an inner handshake initiation is forwarded by destination address. Each relay remembers where it came from, keyed by the initiator's WireGuard index. Every later packet is switched by its receiver index alone, so **no packet carries a source, and only the first carries a destination**.
- **Anonymous initiators:** a node needn't announce itself to reach others (`--no-announce`). The destination learns its key, not its address or location.
- **Roaming:** a neighbor's endpoint is wherever its last authenticated packet came from.

### Tested (localhost)

| Test | Result |
|---|---|
| C → B through A (1 relay), C anonymous | ~0.8 ms ping; A relayed 21 packets with 0 sessions |
| C → D through A and B (2 relays), 193 s | 184 pings, 0 lost, survives the inner rekey at 120 s; each relay switched 356 packets with 0 sessions |

## Try it

```sh
cargo build --release
V=target/release/vork
$V keygen --out a.key; $V keygen --out b.key; $V keygen --out c.key   # prints address and wg key

$V run --key a.key --listen 0.0.0.0:51820                             # e.g. cafa-cloud (public IP)
$V run --key b.key --listen 0.0.0.0:51821 --peer HOST:51820@<A wg key>  # home (behind NAT)
$V run --key c.key --listen 0.0.0.0:51822 --peer HOST:51820@<A wg key> \
       --no-announce --ping <B address>                                 # laptop, anonymous
```

## Lite nodes (ESP32, ESPHome)

ESPHome's WireGuard component has **one peer and IPv4 only**, and it can't accept unknown peers. So a small device runs **vork Lite**: the same protocol and identity, minus relaying. It isn't a different kind of node to the network, it just announces that it doesn't relay (`FLAG_RELAY` unset), and no one routes through it. The same build option suits battery devices and phones.

- **Outer layer unchanged.** The stock `wireguard:` component with a single upstream peer (a full node). That's why the layer travels as plain IPv4/UDP inside the tunnel: the Lite node needs only a UDP socket, no WireGuard changes.
  ```yaml
  wireguard:
    address: 10.89.0.2
    peer_allowed_ips: [10.89.0.1/32]
    peer_persistent_keepalive: 25s
  ```
- **No flooding to Lite nodes.** The upstream doesn't send a Lite node every announce. The Lite node asks for the path it needs (path request, phase 2) and uses the upstream as its default route.
- **End-to-end layer** needs a second WireGuard-compatible session engine on the device, running over `FWD` instead of a socket. To check: whether the Noise core of wireguard-lwip (which esp_wireguard is built on) can be used without its netif, or whether the fugo101/wireguard_lwip fork (16 peers, custom output for relays) fits.
- **Time:** WireGuard handshakes carry a timestamp the responder checks, so the device needs SNTP (ESPHome's `time:`). Announce sequence numbers must not depend on the clock (see below).
- **Identity:** the device's own (server-type) identity in encrypted flash, delegated by its owner. Ed25519 via libsodium (an ESP-IDF component).
- **Budget:** the Lite node side of L0 must stay small: fixed binary layouts, no CBOR/JSON in L0, a handful of neighbors and sessions.
- **Goal:** an ESPHome external component (`external_components:`), so that Home Assistant reaches a device by its identity, end to end, through any number of relays.

## Known limits and next steps

- **Sequence numbers** are wall-clock milliseconds here. The spec must say only "monotonic per identity" so devices without a clock can use a persisted counter.
- **A responder can't start a handshake to an anonymous initiator** (it has no address to route to). The initiator rekeys, so it works in practice; path labels like RNS link IDs would fix it properly.
- **Index collisions:** relays switch on 24-bit random WireGuard indices. Collisions are rare but possible; the switch key could include a short path label.
- **Route choice** still uses the advertised hop count. Next: score paths by measured delivery (otherwise a hostile node can attract traffic as a sinkhole).
- **No hole punching yet** (phase 2): everything goes through relays.
- **Eclipse resistance:** several bootstrap sources, diverse neighbors, long-lived anchors.
- **Sybil:** proof-of-work stamps on announces.
- **boringtun** 0.7.1 is used as-is; it needs payloads framed as IP packets. Check its maintenance status against the forks (defguard, nordisk) before relying on it.

## Phases

1. **Links, gossip, hash routing, forwarding** (this).
2. NAT: addresses observed by peers, hole punching, path requests, Lite profile.
3. Overlay IPv6 as a compatibility view (TUN): SSH, SMB and browsers by identity.
4. retifs as the storage layer (its `Transport` trait over this network).
5. YubiKey identities, device delegation, islands.
6. Nested WireGuard for the strong privacy class.

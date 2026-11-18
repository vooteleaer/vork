# vØrk: roadmap

Each milestone ends in something that runs and is measured (proto first). "Done when" is the acceptance test. Order follows two rules: make vØrk usable on our own machines early, so that using it finds the problems; and check the riskiest unknowns before much is built on top of them.

```
M0 prototype (done)
 ├─ M1 config & service ── M9 storage on vØrk (next) ─┬─ M2 IPv6 sessions & TUN ── M3 DNS, whois, identity firewall
 │                                                    └─ M10 embedded stack (no admin rights)
 ├─ M4 hardening
 ├─ S1 ESP32 spike (parallel, early) ── M5 ESPHome component
 ├─ M6 performance
 ├─ M7 identities (YubiKey, delegation) ── M8 islands
 ├─ M11 strong privacy (nested WireGuard)
 └─ M12 protocol v1: spec freeze, test vectors, review
```

Milestone numbers are names, not order: M9 comes right after M1, because storage is the first real application and it brings the shared node and the native API with it.

## M0 Prototype — done

Links with fresh-handshake liveness, signed announces, path labels swapped per hop, several paths per session with probing and failover, Lite profile, path requests, observed addresses, direct links, router port mapping, ping and bench. Tested on localhost and between a PC and cafa-cloud across a router (~450–500 Mbit/s, CPU-bound on the server).

## M1 Config, service, persistence — in progress

- **Goal:** a node that runs 24/7 (cafa-cloud as a public backbone) and survives restarts.
- **Scope:** a config file (TOML: key path, listen address, bootstrap peers, flags); the announce sequence number as a stored counter instead of the clock (open question 3); remembered neighbors (reconnect after restart without bootstrap); a state directory for what the node chooses once and keeps (sequence counter, neighbors, later the IPv4 alias range and aliases); a systemd unit and a Windows service; log levels; `vork status` over a local control socket.
- **Done when:** cafa-cloud runs vØrk as a service through a restart and a reboot; a PC reconnects without `--peer` after its first contact.
- **Status:** implemented and tested locally: `vork init`, `vork.toml`, the state directory (key created on first run, sequence counter, remembered neighbors), reconnecting without bootstrap peers, `vork status` over a Unix socket / named pipe, log levels and a log file, `packaging/vork.service`, `vork service install|uninstall` on Windows. cafa-cloud runs it as a service since 2026-11-18 (`5f7a9ac6…`, UDP 51820): it reconnects to remembered neighbors after `systemctl restart`, and a PC reconnects to it without `--peer`. Left: a reboot of cafa-cloud, and a port forward of UDP 51820 to it so nodes outside the LAN reach it.
- **Depends on:** nothing.

## S1 ESP32 spike (parallel, early)

- **Goal:** find out whether an ESP32 can be a Lite node with the end-to-end layer, before the wire format freezes.
- **How:** ESP-IDF on an ESP32-S3. Outer layer: the stock ESPHome/esp_wireguard tunnel to a full node. Layer messages: a UDP socket on port 4799 in the tunnel. Inner layer: try (a) the WireGuard core of wireguard-lwip without its netif, sending its messages in `DATA`; (b) the fugo101/wireguard_lwip fork (16 peers, custom output). Ed25519 from libsodium.
- **Done when:** a PC pings the ESP32 by its address through cafa-cloud, end to end encrypted; flash and RAM use measured.
- **Risks:** the inner layer may not fit the stock libraries; then the outcome is a change to the wire format (for example, a lighter inner layer for Lite nodes) decided before M12.

## M2 IPv6 sessions and TUN

- **Goal:** ordinary applications reach peers by identity.
- **Scope:** sessions carry IPv6 packets between overlay addresses (ADR-0017, ADR-0021), one per identity, all of a machine's identities on its TUN interface, source checked against the session's identity; the ping/bench/candidate functions move to a control service on the overlay; a TUN interface (Linux `/dev/net/tun`; Windows wintun); the node drops packets whose overlay source doesn't match the session's identity; **flow pinning** (a TCP/UDP flow stays on one path, chosen by hash of its 5-tuple; spray stays for bulk native traffic); MTU set so both layers fit in 1500 (inner MTU about 1360, to be measured).
- **Done when:** `ssh` to `fd…` (a home machine) from a laptop behind another NAT works through relays and then over a direct link; an `iperf3` TCP test runs without reordering trouble over two paths.
- **Depends on:** M1, M9 (sessions).

## M3 Daemon API, whois, DNS, identity firewall

- **Goal:** applications know who is on the other end, and names replace addresses.
- **Scope:** the rest of the **shared node** (ADR-0020; M9 brings the first part): status, peers, `whois(overlay address) → identity`; **DNS** (ADR-0018): `<address>.vork` and petnames from a local file, AAAA and PTR, and **IPv4 aliases** handed out by `A` queries with translation at the node (ADR-0021), wired into the OS only for its own domain (systemd-resolved routing domain, Windows NRPT, macOS `/etc/resolver`); an **identity firewall**: which identities may reach which ports/services.
- **Done when:** `ssh nas.vork` works over IPv6 and an IPv4-only program reaches the same name through its alias; a service sees the caller's identity through whois; a connection from an identity not on the list is refused; two programs on one machine, each with its own identity, are reachable through the same node.
- **Depends on:** M2.

## M4 Hardening

- **Goal:** the open network survives hostile nodes (threat model, "planned" rows).
- **Scope:** per-neighbor limits for `PATH_OPEN`, announces and path requests, and caps on relay and route tables; proof-of-work stamps on announces; outside address taken only when several neighbors agree; bootstrap from several sources, neighbor diversity, long-lived anchors; **owner-first relaying** (ADR-0019): two classes (own, relayed), relayed traffic limited to the measured uplink minus the owner's use and backing off when the owner's round trip rises, shared fairly per path and neighbor; path choice by measured delivery instead of hop count; fuzzing of the message decoder (`cargo-fuzz`); a review of `unsafe` (currently only the socket options).
- **Done when:** a node flooded with `PATH_OPEN` from one neighbor keeps serving its other neighbors; a node relaying a saturating bench keeps its own ping within a few ms of idle and its own bench near full speed; the fuzzer runs clean for 24 h; each "planned" row in the threat model is implemented or moved to a later milestone with a reason.
- **Depends on:** M1.

## M5 ESPHome component

- **Goal:** `external_components: vork` in ESPHome; Home Assistant reaches the device by identity.
- **Scope:** depends on S1. The component brings up the Lite layer on top of the `wireguard:` component; the inner session delivers IPv6 (or IPv4) packets into lwIP, so the device's native API and MQTT work unchanged.
- **Done when:** Home Assistant adds the device by its vØrk name and it stays reachable through a relay.
- **Depends on:** S1, M2.

## M6 Performance

- **Goal:** the CPU stops being the limit on ordinary hardware.
- **Scope:** batched socket calls (`sendmmsg`/`recvmmsg`, UDP GSO/GRO on Linux), buffer reuse without per-packet allocation, several threads (sharded by neighbor); the bench as a regression test.
- **Done when:** at least 2× the M0 numbers on cafa-cloud and on the 2-core VM, measured with `scripts/bench-local.sh` and between machines.
- **Depends on:** nothing (can run alongside).

## M7 Identities

- **Goal:** "the device is hardware, you are your key."
- **Scope:** ADR-0013: a person's identity on a YubiKey (PIV Ed25519/X25519, firmware 5.7+); short-lived session keys while the key is plugged in, wiped when it is pulled; device and service identities delegated by their owner, with epochs for moving an identity between machines; recovery from a seed.
- **Done when:** moving the YubiKey from one laptop to another moves the identity (same address) within seconds; pulling it ends the identity there.
- **Depends on:** M3.

## M8 Islands

- **Goal:** closed groups inside the open network (ADR-0014).
- **Scope:** membership by signed invitations and revocations; island announces encrypted to the members; island policy (direct links allowed, relays preferred among members); later backed by a storage library.
- **Depends on:** M7.

## M9 Storage on vØrk — next

- **Goal:** vØrk storage runs on vØrk with no Reticulum code, its format unchanged so existing libraries move over (ADR-0015, ADR-0022).
- **Scope, in steps that each run:**
  1. *Identities and destinations:* **done.** vØrk addresses and key files are storage identities; named destinations `SHA-256(SHA-256(name)[0..10] ‖ address)[0..16]`; announces of several destinations per node with app data; path requests for destinations.
  2. *Sessions and requests:* sessions to a destination, anonymous opener with optional identify, a session nonce; `request(path, body) → response` up to 32 MiB, fragmented, reliable, flow-controlled over the session's paths. **Done:** 32 MiB through a relay at ~200 Mbit/s on one VM, ~45 Mbit/s with 5 % loss each way; over two relayed paths too.
  3. *Shared node and native API* (the part of ADR-0020 storage needs): applications attach over the local socket/pipe, register destinations, announce, serve and send requests; a Rust client crate. **Done:** [the local API](06-local-api.md), `crates/vork-api`; an application on one node answered 2 MB requests from an application on another, through a relay.
  4. *Storage transport:* storage's `Transport` and server side over the native API; storage announces, path lookups, identify and route information from vØrk; placement by measured round trip and direct-vs-relayed.
  5. *Prns removed:* shared-instance, remote-control and NomadNet code dropped; no Reticulum crate in the build.
- **Done when:** `vork put` on one machine and `vork get` on another across a router work over vØrk; a library written under retifs opens unchanged; `cargo tree` has no Prns.
- **Depends on:** M1.

## M10 Embedded stack (no admin rights)

- **Goal:** an application embeds vØrk and gets `TcpStream`/`UdpSocket` to identities without a TUN interface (like Tailscale's tsnet).
- **Scope:** a userspace TCP/IP stack (smoltcp) on the same IPv6 sessions; a Rust crate first, a C ABI later.
- **Depends on:** M2, M9.

## M11 Strong privacy

- **Goal:** the strong class (ADR-0016): no single relay knows both ends.
- **Scope:** nested WireGuard over 2–3 relays; receiver-chosen inbound part (introduction points); one-off initiator keys; optional cover traffic.
- **Depends on:** M4.

## M12 Protocol v1

- **Goal:** a wire format others can implement.
- **Scope:** freeze the formats; test vectors for every message and derivation; a second implementation of at least the Lite side (from S1/M5) checked against them; outside review.
- **Depends on:** S1, M2, M4 (the formats they change).

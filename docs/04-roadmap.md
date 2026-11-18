# vØrk: roadmap

Each milestone ends in something that runs and is measured (proto first). "Done when" is the acceptance test. Order follows two rules: make vØrk usable on our own machines early, so that using it finds the problems; and check the riskiest unknowns before much is built on top of them.

```
M0 prototype (done)
 ├─ M1 config & service ─┬─ M2 IPv6 sessions & TUN ── M3 daemon API, whois, DNS, identity firewall
 │                       └─ M4 hardening
 ├─ S1 ESP32 spike (parallel, early) ── M5 ESPHome component
 ├─ M6 performance
 ├─ M7 identities (YubiKey, delegation) ── M8 islands
 ├─ M9 native message API & retifs transport ── M10 embedded stack (no admin rights)
 ├─ M11 strong privacy (nested WireGuard)
 └─ M12 protocol v1: spec freeze, test vectors, review
```

## M0 Prototype — done

Links with fresh-handshake liveness, signed announces, path labels swapped per hop, several paths per session with probing and failover, Lite profile, path requests, observed addresses, direct links, router port mapping, ping and bench. Tested on localhost and between a PC and cafa-cloud across a router (~450–500 Mbit/s, CPU-bound on the server).

## M1 Config, service, persistence

- **Goal:** a node that runs 24/7 (cafa-cloud as a public backbone) and survives restarts.
- **Scope:** a config file (TOML: key path, listen address, bootstrap peers, flags); the announce sequence number as a stored counter instead of the clock (open question 3); remembered neighbors (reconnect after restart without bootstrap); a systemd unit and a Windows service; log levels; `vork status` over a local control socket.
- **Done when:** cafa-cloud runs vØrk as a service through a restart and a reboot; a PC reconnects without `--peer` after its first contact.
- **Depends on:** nothing.

## S1 ESP32 spike (parallel, early)

- **Goal:** find out whether an ESP32 can be a Lite node with the end-to-end layer, before the wire format freezes.
- **How:** ESP-IDF on an ESP32-S3. Outer layer: the stock ESPHome/esp_wireguard tunnel to a full node. Layer messages: a UDP socket on port 4799 in the tunnel. Inner layer: try (a) the WireGuard core of wireguard-lwip without its netif, sending its messages in `DATA`; (b) the fugo101/wireguard_lwip fork (16 peers, custom output). Ed25519 from libsodium.
- **Done when:** a PC pings the ESP32 by its address through cafa-cloud, end to end encrypted; flash and RAM use measured.
- **Risks:** the inner layer may not fit the stock libraries; then the outcome is a change to the wire format (for example, a lighter inner layer for Lite nodes) decided before M12.

## M2 IPv6 sessions and TUN

- **Goal:** ordinary applications reach peers by identity.
- **Scope:** sessions carry IPv6 packets between overlay addresses (ADR-0017); the ping/bench/candidate functions move to a control service on the overlay; a TUN interface (Linux `/dev/net/tun`; Windows wintun); the node drops packets whose overlay source doesn't match the session's identity; **flow pinning** (a TCP/UDP flow stays on one path, chosen by hash of its 5-tuple; spray stays for bulk native traffic); MTU set so both layers fit in 1500 (inner MTU about 1360, to be measured).
- **Done when:** `ssh` to `fd…` (a home machine) from a laptop behind another NAT works through relays and then over a direct link; an `iperf3` TCP test runs without reordering trouble over two paths.
- **Depends on:** M1.

## M3 Daemon API, whois, DNS, identity firewall

- **Goal:** applications know who is on the other end, and names replace addresses.
- **Scope:** a local IPC API (Unix socket / named pipe): status, peers, `whois(overlay address) → identity`, services; **DNS** (ADR-0018): `<address>.vork` and petnames from a local file, AAAA and PTR, wired into the OS only for its own domain (systemd-resolved routing domain, Windows NRPT, macOS `/etc/resolver`); an **identity firewall**: which identities may reach which ports/services.
- **Done when:** `ssh nas.vork` works; a service sees the caller's identity through whois; a connection from an identity not on the list is refused.
- **Depends on:** M2.

## M4 Hardening

- **Goal:** the open network survives hostile nodes (threat model, "planned" rows).
- **Scope:** per-neighbor limits for `PATH_OPEN`, announces and path requests, and caps on relay and route tables; proof-of-work stamps on announces; outside address taken only when several neighbors agree; bootstrap from several sources, neighbor diversity, long-lived anchors; relay priorities (own and island traffic first, strangers under a quota); path choice by measured delivery instead of hop count; fuzzing of the message decoder (`cargo-fuzz`); a review of `unsafe` (currently only the socket options).
- **Done when:** a node flooded with `PATH_OPEN` from one neighbor keeps serving its other neighbors; the fuzzer runs clean for 24 h; each "planned" row in the threat model is implemented or moved to a later milestone with a reason.
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
- **Scope:** membership by signed invitations and revocations; island announces encrypted to the members; island policy (direct links allowed, relays preferred among members); later backed by a retifs library.
- **Depends on:** M7.

## M9 Native message API and retifs transport

- **Goal:** native applications use identities and named services without IP; retifs runs over vØrk (ADR-0015).
- **Scope:** `connect(identity, service)`, request/response and datagrams over the IPC API and as a Rust crate; an implementation of retifs's `Transport` trait (`connect`, `link_nonce`, `request`) and its server side; placement in retifs uses measured round trip and direct-vs-relayed instead of hop count.
- **Done when:** retifs's network examples pass over vØrk between two machines.
- **Depends on:** M3.

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

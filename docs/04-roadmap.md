# vØrk: roadmap

Each milestone ends in something that runs and is measured (proto first). "Done when" is the acceptance test. Order follows two rules: make vØrk usable on our own machines early, so that using it finds the problems; and check the riskiest unknowns before much is built on top of them.

## Where it stands (2026-11-20)

vØrk replaces Tailscale on the author's own machines (a Linux server, a Windows PC, Android phones, a TrueNAS box), released as [v0.4.0](https://github.com/vooteleaer/vork/releases/tag/v0.4.0), a pre-release: installers for Windows (MSI), Android, Debian/Ubuntu, static Linux binaries, a Docker setup for NAS boxes and a Home Assistant add-on (Haiku: v0.3.2). v0.4.0 brings vØrk storage, with installers of its own for Windows, Android, Debian/Ubuntu and Docker.

Built since the prototype: the node as a service (M1); the TUN interface on Linux, Windows and Android with IPv6 sessions and IPv4 delivered as IPv4 (M2); `.vork` names, devices linked with consent and one group per owner, the walled-garden firewall (M3); exit nodes (Linux offers, Windows and Android use); reachability learned from others, LAN discovery and router port mapping; a dashboard on every platform (a window on the desktop, the Android app, the browser); self-updating with signed releases; the local API for applications, and vØrk storage on it (M9; its Windows installer released). Next: hardening (M4), Linux as an exit client, routing for big networks, a project release key.

```
M0 prototype (done)
 ├─ M1 config & service (done) ── M9 local API & destinations (done) ─┬─ M2 IPv6 sessions & TUN (mostly) ── M3 DNS, devices, firewall (mostly)
 │                                                    └─ M10 embedded stack (no admin rights)
 ├─ M4 hardening
 ├─ S1 ESP32 spike (parallel, early) ── M5 ESPHome component
 ├─ M6 performance
 ├─ M7 identities (YubiKey, delegation) ── M8 islands
 ├─ M11 strong privacy (nested WireGuard)
 └─ M12 protocol v1: spec freeze, test vectors, review
```

Milestone numbers are names, not order: M9 came right after M1, as it brings the shared node and the native API that applications use.

## M0 Prototype — done

Links with fresh-handshake liveness, signed announces, path labels swapped per hop, several paths per session with probing and failover, Lite profile, path requests, observed addresses, direct links, router port mapping, ping and bench. Tested on localhost and between a PC and the server across a router (~450–500 Mbit/s, CPU-bound on the server).

## M1 Config, service, persistence — done

- **Goal:** a node that runs 24/7 (a home server as a public backbone) and survives restarts.
- **Scope:** a config file (TOML: key path, listen address, bootstrap peers, flags); the announce sequence number as a stored counter instead of the clock (open question 3); remembered neighbors (reconnect after restart without bootstrap); a state directory for what the node chooses once and keeps (sequence counter, neighbors, later the IPv4 alias range and aliases); a systemd unit and a Windows service; log levels; `vork status` over a local control socket.
- **Done when:** a server runs vØrk as a service through a restart and a reboot; a PC reconnects without `--peer` after its first contact.
- **Status:** implemented and tested locally: `vork init`, `vork.toml`, the state directory (key created on first run, sequence counter, remembered neighbors), reconnecting without bootstrap peers, `vork status` over a Unix socket / named pipe, log levels and a log file, `packaging/vork.service`, `vork service install|uninstall` on Windows. The server runs it as a service since 2026-11-18 (UDP 51820): it reconnects to remembered neighbors after `systemctl restart`, and a PC reconnects to it without `--peer`. Since then the server has run it through reboots behind a port forward, and installers set it up: an MSI on Windows, a .deb, a Docker setup (ADR-0029).
- **Depends on:** nothing.

## S1 ESP32 spike (parallel, early)

- **Goal:** find out whether an ESP32 can be a Lite node with the end-to-end layer, before the wire format freezes.
- **How:** ESP-IDF on an ESP32-S3. Outer layer: the stock ESPHome/esp_wireguard tunnel to a full node. Layer messages: a UDP socket on port 4799 in the tunnel. Inner layer: try (a) the WireGuard core of wireguard-lwip without its netif, sending its messages in `DATA`; (b) the fugo101/wireguard_lwip fork (16 peers, custom output). Ed25519 from libsodium.
- **Done when:** a PC pings the ESP32 by its address through the server, end to end encrypted; flash and RAM use measured.
- **Risks:** the inner layer may not fit the stock libraries; then the outcome is a change to the wire format (for example, a lighter inner layer for Lite nodes) decided before M12.
- **Status:** done (2026-11-24). The inner layer is its own small WireGuard responder over Monocypher (no stock library fitted), checked against boringtun; Ed25519 from Monocypher. A PC pings an ESP32-S3 end to end through relays (14–120 ms); flash +35 kB, static RAM +1 kB.

## M2 IPv6 sessions and TUN — mostly done

- **Goal:** ordinary applications reach peers by identity.
- **Scope:** sessions carry IPv6 packets between overlay addresses (ADR-0017, ADR-0021), one per identity, all of a machine's identities on its TUN interface, source checked against the session's identity; the ping/bench/candidate functions move to a control service on the overlay; a TUN interface (Linux `/dev/net/tun`; Windows wintun); the node drops packets whose overlay source doesn't match the session's identity; **flow pinning** (a TCP/UDP flow stays on one path, chosen by hash of its 5-tuple; spray stays for bulk native traffic); MTU set so both layers fit in 1500 (inner MTU about 1360, to be measured).
- **First part done (2026-11-18):** Linux TUN (`tun = "vork0"`), the node's identity as `fd…`/128 and `fd00::/8` routed to the interface, IPv6 as session datagrams with the source checked against the identified far end, answers on the session they came in on. Two nodes in separate network namespaces: ping6 0.2–0.3 ms, HTTP over `fd…`, 20 MB at ~1.1 Gbit/s. Then Windows (wintun: `tun = "vork"` with wintun.dll next to vork.exe, needs the service's administrator rights; built, not run yet) and Android (the app's VpnService makes the interface, `fd00::/8` only, and hands it to the node over `tun.sock`, `tun = "fd"`; handover tested on Linux, the app not on a phone yet). Left: applications' identities on the interface, flow pinning, the MTU measured, turning the Android interface off without restarting the node.
- **Since (2026-11-19/20):** Windows (wintun) and Android run every day; IPv4 between devices at a derived address the same everywhere, translated at the nodes and delivered as IPv4 at the far end (a server listening on IPv4 only is reached; one refusing IPv4 gets the conversation as IPv6) (ADR-0021); exit nodes carry a device's internet (ADR-0026): Linux offers, Android and Windows use one, the LAN kept local. Haiku: the node and an IPv4-only interface (its tunnel driver), no mDNS.
- **Done when:** `ssh` to `fd…` (a home machine) from a laptop behind another NAT works through relays and then over a direct link; an `iperf3` TCP test runs without reordering trouble over two paths.
- **Depends on:** M1, M9 (sessions).

## M3 Daemon API, whois, DNS, identity firewall — mostly done

- **Goal:** applications know who is on the other end, and names replace addresses.
- **Scope:** the rest of the **shared node** (ADR-0020; M9 brings the first part): status, peers, `whois(overlay address) → identity`; **DNS** (ADR-0018): `<address>.vork` and petnames from a local file, AAAA and PTR, and **IPv4 aliases** handed out by `A` queries with translation at the node (ADR-0021), wired into the OS only for its own domain (systemd-resolved routing domain, Windows NRPT, macOS `/etc/resolver`); an **identity firewall**: which identities may reach which ports/services.
- **Progress:** the resolver at `fd00::53` answers `<name>.vork` for this node and the owner's devices and `<address>.vork` for any node, wired in through systemd-resolved, NRPT and the Android VPN (ADR-0025).
- **Progress (identity firewall):** the walled garden (ADR-0027): the owner's devices reach a node's services, other nodes what `allow` opens; relaying untouched.
- **Progress (devices):** devices are linked with both sides agreeing, by QR code or link (ADR-0024), and an owner's devices are one group: a device linked to one is linked to all (ADR-0030). Left: whois for applications, several identities per node.
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
- **Status:** done (2026-11-25), published 2026-11-26 as [vork-lite](https://github.com/vooteleaer/vork-lite) (an ESP-IDF library) and [esphome-vork-lite](https://github.com/vooteleaer/esphome-vork-lite) (the ESPHome component), v0.1.0. It needs no `wireguard:` block after all: the device opens vØrk's outer link itself (to a node found by mDNS, else the bootstrap node), makes its own identity, is linked with a one-time code in its log, and its native API is announced by the owner's nodes on their local networks, so Home Assistant discovers it and gives it its API key.
- **Depends on:** S1, M2.

## M6 Performance

- **Goal:** the CPU stops being the limit on ordinary hardware.
- **Scope:** batched socket calls (`sendmmsg`/`recvmmsg`, UDP GSO/GRO on Linux), buffer reuse without per-packet allocation, several threads (sharded by neighbor); the bench as a regression test.
- **Done when:** at least 2× the M0 numbers on the server and on the 2-core VM, measured with `scripts/bench-local.sh` and between machines.
- **Storage (measured 2026-11-18, [technical overview](technical-overview.md#storage-over-vørk-2026-11-18)):** 200 MiB PC ← server took 45 s over vØrk (4 links); 16 links halve it; the stream alone does ~200 Mbit/s. Since then placement pushes to every holder in parallel, in large batches with several in flight. Next: fewer copies between storage and the node (the local API carries every byte twice).
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

## M9 Local API and destinations — done

- **Goal:** applications use the machine's node: named services of an identity, sessions to them and requests over them, with no node of their own.
- **Done:** identities and destinations (`SHA-256(SHA-256(name)[0..10] ‖ address)[0..16]`), announces of several destinations per node with app data, path requests for destinations; sessions to a destination with an anonymous opener that can identify, and `request(path, body) → response` up to 32 MiB, reliable and flow-controlled over the session's paths (32 MiB through a relay at ~200 Mbit/s); [the local API](06-local-api.md) and its client crate `crates/vork-api` (an application on one node answered 2 MB requests from one on another, through a relay). Tested on Linux and Windows (named pipes).
- **Storage on vØrk (done 2026-11-18):** vØrk storage (formerly retifs) runs on this with no Reticulum code (ADR-0015, ADR-0022): `vorkd` attaches to the machine's node, registers `vork.node`, `vork.library` and `vork.keyring`, and comes back by itself when the node restarts. Tested across a relay: `put` on one node and `get` on another, `replicate` with a lease and its signed receipt, a library made on one node and joined from the other. Since 2026-11-23 the format carries vØrk's names (ADR-0015 addendum; retifs libraries come over with `vork-import-retifs`), storage's pages are in the dashboard (ADR-0032), and Windows has its own installer. Next: installers for Linux, Docker, Haiku and Android.
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

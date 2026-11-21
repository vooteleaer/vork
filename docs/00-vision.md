# vØrk: vision

## What it is

A network where you reach a person, a device or a service by its cryptographic identity, wherever it is, with no central server anywhere: no coordination server, no directory, no relay operator. Any node can carry traffic for others. It takes the good ideas of Reticulum (addresses are hashes of keys, no source addresses, everything encrypted, any node may be hostile) and puts them on plain IP, where they can be fast.

It is meant to be:

- **Serverless.** Every node runs the same code. A node that happens to have a public address and bandwidth ends up relaying more, but nothing is configured to be special, and the network keeps working when any node disappears.
- **At least as private as Reticulum.** Packets carry no source address. Your IP address is seen only by your direct neighbors. Multi-hop paths for stronger privacy where it matters.
- **Fast.** WireGuard does the cryptography; packets travel over UDP; relays only swap a label. Hundreds of Mbit/s on ordinary hardware, more with batching.
- **Usable by existing software.** Ordinary IP applications (SSH, SMB, browsers, Home Assistant) reach peers by identity through an overlay IPv6 address. Native applications use identities and named services directly.
- **Small enough for microcontrollers.** An ESP32 running ESPHome joins as a Lite node using its stock WireGuard component.

## Principles

1. **Identity, not location.** An address is the hash of keys. IP addresses only move packets between neighbors; inside the network nothing is addressed by IP.
2. **Proof, not trust.** Identities, announces and permissions are signatures that anyone checks locally. Where proof is impossible (whether a relay really forwards), a node measures and routes around; it never falls back on a trusted party.
3. **Every node may be hostile.** A hostile node can delay, drop and observe metadata. It can't forge, read or impersonate. Trusted nodes (your own, your island's) make things faster or more private, but correctness never depends on them.
4. **No roles.** Relaying, address reflection: properties a node has (and others measure), not roles anyone assigns.
5. **The device is hardware; you are your key.** Identities move between machines; a person's identity lives on a hardware key.
6. **Proto first.** Every step is a working, measured prototype before the next one is built on it.

## Layers

| Layer | What | Status |
|---|---|---|
| L3 Applications | native API (identity + service) and overlay IPv6 for existing software | in use: the TUN interface, `.vork` names, IPv4, exit nodes, the local API |
| L2 Islands | closed groups inside the open network | proposed (an owner's devices are one group today, ADR-0030) |
| L0 Network | identities, links, announces, paths, relaying, NAT traversal | prototype, in daily use; v0.3.1 released |

## Non-goals

- **Radio links** (LoRa, packet radio). vØrk is IP-only; Reticulum remains the network for radio. A bridge between the two is possible later because both address by key hash.
- **Resistance to a global passive adversary.** No low-latency network can have strong anonymity, low overhead and low latency at once (the anonymity trilemma). vØrk aims at I2P/Tor-class privacy, not mixnet-class.
- **A global naming system.** Names are petnames kept by each user, not a registry.
- **Exit to the ordinary internet** as a core feature. If it comes, it is a service like any other.

## Relation to other work

- **Reticulum:** the model for identity-addressing and hostile-node thinking. vØrk fixes its asymmetric-link problem (links count only when proven both ways), uses IP-sized packets, and needs no configured interfaces or transport nodes.
- **Tailscale, Nebula, ZeroTier:** the same NAT traversal and relaying techniques, without their coordination servers.
- **Yggdrasil, cjdns:** addresses from keys and overlay IPv6, without privacy-oriented paths.
- **I2P, Tor:** unidirectional/multi-hop paths and label switching, for the strong privacy class.

# vØrk: open questions

Decisions still to make. Each names who it blocks.

## Project

1. **Repository and licence.** Where does it live (GitHub `vooteleaer/vork`?), and which licence? `Cargo.toml` says MIT as a placeholder. Blocks: publishing.
2. **Name in technical places.** `vork` for crate, binary, service and DNS; `vØrk` for prose. Is that the line?

## Protocol

3. **Announce sequence numbers.** v0 uses the clock (ms). Devices without a clock need a stored counter. Spec v1: "strictly increasing per identity", with the clock as one way to get it? Blocks: M1, S1.
4. **Address length.** 16 bytes (128 bits, like RNS). Enough against collisions and grinding for addresses; is it enough for short names (see 9)? Blocks: M12.
5. **The inner layer on Lite nodes.** Full WireGuard, or a lighter end-to-end layer if S1 shows the stock libraries can't do it? Blocks: M12.
6. **The layer port (4799) and link addresses (10.89.0.1/2).** Keep, or pick something registered/less likely to clash? Blocks: M12.
7. **Default number of paths.** 2 through different first hops. Should it depend on the privacy class or the traffic (bulk vs interactive)? Blocks: M2.

## Applications and names

8. ~~**The DNS domain.**~~ Decided: `.vork`, because it isn't registered and can't be mistaken for anything else (ADR-0018).
9. **Short names.** Should a unique prefix of an address work (`0e5cee8a.vork`), or only full addresses and petnames? A prefix can be ground by an attacker to collide; a long prefix (16+ hex digits) makes that expensive. Blocks: M3.
10. **Services: names or ports.** Native API uses names; IP uses ports. How does a name map to a port: per-node config, a convention (hash of the name), or SRV records? Blocks: M3, M9.
11. **TUN on Windows.** wintun ships a signed driver in a DLL; the Windows service needs admin rights once. Acceptable? Blocks: M2.

## Network policy

12. **Open by default?** A new node relays for strangers (within quotas) unless told not to. Or relay only for own/island identities by default? Blocks: M4.
13. **Direct links inside islands by default?** Outside islands they stay opt-in. Blocks: M8.
14. **The public backbone.** cafa-cloud as the first always-on public node: under whose name, and with which limits? Blocks: M1.

## Identities

15. **Person identity without a YubiKey.** A software key in the OS keychain as a fallback, or YubiKey only? Blocks: M7.
16. **How long delegations last** (device keys signed by a person's identity): minutes with constant renewal while the YubiKey is in, or days? Blocks: M7.

## Dependencies

17. **boringtun.** 0.7.1 works but sees little maintenance; forks exist. Stay, fork, or move to another implementation before v1? Blocks: M6, M12.
18. **retifs coupling.** vØrk must run without retifs (L0 first). Should islands (M8) wait for retifs, or start with standalone signed invitation chains and move into retifs later? Blocks: M8.

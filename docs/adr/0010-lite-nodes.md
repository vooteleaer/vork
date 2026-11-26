# ADR-0010: Lite nodes

- **Status:** accepted
- **Date:** 2026-11-17
- **Irreversible:** no

## Context
Microcontrollers and battery devices can't relay, can't take announce floods and have only their platform's WireGuard.

## Decision
A Lite node announces with the relay flag cleared. It never relays, receives no floods, asks for the paths it needs (`PATH_REQ`), and accepts only paths that end at it. Nobody routes through a Lite node unless it is the destination.

## Consequences
- An ESP32 can join with ESPHome's stock WireGuard (S1/M5 to confirm the inner layer).
- Lite nodes depend on full neighbors for reachability.

## Addendum (2026-11-26)
The first Lite node is an ESP32 ([vork-lite](https://github.com/vooteleaer/vork-lite), with an [ESPHome component](https://github.com/vooteleaer/esphome-vork-lite)). ESPHome's WireGuard turned out not to be needed: the device opens the outer link to one full node itself with its own key, and its inner layer is a WireGuard responder of its own. It announces `FLAG_NO_LINKING` (an owner links it on their side, with a one-time code from its log) and names its services when the owner's devices ask for its paths; their nodes announce those on their local networks for it.

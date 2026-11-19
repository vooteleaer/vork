# ADR-0024: Devices are linked with both sides agreeing

- **Status:** accepted (built 2026-11-18)
- **Date:** 2026-11-18
- **Irreversible:** no

## Context
A node's `devices` are the owner's other devices: they get direct links (ADR-0011) and are listed as the owner's private network, as a tailnet is. Adding one by hand on one side only made a one-sided claim; picking one from a list of every node heard was both too wide (an open network has many) and too easy to get wrong.

## Decision
1. **A link needs both sides.** Node A asks node B (`link`); B's owner allows or dismisses it (the app shows a dialog; on a server `vork requests`, `vork allow`, `vork dismiss`). Only then does each list the other in `devices`. Either side may unlink; the other is told and drops it too. If both asked each other, they are linked.
2. **Pairing codes, as Resilio's links.** A node hands out a code (`link_code`, shown as a QR code and as a `vork-link:ADDRESS?c=CODE&n=NAME` link): valid for a while (10 minutes by default, at most 30 days), for one device or any number, linking at once or still asking the owner each time. A request carrying a valid code that doesn't ask links at once: showing the code was the owner's consent. A code's uses are counted; an expired or used-up code is an ordinary request.
3. **The app pairs by code alone.** One device shows its code; the other scans it (Google's code scanner on Android) and is linked. There is no list of nodes to pick from. Without a camera (a server), the code is printed in the terminal to be scanned, or its link pasted.
4. **Messages** are requests to the built-in path `/vork/link` (`{"op": "request"|"accept"|"dismiss"|"remove", "name", "code"}`) in a session where the sender identified, so the receiver knows who it is. Unanswered messages wait in an outbox (`links.json`) and are sent again every 20 s until the other side answers.

## Consequences
- A node can't make itself someone's device; a leaked code works only until it expires or is used.
- The owner's network is the set of mutual links; whether linking is transitive (a device of my device is mine) is still open. (It is since ADR-0030: the owner's devices are one group.)
- Storage's keyring is another list of the owner's devices; joining the two is next.

## Addendum (2026-11-19): links carry where to dial
A link (and its QR code) also carries `a=HOST:PORT,…`: the addresses other nodes proved they reach this node at from outside (ADR-0023), the router's mapping, and its LAN address, at most four. The scanning node asks them for the key right away, so two nodes with no node in common connect from the scan alone; the code still decides whether they link.

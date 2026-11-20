# ADR-0030: A device of my device is mine: one owner, one group

- **Status:** accepted (built 2026-11-19)
- **Date:** 2026-11-19
- **Irreversible:** no

## Context
ADR-0024 links two devices with both sides agreeing and left open whether linking is transitive. It wasn't: a laptop and a phone each linked to the home server didn't see each other until they were linked too, and every new device had to be linked with every other. In Tailscale, too, a device joins the owner's network once and sees all of it.

## Decision
1. **The owner's devices are one group.** Linking a device to any device of the group links it to all of them; unlinking it on any device unlinks it everywhere. The consent of ADR-0024 is still given once, between the new device and the one it's linked with.
2. **Each device keeps the group** (in `links.json`): for every device, when it was last linked and last unlinked (Unix seconds). The later of the two holds; a tie is unlinked. Linking or unlinking here sets "now", and always beats what's known, even if another device's clock ran ahead.
3. **Devices tell each other** (`/vork/devices`, in a session where the sender identified): the whole group, to each device until it has taken this revision, and again every 30 minutes. Only a node's own devices are heard; what they say about the receiver and the sender themselves is ignored, and times more than 5 minutes ahead of the receiver's clock are held back.
4. **Merging is per device, the later time wins**, so the group converges whatever order the messages come in, and a device that hadn't heard of an unlink can't bring the unlinked device back.
5. **An unlinked device leaves the whole group.** It's told (`remove`, ADR-0024) by the device it was unlinked on, and drops all of the owner's devices; the others drop it as they hear the group.
6. **`vork.toml` stays the list in force.** At start the group is made to agree with its `devices` (edited by hand, or a node from before groups: its devices count as linked long ago).

## Consequences
- Pairing a new device once is enough; the three devices of the example see each other within seconds of the second link.
- A device in the group can bring any node in: trusting a device now means trusting what it links. It already could reach the owner's services; now it can add to them too. Unlinking it anywhere undoes what it added only if those are unlinked too.
- Clock skew decides only races between a link and an unlink of the same device a few minutes apart.
- Unlink tombstones are kept; a group has at most a few hundred entries (512 taken from a message).
- Nodes from before this decision ignore `/vork/devices`; they are linked to the group's new devices only once they run a build that has it.

## Addendum: the map of the owner's network

The dashboard's map shows only the owner's devices and how they reach each other. Every 30 s a node asks each device, in a session where it identified, how that device reaches the others (`/vork/paths`): directly, or through which neighbor and in how many hops. A device answers only the owner's devices. The map draws a relay as a small grey point and, when a path has more hops than the relay, the nodes between that aren't known as one "?" point. A device that doesn't answer yet (an older build) is drawn as this node sees it.

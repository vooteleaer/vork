# ADR-0014: Islands: closed groups inside an open network

- **Status:** proposed
- **Date:** 2026-11-17
- **Irreversible:** no

## Context
The open network gives reachability; groups of people need privacy and trust among themselves (family, a company, friends) without a controller.

## Decision
An island is a group whose membership is a chain of signed invitations from its founder, with signed, monotonic revocations. Members' island announces are encrypted to the island and stored under a rotating key, so outsiders can't see who belongs. Island policy can allow direct links and prefer members as relays. Later the membership can live in a storage library (ADR-0015).

## Consequences
- No coordinator; leaving and revoking are signatures.
- One IP address still links a member's identities across islands unless relays are used.

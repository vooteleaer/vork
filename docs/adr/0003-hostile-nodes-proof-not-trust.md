# ADR-0003: Every node may be hostile; proof instead of trust

- **Status:** accepted
- **Date:** 2026-11-18
- **Irreversible:** no

## Context
An open network will contain nodes that lie, drop traffic and watch. Designs that trust a coordinator or the nodes on a path inherit their failures.

## Decision
The invariant: a hostile node can only delay, drop and observe metadata; it can't forge, read or impersonate. Everything that can be proven is a signature checked locally (identities, announces, permissions, membership). Where proof is impossible (whether a relay forwards), nodes measure, keep alternatives and route around; they never fall back on trusting a party. Trusted nodes (own, island) may make paths faster or more private, never more correct.

## Consequences
- Every feature needs an answer to "what if this node lies?"; the threat model (docs/03) lists them.
- Some attacks are only made expensive, not impossible (Sybil, timing correlation).

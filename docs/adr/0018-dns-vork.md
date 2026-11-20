# ADR-0018: Names: the .vork DNS domain

- **Status:** accepted
- **Date:** 2026-11-18
- **Irreversible:** no

## Context
Applications use names, and addresses are 32 hex digits. A global registry would be a central party.

## Decision
The node answers DNS for **`.vork`**: `<address>.vork` resolves to the overlay IPv6 address (AAAA), petnames (`nas.vork`, from a local file) resolve the same way, and PTR maps overlay addresses back. The OS sends only `.vork` queries to the node (systemd-resolved routing domain, Windows NRPT, macOS `/etc/resolver/vork`). `.vork` was chosen because it isn't a registered top-level domain, so it can't be mistaken for anything else.

## Consequences
- Names without a registry; everything else in DNS is untouched.
- If `.vork` were ever delegated publicly, names under it would clash; considered unlikely and accepted.
- Short prefixes of addresses are open question 9.

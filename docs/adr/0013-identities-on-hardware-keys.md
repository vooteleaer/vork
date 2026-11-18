# ADR-0013: Identities on hardware keys; devices are delegated

- **Status:** proposed
- **Date:** 2026-11-17
- **Irreversible:** yes

## Context
"The device is hardware, you are your key." A person's identity should move between machines, survive a stolen laptop, and not live in files.

## Decision
A person's identity lives on a YubiKey (PIV, Ed25519/X25519, firmware 5.7+), imported from a seed kept offline. While the key is plugged in, it signs a short-lived session key for the machine; pulling it wipes the session key and the identity leaves that machine. Servers and services have their own identities (on a Nano YubiKey, TPM-sealed, or a file) delegated by their owner, with epochs so that the newest placement wins when an identity moves. A machine also has an anonymous L0 identity, so it can relay without its owner's key.

## Consequences
- Moving an identity is moving a key; a stolen machine holds nothing lasting.
- The inner layer's static key becomes a delegated key; certificates travel inside the session.
- Open questions 15–16.

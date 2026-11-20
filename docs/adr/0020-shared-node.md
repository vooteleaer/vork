# ADR-0020: One shared node per machine

- **Status:** proposed
- **Date:** 2026-11-18
- **Irreversible:** no (but the IPC protocol becomes part of the spec)

## Context
Several applications on one machine (a TUN interface, a messenger, a service) all need the network. Each running its own node would mean several sockets, several sets of neighbors and paths, and identities that can't share a machine's links. Reticulum solves this with a *shared instance*: the first program starts the transport, later ones attach to it through a local socket. Its weak spot showed in practice: the RPC between a client and a shared instance of a different implementation (Python `rnstatus` against Rust `prnsd`) didn't match, because it wasn't specified.

## Decision
- **One vØrk node per machine.** Applications attach to it over local IPC (a Unix socket; a named pipe on Windows).
- **The first application starts it.** If no shared node is running, a program starts one itself and later programs attach to that. The system service (M1) is the same node started at boot.
- **Many identities in one node.** An application registers its identity and services over IPC; the node announces them, opens paths and carries their sessions. The machine's own L0 identity, a person's identity (ADR-0013) and service identities share the node's links.
- **Keys stay with the application that owns them** where the platform allows: the node asks the application (or its hardware key) to sign and to do its handshakes, and never lets one application use another's identity.
- **Access is limited** by socket permissions (a `vork` group on Unix, an ACL on the named pipe).
- **The IPC protocol is specified and versioned** like the wire format, so clients and nodes in different languages and versions work together. It covers status, peers, `whois`, registering identities and services, sessions and the native message API (ADR-0017).
- **One TUN interface per machine**, owned by the shared node.
- **Embedded mode** (M10) remains for programs that can't reach or start a shared node.

## Consequences
- Applications get the network without each running a node; identities share links and relays.
- The node becomes a local privilege boundary: socket permissions and identity separation matter.
- One more protocol to specify and test (IPC), part of M3 and of v1 (M12).

# vØrk: the local API

How applications on a machine use the machine's node (ADR-0020) instead of running their own. Implemented in `crates/vork-api` (protocol and async Rust client) and `src/api.rs` (the node's side). Version 1, prototype.

## Endpoint

- Unix: `api.sock` in the node's state directory (`/var/lib/vork/api.sock` for the system node, `~/.local/state/vork/api.sock` for a user's). Mode 0666: any local program may use the network; acting as an identity needs that identity's secret.
- Windows: the named pipe `\\.\pipe\vork-api-<UDP port>` (`vork-api-51820` by default).
- Applications look at `$VORK_API` first, then the system node, then the user's (`vork_api::default_endpoint`).

## Frames

Each frame is `length[4] kind[1] body`, big-endian, at most 32 MiB + 4 KiB. Strings are `len[1] bytes`. `known` is `public[64]` (X25519 ‖ Ed25519) `name_hash[10]` `hops[1]` `app_len[2]` `app_data`.

Application → node:

| Kind | Frame | Answer |
|---|---|---|
| 1 | `HELLO version[2]` | `HELLO` |
| 2 | `REGISTER req[4] secret[64] name` — host destination `name` of the identity with this secret | `REGISTERED` |
| 3 | `ANNOUNCE dest[16] app_data` — announce a registered destination now and from now on | — |
| 4 | `RECALL req[4] dest[16]` — what is known, without asking the network | `KNOWN` |
| 5 | `FIND req[4] dest[16] timeout_ms[4]` — ask for a path and wait | `KNOWN` |
| 6 | `OPEN req[4] dest[16]` — a session, opened anonymously | `OPENED` or `ERROR` |
| 7 | `IDENTIFY session[4] as_dest[16]` — prove to the far end we are the identity of our destination | — |
| 8 | `REQUEST req[4] session[4] path body` | `RESPONSE` or `ERROR` |
| 9 | `CLOSE session[4]` | — |
| 10 | `RESPOND token[8] status[1] body` — answer an `INCOMING` or a `PAGE` | — |
| 11 | `SUBSCRIBE` — pass on every announce heard | `HEARD`s |
| 12 | `DEVICES` `hash[16]…` — the owner's other devices (destinations or identities); sessions with them get direct links (ADR-0011). Replaces the client's earlier list; dropped when it disconnects | — |
| 13 | `PAGES id title count[1]` then `count` × `path title` — pages this application adds to the node's dashboard (ADR-0032); none: forgotten | — |

Node → application:

| Kind | Frame |
|---|---|
| 1 | `HELLO version[2] address[16]` (the node's own address) |
| 2 | `REGISTERED req[4] dest[16]` |
| 3 | `KNOWN req[4] found[1] (known)` |
| 4 | `OPENED req[4] session[4] nonce[32]` |
| 5 | `RESPONSE req[4] status[1] body` |
| 6 | `ERROR req[4] message` |
| 7 | `HEARD dest[16] known` |
| 8 | `INCOMING token[8] dest[16] session[4] nonce[32] identified[1] (requester[16]) path body` |
| 9 | `CLOSED session[4]` |
| 10 | `PAGE token[8] id path body` — the dashboard asks for one of this application's pages, or a call a page makes; answer with `RESPOND`, status 0 and the content |

Status: 0 ok, 1 no such path, 2 refused. A destination's registration ends with its application's connection.

## Pages in the dashboard

An application may add pages to the node's dashboard (ADR-0032), in its menu under the application's title, on the web page, in the vØrk window and in the Android app alike. `id` is 1–32 of `a-z 0-9 -`; a path starts with `/`; at most 8 pages and 16 applications. The node keeps them in `apps.json` in its state directory: they stay in the menu after the application disconnects or the node restarts, shown as not running, until the application says them again (or says none), or the owner removes them.

A page's content is the HTML the application answers to `PAGE` for its path. The dashboard shows it in a sandboxed frame of its own origin with the dashboard's colors as CSS variables (`--bg`, `--panel`, `--text`, `--dim`, `--accent`, `--line`…), and gives its scripts:
- `vork.call(path, body)`: a `PAGE` for that path with that body (a string, or JSON), answered as `{status, body}`;
- `vork.go(path)`: shows another of the application's pages.

The frame can't reach the dashboard, its key or the node: only its own application, through `vork.call`. The dashboard waits 15 s for an answer.

`cargo run -p vork-api --example pages -- ENDPOINT` adds two pages and answers a call.

## Example

`cargo run -p vork-api --example api_demo -- ENDPOINT serve NAME` hosts a destination with a fresh identity and answers `/echo`; `... ask DEST BYTES [NAME]` finds it, opens a session (identifying as its own destination `NAME` if given) and sends three requests.

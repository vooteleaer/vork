# vØrk in Docker

One image, `vooteleaer/vork`, two containers in one compose file:

| Container | What | Volume |
|---|---|---|
| `vork-node` | the vØrk node (`vork run`); owns UDP port 51820 | `node` → `/var/lib/vork` |
| `vork` | vorkd, the attribute extractor, the web UI (port 8741) | `vork` → `/data/store`, and `node` |

```sh
docker compose up -d
docker logs vork          # the web UI's pairing link
```

Then open `http://<this machine>:8741/` with the link. The network page
shows the node and its connections, and adds and removes them: a connection
is another node's `host:port` (its key is asked for on first contact), written
to the node's `vork.toml` and dialed at once.

The node writes its configuration and identity at the first start
(`/var/lib/vork/vork.toml`, `state/node.key`); its sockets (`state/api.sock`,
`state/control.sock`) are how the storage container reaches it. vorkd
reconnects by itself when the node restarts.

Both containers use the host's network (the node's UDP port is the host's;
forward it on the router for nodes outside the LAN to reach this one) and run
as user 65532, so the shared volume's files are theirs.

## Settings

In `compose.yaml`, service `vork`:

- `DEVICE`: this device's name for keep rules (default: the host's name).
- `EXTRACT_NS`: libraries whose files get attributes (`photos,music`).
- `VORK_WEB_LISTEN`, `VORK_WEB_ALLOW`, `VORK_WEB_PAIRING`: see ADR-0037.

The node: `peers`, `listen` and the rest in the volume's `vork.toml`
(`docker compose restart node` after editing it).

## Building

`build.sh` stages the binaries (built beforehand for amd64 and arm64:
`vork vorkd vork-storage vork-extractd vork-web`, all from this repository,
e.g. with `cargo zigbuild --target x86_64-unknown-linux-gnu.2.31`) and runs
`docker buildx build` for both platforms; `--push` publishes the image.

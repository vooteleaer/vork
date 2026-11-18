# vØrk storage in Docker

Two images, one compose file:

| Image | What | Volume |
|---|---|---|
| `vooteleaer/prnsd` | Reticulum: prnsd, the shared instance; owns the interfaces | `reticulum` → `/var/lib/prnsd` |
| `vooteleaer/vork` | vorkd, the attribute extractor, the web UI (port 8741) | `vork` → `/data/store`, and `reticulum` |

```sh
docker compose up -d
docker logs vork          # the web UI's pairing link
```

Then open `http://<this machine>:8741/` with the link. The page **Reticulum**
shows Reticulum's state and the traffic per interface, switches connections
on and off, adds and removes them, and edits the whole configuration file.
Changes are applied at once: prnsd runs with `--service`, and the vØrk storage
container, which sees its volume, asks it to reload through prnsd's control
files there (`prnsd interfaces apply`). A configuration Reticulum refuses is
put back as it was.

vØrk storage never starts Reticulum. Without the `reticulum` service, it joins any
shared instance on the host (`127.0.0.1:37428`): Prns's or Python's `rnsd`
(the Reticulum page then shows state and edits the file, applied at
Reticulum's next start).

Both containers use the host's network (AutoInterface needs the LAN; added
listeners need no port mapping) and run as user 65532, so the shared volume's
files are theirs.

## Settings

In `compose.yaml`, service `vork`:

- `DEVICE`: this device's name for keep rules (default: the host's name).
- `EXTRACT_NS`: libraries whose files get attributes (`photos,music`).
- `VORK_WEB_LISTEN`, `VORK_WEB_ALLOW`, `VORK_WEB_PAIRING`: see ADR-0037.

Radios: map the device into `reticulum` (`devices:`, `group_add: ["20"]`) and
add the interface on the Reticulum page, by its `/dev/serial/by-id/…` link.

## Building

`build.sh` stages the binaries (built beforehand for amd64 and arm64: vØrk storage'
`vorkd vork vork-extractd`, vork-desktop's `vork-web`, and Prns's
`prnsd` with `--features tokio-host,observability`) and runs
`docker buildx build` for both platforms; `--push` publishes them.

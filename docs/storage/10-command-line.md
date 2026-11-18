# Using vØrk storage from the command line

Everything the apps do goes through `vorkd`'s local API ([native API](07-api.md)); the `vork` command is the same API on a terminal. `vork` with no arguments prints every command.

## Installing

vØrk storage runs on Linux, Windows, Android and Haiku. The daemon, `vorkd`, attaches to the machine's vØrk node through its local API (`/var/lib/vork/api.sock` for the system node, else `~/.local/state/vork/api.sock`; `VORK_API` names another), and reconnects by itself when the node restarts. Every package below brings the node.

**Debian, Ubuntu, Raspberry Pi OS (amd64, arm64):** install a `vork_*.deb` (built by `packaging/deb/build-deb.sh`) with `sudo apt install ./vork_*.deb`. It runs the vØrk node as a system service (`vork status`) and starts `vorkd` in every user's session; see `/usr/share/doc/vork/README.Debian`. It needs glibc 2.31 or newer: Debian 11, Ubuntu 20.04 and 22.04, elementary OS 6 and 7, Raspberry Pi OS bullseye and later. The GTK apps (vork-desktop, Photos, Map, Contacts, Calendar) as .deb need GTK 4.12 and libadwaita 1.4 (Debian 13, Ubuntu 24.04); on older systems use their Flatpaks.

**Windows 10 and 11:** run `vork-setup-*.exe` (built by `packaging\windows\build-setup.ps1`). It installs for the current user, no administrator needed: the programs in `%LOCALAPPDATA%\Programs\vork`, `vork` on the PATH, and a utility in the notification area, started at login, which starts the vØrk node and `vorkd` on it. Uninstall from Apps & features; your data stays.

**From source:**

```sh
cargo install --path .                     # vork: the vØrk node (vork init, vork run)
cargo install --path crates/vork-storage-node      # vorkd and the vork CLI
cargo install --path crates/vork-fuse      # vork-mount (needs fuse3)
cargo install --path crates/vork-gateway   # vork-gateway
cargo install --path crates/vork-extract   # vork-extractd
cargo install --path crates/vork-cloudfiles   # on Windows: vork-cloudfiles
cp packaging/systemd/*.service ~/.config/systemd/user/
systemctl --user enable --now vorkd
```

**Settings** live in `vork.toml` in the store (`~/.local/share/vork/` on Linux, `%LOCALAPPDATA%\vork\` on Windows, `~/config/var/vork` on Haiku), made with every setting commented out on first start: disk budget, device name, how often to announce, how much to hold for others, and so on. Flags given to `vorkd` override the file.

## Records, attributes and queries

```sh
vork rec add photos album="Prangli 2026" --file boat.jpg
vork query photos 'album=Prangli 2026'
vork rec history photos <id>
vork query '*' type=image/jpeg --save Photos   # saved in your library me: every device has it
vork saved run Photos --watch                  # live: again whenever it changes
```

## As a file tree

On Linux and Windows alike:

```sh
vork mount photos            # ~/vork/photos (or give a folder)
vork mount                   # what's mounted where
vork unmount photos
vork mount photos --via home # a library your home node holds, nothing of it kept here
```

(For `--via`, the home node admits this one first: `vork app add laptop --ns photos --write --device <this node>`; `home` is a contact or a node address.)

- **Linux:** a FUSE mount. vorkd mounts every library by itself at `~/vork/<library>` (the user's own store: not with `--store` or `--no-mounts`); `vork unmount NAME` takes one down until vorkd starts again. Attributes are `user.retifs.*` xattrs; `.query/<terms>/` lists matches (`ls ~/vork/photos/.query/album=Prangli\ 2026/`); `systemctl --user enable --now vork-mount@photos` mounts it at login.
- **Windows:** a folder in Explorer. Files are fetched when opened, "Always keep on this device" keeps them here, **Share with vork…** is in the context menu, and a library you may only read is a read-only folder. `vork unmount photos --forget` also removes it from Explorer.
- **Haiku:** each library is a volume in `~/vork`, with its attributes as Haiku attributes and live queries in Tracker.
- Photos in albums also show under `Albums/<album>/`: the same files, and deleting one there only takes it out of the album.

As websites, one origin per library, read-only: `systemctl --user enable --now vork-gateway`, then open <http://localhost:8740/>.

## Attributes from what's in files

As BeOS gave files theirs when they arrived, `vork-extractd` watches your libraries and describes new content, whatever route it came in by: `mime` and `type`, a photo's size, `camera`, `taken` and `geo`, a recording's `audio.artist`, `audio.title` and `duration`, a document's `doc.title`, `doc.author` and `doc.pages`. Then `vork query photos has:camera` or `type=audio/mpeg audio.artist=Bach` finds them. It's an app that may write only those keys:

```sh
vork app add extract --ns photos,music --write \
  --keys mime,width,height,taken,camera,geo,duration,'exif.*','audio.*','doc.*','block:*',@content,type,extracted \
  > ~/.config/vork/extract.token
systemctl --user enable --now vork-extractd
```

An app in a container of its own (Music Assistant, say) needs only the app socket, not the store: `vorkd --app-socket /srv/vork-apps/apps.sock` (or `app_socket` under `[node]` in `vork.toml`) opens a second one there, and that folder alone is mounted into the container. The store's own `apps.sock` stays as it was.

One of your nodes running it is enough ([ADR-0023](adr/0023-attribute-extraction.md)).

## Sharing

**A whole library** with another device or person:
1. `vork library link photos` prints a `retifs:ns/…` link (the address and your node). Whoever has it can read.
2. On their side, `vork open <link>` joins it.
3. To let them write too, give them an invitation instead: `vork invite create photos` prints a one-use `retifs:invite/…` link; `vork open <link>` joins and gets the grant in one step (or as soon as your node is reachable).

**Part of a library** (an album, a folder): `vork share create prangli --from photos 'album=Prangli 2026' --omit geo` makes a library `prangli` holding just those records (without `geo`; shares leave location out by default anyway, `--keep geo` keeps it), kept up to date by your nodes, and prints its link. `vork share list photos` and `share rm` list and end shares.

**One file**, outside any library: `vork put FILE --hint` stores it and prints its read-cap with your node as a hint (`retifs:1:…@<node>`); `vork get <read-cap>` on another device fetches it from the hinted node, or any it knows holds it.

**Contacts:** `vork me link --name Alice` gives a link others add you with (`vork open <link>`), and `vork me` shows six words to compare in person. Contacts live in your library `me`, and their names work in place of node addresses (`--from Alice`).

## Copies and devices

Every file has three copies by default, yours and two on nearby nodes (encrypted). `vork library copies photos 4` sets a library's own number; `vork nodes` shows the storage nodes this one knows. Give each node its site (`site = "prangli-home"` in `vork.toml`) and one copy is always kept at another site. A node keeps everything of the libraries it made. Of a library it joined, it keeps the records, and fetches content as it's read, keeping it by tags and rules: tag what it should keep (`keep:<device>=true`, or a keep-rule record). `vork library keep photos all` keeps all of one library, and `keep_joined = "all"` in `vork.toml` all of every library joined (a NAS, a backup node).

## When something goes wrong

Each node keeps its last output in `<store>/crash/log`. It writes a report when it panics and when it finds that its last run didn't stop cleanly. Join a library named `vork-crashlogs` with a write grant on each of your devices, and the reports go there as `/<device>/<time>-<kind>-<source>.txt`, readable from any of them ([ADR-0036](adr/0036-crash-reports.md)). Crashes of the Android apps go there too.

- `vork report "Music stopped while playing"` adds your own report, with the daemon's last output.
- `vork query vork-crashlogs device=tablet` lists one device's reports.
- `crash_library = "..."` in `vork.toml` names another library; `""` keeps reports on the device.

## Leaving and deleting

- `vork library leave photos` removes a library from this device; others keep theirs. One made on this device needs `--mine`: if no other device has it, it's gone.
- `vork library delete photos --yes` deletes it on every member's device ([ADR-0034](adr/0034-deleting-a-library.md)): only a member who may invite others can.

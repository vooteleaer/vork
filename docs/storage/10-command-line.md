# Using vØrk storage from the command line

Everything the dashboard's storage pages do goes through `vorkd`'s local API ([native API](07-api.md)); the `vork-storage` command is the same API on a terminal. `vork-storage` with no arguments prints every command. The vØrk node's own `vork` command passes storage commands on to it, so `vork put FILE` is `vork-storage put FILE` (`vork help-storage` lists them); `vork status` on its own is the network node's.

## Installing

vØrk storage runs on Linux, Windows, Android and Haiku, on the machine's vØrk node: [install vØrk](../install.md) first. The daemon, `vorkd`, attaches to the node through its local API (`/var/lib/vork/api.sock` for the system node, else `~/.local/state/vork/api.sock`; a named pipe on Windows; `VORK_API` names another), and reconnects by itself when the node restarts.

**Windows 10 and 11:** run `vork-storage-<version>.msi` (see [installing vØrk storage](../install.md#installing-vørk-storage)). It installs for the current user, no administrator needed: the programs in `%LOCALAPPDATA%\Programs\vork-storage`, on the PATH, and `vork-storage-run` started at login, which keeps `vorkd`, its dashboard pages (`vork-web --dashboard`) and the Explorer folders (`vork-cloudfiles --all`) running. Uninstall from Apps & features; your store stays.

**Linux (`.deb`), Docker, Haiku, Android:** installers are coming. Until then, from source.

**From source:**

```sh
cargo install --path crates/vork-storage-node   # vorkd, vork-storage, vork-storage-run, vork-import-retifs
cargo install --path clients/web                # vork-web: storage's pages in the vØrk dashboard
cargo install --path crates/vork-fuse           # on Linux: vork-mount (needs fuse3)
cargo install --path crates/vork-gateway        # vork-gateway
cargo install --path crates/vork-extract        # vork-extractd
cargo install --path crates/vork-cloudfiles     # on Windows: vork-cloudfiles
vork-storage-run                                # vorkd and vork-web --dashboard (and vork-cloudfiles on Windows), kept running
```

`vork-storage-run` looks for the programs beside itself and keeps their output in `<store>/logs`; `vork-storage-run --stop` stops them (vorkd closes its store cleanly). Start it at login the way your system does (a systemd user service, a desktop autostart entry).

**The store** is `%LOCALAPPDATA%\vork-storage` on Windows, `~/.local/share/vork-storage` on Linux (`$XDG_DATA_HOME/vork-storage` when set), `~/config/var/vork-storage` on Haiku; `VORK_STORE` or `--store DIR` names another.

**Settings** live in `vork.toml` in the store, made with every setting commented out on first start: disk budget, device name, how often to announce, how much to hold for others, and so on. Flags given to `vorkd` override the file. The dashboard's settings page changes the same file.

**From retifs:** `vork-import-retifs` copies libraries kept by a retifs daemon into vØrk storage, encrypting their content again with vØrk storage's keys. It only reads from retifs (run it against a copy of the store), and keeps its progress in a file, so a stopped import goes on where it was:

```sh
vork-import-retifs --from RETIFS_SOCKET --to VORK_STORE --list               # retifs's libraries
vork-import-retifs --from RETIFS_SOCKET --to VORK_STORE --import [LIBRARY]... # all, or the ones named
vork-import-retifs --from RETIFS_SOCKET --to VORK_STORE --verify             # compares what was imported
vork-import-retifs --from RETIFS_SOCKET --export LIBRARY --out DIR           # a library as plain files, by their paths
```

## Records, attributes and queries

```sh
vork-storage rec add photos album="Prangli 2026" --file boat.jpg
vork-storage query photos 'album=Prangli 2026'
vork-storage rec history photos <id>
vork-storage query '*' type=image/jpeg --save Photos   # saved in your library me: every device has it
vork-storage saved run Photos --watch                  # live: again whenever it changes
```

## As a file tree

On Linux and Windows alike:

```sh
vork-storage mount photos    # ~/vork/photos (or give a folder)
vork-storage mount           # what's mounted where
vork-storage unmount photos
vork-storage mount photos --via home # a library your home node holds, nothing of it kept here
```

(For `--via`, the home node admits this one first: `vork-storage app add laptop --library photos --write --device <this node>`; `home` is a contact or a node address.)

- **Linux:** a FUSE mount. vorkd mounts every library by itself at `~/vork/<library>` (the user's own store: not with `--store` or `--no-mounts`); `vork-storage unmount NAME` takes one down until vorkd starts again. Attributes are `user.vork.*` xattrs; `.query/<terms>/` lists matches (`ls ~/vork/photos/.query/album=Prangli\ 2026/`). `vork-storage mount photos DIR` mounts one elsewhere. `vorkd --mounts DIR` mounts every library in DIR instead, also for a given store, open to other users and shown as DIR's owner's files: a NAS's storage container uses it to give other containers the libraries (`packaging/docker/storage.yaml`). `vork-mount --allow-other --owner UID:GID` does the same for one.
- **Windows:** a folder in Explorer. Files are fetched when opened, "Always keep on this device" keeps them here, **Share with vork…** is in the context menu, and a library you may only read is a read-only folder. `vork-storage unmount photos --forget` also removes it from Explorer. The folders are `vork-cloudfiles --all [FOLDER]`'s (what `vork-storage-run` starts; `%USERPROFILE%\vork` unless told), `vork-cloudfiles --wait SECONDS` sets how long opening a file waits for content (default 30), and `vork-cloudfiles --unregister --all` takes every library out of Explorer, its data kept.
- **Haiku:** each library is a volume in `~/vork`, with its attributes as Haiku attributes and live queries in Tracker.
- Photos in albums also show under `Albums/<album>/`: the same files, and deleting one there only takes it out of the album.

As websites, one origin per library, read-only: run `vork-gateway`, then open <http://localhost:8740/>.

## The dashboard pages

`vork-web --dashboard` adds storage's pages (Libraries, Announces, Status, Settings, Reports) to the machine's vØrk dashboard over the node's local API (`VORK_API`, else the system node's), and comes back when the node restarts. It serves nothing itself: the dashboard decides who sees them, its owner ([ADR-0032](../adr/0032-application-pages.md)). `vork-storage-run` starts it.

Without a vØrk dashboard to join, `vork-web` serves the same pages itself ([ADR-0037](adr/0037-web-ui.md)): `--listen ADDR:PORT` (default `127.0.0.1:8741`), `--allow CIDR,…` (default loopback only), `--host NAME,…` for names it may be opened by, and `--pairing` to let a browser in only by a pairing link from its log (`vork-web --pair-link` prints it again).

## Attributes from what's in files

As BeOS gave files theirs when they arrived, `vork-extractd` watches your libraries and describes new content, whatever route it came in by: `mime` and `type`, a photo's size, `camera`, `taken` and `geo`, a recording's `audio.artist`, `audio.title` and `duration`, a document's `doc.title`, `doc.author` and `doc.pages`. Then `vork-storage query photos has:camera` or `type=audio/mpeg audio.artist=Bach` finds them. It's an app that may write only those keys:

```sh
vork-storage app add extract --library photos,music --write \
  --keys mime,width,height,taken,camera,geo,duration,'exif.*','audio.*','doc.*','block:*',@content,type,extracted \
  > ~/.config/vork/extract.token
vork-extractd --token-file ~/.config/vork/extract.token
```

An app in a container of its own (Music Assistant, say) needs only the app socket, not the store: `vorkd --app-socket /srv/vork-apps/apps.sock` (or `app_socket` under `[node]` in `vork.toml`) opens a second one there, and that folder alone is mounted into the container. The store's own `apps.sock` stays as it was.

One of your nodes running it is enough ([ADR-0023](adr/0023-attribute-extraction.md)).

## Sharing

**A whole library** with another device or person:
1. `vork-storage library link photos` prints a `vork:ns/…` link (the address and your node). Whoever has it can read.
2. On their side, `vork-storage open <link>` joins it.
3. To let them write too, give them an invitation instead: `vork-storage invite create photos` prints a one-use `vork:invite/…` link; `vork-storage open <link>` joins and gets the grant in one step (or as soon as your node is reachable).

**Part of a library** (an album, a folder): `vork-storage share create prangli --from photos 'album=Prangli 2026' --omit geo` makes a library `prangli` holding just those records (without `geo`; shares leave location out by default anyway, `--keep geo` keeps it), kept up to date by your nodes, and prints its link. `vork-storage share list photos` and `share rm` list and end shares.

**One file**, outside any library: `vork-storage put FILE --hint` stores it and prints its read-cap with your node as a hint (`vork:1:…@<node>`); `vork-storage get <read-cap>` on another device fetches it from the hinted node, or any it knows holds it.

**Contacts:** `vork-storage me link --name Alice` gives a link others add you with (`vork-storage open <link>`), and `vork-storage me` shows six words to compare in person. Contacts live in your library `me`, and their names work in place of node addresses (`--from Alice`).

## Copies and devices

Every file has three copies by default, yours and two on nearby nodes (encrypted). `vork-storage library copies photos 4` sets a library's own number; `vork-storage nodes` shows the storage nodes this one knows. Give each node its site (`site = "prangli-home"` in `vork.toml`) and one copy is always kept at another site. A node keeps everything of the libraries it made. Of a library it joined, it keeps the records, and fetches content as it's read, keeping it by tags and rules: tag what it should keep (`keep:<device>=true`, or a keep-rule record). `vork-storage library keep photos all` keeps all of one library, and `keep_joined = "all"` in `vork.toml` all of every library joined (a NAS, a backup node).

## When something goes wrong

Each node keeps its last output in `<store>/crash/log`. It writes a report when it panics and when it finds that its last run didn't stop cleanly. Join a library named `vork-crashlogs` with a write grant on each of your devices, and the reports go there as `/<device>/<time>-<kind>-<source>.txt`, readable from any of them ([ADR-0036](adr/0036-crash-reports.md)). Crashes of the Android apps go there too.

- `vork-storage report "Music stopped while playing"` adds your own report, with the daemon's last output.
- `vork-storage query vork-crashlogs device=tablet` lists one device's reports.
- `crash_library = "..."` in `vork.toml` names another library; `""` keeps reports on the device.

## Leaving and deleting

- `vork-storage library leave photos` removes a library from this device; others keep theirs. One made on this device needs `--mine`: if no other device has it, it's gone.
- `vork-storage library delete photos --yes` deletes it on every member's device ([ADR-0034](adr/0034-deleting-a-library.md)): only a member who may invite others can.

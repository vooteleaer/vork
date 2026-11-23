# vØrk storage for Haiku

The status and settings utility with the Haiku API: this device (vØrk storage,
its storage, libraries, libraries heard on the network), the network
(the vØrk node's connections) and settings, as on Linux, Windows and Android.

- `capi/`: the shared core (`../core`) as a C library, JSON in and out
  (`vork_desktop.h`).
- `app/`: the BeAPI app over it; `BJson` reads the answers into BMessages,
  and the core's blocking calls run in a looper of their own.
- `launch/vork-storage`: vØrk storage (`x-vnd.vork-storage-daemon`,
  `vork-storage-run`, which keeps vorkd and vork-web running) as a
  launch_daemon job after the vØrk node's (`x-vnd.vork-node`, from the vork
  package). The utility starts, stops and restarts both with `launch_roster`.
- The Deskbar replicant (in the app): the shared status icon in the tray, its
  state from a thread of its own, a menu with the state, Open, Restart and
  Remove. The app adds it when it starts (`vork --deskbar` only adds it);
  Deskbar keeps it across restarts. The app is linked with `-rdynamic` and
  `$ORIGIN/lib`, so Deskbar can load it and find the core next to it.

## Building, on Haiku

    pkgman install rust_bin gcc haiku_devel
    clients/haiku/build.sh

in this repository (`cargo build --release -p vork-storage-node -p vork-web`
first). The result is an app folder, `clients/haiku/build/vork`: `vork` (the
app), `lib/libvork_desktop.so`, and `bin/` with `vorkd`, `vork-web`,
`vork-storage` and `vork-storage-run` when they are built.

vorkd needs the same patched crates as the node (mio, if-addrs; see
`packaging/haiku/package.sh`) until their releases have the fixes.

## The package

    clients/haiku/build.sh && clients/haiku/package.sh
    pkgman install clients/haiku/build/vork_storage-<version>-x86_64.hpkg

A system package, `vork_storage`, which requires `vork` (vØrk the network,
`packaging/haiku`): the app in `/boot/system/apps/vork-storage` (with `lib/`
and `bin/`), the `vork-storage` command on the PATH, the jobs in
`data/user_launch/vork-storage` (at every login, and one that adds the
Deskbar icon), the userlandfs module (`vork-haikufs`) as
`add-ons/userlandfs/vork`, and the app in the Deskbar's Applications menu.
Every library is a volume in `~/vork/<library>`: vorkd mounts them once
the desktop is up, a library joined later within seconds, and unmounts one
left. Not at login from a launch job: a userlandfs mount there blocked the
user's launch_daemon (no desktop).

The store is `~/config/var/vork-storage` (libvork's default on Haiku).

# vØrk storage for Haiku

The status and settings utility with the Haiku API: this device (vØrk storage,
its storage, libraries, libraries heard on the network), the network
(Reticulum's connections) and settings, as on Linux, Windows and Android.

- `capi/`: the shared core (`../core`) as a C library, JSON in and out
  (`vork_desktop.h`).
- `app/`: the BeAPI app over it; `BJson` reads the answers into BMessages,
  and the core's blocking calls run in a looper of their own.
- `launch/vork`: vØrk storage as a launch_daemon job (`x-vnd.retifs-daemon`),
  after Reticulum: the Prns package's prnsd (`x-vnd.prns-prnsd`, with Prns's
  own app and Deskbar icon; prnsd-haiku's `haiku/`). The utility starts,
  stops and restarts both with `launch_roster`, or (right after installing,
  before the launch_daemon has read the jobs at a login) runs them itself.
- The Deskbar replicant (in the app): the shared status icon in the tray, its
  state from a thread of its own, a menu with the state, Open, Restart and
  Remove. The app adds it when it starts (`vork --deskbar` only adds it);
  Deskbar keeps it across restarts. The app is linked with `-rdynamic` and
  `$ORIGIN/lib`, so Deskbar can load it and find the core next to it.

## Building, on Haiku

    pkgman install rust_bin gcc haiku_devel
    haiku/build.sh

with vØrk storage next to vork-desktop. The result is an app folder,
`haiku/build/vork`: `vork` (the app), `lib/libvork_desktop.so`, and
`bin/` with vorkd and the `vork` command when they are built. prnsd is
the Prns package's (prnsd-haiku: `haiku/build.sh && haiku/package.sh`).

vorkd needs mio with Haiku's accept fix (tokio-rs/mio#2036) until
mio has it in a release: prnsd-haiku carries it; vorkd is built with
`--config 'patch.crates-io.mio.path="…/prnsd-haiku/third_party/mio"'`.

## The package

    haiku/build.sh && haiku/package.sh
    pkgman install haiku/build/vork-<version>-x86_64.hpkg

A system package: the app in `/boot/system/apps/vork` (with `lib/` and `bin/`:
vorkd, vØrk storage), the `vork` command on the PATH, the jobs in
`data/user_launch/vork` (at every login, and one that adds the Deskbar
icon), the userlandfs module (`vork-haikufs`, built in
vØrk storage) as `add-ons/userlandfs/vork`, and the app in the Deskbar's
Applications menu. It requires the `prns` package (prnsd, Reticulum).
Every library is a volume in `~/vork/<library>`: vorkd mounts them once
the desktop is up, a library joined later within seconds, and unmounts one
left. Not at login from a launch job: a userlandfs mount there blocked the
user's launch_daemon (no desktop).

The store is `~/config/var/vork` (libvork' default on Haiku).

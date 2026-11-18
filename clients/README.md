# vork-desktop

A desktop management utility for [vØrk storage](https://github.com/vooteleaer/retifs), in the spirit of the Nextcloud desktop client: a tray icon and a window to see and manage what vØrk storage holds, where, and who shares it. Native widgets on each platform over a shared Rust core on `libvork`: GTK 4 / libadwaita on Linux (`gtk/`), Win32 on Windows (`windows/`), BeAPI on Haiku (`haiku/`), Compose on Android (`android/`, where the app also runs `vorkd`), and a web page for nodes without a screen (`web/`, `vork-web`: a NAS or a container; reachable only from the networks its installer allows, optionally only by browsers paired with a link from its log, vØrk storage ADR-0037).

> A demo app of [vØrk storage](https://github.com/vooteleaer/retifs), part of its test release. To try it, start with [getting started](https://github.com/vooteleaer/retifs/blob/main/docs/storage/getting-started.md).

It manages vØrk storage (this node, storage, libraries, sharing, devices); browsing content is left to apps.

Status: milestones 1 and 2 on Linux (overview; each library's members, sharing address, what this device keeps). Plan: [docs/plan.md](../docs/clients/plan.md).

## Build and run (Linux)
Needs GTK 4 and libadwaita development files (`libgtk-4-dev libadwaita-1-dev` on Debian) and a running `vorkd`.

    cargo run -p vork-desktop-gtk

It talks to the daemon of the default store (`$VORK_STORE`, else the platform default). `core/` builds and tests everywhere (`cargo test`).

## License

[MIT](../LICENSE)

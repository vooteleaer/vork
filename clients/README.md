# Clients

- [`music-assistant/`](music-assistant): a Music Assistant provider for a vØrk storage library.
- [`android/`](android): the vØrk app for Android (`android/net`): the node, its dashboard, a VPN for the phone's apps. Built with `android/build-apk.ps1`.
- [`net-desktop/`](net-desktop): `vork-net`, the vØrk window and tray icon for Windows and Linux: the node's dashboard in a window of its own.

Both show the same dashboard as the node's web page (`src/web/index.html`).

- [`web/`](web): `vork-web`, vØrk storage's pages. With `--dashboard` they appear in the vØrk dashboard ([ADR-0032](../docs/adr/0032-application-pages.md)); without it, it serves them itself to a browser ([vØrk storage ADR-0037](../docs/storage/adr/0037-web-ui.md)). Released in vØrk storage's installer.

The other folders (`core/`, `gtk/`, `windows/`, `haiku/`, `android/app`) hold experimental apps, among them the earlier desktop utilities for vØrk storage ([plan](../docs/clients/plan.md), [platforms](../docs/clients/platforms.md)); they aren't part of the releases.

## License

[MIT](../LICENSE)

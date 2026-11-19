# vork-net: the desktop window

The node's own dashboard (`src/web/index.html`, the page `web =` serves) in a window of its own, with a tray icon, for Windows and Linux. It is the same page as in the browser, so whatever the dashboard learns shows up in both.

- **No port, no certificate, no key.** The page's `/api/*` requests don't go over the network: the window hands them straight to the machine's node through its control endpoint (the named pipe or Unix socket `vork status` uses). The node's `web =` setting can stay off.
- **Tray icon.** Closing the window keeps vork-net in the tray. A left click or *Open vØrk* brings the window back; *Quit* ends it. `--hidden` starts it in the tray, for starting with the session.
- **No web view?** Where the system has none (Haiku, or a Linux without WebKitGTK), or with `--browser`, vork-net serves the page on `127.0.0.1` with a random key and opens it in the browser. Only that link's key reaches the API, and only under the host name `127.0.0.1`.

```
vork-net [--config FILE] [--browser] [--hidden]
```

It finds the node the way `vork` does: `--config` or `$VORK_CONFIG`, else the system node's configuration (`%ProgramData%\vork\vork.toml`, `/etc/vork/vork.toml`), else the user's own.

## Who may change things

- **Windows:** the users signed in at the computer may manage the node (the control pipe's ACL), so the window can do everything.
- **Linux:** root and the node's own user may manage it. Anyone else can see only its status, until the node's configuration names them:

  ```toml
  operators = ["alice"]
  ```

## Building

It is its own Cargo workspace with its own lock file: the web view's GTK 3 dependencies hold some shared crates back, and the node shouldn't follow them.

```sh
cargo build --release --manifest-path clients/net-desktop/Cargo.toml
# without the window (Haiku): add --no-default-features
```

- **Windows:** uses WebView2, which comes with Windows 10 and 11.
- **Linux:** needs WebKitGTK 4.1 and an app indicator library (Debian: `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev libxdo-dev`).

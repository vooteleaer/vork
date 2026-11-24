# vØrk storage on every platform

One vØrk storage (the same Rust daemon, `vorkd`) on the vØrk node of each device, and one shared core (`clients/core/`), with a native front end per platform (the user's choice, 2026-10-21: no webview). This is where each platform stands and what's next.

**Now (2026-11-23).** The native front ends gave way to one: storage's pages in the vØrk dashboard (`vork-web --dashboard`, [vØrk ADR-0032](../adr/0032-application-pages.md)), the same on the web, in the vØrk window and in the Android app. Native code stays where the platform needs it: file systems (FUSE, Cloud Files, userlandfs) and background services. Each platform gets its own storage installer, apart from vØrk's:

| | Storage installer | Files |
|---|---|---|
| **Windows** | `vork-storage-<version>.msi`, per user; `vork-storage-run` from login | Cloud Files, `%USERPROFILE%\vork` |
| **Linux** | `.deb`: coming | FUSE mount, `~/vork` |
| **Docker** | coming | none (`vorkd --no-mounts`) |
| **Haiku** | hpkg: coming | userlandfs volumes in `~/vork` |
| **Android** | APK: coming (the storage app beside the vØrk app) | DocumentsProvider planned |

The table and sections below are the history up to then.

| | Storage | Network | Utility | Files | Packaging |
|---|---|---|---|---|---|
| **Linux** | runs (amd64, arm64) | the vØrk node: a system service (.deb), or a user unit (`packaging/systemd/vork.service`) | GTK 4 / libadwaita, tray (StatusNotifierItem) | FUSE mount | .deb (vork amd64+arm64, utility amd64), Flatpak (utility) |
| **Windows** | runs | the vØrk node, started and kept running by the utility | Win32, notification area | Cloud Files | vork-setup.exe (per user) |
| **Android** | runs (phone, arm64; emulator, x86_64) | the vØrk node inside the app (`libvork.so`, run by the foreground service) | Kotlin + Compose, foreground service | — (DocumentsProvider planned) | debug APK (`clients/android/build-apk.ps1`) |
| **Haiku** | runs (x86_64, native build) | the vØrk node, a launch_daemon job of the vork package | Haiku API (BeAPI) window, Deskbar replicant | userlandfs module (vork-haikufs): attributes, queries | hpkg (`clients/haiku/package.sh`), system package |

**Off Reticulum (2026-11-18).** Every client now shows the vØrk node and its connections instead of Reticulum's: the node's address and uptime, the connections from `peers` in its `vork.toml` (another node's `host:port`, its key asked for on first contact, or `host:port@key`) and the neighbors that found it. Adding one writes it to `vork.toml` and dials it at once through the control socket. No package needs Prns or a shared instance any more. Not yet tried on a device: the Android app's node (the node and core type-check for `aarch64-linux-android`; the JNI crate still takes the retifs Contacts and Calendar cores and is built outside the workspace), the Haiku package, and the Flatpak's access to the host node. The sections below are the history up to then.

## Linux
Done: the `vork` .deb for amd64 and arm64 (glibc 2.36+: Debian 12, Ubuntu 22.04, Raspberry Pi OS bookworm), vorkd started once Reticulum (Prns, installed apart) is up; the utility's .deb for amd64 (GTK 4.12+, libadwaita 1.4+: Debian 13, Ubuntu 24.04), in the tray from login.

Next:
- The utility for arm64 (a Raspberry Pi desktop): the same .deb, cross-built against arm64 GTK.
- Libraries and Find exist only here; Windows gets them next, then Android.
- Other distributions: an .rpm from the same script (Fedora), later an AUR recipe. A Flatpak for the utility only (it needs `--filesystem=xdg-data/vork` for the daemon's socket, and systemctl through the portal), after the native packages.
- GNOME shows no tray icon without the AppIndicator extension: the utility then works as a window, which it does now.

## Windows
Done: the utility (Win32, Common Controls 6, per-monitor DPI) with Overview, Network and Settings, the notification-area icon, and starting vØrk storage at login; vork-setup.exe per user, no administrator.

Next: Libraries and Find (as on Linux); a signed installer (SmartScreen warns about unsigned ones); the service state should name the store a running vorkd serves, not just that one runs.

## Android
**Runs (2026-10-30)**, in an Android 14 emulator (x86_64) with the app from `android/`: the foreground service starts the bundled prnsd and vorkd; a TCP connection added in the app connects to the testbed; the testbed's public libraries are heard, and Join adds one, fetched as needed. `vorkd`, `vork` and `prnsd` (feature `tokio-cloud-host`: TCP, no serial or Bluetooth) build unchanged with NDK 27 for `aarch64-linux-android` and `x86_64-linux-android` (API 26). Not yet on a real phone.

What it took:
- **The shared instance on TCP:** the app writes prnsd's first config with `shared_instance_type = tcp` (port 37428). On Android prnsd listens on the abstract socket by default while Prns clients look for TCP (to report to Prns).
- **prnsd run by the app** (`prnsd run`) can't apply interface changes live (`--apply` needs a managed daemon): the core restarts it instead (`VORK_PRNSD_APPLY=restart`), and vorkd with it, as a Prns client doesn't attach to a new shared instance by itself (to report to Prns).
- **Phone defaults**, set once and changeable in Settings: nothing held for others, 1 GB kept free (the desktop default, 5 % between 5 and 50 GB, could leave a phone with less free space no room at all), the device named after the model.
- **On the other end:** a prnsd interface a phone connects to needs `--ingress-control false`, or the phone's announces are held back.

Next: an arm64 build on a real phone; Find and libraries' contents; the DocumentsProvider; battery (below).

The plan it follows:

Plan, one app, "vork":
- **Kotlin and Jetpack Compose** (Material 3), the platform's own toolkit.
- **A foreground service** runs vorkd as an executable shipped as a native library (`libvorkd.so` in `jniLibs`, run from `nativeLibraryDir`: since Android 10 an app may not execute files it wrote, but may execute its native libraries), started at boot and after an update. Its notification is Android's tray: the same states, Open and Stop.
- **Reticulum (2026-11-04):** never bundled. Prns's own app (`org.personal.hopspot`, the `wip/android` branch: Wi-Fi, Bluetooth LE, USB, RNode over BLE) is the phone's Reticulum; vorkd joins its shared instance on localhost TCP 37428 once it's up (only the port: no RPC key), and so does any other app's (Columba, Sideband). Connections are set up in Prns; the Network tab says whether Reticulum runs and opens Prns.
- **The UI talks to the core through JNI:** `core/` built as a library for Android with a thin JNI layer returning JSON (status, Reticulum, settings, libraries, find). The same calls as the GTK and Win32 front ends, no second protocol implementation in Kotlin.
- **A phone is a small node:** storage-less or a small store limit by default (ADR-0031), holding nothing for others unless the user says so; keep rules decide what stays offline.
- **Files:** a `DocumentsProvider` (Storage Access Framework) shows libraries in the Files app and every app's file picker, Android's counterpart of FUSE and Cloud Files. After the utility.
- **Open questions:** battery (Doze ends background networking: sync in windows, or when charging), the foreground service type Android 14+ accepts for this (`dataSync` is time-limited from Android 15; `connectedDevice` or `specialUse`), a multicast lock for Reticulum's AutoInterface on Wi-Fi.
- **Later:** the long-term Android ROM with vØrk storage for everything (separate project).

## Haiku
Haiku's own way, not Linux's: settings in `~/config/settings/vork`, launch_daemon, hpkg, the Haiku API for UI, attributes and live queries as Haiku does them.

- **Reticulum:** the prns-haiku project (prnsd port, Network preferences add-on, Deskbar replicant). It cross-checks for `x86_64-unknown-haiku` with one patch; not linked or run yet.
- **vorkd runs (2026-10-30)** on Haiku nightly hrev60299 (x86_64, a QEMU VM): built natively with HaikuPorts' Rust 1.94.1 and gcc 13 (`--ignore-rust-version`: Prns asks for 1.96 but builds), with prns-haiku's keepalive patch and no change to vØrk storage. With a shared instance reached over a tunnel it attached, heard the testbed's public libraries, joined one lazily, and held 1.2 GB that other nodes placed on it within 20 minutes: the store (redb, slabs, file locking, mmap), tokio and mio, and Reticulum links in both directions work. Not yet: files in Tracker (userlandfs, the Haiku API) and prnsd itself.
- **prnsd runs on Haiku (2026-10-30)** from a local fork of Prns, `prnsd-haiku`: branch `haiku` on the Prns release, a few small commits each behind `cfg(target_os = "haiku")`, meant for upstream (`HAIKU.md` there). mDNS LAN discovery left out (no `IP_PKTINFO` on Haiku); if-addrs and mio patched (mio assumed an accepted socket inherits non-blocking mode on Haiku, it does not, and the runtime stopped at the first local client); lifecycle locks through `flock`. A Haiku node now runs on its own: prnsd connected to the testbed, vorkd on it, a library heard, joined and searched. Next: RNode over USB serial. vorkd on Haiku needs the same mio fix (its local API), until mio upstream has it.
- **Files in Tracker (2026-10-31):** vork-haikufs (in vØrk storage) mounts a library as a volume through userlandfs: records as files, their attributes as Haiku attributes (Tracker columns by MIME type), queries in Haiku's language including live ones (Tracker's Find). Read-only so far.
- **The utility (2026-10-31)**, `haiku/`: a Haiku API window over the shared core through a C library (`haiku/capi`, JSON in and out, the same calls as Android's JNI layer, now `core::api`). This device, Network and Settings work against vorkd and prnsd running as launch_daemon jobs (`x-vnd.vork-storage-daemon`, `x-vnd.retifs-prnsd`, started and stopped with `launch_roster`). The store is `~/config/var/vork`. The Deskbar replicant shows the shared status icon with a menu (state, Open, Restart, Remove), and the hpkg installs it all: app, programs, the `vork` command, launch jobs at login, the Deskbar icon and the Applications menu entry (tested: a clean install and reboot brings everything up).
- **Files: userlandfs**, natively (ADR-0002): records with their attributes as Haiku attributes, indexes and live queries in Tracker. Haiku is where vØrk storage's model fits best: BeFS already has records with attributes and queries.
- **Utility:** a BeAPI app (C++) over the core through a C interface (cbindgen), or Rust bindings to the Haiku API; a Deskbar replicant for the state.
- **Test machine:** a Haiku nightly VM in QEMU (KVM, in WSL here), driven headless; real hardware later.

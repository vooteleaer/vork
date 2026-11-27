# Installing vØrk

This guide takes you from nothing to your devices reaching each other by name. It takes about ten minutes per device. vØrk is an early prototype: keep all your devices on the same version, and don't rely on it for anything important yet.

**Contents:** [which file](#which-file) · [one reachable device](#before-you-start-one-reachable-device) · [Windows](#windows) · [Android](#android) · [Debian, Ubuntu, Raspberry Pi](#debian-ubuntu-raspberry-pi) · [any Linux](#any-linux-the-node-alone) · [Docker and NAS](#docker-and-nas-truenas-scale) · [Home Assistant](#home-assistant) · [Haiku](#haiku) · [linking your devices](#link-your-devices) · [exit node](#use-an-exit-node) · **[the dashboard in a browser and its key](#the-dashboard-in-a-browser-its-key)** · [updates](#updates) · [vØrk storage](#installing-vørk-storage) · [when something doesn't work](#when-something-doesnt-work)

## Which file

Downloads are on the [releases page](https://github.com/vooteleaer/vork/releases). Take the newest release, and the file for each device from the table below.

| Device | File | Section |
|---|---|---|
| Windows 10/11 | `vork-<version>.msi` | [Windows](#windows) |
| Android, Android TV | `vork-net-<version>.apk` | [Android](#android) |
| Debian 12, Ubuntu 22.04 or newer, elementary OS 7, Linux Mint 21 (PC) | `vork_<version>_amd64.deb` | [Debian, Ubuntu, Raspberry Pi](#debian-ubuntu-raspberry-pi) |
| Raspberry Pi OS 64-bit (bookworm), Debian or Ubuntu on ARM64 | `vork_<version>_arm64.deb` | [Debian, Ubuntu, Raspberry Pi](#debian-ubuntu-raspberry-pi) |
| Any other Linux, or a server without a screen | `vork-linux-x86_64`, `vork-linux-aarch64` | [Any Linux](#any-linux-the-node-alone) |
| A NAS or server with Docker (TrueNAS SCALE) | `packaging/docker/node.yaml` | [Docker and NAS](#docker-and-nas-truenas-scale) |
| Home Assistant OS or Supervised | the add-on repository | [Home Assistant](#home-assistant) |
| Haiku | `vork-<version>-1-x86_64.hpkg` | [Haiku](#haiku) |

vØrk storage, an application on vØrk, has installers of its own: see [installing vØrk storage](#installing-vørk-storage).

Each file's SHA-256 is in the release's `SHA256SUMS`: `sha256sum -c SHA256SUMS --ignore-missing` on Linux, `Get-FileHash <file>` on Windows.

## Before you start: one reachable device

Your devices find each other through vØrk nodes. At least one of them is easiest **reachable from the internet**: a home server, a NAS, an always-on PC. Each install picks a free UDP port and asks your router to open it (UPnP, NAT-PMP, PCP), which most home routers do. The dashboard's **Settings → Network** shows the port and whether the router opened it; if it didn't, forward that UDP port on the router to the device by hand, or choose a port there yourself.

Even with no port open anywhere, devices still reach each other: what can't connect directly goes through relays.

## Windows

1. Run `vork-<version>.msi`. Windows may say it protected your PC (the installer isn't code-signed yet): **More info → Run anyway**. Accept the licence; Windows asks for administrator rights.
2. The node is installed as a service and starts with Windows. The **vØrk** window opens at the end of the install, and its icon stays in the notification area (it starts at login).
3. Go on to [linking your devices](#link-your-devices).

To update, run a newer MSI over it. To remove it: **Settings → Apps → vØrk → Uninstall**. The node's identity and settings stay in `C:\ProgramData\vork` (delete the folder to forget them).

## Android

1. Open `vork-net-<version>.apk` on the phone (allow installing from your browser or file manager when asked).
2. Open **vØrk**. The node starts and keeps running in the background.
3. To let your apps reach your devices (names like `laptop.vork`), turn on the switch on the dashboard: Android asks to allow a VPN connection once.
4. Go on to [linking your devices](#link-your-devices): the app scans pairing codes with the camera.

To update, install a newer APK over it.

**Android TV** (a TV box such as the Mi Box, or a TV running Android TV): the same APK, from 0.3.0 on (it carries 32-bit ARM too, which most TV boxes run).

- Install it from a USB stick with a file manager, or from a computer with `adb install vork-net-<version>.apk` (turn on developer options and network debugging on the TV first). **vØrk** then appears among the TV's apps.
- Move around the dashboard with the remote's arrows; the middle button presses.
- Link it by showing a code on the TV (**Devices → Show a code**) and scanning it with your phone's vØrk app; typing a link with a remote is no fun.
- Some TV boxes (Mi Box) have no screen to allow a VPN on, so the switch is refused at once. Allow it once from a computer, then turn the switch on again:

  ```
  adb shell appops set org.vork.net ACTIVATE_VPN allow
  ```

## Debian, Ubuntu, Raspberry Pi

Download the `.deb` for your machine (`amd64` for a PC, `arm64` for a Raspberry Pi with a 64-bit system), then:

```sh
sudo apt install ./vork_<version>_<arch>.deb
```

This installs the node as a system service (`vork status` shows it) with its network interface on, and the **vØrk** window in the applications menu, which also starts in the tray at login. The user who installed it may manage the node from the window (`operators` in `/etc/vork/vork.toml`). If the window shows the node but can't change anything ("only the node's own user (or root) may do this"), add your user there, `operators = ["you"]`, and run `sudo systemctl restart vork`; installing or upgrading the package with `sudo apt install` adds the user who ran it while no one is named.

On a Raspberry Pi or server without a screen the window isn't needed: the `.deb` still brings its libraries. To do without them, use the [bare node](#any-linux-the-node-alone) instead. To see the dashboard in a browser on another machine, see [the dashboard in a browser and its key](#the-dashboard-in-a-browser-its-key).

> Packages of 0.2.0 still listen on UDP 51820. If another device behind the same router already uses it, give this one another port: `sudo sed -i 's|^listen = .*|listen = "0.0.0.0:47821"|' /etc/vork/vork.toml && sudo systemctl restart vork`. Later versions pick a free port by themselves.

To update, install a newer `.deb` the same way. To remove it: `sudo apt remove vork` (the identity in `/etc/vork` and `/var/lib/vork` stays).

## Any Linux: the node alone

One static file with no dependencies, for any Linux, old ones too (systemd for the service). As root:

```sh
install -m 755 vork-linux-x86_64 /usr/local/bin/vork      # or vork-linux-aarch64
vork init --tun vork0 --port auto     # /etc/vork/vork.toml and this machine's identity
curl -fsSLo /etc/systemd/system/vork.service https://raw.githubusercontent.com/vooteleaer/vork/main/packaging/vork.service
systemctl daemon-reload && systemctl enable --now vork
vork status
```

`--tun vork0` turns on the network interface, so programs here reach vØrk addresses and `.vork` names; `--port auto` picks a free UDP port. For the dashboard in a browser, see [the dashboard in a browser and its key](#the-dashboard-in-a-browser-its-key).

## Docker and NAS (TrueNAS SCALE)

[`packaging/docker/node.yaml`](../packaging/docker/node.yaml) runs the node in one small container on the host's network, with its interface, so the NAS's apps become reachable from your devices at `nas-name.vork:<port>`.

1. Make a folder (a dataset on TrueNAS) for the node's identity, and put its path in place of `/mnt/tank/apps/vork` in the file.
2. On TrueNAS SCALE: **Apps → Discover → ⋮ → Install via YAML**, and paste the file. Elsewhere: `docker compose -f node.yaml up -d`.
3. The app's **Web UI** button opens the dashboard, which asks for its key the first time: see [the dashboard in a browser and its key](#the-dashboard-in-a-browser-its-key).
4. Go on to [linking your devices](#link-your-devices), from the dashboard.

## Home Assistant

1. **Settings → Add-ons → Add-on store → ⋮ → Repositories**, add `https://github.com/vooteleaer/vork-home-assistant`.
2. Install **vØrk** and start it. Turn on **Show in sidebar** if you like.
3. **Open Web UI** opens the dashboard inside Home Assistant. Go on to [linking your devices](#link-your-devices).

Your devices then reach Home Assistant at `http://homeassistant.vork:8123`. The add-on's **Documentation** tab has the rest.

## Haiku

```sh
pkgman install ./vork-<version>-1-x86_64.hpkg
```

The node starts at the next login (or now: `/boot/system/apps/vork/vork-node &` in Terminal). Its dashboard is **vØrk** in the Deskbar's Applications menu. Haiku has no IPv6: the node's interface is IPv4 only, so programs there reach your devices at their `198.18.x.x` addresses (shown in the dashboard), not by `.vork` name.

## Link your devices

Linking makes two devices each other's: they reach each other's services and names, and nothing else can. Your devices are one group: a device linked to any of them is linked to all.

1. On a device that already runs vØrk, open the dashboard: **Devices → Show a code**. It shows a QR code and a link, good for 10 minutes and one device.
2. On the new device:
   - **Android:** scan the QR code from the app (**Devices → Scan**).
   - **A dashboard** (Windows, Linux, Home Assistant, NAS): **Devices**, paste the link under *Or paste another device's link*, **Link**.
   - **A terminal:** `vork link 'vork-link:…'`.
3. Both now list each other under **Devices**. Try it: `ping <name>.vork`, or open `http://<name>.vork:<port>` in a browser.

**If linking doesn't get through** (the new device is elsewhere, and the one that made the code can't be reached from outside): a new node starts from the [public nodes](../README.md#public-nodes-to-start-from) by itself, so wait half a minute and use the link again. With that turned off, give it a connection first: **Settings → Connections**, add `host:port` of one of your nodes that is reachable from the internet.

## Use an exit node

An exit node sends a device's internet traffic out through home, for when you're on someone else's Wi-Fi. The exit is any device of yours that stays at home (a home server, a Raspberry Pi, a Windows PC, an old phone on the charger): on its dashboard's **Exit node** page turn on **Offer this node as an exit** (or add `exit = true` to its `vork.toml`), and it restarts. Then, on the phone or laptop you take with you: the dashboard's **Exit node** page, **Automatic** (the fastest of your exits). Your local network stays reachable directly; if the exit stops answering, the device goes back to its own connection. A Linux laptop needs systemd-resolved (most desktops have it) for its names to go through the exit as well.

## The dashboard in a browser: its key

Where the dashboard is a window of its own (the vØrk window on Windows and Linux, the Android app) or inside Home Assistant (**Open Web UI**), there's nothing to do: it talks to the node directly. On a machine without a screen (a server, a Raspberry Pi, a NAS) the node serves the dashboard to a browser instead, and every request to it carries a **key**, so being on the same network isn't enough to change the node.

1. **Turn it on** (the Docker, TrueNAS, Home Assistant and Haiku setups already have): add `web = "0.0.0.0:8742"` to the node's `vork.toml` and restart the node. `0.0.0.0` lets any network reach the page (still only with the key); `127.0.0.1:8742` keeps it to that machine.
2. **Get the link**, which carries the key after `#t=`:

   | Where | How |
   |---|---|
   | Linux (`.deb` or bare node) | `sudo vork web-link` |
   | Windows | `vork web-link` in a terminal opened as administrator |
   | Docker, TrueNAS | `docker exec <container> /var/lib/vork/bin/vork web-link --config /var/lib/vork/vork.toml`, or the container's log (the node prints it at start) |
   | Home Assistant | the add-on's **Log** tab (for a browser outside Home Assistant) |
   | Haiku | **vØrk** in the Deskbar's Applications menu opens it |

   It prints something like `https://my-server:8742/#t=3f9c…` and the certificate's SHA-256.
3. **Open the link once.** The browser keeps the key for that address and takes it out of the address bar; from then on `https://my-server:8742/` is enough. The same dashboard at another address (its LAN IP, its `.vork` name, another browser) asks for the key again: paste the link, or just the key, into the page that asks.
4. **The certificate:** the node makes its own, so the browser warns once. Compare the SHA-256 `vork web-link` printed with the one the browser shows (the padlock or the warning page's details), then accept. With a domain of your own you can give it a real one instead: `web_cert` and `web_key` in `vork.toml` (Let's Encrypt's `fullchain.pem` and `privkey.pem`); restart the node when they're renewed.

**Keep the link private:** anyone who has it and can reach the page can change the node. If it got out, make a new key: stop the node, delete its `web.token`, start it again, and open the new link `vork web-link` prints. The old link stops working.

| Where | `web.token` |
|---|---|
| Linux | `/var/lib/vork/web.token` |
| Windows | `C:\ProgramData\vork\state\web.token` |
| Docker, TrueNAS | `state/web.token` in the node's folder |
| Home Assistant | `/data/state/web.token` in the add-on (or reinstall the add-on) |

To make a browser forget the key, clear the page's site data.

## Updates

The dashboard's **Updates** page shows the newest release and what's new in it, and offers this device's file when it's newer: install it over the old one, as above. Your settings and devices stay.

## Installing vØrk storage

[vØrk storage](storage/README.md) keeps your files on your own devices, encrypted, in sync and backed up by them. It's a separate installer per platform, and it runs on the machine's vØrk node: install vØrk first, as above. It's a prototype: use copies of your files, not the only ones.

| Device | File |
|---|---|
| Windows 10/11 | `vork-storage-<version>.msi` |
| Android | `vork-storage-<version>.apk`, beside the vØrk app |
| Debian, Ubuntu, Raspberry Pi | `vork-storage_<version>_amd64.deb` or `_arm64.deb` |
| Docker and NAS (TrueNAS SCALE) | [`packaging/docker/storage.yaml`](../packaging/docker/storage.yaml) |
| Haiku | coming |

**Windows:**

1. Run `vork-storage-<version>.msi` (Windows may warn as for vØrk's own installer: **More info → Run anyway**). It installs for you only, with no administrator rights, into `%LOCALAPPDATA%\Programs\vork-storage`. It refuses to install if vØrk isn't installed.
2. Storage starts at once, and at every login after that (`vork-storage-run` keeps it running).
3. Open the vØrk dashboard: a **Storage** section appears in its menu, with your libraries, the public libraries heard, status and settings.
4. Your libraries appear in Explorer under `%USERPROFILE%\vork`. Files that aren't on this device yet are fetched when you open them.

Next: [getting started](storage/getting-started.md) with two devices. To update, run a newer MSI over it. To remove it: **Settings → Apps → vØrk storage → Uninstall**. Your libraries leave Explorer, but the store (`%LOCALAPPDATA%\vork-storage`) and the `%USERPROFILE%\vork` folder stay.

**Android:** install `vork-storage-<version>.apk` beside the vØrk app (`vork-net`); storage runs on that app's node, so it needs no network permission or VPN of its own. Open it to join libraries; allow Contacts and Calendar there if you want your address books and calendars in Android's own apps.

**Debian, Ubuntu, Raspberry Pi:** `sudo apt install ./vork-storage_<version>_amd64.deb` (with vØrk's own package installed). Storage runs for every user who signs in (a systemd user service), its store in `~/.local/share/vork-storage`, your libraries as folders in `~/vork`. Remove it with `sudo apt remove vork-storage`; the store stays.

**Docker and NAS:** `storage.yaml` runs storage beside a node from `node.yaml` (or the vork package's): change the folders, the device's name and the disk limit at its top, then install it as the node (TrueNAS: **Apps → Discover → ⋮ → Install via YAML**). Its containers have no network of their own; they reach the node through its API socket. Join your libraries from the dashboard's **Storage** pages. Other containers (a media server, say) can have the libraries as folders: `storage.yaml` says what to uncomment (FUSE in the storage container, a shared bind mount).

On Haiku, until its package comes, storage can be built from source: [command line](storage/10-command-line.md#installing).

## When something doesn't work

- **What the node says:** the dashboard's **Log** page; on Linux `journalctl -u vork`; on Windows `C:\ProgramData\vork\state\vork.log`; in Docker the container's log.
- **Is it running, and with whom:** `vork status` (neighbors, sessions, whether others reached it from outside).
- **Devices don't reach each other:** check that both list each other under **Devices**, and that at least one of your nodes is reachable from the internet (**Settings → Network**), or add a public node as a connection.
- **A program can't reach a device by name:** try its `198.18.x.x` address from the dashboard; on Android the VPN switch must be on.

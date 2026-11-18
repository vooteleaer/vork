# Getting started: two devices and the apps

This walks you through trying vØrk storage on two of your own devices, then the apps on top of it. It takes about half an hour. Everything here is a **test release**: use copies of your photos and documents, not the only ones.

You'll need two devices: a computer (Windows or Linux) and a phone (Android), or two computers. Haiku works too.

## 1. Reticulum first

vØrk storage runs on [Reticulum](https://reticulum.network) and joins the device's Reticulum *shared instance*; it never brings its own. Any Reticulum implementation with a shared instance will do; start one on each device. Ones that have been used with vØrk storage:

- **Linux, Windows, macOS:** the reference implementation's `rnsd` (`pip install rns`), or `prnsd` from [Prns](https://github.com/KenAKAFrosty/Prns/releases/tag/v0.3.8) (Debian packages: [Prns packages for vØrk storage](https://github.com/vooteleaer/Prns/releases), a fork's build until Prns publishes its own).
- **Android:** a Reticulum app that runs a shared instance, e.g. Prns's Android app (`personal-hopspot-*.apk` from the [Prns packages for vØrk storage](https://github.com/vooteleaer/Prns/releases), a development build). Sideband or Columba may also work, but haven't been tried.
- **Haiku:** the `prns` package from the same release; it starts at login.

The two devices must reach each other over Reticulum: on the same network they find each other by themselves (AutoInterface); otherwise give both the same TCP interface to a hub in their Reticulum config.

## 2. Install vØrk storage

From the [release](https://github.com/vooteleaer/retifs/releases):

- **Windows 10/11:** run `vork-setup-*.exe`. It installs for you only (no administrator) and puts the vØrk storage utility in the notification area.
- **Debian, Ubuntu, Raspberry Pi OS, elementary OS:** `sudo apt install ./vork_*.deb` (vØrk storage itself, with your libraries as folders in `~/vork`; Debian 11, Ubuntu 20.04 and newer).
  - The apps (the utility, Photos, Map, Contacts, Calendar) on Debian 13 or Ubuntu 24.04 and newer: their `.deb`s (`vork-desktop_*.deb` and so on).
  - On older systems (elementary OS 7, Ubuntu 22.04) or any other distribution: their Flatpaks, `flatpak install --user ./org.vork.Desktop-*.flatpak` (and `org.vork.Photos`, `.Map`, `.Contacts`, `.Calendar`). They use the vØrk storage the `.deb` installed.
- **Android:** install `vork-android-*.apk` (allow installing from your browser or file manager). Open it once: it starts vØrk storage and stays running.
- **Haiku:** `pkgman install` the `prns` and `vork` packages (or open them in Tracker), then restart.

Every install also joins the library **vork-installers** by itself (nothing of it is fetched until you open it): new versions of vØrk storage and the apps arrive there, through vØrk storage. Leave it if you don't want it; it isn't joined again.

Open the utility (the tray icon, or the vØrk storage app). It should say vØrk storage is running and show this device. If it waits for Reticulum, check step 1.

## 3. Your first library

A **library** is where records live: "photos", "family", "work". Each has its own key.

1. In the utility: **New library**, name it `test`.
2. On Windows, it appears in Explorer under `C:\Users\<you>\vork\test`. On Linux, mount it: `vork mount test` (then `~/vork/test`). On Haiku it's a volume in `~/vork`.
3. Copy a few files in. They're now records, encrypted, with three copies kept as soon as other nodes are around.

## 4. The second device

1. On the first device, open the library's menu: **Invite…**, choose **Read and write**, **Make invitation**. Copy the link (or show the QR code).
2. On the second device: **Join a library** → **Join with link**, paste it.
3. In a moment the library is there too, with the same files. Change something on either side: the other follows. Turn one off, change things on both, turn it back on: both changes are kept.

Things to try:
- **Members…** shows who reads and writes. Removing someone rotates the library's key: they keep what they had, not what comes next.
- **Copy link** gives a read-only link for anyone you trust with everything in the library.
- **Leave library…** removes it from one device; **Delete everywhere…** (for those who may invite) removes it from every member's device.

## 5. The apps

The apps read the same libraries; none of them owns its data. They're demos of what vØrk storage makes possible, and their code is in their own repositories: [Photos](https://github.com/vooteleaer/retifs-photos), [Map](https://github.com/vooteleaer/retifs-map), [Contacts](https://github.com/vooteleaer/retifs-contacts), [Calendar](https://github.com/vooteleaer/retifs-calendar), and the utility itself, [vork-desktop](https://github.com/vooteleaer/retifs-desktop).

On Android, an app asks vØrk storage before it may use any library: open it, tap **Choose libraries…**, and allow it in vØrk storage (which libraries, and whether it may add to them).

### Photos

- **Android:** install `vork-photos-android-*.apk`. It backs up the phone's camera pictures into a library you choose (Backup tab), each with its thumbnails, so every device shows them at once. Long-press to select: star (favourite), **+** to put them in albums. Optionally, what's backed up leaves the phone (to its trash) and the library is where your photos are.
- **Windows, Linux, Haiku:** the photos are in the library's folder like any files, and albums under `Albums/<album>/`. On Linux and Haiku there's also the Photos app (`vork-photos` package).
- An album can be moved to another library with all its photos: open it and tap the library's name at the top.

### Map

Install `vork-map-android-*.apk` (or the `vork-map` package on Linux and Haiku). It shows everything that says where it is: your photos, contacts' addresses, events. The map itself comes from vØrk storage libraries, not the internet: in the vØrk storage app, **Announces** lists public libraries heard on the network (a map of Estonia and its places, for example); join one and the map has tiles and search.

### Contacts and Calendar

On Linux (`vork-contacts`, `vork-calendar` packages): an address book in your own library `me`, and calendars, one per library holding events. A family's calendar is a library shared with the family. On Haiku, contacts are People files in the `me` volume, and Calendar is the `vork_calendar` package.

## When something goes wrong

- Logs: Windows `%LOCALAPPDATA%\vork\logs\`; Linux `journalctl --user -u vorkd`; Android `adb logcat | grep -i vork`.
- `vork nodes` shows the other nodes this one knows; `vork library info <library>` what a library has and who's in it.
- Please report issues at <https://github.com/vooteleaer/retifs/issues>, with what you did, what you expected and what happened.

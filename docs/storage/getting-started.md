# Getting started: two devices

This walks you through trying vØrk storage on two of your own devices. It takes about half an hour. vØrk storage is a **prototype**: use copies of your photos and documents, not the only ones.

You'll need two devices that run vØrk. The storage installer is for Windows today; on Linux and Haiku, storage is built from source ([command line](10-command-line.md#installing)). Installers for Linux (`.deb`), Docker, Haiku and Android are coming.

## 1. The network

vØrk storage runs on vØrk: it uses the machine's vØrk node and needs it installed first. Install vØrk on both devices and link them, as in [installing vØrk](../install.md). Linked devices reach each other wherever they are; storage finds its other devices through the node.

## 2. Install vØrk storage

- **Windows 10/11:** run `vork-storage-<version>.msi` from the [releases page](https://github.com/vooteleaer/vork/releases). It installs for you only (no administrator) and starts storage at once and at every login. See [installing vØrk storage](../install.md#installing-vørk-storage).
- **Linux, Haiku:** build it from source and start it: [command line](10-command-line.md#installing).

Open the vØrk dashboard. A **Storage** section appears in its menu once storage runs: **Libraries**, **Announces**, **Status** and **Reports**, and its settings under the dashboard's **Settings**. **Status** should show this device.

## 3. Your first library

A **library** is where records live: "photos", "family", "work". Each has its own key.

1. In the dashboard: **Storage → Libraries**, **New library**, name it `test`.
2. On Windows, it appears in Explorer under `%USERPROFILE%\vork\test`. On Linux, vorkd mounts it at `~/vork/test`. On Haiku it's a volume in `~/vork`.
3. Copy a few files in. They're now records, encrypted, with three copies kept as soon as other nodes are around.

## 4. The second device

1. On the first device, open the library: under **Invite a device or a person**, choose **read and write**, **Make the invitation**, and copy the link.
2. On the second device: **Storage → Libraries**, **Join with link**, paste it.
3. In a moment the library is there too, with the same files. Change something on either side: the other follows. Turn one off, change things on both, turn it back on: both changes are kept.

Things to try:
- **Members** shows who reads and writes. Removing someone rotates the library's key: they keep what they had, not what comes next.
- **Read link** gives a read-only link for anyone you trust with everything in the library.
- **Leave on this device** removes it from one device; **Delete everywhere** (for those who may invite) removes it from every member's device.

## When something goes wrong

- Logs: `logs\` in the store on Windows (`%LOCALAPPDATA%\vork-storage\logs\`, one file per program); on Linux and Haiku, where you started `vorkd`.
- **Storage → Status** in the dashboard, or `vork-storage nodes`, shows the other storage nodes this one knows; `vork-storage library info <library>` what a library has and who's in it.
- **Storage → Reports** collects crash reports ([ADR-0036](adr/0036-crash-reports.md)).
- Please report issues at <https://github.com/vooteleaer/vork/issues>, with what you did, what you expected and what happened.

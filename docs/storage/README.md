# vØrk storage

> *Your files, photos and contacts on your own devices, encrypted, kept in sync and backed up by them, with no company in the middle.*

> [!IMPORTANT]
> **Installers:** Windows has its own installer, `vork-storage-<version>.msi`, on vØrk's [releases page](https://github.com/vooteleaer/vork/releases). Installers for Linux (`.deb`), Docker, Haiku and Android are coming; until then storage is built from source there ([command line](10-command-line.md#installing)). vØrk storage runs on vØrk: install [vØrk](../install.md) first.

> [!WARNING]
> **Status: a working prototype, not for data you can't afford to lose yet.** The storage formats may still change, and the encryption is waiting for an independent review. Keep another copy of anything important.

## What is vØrk storage?

Most people keep their photos and documents "in the cloud": on a company's servers, which you rent space on, log in to, and trust with everything in it.

vØrk storage keeps them on **your own devices** instead: your phone, your laptop, a computer at home, a small box at a relative's. Each device has what it needs, they keep each other up to date, and between them they keep **three copies** of everything by default, so losing one device doesn't lose anything. Everything is **encrypted before it leaves the device it was made on**: a device that only stores your data for you, yours or someone else's, sees nothing but noise.

On your computer it looks like an ordinary folder (in Windows Explorer, on Linux, on Haiku). Files open as usual; the ones not on this device yet are fetched when you open them. You manage it from the vØrk dashboard, where vØrk storage adds its own pages ([ADR-0032](../adr/0032-application-pages.md)). Apps that work on the same data live in their own repositories, as demos: [vØrk Photos](https://github.com/vooteleaer/vork-photos), [vØrk Music](https://github.com/vooteleaer/vork-music), [vØrk Map](https://github.com/vooteleaer/vork-map), [vØrk Contacts](https://github.com/vooteleaer/vork-contacts) and [vØrk Calendar](https://github.com/vooteleaer/vork-calendar) (first written for retifs, vØrk storage's predecessor), and a [Music Assistant provider](../../clients/music-assistant) in this repository.

## Why not just Google Drive, Dropbox or iCloud?

| | A cloud service | vØrk storage |
|---|---|---|
| **Who can read your files** | The company, and whoever it's made to give them to | Only you and the people you share with: the keys never leave your devices |
| **Where they are** | The company's data centres | Your devices, and the ones you choose to hold copies |
| **Without internet** | Mostly not | Yes: everything on the device works, and devices sync over whatever connects them (the local network, Wi-Fi, a cable) |
| **If the company changes its prices, its terms, or shuts down** | You move everything, if you can | Nothing changes: there's no company |
| **Cost** | A monthly fee for space | The disks you already have |
| **Sharing** | A link to their server, or an account with them | With a person: an invitation they open on their device, read-only or to add to it |
| **Taking it back** | Ask the company to delete it | Delete a shared library on every member's device at once |

vØrk storage runs on [vØrk](../../README.md), a network of equal nodes without any central infrastructure, where any node can relay for the others. Your devices find each other by their keys, not by addresses a provider hands out. (It began as retifs, on [Reticulum](https://reticulum.network), and moved to vØrk in 2026: [ADR-0015](../adr/0015-storage.md), [ADR-0022](../adr/0022-storage-leaves-reticulum.md). Libraries kept by retifs come over with `vork-import-retifs`.)

### The honest trade-offs

- **Your devices do the work.** Copies are kept by devices that are switched on now and then. A phone alone isn't enough; a home computer or a small always-on box makes it work well.
- **You are your keys.** No company can reset your password. If every device holding your keys is lost at once, the data is lost with them, unless you made a [recovery kit](adr/0033-recovery.md) (24 words on paper).
- **Deleting reaches devices, not memories.** Deleting a shared library removes it from every member's device, but not what someone already copied out of it.
- **It's early.** See the warning above.

## How it works, in short

- **Records, not just files.** Every photo, contact or document is a record: its content plus what's known about it (when and where a photo was taken, who's in it, which albums it's in). You find things by asking ("photos taken in Prangli in 2026"), not by remembering which folder they went in. The folder view is there for the programs that need folders.
- **Libraries.** Records live in libraries: "photos", "family", "work". Each library has its own key, and sharing a library means giving its key to someone's device.
- **Encrypted pieces.** Content is cut into pieces and each piece is encrypted on its own. Any device may hold pieces for you, the nearest first; only a device with the library's key can put them back together and read them.
- **Sync without a server.** Every change is a signed entry that every member's device merges, in any order, to the same result. Two people editing the same library offline are both kept.

The details are in the [architecture](01-architecture.md).

## Try it

**[Getting started](getting-started.md)** walks you through two devices in about half an hour. In short: install vØrk on both and link them, then install vØrk storage on both. Installers are on the [releases page](https://github.com/vooteleaer/vork/releases).

- **Windows 10 and 11:** `vork-storage-<version>.msi` installs for the current user, no administrator needed (vØrk itself must be installed first). Your libraries appear in Explorer under `%USERPROFILE%\vork`, and storage's pages in the vØrk dashboard. See [installing vØrk storage](../install.md#installing-vørk-storage).
- **Linux (`.deb`), Docker, Haiku, Android:** installers are coming. Until then, build from source.
- **From source**, and everything the `vork-storage` command can do: [using vØrk storage from the command line](10-command-line.md).

---

## For developers

**vØrk storage** is a distributed, always-encrypted data system on the vØrk network, in Rust. It combines **IPFS-style content addressing**, **BeFS-style attributes and queries** and **identity-addressed networking** (vØrk; its identities and crypto primitives are the ones it took from Reticulum).

It has no files as such: it has **records** (a stable ID, attributes and encrypted data blocks) and a **database**. Applications use it directly through a native API (records, attributes, queries, history; no paths), the way apps attach to the vØrk node. A file tree is one view of that database, for software that needs one.

### The shift

- **From location to identity**: data isn't "on server X". It's *mine*, wherever it physically lives.
- **From files to records**: data and its meaning, not names and folders.
- **From hierarchy to queries**: structure is what you ask of the data, not where you put it.
- **From trust to crypto**: you don't have to believe the server behaves.
- **From network dependence to independence**: the network is for syncing, not a prerequisite (local-first).
- **From machine to person**: the computer is hardware, and you are your key.

### Why a network like this

| The network (vØrk) | vØrk storage |
|---|---|
| No IP addresses, only identities | No file paths, only records and libraries |
| No central address allocation | No central server or owner |
| Any node relays for any other | Storage over any node, disk, stick |
| Crypto by default | Crypto by default, with no unencrypted mode |
| Needs no infrastructure | Works offline, syncs when it can |

### Three primitives

```mermaid
flowchart BT
    C["Encrypted chunks<br/><i>content</i>"]
    D["Prolly-tree database<br/><i>attributes · indexes · queries</i>"]
    R["Signed chain of roots<br/><i>history · libraries · permissions</i>"]
    U["Files · folders · albums · packages · versions · branches"]
    C --> U
    D --> U
    R --> U
```

1. **Encrypted chunks.** Content-defined chunking, and each chunk is encrypted on its own. The CID is computed over the ciphertext, so storage nodes can verify integrity without ever seeing the content.
2. **Prolly-tree database.** Signed key-value attributes on records, held in content-addressed indexes. A folder is a query, and sync transfers only the branches that differ.
3. **Signed chain of roots.** Every writer signs its own chain of commits; members merge them as a CRDT. A library is identified by its creator's key; read = key, write = a grant from a member who holds `grant`.

### Principles

1. **Always encrypted.** There is no unencrypted mode. "Public" means the key travels with the reference.
2. **Three primitives**, and everything else is a combination of them.
3. **Only established crypto primitives** (Ed25519, X25519, HKDF, AES + HMAC-SHA256, the set Reticulum uses). No new crypto.
4. **Local-first.** Writes are local and immediate.
5. **The BeOS way, not the WinFS way.** Free-form attributes, schema as convention, no relational engine.
6. **The file tree is a compatibility view,** not the primary interface.
7. **Storage nodes are blind.** They hold hash-addressed noise.
8. **Redundant by default.** Three copies from the moment something is written, one at another site once nodes say where they are.
9. **Data outlives apps.** Records belong to no app; each app shows its part and keeps what it doesn't understand.

See the [vision](00-vision.md).

### Non-goals

- IPFS compatibility.
- Unencrypted storage.
- Being a relational database (no joins, no enforced schemas).
- Guaranteed deletion of what others already have. Deleting a library everywhere ([ADR-0034](adr/0034-deleting-a-library.md)) reaches every member's device; beyond that, deletion means destroying the key.
- A single global namespace replacing DNS.

### What's built

| Part | | |
|---|---|---|
| Content layer | Per-chunk encryption, placement and repair, leases and storage proofs, three copies (one at another site) | prototype works |
| Database | Records and queries, multi-writer commit chains merged as a CRDT, Prolly-tree state and checkpoints, keep rules, blind index | prototype works |
| Identity and permissions | Grants (also limited by key and time), revokes with key rotation, invitations, shares, contacts | prototype works; recovery kit built; YubiKey next |
| Linux | `vorkd` on the machine's vØrk node, FUSE mount with xattrs and query folders, local web gateway | prototype works; `.deb` coming |
| Windows | Explorer folder (Cloud Files: placeholders fetched on open), a per-user installer that keeps storage running from login | prototype works, released |
| The dashboard | storage's pages in the vØrk dashboard (`vork-web --dashboard`, [ADR-0032](../adr/0032-application-pages.md)) | prototype works |
| Haiku | Libraries as volumes with native attributes and live queries in Tracker | prototype works; package coming |
| Android | `vorkd` inside an app, beside the vØrk app | worked on Reticulum; on vØrk, coming |

See the [prototype plan](05-prototype-plan.md).

### Documentation

| Document | Contents |
|---|---|
| [Vision](00-vision.md) | Paradigm, principles, non-goals, "why not X" |
| [Architecture](01-architecture.md) | The three primitives and everything built on them |
| [Irreversible decisions](02-irreversible-decisions.md) | Format and crypto decisions that can't change later |
| [Prior art](03-prior-art.md) | BeOS, WinFS, IPFS, rnfs, Tahoe-LAFS, Dolt and others: what worked and what didn't |
| [Threat model](04-threat-model.md) | Who we defend against (draft) |
| [Prototype plan](05-prototype-plan.md) | Prototype-first ordering |
| [Open questions](06-open-questions.md) | Unresolved topics |
| [Native API](07-api.md) | `libvork`: records, attributes, blocks, queries, history (draft) |
| [External review](09-external-review.md) | What the crypto review should look at |
| [Getting started](getting-started.md) | Two devices, step by step |
| [Command line](10-command-line.md) | Installing from source, and every `vork-storage` command by task |
| [ADRs](adr) | Decision log (storage's own numbering) |
| [Recon](recon) | Notes from experiments, most from the Reticulum years: history, not current state |

## Contributing

Ideas are welcome in Discussions. The design direction is fixed and written down in the [vision](00-vision.md). A proposal must support at least one principle and must not violate any. Big changes need a working prototype. The core is strict, while the ecosystem (applications, clients, tools) is open.

## Contact

Issues and discussions are on [GitHub](https://github.com/vooteleaer/vork).

## License

[MIT](../../LICENSE)

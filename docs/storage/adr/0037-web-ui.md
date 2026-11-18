# ADR-0037: A web UI for nodes without a screen

- **Status:** proposed, 2026-11-13
- **Date:** 2026-11-13
- **Irreversible:** no. A front end over the local API, like the GTK, Win32, Haiku and Android ones. No format, no crypto.

## Context
A node in a container (the NAS's TrueNAS app) or on a headless machine has no screen. Joining a library with a link, inviting a device, watching sync, or reading crash reports (ADR-0036) meant ssh and `docker exec vork …`. TrueNAS apps conventionally have a web UI. The user asked for one (2026-11-13), with the installer deciding where it may be reached from.

## Principles
- **No central server:** each node's UI shows that node only, as the GTK utility does. Managing another own node stays over Reticulum (`--via`, ADR-0019), not the web.
- **Always encrypted:** the UI talks to the local daemon only. Nothing leaves the machine except the page to the browser that was let in.
- **One implementation:** the page calls the same `core::api` functions (JSON) the Android and Haiku front ends call. There's no second implementation of the daemon's protocol.

## Options
1. **No web UI; use ssh.** Works, but not for a NAS's owner, and not on a phone.
2. **A web UI open to the network, with a password.** Passwords get reused and guessed, and the UI can do everything the owner can.
3. **A web UI the installer limits to chosen networks, optionally entered by a pairing link from the node's log.** Chosen.

## Decision
- **`vork-web`**, a fifth front end in vork-desktop (`web/`), serves one page and a JSON API over HTTP. It covers:
  - the node's status and its Reticulum;
  - its libraries: details, keep all or as used, favourite, read link, invitations, members, announcing, leaving and deleting;
  - Announces, and joining with a link;
  - creating a library;
  - the crash library's reports (ADR-0036), and writing one;
  - the node's settings.

  The layout is the apps' own: libraries on the left with Announces above them, the data on the right. Light and dark follow the browser.
- **Where it's reached from is the installer's choice:**
  - `--listen ADDR:PORT` (`VORK_WEB_LISTEN`), by default `127.0.0.1:8741`.
  - `--allow CIDR,…` (`VORK_WEB_ALLOW`), by default loopback only. A request from any other address is refused before anything else happens.
  - The NAS's app allows its LAN and its tailnet. A desktop leaves the defaults.
- **Who:** the installer's choice too.
  - **By default, the allowed networks are trusted:** whoever reaches it from there may use it. The user, 2026-11-13: having set it up, they expect to reach the node at once.
  - **`--pairing`** (`VORK_WEB_PAIRING=1`) is for networks others share too (guests on a home LAN, others' devices on a tailnet). A browser is let in once by a pairing link (`/pair/<128-bit token>`). The link is printed in the log, and `vork-web --pair-link` prints it again. Opening it sets a session cookie (`HttpOnly`, `SameSite=Strict`, one year), and a new pairing token replaces the used one. Sessions are kept hashed in `<store>/web/sessions`; deleting the file signs every browser out.
- **Host names:** the UI answers only when opened by an IP address or `localhost`, or by a name given with `--host` (`VORK_WEB_HOSTS`, e.g. `truenas.local`). Without a pairing session, a name another site points at this machine (DNS rebinding) would otherwise let that site's page use the API.
- **Requests:**
  - Changes are `POST` with an `X-Vork: 1` header. A cross-site form can't send that header, and no CORS is allowed.
  - Answers carry `Cache-Control: no-store`, and the page sets a strict CSP (no outside scripts, styles, fonts or images).

## Addendum 2026-11-17: Reticulum, and the containers
- **A Reticulum page:** Reticulum's state, the traffic per interface (configured, and those running but not in the file, such as discovered ones), connections switched on and off, added (TCP) and removed, and the whole configuration file edited. It uses the same core as the desktop utilities: `prnsd`'s command line where there is one, applied live, else the file, used at Reticulum's next start. A saved file that the running prnsd refuses is put back, and the refusal is shown.
- **The containers** (`packaging/docker`): two images in one compose file, `vooteleaer/prnsd` (Reticulum) and `vooteleaer/vork` (vorkd, the extractor and this UI). Reticulum runs `prnsd run --service`; the vØrk storage container mounts Reticulum's volume and carries `prnsd` only as a control command (`RETICULUM_CONFIG_DIR`, `PRNSD_STATE_DIR`): prnsd's control files there (a lock, a reload request and its result) work across the two containers. vØrk storage still never starts Reticulum; without the Reticulum container it joins any shared instance on the host.
- **More is reachable from the page:** whoever may use the UI can now also change this machine's Reticulum (connections, listeners, radios in the file). The images therefore turn pairing on by default; the allowed networks are the private ranges.

## Impact on the threat model
- **Whoever the installer lets in can do what the node's owner can:** join, invite, delete. That's anyone on the allowed networks, or, with `--pairing`, only browsers paired by the token from the node's log (root on the machine, or the NAS's admin UI). The default suits a network only the owner uses. Where others share it, the installer narrows `--allow` or turns pairing on.
- **Plain HTTP:** on an allowed LAN, the session cookie and the pages can be read by anyone who can see the traffic. That's the same trust a LAN's admin UIs get. Tailscale encrypts its own part of the way.
- **DNS rebinding:** another site's page can't reach the API through its own name: the Host check refuses it, and with pairing the session cookie isn't sent to it either. Changes also need the header.

## Consequences
- A NAS's owner manages vØrk storage from the TrueNAS app's Web UI button.
- Not yet: TLS, a "Web UI" portal entry in the TrueNAS app (how a custom app declares one is still to be found), packaging `vork-web` for Debian and Windows (the GTK and Win32 utilities cover those desktops).

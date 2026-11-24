# vØrk storage for Music Assistant

A [Music Assistant](https://music-assistant.io) music provider for vØrk storage: your music in a vØrk storage library, encrypted and kept on your own devices, played on any of Music Assistant's players.

## What it does

- **Lists the library without fetching audio.** Every audio record is a track; artists and albums come from the attributes vØrk storage's extractor reads from the files (`audio.title`, `audio.artist`, `audio.album`, `audio.albumartist`, `audio.track`, `audio.disc`, `audio.year`, `audio.genre`, `duration`). Where those are missing, they come from the folders: `Artist/Album (Year)/NN - Title.ext`, with discs as `1-01` or `CD1/`.
- **Plays from wherever the pieces are.** vorkd reads a track in pieces. Pieces not on this node yet are fetched from the library's other nodes as it plays, so a lazily joined library plays without being copied whole.
- **Follows changes.** It uses a live query: a track added on any device appears in Music Assistant.
- **Favourites are the library's own** (`favorite = true` on the record), so they're the same on every device.
- **Covers live in the library.** An album's cover is the image in its folder (`cover.jpg`, `folder.jpg`, `front.*`). When Music Assistant finds a cover for an album that has none, the provider writes it into the library there (`source = music-assistant`). Every device then has it, without the internet.

## Install

Music Assistant loads every folder in its `providers/` that has a `manifest.json`. Mount `provider/vork` there, and give the container an app socket of vorkd's.

1. On the node Music Assistant runs next to, vØrk storage must have the music library. If it isn't the library's own device, join the library from there:
   - On the library's device: `vork-storage library address music` and `vork-storage me` (its storage node).
   - Here: `vork-storage library join music ADDRESS --from NODE --lazy`, where NODE is that device's storage node. It prints this node's writer key.
   - On the library's device: `vork-storage library grant music WRITER`. Write access is for favourites and covers.
2. Give vorkd a second app socket, in a folder of its own, so the container sees nothing else of the store: `vorkd --app-socket /srv/vork-apps/apps.sock` (or `app_socket` under `[node]` in the store's `vork.toml`).
3. Make a token for Music Assistant that is allowed only the music library: `vork-storage app add music-assistant --library music --write`
4. Mounts for the Music Assistant container:
   - `/srv/vork-apps` → `/vork`
   - `provider/vork` → `/app/venv/lib/python3.14/site-packages/music_assistant/providers/vork` (read-only)
5. In Music Assistant, go to Settings → Providers → Add → vØrk storage and enter the socket (`/vork/apps.sock`), the token and the library.

Tested with Music Assistant 2.10.12 on TrueNAS SCALE, vØrk storage running as its own app. The provider needs Linux, because the app socket is a Unix socket.

## License

MIT, as vØrk.

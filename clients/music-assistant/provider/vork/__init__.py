"""vØrk storage as a Music Assistant music provider.

The music is a vØrk storage library: every audio record in it is a track, with the
attributes vØrk storage's extractor reads from the file (``audio.title``,
``audio.artist``, ``audio.album``, ``audio.albumartist``, ``audio.track``,
``audio.disc``, ``audio.year``, ``audio.genre``, ``duration``). Albums and
artists are made from those, so the library is listed without fetching a byte
of audio. A track plays from its content block in pieces (``ReadRange``):
vØrk storage fetches what isn't on this node yet, from the library's other nodes.

Favourites are the library's own (``favorite = true`` on the record, as in every
vØrk storage app). Artwork: an album's cover is an image record in its folder
(``cover.jpg``, ``folder.jpg``, ``front.*``); artwork Music Assistant finds
for an album that has none is written into the library there, so every device
has it, without the internet.
"""

from __future__ import annotations

import asyncio
import hashlib
import re
import posixpath
from collections.abc import AsyncGenerator
from typing import TYPE_CHECKING, Any

from music_assistant_models.enums import ContentType, ImageType, MediaType, ProviderFeature, StreamType
from music_assistant_models.errors import MediaNotFoundError, SetupFailedError
from music_assistant_models.media_items import (
    Album,
    Artist,
    AudioFormat,
    BrowseFolder,
    ItemMapping,
    MediaItemImage,
    MediaItemMetadata,
    ProviderMapping,
    Track,
    UniqueList,
)
from music_assistant_models.streamdetails import StreamDetails

from music_assistant.models.music_provider import MusicProvider

from .client import Client, Record, VorkError

if TYPE_CHECKING:
    from music_assistant_models.config_entries import ConfigEntry, ProviderConfig
    from music_assistant_models.provider import ProviderManifest

    from music_assistant.mass import MusicAssistant
    from music_assistant.models import ProviderInstanceType

CONF_SOCKET = "socket"
CONF_TOKEN = "token"
CONF_LIBRARY = "library"
CONF_SAVE_ARTWORK = "save_artwork"
DEFAULT_SOCKET = "/vork/apps.sock"

# Every record with a path: audio and images are told apart here, by the
# extractor's `mime` or else by the name, so a library works before (or
# without) the extractor.
FILES = ["has:path"]
AUDIO_EXT = {"mp3", "flac", "ogg", "oga", "opus", "m4a", "m4b", "aac", "wav", "aif", "aiff", "alac", "wma", "ape", "wv", "mka", "dsf"}
IMAGE_EXT = {"jpg", "jpeg", "png", "webp", "gif"}
COVER_NAMES = ("cover", "folder", "front", "album", "albumart")
# Bytes asked of vorkd at a time while playing: big enough to keep a
# player fed, small enough that a missing piece is fetched quickly.
PIECE = 512 << 10
# How long a piece may take to arrive from other nodes before playing fails.
PIECE_WAIT_MS = 60_000
# How often the artwork Music Assistant found is copied into vØrk storage.
ARTWORK_EVERY = 30 * 60
# While the library keeps changing (an import, the extractor describing
# files), it's looked at no more often than this, in seconds: each look reads
# every record, and each sync makes Music Assistant compare its whole library.
SETTLE = 30
# Music Assistant syncs once the library has been quiet for SETTLE seconds,
# or after this long at the latest while it keeps changing.
SYNC_AT_LATEST = 10 * 60

SUPPORTED_FEATURES = {
    ProviderFeature.BROWSE,
    ProviderFeature.LIBRARY_ARTISTS,
    ProviderFeature.LIBRARY_ALBUMS,
    ProviderFeature.LIBRARY_TRACKS,
    ProviderFeature.ARTIST_ALBUMS,
    ProviderFeature.ARTIST_TOPTRACKS,
    ProviderFeature.SEARCH,
    ProviderFeature.LIBRARY_TRACKS_EDIT,
    ProviderFeature.LIBRARY_ALBUMS_EDIT,
    ProviderFeature.LIBRARY_ARTISTS_EDIT,
}


async def setup(mass: MusicAssistant, manifest: ProviderManifest, config: ProviderConfig) -> ProviderInstanceType:
    """A provider instance for one vØrk storage library."""
    return VorkProvider(mass, manifest, config, SUPPORTED_FEATURES)


def _key(*parts: str) -> str:
    return hashlib.sha256("\0".join(p.casefold().strip() for p in parts).encode()).hexdigest()[:20]


def _split_artists(s: str | None) -> list[str]:
    """Artists from a tag: on ";" and " / ", then Music Assistant's own "feat." rules."""
    if not s:
        return []
    from music_assistant.helpers.tags import split_artists

    out: list[str] = []
    for part in (a.strip() for a in s.replace(" / ", ";").split(";")):
        if part:
            out.extend(a.strip() for a in split_artists(part) if a.strip() and a.strip() not in out)
    return out


def _ext(rec: Record) -> str:
    path = rec.text("path") or ""
    base = posixpath.basename(path)
    return base.rsplit(".", 1)[-1].lower() if "." in base else ""


def _is_audio(rec: Record) -> bool:
    mime = rec.text("mime") or ""
    return mime.startswith("audio/") or (not mime and _ext(rec) in AUDIO_EXT)


def _is_image(rec: Record) -> bool:
    mime = rec.text("mime") or ""
    return mime.startswith("image/") or (not mime and _ext(rec) in IMAGE_EXT)


_TRACKNO = re.compile(r"^\s*(?:(\d{1,2})[-.](?=\d))?(\d{1,3})\s*[-._ ]+\s*(.+)$")
_YEAR = re.compile(r"\s*[\(\[](\d{4})[\)\]]\s*$")


def _from_path(rec: Record) -> dict[str, Any]:
    """Title, track, disc, album, album artist and year from `Artist/Album (Year)/NN - Title.ext`."""
    parts = [p for p in (rec.text("path") or "").split("/") if p]
    out: dict[str, Any] = {}
    if not parts:
        return out
    stem = posixpath.splitext(parts[-1])[0]
    m = _TRACKNO.match(stem)
    if m:
        if m.group(1):
            out["disc"] = int(m.group(1))
        out["track"] = int(m.group(2))
        stem = m.group(3)
    out["title"] = stem.strip()
    folders = parts[:-1]
    if folders and re.fullmatch(r"(?i)(cd|disc|disk)\s*\d+", folders[-1]):
        out.setdefault("disc", int(re.sub(r"\D", "", folders[-1])))
        folders = folders[:-1]
    if folders:
        album = folders[-1]
        y = _YEAR.search(album)
        if y:
            out["year"] = int(y.group(1))
            album = album[: y.start()]
        out["album"] = album.strip()
    if len(folders) >= 2:
        out["albumartist"] = folders[-2].strip()
    return out


def _tags(rec: Record) -> dict[str, Any]:
    """The track's tags (the extractor's), each missing one from its path."""
    p = _from_path(rec)
    artists = _split_artists(rec.text("audio.artist"))
    albumartist = rec.text("audio.albumartist") or p.get("albumartist")
    return {
        "title": rec.text("audio.title") or p.get("title"),
        "artists": artists or ([albumartist] if albumartist else []),
        "album": rec.text("audio.album") or p.get("album"),
        "albumartist": albumartist,
        "track": rec.int("audio.track") or p.get("track"),
        "disc": rec.int("audio.disc") or p.get("disc"),
        "year": rec.int("audio.year") or p.get("year"),
        "genre": rec.text("audio.genre"),
    }


def _content_type(rec: Record) -> ContentType:
    mime = (rec.text("mime") or "").lower()
    sub = mime.split("/", 1)[1] if "/" in mime else ""
    guess = {"mpeg": "mp3", "x-flac": "flac", "mp4": "m4a", "x-m4a": "m4a", "x-wav": "wav", "vnd.wave": "wav", "x-aiff": "aiff"}.get(sub, sub)
    ct = ContentType.try_parse(guess) if guess else ContentType.UNKNOWN
    if ct == ContentType.UNKNOWN:
        path = rec.text("path") or ""
        ext = path.rsplit(".", 1)[-1] if "." in path else ""
        ct = ContentType.try_parse(ext) if ext else ContentType.UNKNOWN
    return ct


class VorkProvider(MusicProvider):
    """One vØrk storage library as a source of tracks, albums and artists."""

    def __init__(self, *args: Any, **kwargs: Any) -> None:
        super().__init__(*args, **kwargs)
        self.client: Client
        self.library: str = ""
        self._tracks: dict[str, Record] = {}
        self._covers: dict[str, Record] = {}  # folder -> image record
        self._albums: dict[str, Album] = {}
        self._artists: dict[str, Artist] = {}
        self._album_tracks: dict[str, list[str]] = {}
        self._artist_albums: dict[str, list[str]] = {}
        self._digest = ""
        self._seen = ""  # what Music Assistant was last given (_signature)
        self._loaded = asyncio.Event()
        self._tasks: list[asyncio.Task[None]] = []

    @property
    def is_streaming_provider(self) -> bool:
        return False

    async def handle_async_init(self) -> None:
        socket = self.get_setup_value(CONF_SOCKET) or DEFAULT_SOCKET
        token = self.get_setup_value(CONF_TOKEN) or ""
        self.library = str(self.get_setup_value(CONF_LIBRARY) or "music").strip()
        self.client = Client(str(socket), str(token))
        try:
            await self.client.call({"op": "ns_info", "ns": self.library})
        except VorkError as err:
            raise SetupFailedError(f"vØrk storage: {err}") from err
        self._tasks.append(asyncio.create_task(self._follow()))
        if self.get_setup_value(CONF_SAVE_ARTWORK) is not False:
            self._tasks.append(asyncio.create_task(self._artwork_keeper()))

    async def unload(self, is_removed: bool = False) -> None:
        for t in self._tasks:
            t.cancel()
        await self.client.close()

    async def get_config_entries(self) -> tuple[ConfigEntry, ...]:
        return ()

    # The library, kept current with a live query.

    async def _follow(self) -> None:
        """Keeps the track list current: a live query answers whenever it changes.

        Changes are gathered: the library is looked at again only SETTLE
        seconds after a change, and Music Assistant syncs once it has been
        quiet that long (or SYNC_AT_LATEST after the first unsynced change),
        and only if something it shows changed.
        """
        from .client import Client as _C

        loop = asyncio.get_running_loop()
        watcher = _C(self.client.socket_path, self.client.token)
        pending: float | None = None  # when a change not yet synced was seen
        try:
            while True:
                try:
                    wait = SETTLE if pending is not None else 120
                    digest, recs = await watcher.live_query(self.library, FILES, self._digest, wait * 1000)
                except VorkError as err:
                    self.logger.warning("vØrk storage: %s; trying again in a minute", err)
                    await watcher.close()
                    await asyncio.sleep(60)
                    continue
                changed = recs is not None and digest != self._digest
                if changed:
                    first = not self._loaded.is_set()
                    self._index([r for r in recs if _is_audio(r)], [r for r in recs if _is_image(r)])
                    self._digest = digest
                    signature = self._signature()
                    if first:
                        self._seen = signature
                    elif signature != self._seen and pending is None:
                        pending = loop.time()
                    self._loaded.set()
                    if first or signature != self._seen:
                        self.logger.info("vØrk storage: library %s has %d tracks, %d albums, %d artists", self.library, len(self._tracks), len(self._albums), len(self._artists))
                if pending is not None and (not changed or loop.time() - pending >= SYNC_AT_LATEST):
                    pending = None
                    self._seen = self._signature()
                    await self.mass.music.start_sync(providers=[self.instance_id])
                if changed:
                    await asyncio.sleep(SETTLE)
        finally:
            await watcher.close()

    def _signature(self) -> str:
        """A digest of what Music Assistant is shown: a change elsewhere (the
        extractor's markers, thumbnails) needs no sync."""
        h = hashlib.sha256()
        for rid in sorted(self._tracks):
            t = self._track(self._tracks[rid])
            h.update(repr((rid, t.name, t.duration, [a.item_id for a in t.artists], t.album.item_id if t.album else None, t.track_number, t.disc_number, t.favorite, sorted(t.metadata.genres or []), _content_type(self._tracks[rid]))).encode())
        for aid in sorted(self._albums):
            a = self._albums[aid]
            h.update(repr((aid, a.name, a.year, [x.item_id for x in a.artists], [i.path for i in a.metadata.images or []])).encode())
        for aid in sorted(self._artists):
            a = self._artists[aid]
            h.update(repr((aid, a.name, [i.path for i in a.metadata.images or []])).encode())
        return h.hexdigest()

    async def _ready(self) -> None:
        await asyncio.wait_for(self._loaded.wait(), timeout=300)

    def _index(self, tracks: list[Record], images: list[Record]) -> None:
        covers: dict[str, Record] = {}
        for img in images:
            path = img.text("path") or ""
            stem = posixpath.splitext(posixpath.basename(path))[0].casefold()
            folder = posixpath.dirname(path)
            if stem in COVER_NAMES and (folder not in covers or COVER_NAMES.index(stem) < COVER_NAMES.index(posixpath.splitext(posixpath.basename(covers[folder].text("path") or ""))[0].casefold() or "album")):
                covers[folder] = img
            for key in ("cover.album", "cover.artist"):
                if img.text(key):
                    covers[f"{key}:{img.text(key)}"] = img
        albums: dict[str, Album] = {}
        artists: dict[str, Artist] = {}
        album_tracks: dict[str, list[str]] = {}
        artist_albums: dict[str, list[str]] = {}
        by_id: dict[str, Record] = {}
        for rec in tracks:
            if not rec.block():
                continue
            by_id[rec.id] = rec
            tags = _tags(rec)
            for name in tags["artists"] + _split_artists(tags["albumartist"]):
                aid = _key(name)
                if aid not in artists:
                    artists[aid] = self._artist(aid, name, covers.get(f"cover.artist:{aid}"))
            alb = self._album_of(rec)
            if alb:
                aid, name, aartist = alb
                if aid not in albums:
                    folder = posixpath.dirname(rec.text("path") or "")
                    cover = covers.get(f"cover.album:{aid}") or covers.get(folder)
                    albums[aid] = self._album(aid, name, aartist, rec, cover)
                    artist_albums.setdefault(_key(aartist), []).append(aid)
                album_tracks.setdefault(aid, []).append(rec.id)
        self._tracks, self._covers, self._albums, self._artists = by_id, covers, albums, artists
        self._album_tracks, self._artist_albums = album_tracks, artist_albums

    def _album_of(self, rec: Record) -> tuple[str, str, str] | None:
        tags = _tags(rec)
        name = tags["album"]
        if not name:
            return None
        aartist = tags["albumartist"] or (tags["artists"] or ["Various Artists"])[0]
        return _key(aartist, name), name, aartist

    def _mapping(self, item_id: str, rec: Record | None = None) -> set[ProviderMapping]:
        fmt = AudioFormat(content_type=_content_type(rec)) if rec else AudioFormat(content_type=ContentType.UNKNOWN)
        return {ProviderMapping(item_id=item_id, provider_domain=self.domain, provider_instance=self.instance_id, available=True, audio_format=fmt)}

    def _image(self, rec: Record | None) -> UniqueList[MediaItemImage] | None:
        if not rec:
            return None
        return UniqueList([MediaItemImage(type=ImageType.THUMB, path=rec.id, provider=self.instance_id)])

    def _artist(self, aid: str, name: str, cover: Record | None) -> Artist:
        return Artist(item_id=aid, provider=self.instance_id, name=name, provider_mappings=self._mapping(aid), metadata=MediaItemMetadata(images=self._image(cover)))

    def _album(self, aid: str, name: str, aartist: str, rec: Record, cover: Record | None) -> Album:
        artist = ItemMapping(media_type=MediaType.ARTIST, item_id=_key(aartist), provider=self.instance_id, name=aartist)
        return Album(
            item_id=aid,
            provider=self.instance_id,
            name=name,
            year=_tags(rec)["year"],
            artists=UniqueList([artist]),
            provider_mappings=self._mapping(aid),
            metadata=MediaItemMetadata(images=self._image(cover)),
        )

    def _track(self, rec: Record) -> Track:
        tags = _tags(rec)
        name = tags["title"] or rec.id[:8]
        artists = [ItemMapping(media_type=MediaType.ARTIST, item_id=_key(a), provider=self.instance_id, name=a) for a in tags["artists"]]
        alb = self._album_of(rec)
        album = self._albums.get(alb[0]) if alb else None
        genre = tags["genre"]
        track = Track(
            item_id=rec.id,
            provider=self.instance_id,
            name=name,
            duration=rec.int("duration") or 0,
            artists=UniqueList(artists),
            album=ItemMapping(media_type=MediaType.ALBUM, item_id=album.item_id, provider=self.instance_id, name=album.name) if album else None,
            track_number=tags["track"] or 0,
            disc_number=tags["disc"] or 0,
            favorite=rec.attrs.get("favorite") is True,
            provider_mappings=self._mapping(rec.id, rec),
            metadata=MediaItemMetadata(genres={g.strip() for g in genre.split(";") if g.strip()} if genre else None, images=album.metadata.images if album else None),
        )
        return track

    # Music Assistant's side.

    async def get_library_tracks(self) -> AsyncGenerator[Track]:
        await self._ready()
        for rec in list(self._tracks.values()):
            yield self._track(rec)

    async def get_library_albums(self) -> AsyncGenerator[Album]:
        await self._ready()
        for album in list(self._albums.values()):
            yield album

    async def get_library_artists(self) -> AsyncGenerator[Artist]:
        await self._ready()
        for artist in list(self._artists.values()):
            yield artist

    async def get_track(self, prov_track_id: str) -> Track:
        await self._ready()
        rec = self._tracks.get(prov_track_id) or await self.client.record(self.library, prov_track_id)
        if not rec:
            raise MediaNotFoundError(f"no track {prov_track_id}")
        return self._track(rec)

    async def get_album(self, prov_album_id: str) -> Album:
        await self._ready()
        if album := self._albums.get(prov_album_id):
            return album
        raise MediaNotFoundError(f"no album {prov_album_id}")

    async def get_artist(self, prov_artist_id: str) -> Artist:
        await self._ready()
        if artist := self._artists.get(prov_artist_id):
            return artist
        raise MediaNotFoundError(f"no artist {prov_artist_id}")

    async def get_album_tracks(self, prov_album_id: str) -> list[Track]:
        await self._ready()
        tracks = [self._track(self._tracks[t]) for t in self._album_tracks.get(prov_album_id, []) if t in self._tracks]
        return sorted(tracks, key=lambda t: (t.disc_number, t.track_number, t.name))

    async def get_artist_albums(self, prov_artist_id: str) -> list[Album]:
        await self._ready()
        return [self._albums[a] for a in self._artist_albums.get(prov_artist_id, []) if a in self._albums]

    async def get_artist_toptracks(self, prov_artist_id: str) -> list[Track]:
        await self._ready()
        return [self._track(r) for r in self._tracks.values() if prov_artist_id in {_key(a) for a in _tags(r)["artists"]}][:25]

    async def search(self, search_query: str, media_types: list[MediaType], limit: int = 5) -> Any:
        from music_assistant_models.media_items import SearchResults

        await self._ready()
        q = search_query.casefold()
        res = SearchResults()
        if MediaType.TRACK in media_types:
            res.tracks = [self._track(r) for r in self._tracks.values() if q in (_tags(r)["title"] or "").casefold()][:limit]
        if MediaType.ALBUM in media_types:
            res.albums = [a for a in self._albums.values() if q in a.name.casefold()][:limit]
        if MediaType.ARTIST in media_types:
            res.artists = [a for a in self._artists.values() if q in a.name.casefold()][:limit]
        return res

    async def browse(self, path: str) -> list[Any]:
        """The library as its folders, as in Explorer or the mount: `instance://Artist/Album (Year)`."""
        await self._ready()
        sub = path.split("://", 1)[1].strip("/") if "://" in path else ""
        prefix = f"/{sub}/" if sub else "/"
        folders: set[str] = set()
        here: list[Record] = []
        for rec in self._tracks.values():
            p = rec.text("path") or ""
            if not p.startswith(prefix):
                continue
            rest = p[len(prefix):]
            if "/" in rest:
                folders.add(rest.split("/", 1)[0])
            else:
                here.append(rec)
        items: list[Any] = [
            BrowseFolder(
                item_id=f"{sub}/{name}".strip("/"),
                provider=self.instance_id,
                path=f"{self.instance_id}://{sub + '/' if sub else ''}{name}",
                name=name,
                is_playable=True,
            )
            for name in sorted(folders, key=str.casefold)
        ]
        tracks = [self._track(r) for r in here]
        tracks.sort(key=lambda t: (t.disc_number, t.track_number, t.name.casefold()))
        return items + tracks

    async def library_add(self, item: Any) -> bool:
        await self._set_favorite(item, True)
        return True

    async def library_remove(self, prov_item_id: str, media_type: MediaType) -> bool:
        if media_type == MediaType.TRACK and prov_item_id in self._tracks:
            await self.client.set(self.library, prov_item_id, {}, ["favorite"])
        return True

    async def _set_favorite(self, item: Any, on: bool) -> None:
        if getattr(item, "media_type", None) != MediaType.TRACK:
            return
        for m in item.provider_mappings:
            if m.provider_instance == self.instance_id and m.item_id in self._tracks:
                await self.client.set(self.library, m.item_id, {"favorite": True} if on else {}, [] if on else ["favorite"])

    # Playing.

    async def get_stream_details(self, item_id: str, media_type: MediaType) -> StreamDetails:
        await self._ready()
        rec = self._tracks.get(item_id) or await self.client.record(self.library, item_id)
        if not rec or not rec.block():
            raise MediaNotFoundError(f"no track {item_id}")
        return StreamDetails(
            provider=self.instance_id,
            item_id=item_id,
            audio_format=AudioFormat(content_type=_content_type(rec)),
            media_type=MediaType.TRACK,
            stream_type=StreamType.CUSTOM,
            duration=rec.int("duration"),
            size=rec.int("size"),
            data={"cap": rec.block()},
            allow_seek=True,
            can_seek=False,
        )

    async def get_audio_stream(self, streamdetails: StreamDetails, seek_position: int = 0) -> AsyncGenerator[bytes]:
        """The track's bytes in pieces; vØrk storage fetches pieces not here yet from the library's nodes."""
        cap = streamdetails.data["cap"]
        size = streamdetails.size  # the record's; vorkd clamps reads to the file's own size
        offset = 0
        while size is None or offset < size:
            want = PIECE if size is None else min(PIECE, size - offset)
            try:
                data = await self.client.read_range(cap, self.library, offset, want, PIECE_WAIT_MS)
            except VorkError as err:
                raise MediaNotFoundError(f"vØrk storage: {err}") from err
            if not data:
                return  # the file's end
            yield data
            offset += len(data)
            if size is None and len(data) < want:
                return  # vorkd reads short only at the file's end

    async def resolve_image(self, path: str) -> str | bytes:
        rec = next((r for r in self._covers.values() if r.id == path), None) or await self.client.record(self.library, path)
        if not rec or not rec.block():
            raise MediaNotFoundError(f"no image {path}")
        return await self.client.read_all(rec.block(), self.library)

    # Artwork Music Assistant found, kept in vØrk storage.

    async def _artwork_keeper(self) -> None:
        from music_assistant.helpers.images import get_image_data

        await self._ready()
        await asyncio.sleep(120)
        while True:
            saved = 0
            try:
                async for album in self.mass.music.albums.iter_library_items_by_prov_id(self.instance_id):
                    mine = next((m.item_id for m in album.provider_mappings if m.provider_instance == self.instance_id), None)
                    ours = self._albums.get(mine or "")
                    if not ours or (ours.metadata.images):
                        continue
                    found = next((i for i in (album.metadata.images or []) if i.type == ImageType.THUMB and i.provider != self.instance_id), None)
                    if not found:
                        continue
                    data = await get_image_data(self.mass, found.path, found.provider)
                    if await self._save_cover(mine, ours, data):
                        saved += 1
            except Exception as err:  # keep going: artwork is a nicety
                self.logger.warning("vØrk storage: saving artwork: %s", err)
            if saved:
                self.logger.info("vØrk storage: %d album covers saved into %s", saved, self.library)
            await asyncio.sleep(ARTWORK_EVERY)

    async def _save_cover(self, album_id: str, album: Album, data: bytes) -> bool:
        tracks = [self._tracks[t] for t in self._album_tracks.get(album_id, []) if t in self._tracks]
        folders = {posixpath.dirname(t.text("path") or "") for t in tracks}
        folder = folders.pop() if len(folders) == 1 else ""
        ext = "png" if data[:4] == b"\x89PNG" else "jpg"
        mime = "image/png" if ext == "png" else "image/jpeg"
        aartist = album.artists[0].name if album.artists else ""
        attrs = {
            "path": posixpath.join(folder or "/Covers", f"cover.{ext}" if folder else f"{aartist} - {album.name}.{ext}"),
            "mime": mime,
            "cover.album": album_id,
            "audio.album": album.name,
            "audio.albumartist": aartist,
            "source": "music-assistant",
        }
        rid = await self.client.create(self.library, attrs, data)
        return bool(rid)

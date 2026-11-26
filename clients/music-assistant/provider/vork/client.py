"""A small asyncio client for vorkd's local API, on its app socket.

Framing: ``u32_be length || msgpack(Envelope)``, the envelope being
``{"v": 0, "id": n, "msg": request}``. Requests are tagged by ``op``,
answers by ``result`` (libvork; app scopes: ADR-0004). The first message on the app
socket is ``Hello`` with the app's token; after it every request is held to
the app's scope (its libraries, read or write).

Attribute values are externally tagged: ``{"Str": "x"}``, ``{"Int": 3}``,
``{"Bool": true}``, ``"Null"``, ``{"Bytes": b"..."}``, ``{"List": [...]}``.
"""

from __future__ import annotations

import asyncio
import struct
from dataclasses import dataclass, field
from typing import Any

import msgpack

PROTOCOL_VERSION = 0
MAX_FRAME = 256 << 20


class VorkError(Exception):
    """The daemon answered with an error, or the connection failed."""


def to_value(v: Any) -> Any:
    """A Python value as a vØrk storage attribute value."""
    if v is None:
        return "Null"
    if isinstance(v, bool):
        return {"Bool": v}
    if isinstance(v, int):
        return {"Int": v}
    if isinstance(v, float):
        return {"Float": v}
    if isinstance(v, (bytes, bytearray)):
        return {"Bytes": bytes(v)}
    if isinstance(v, (list, tuple)):
        return {"List": [to_value(x) for x in v]}
    return {"Str": str(v)}


def from_value(v: Any) -> Any:
    """A vØrk storage attribute value as a Python value."""
    if v == "Null" or v is None:
        return None
    if isinstance(v, dict) and len(v) == 1:
        (kind, inner), = v.items()
        if kind == "List":
            return [from_value(x) for x in inner]
        return inner
    return v


@dataclass
class Record:
    """A record as the daemon shows it: ID, attributes, content blocks (read caps)."""

    id: str
    attrs: dict[str, Any] = field(default_factory=dict)
    blocks: list[str] = field(default_factory=list)

    @classmethod
    def parse(cls, raw: dict[str, Any]) -> Record:
        return cls(
            id=raw["id"],
            attrs={k: from_value(v) for k, v in raw.get("attrs", [])},
            blocks=list(raw.get("blocks", [])),
        )

    def text(self, key: str) -> str | None:
        v = self.attrs.get(key)
        return v if isinstance(v, str) and v.strip() else None

    def int(self, key: str) -> int | None:
        v = self.attrs.get(key)
        return v if isinstance(v, int) and not isinstance(v, bool) else None

    def block(self, role: str | None = None) -> str | None:
        """The content block, or the one for `role` (``block:<role>`` = its index or cap)."""
        if role:
            v = self.attrs.get(f"block:{role}")
            if isinstance(v, int) and 0 <= v < len(self.blocks):
                return self.blocks[v]
            if isinstance(v, str) and v:
                return v
            return None
        return self.blocks[0] if self.blocks else None


class Connection:
    """One connection to the app socket, past ``Hello``. Calls are serialised.

    vorkd answers a connection's requests one at a time, in order. A call
    that doesn't finish (cancelled while waiting for its answer, or failed
    mid-frame) leaves its answer on the socket, where the next call would
    read it as its own: such a connection is `broken` and must not be used
    again. Answers are also checked against the request's ID.
    """

    def __init__(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        self._reader = reader
        self._writer = writer
        self._next = 1
        self._lock = asyncio.Lock()
        self.broken = False

    @classmethod
    async def open(cls, socket_path: str, token: str) -> Connection:
        try:
            reader, writer = await asyncio.open_unix_connection(socket_path, limit=MAX_FRAME)
        except OSError as err:
            raise VorkError(f"can't reach vorkd at {socket_path}: {err}") from err
        conn = cls(reader, writer)
        try:
            await conn.call({"op": "hello", "token": token})
        except BaseException:
            conn.abort()
            raise
        return conn

    async def call(self, msg: dict[str, Any]) -> dict[str, Any]:
        async with self._lock:
            if self.broken:
                raise VorkError("connection to vorkd out of step")
            mid = self._next
            self._next += 1
            body = msgpack.packb({"v": PROTOCOL_VERSION, "id": mid, "msg": msg}, use_bin_type=True)
            # Broken until the whole answer is read: a cancellation (or any
            # error) in between leaves the connection out of step.
            self.broken = True
            try:
                self._writer.write(struct.pack(">I", len(body)) + body)
                await self._writer.drain()
                head = await self._reader.readexactly(4)
                (n,) = struct.unpack(">I", head)
                if n > MAX_FRAME:
                    raise VorkError(f"connection to vorkd sent a frame of {n} bytes")
                env = msgpack.unpackb(await self._reader.readexactly(n), raw=False)
            except (OSError, asyncio.IncompleteReadError) as err:
                raise VorkError(f"connection to vorkd lost: {err}") from err
            if not isinstance(env, dict) or env.get("id", mid) != mid:
                raise VorkError(f"connection to vorkd out of step: answer {env.get('id') if isinstance(env, dict) else '?'} to request {mid}")
            self.broken = False
        answer = env.get("msg", env)
        if answer.get("result") == "error":
            raise VorkError(answer.get("message", "error"))
        return answer

    def abort(self) -> None:
        """Closes without waiting (from a cancelled call)."""
        self.broken = True
        self._writer.close()

    async def close(self) -> None:
        self.broken = True
        self._writer.close()
        try:
            await self._writer.wait_closed()
        except OSError:
            pass


class Client:
    """vØrk storage as an app sees it: a socket path and a token, connections made as needed.

    Each call has a connection to itself, taken from a few kept idle: vorkd
    answers one connection's requests one at a time, so one shared
    connection would make every call wait behind a stream's read that is
    fetching pieces (up to its wait), and one cancelled call would leave its
    answer for the next (a cover's bytes played as a track).
    """

    MAX_IDLE = 4

    def __init__(self, socket_path: str, token: str) -> None:
        self.socket_path = socket_path.strip()
        self.token = token.strip()
        self._idle: list[Connection] = []

    async def _take(self, fresh: bool) -> Connection:
        while self._idle and not fresh:
            conn = self._idle.pop()
            if not conn.broken:
                return conn
            conn.abort()
        return await Connection.open(self.socket_path, self.token)

    def _give(self, conn: Connection) -> None:
        if conn.broken or len(self._idle) >= self.MAX_IDLE:
            conn.abort()
        else:
            self._idle.append(conn)

    async def call(self, msg: dict[str, Any]) -> dict[str, Any]:
        """A request on a connection of its own; on a fresh one again if a kept one was lost."""
        for attempt in (0, 1):
            conn = await self._take(fresh=attempt > 0)
            try:
                answer = await conn.call(msg)
            except VorkError as err:
                self._give(conn)  # closed if broken; kept if vorkd only said no
                if not conn.broken or attempt:
                    raise
                continue
            except BaseException:
                conn.abort()  # cancelled mid-call: its answer may still come
                raise
            self._give(conn)
            return answer
        raise VorkError("unreachable")

    async def close(self) -> None:
        idle, self._idle = self._idle, []
        for conn in idle:
            await conn.close()

    # The requests the provider uses.

    async def ping(self) -> dict[str, Any]:
        return await self.call({"op": "ping"})

    async def query(self, ns: str, terms: list[str]) -> list[Record]:
        r = await self.call({"op": "query", "ns": ns, "terms": terms, "at": None})
        return [Record.parse(x) for x in r.get("records", [])]

    async def record(self, ns: str, rid: str) -> Record | None:
        r = await self.call({"op": "record_get", "ns": ns, "id": rid})
        recs = r.get("records") or []
        return Record.parse(recs[0]) if recs else None

    async def live_query(self, ns: str, terms: list[str], since: str, timeout_ms: int) -> tuple[str, list[Record] | None]:
        """Waits until the answer to `terms` differs from digest `since`; the new digest and records (None: unchanged)."""
        r = await self.call({"op": "live_query", "ns": ns, "terms": terms, "since": since, "timeout_ms": timeout_ms})
        recs = r.get("records")
        return r.get("digest", ""), ([Record.parse(x) for x in recs] if recs is not None else None)

    async def set(self, ns: str, rid: str, attrs: dict[str, Any], unset: list[str] | None = None) -> None:
        await self.call({"op": "record_set", "ns": ns, "id": rid, "attrs": [[k, to_value(v)] for k, v in attrs.items()], "unset": unset or [], "data": None})

    async def create(self, ns: str, attrs: dict[str, Any], data: bytes | None = None) -> str:
        r = await self.call({"op": "record_create", "ns": ns, "attrs": [[k, to_value(v)] for k, v in attrs.items()], "data": data})
        recs = r.get("records") or []
        return recs[0]["id"] if recs else ""

    async def read_range(self, cap: str, ns: str | None, offset: int, length: int, wait_ms: int = 30_000) -> bytes:
        r = await self.call({"op": "read_range", "cap": cap, "ns": ns, "offset": offset, "len": length, "wait_ms": wait_ms})
        return bytes(r.get("data", b""))

    async def read_all(self, cap: str, ns: str | None, limit: int = 32 << 20) -> bytes:
        """A small file whole (an image): read in pieces up to `limit` bytes."""
        out = bytearray()
        piece = 1 << 20
        while len(out) < limit:
            data = await self.read_range(cap, ns, len(out), piece)
            out += data
            if len(data) < piece:
                break
        return bytes(out)

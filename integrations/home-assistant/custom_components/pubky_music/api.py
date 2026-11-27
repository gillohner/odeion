"""A small client for the Odeion node's HTTP API and its `/v1/events` websocket."""

from __future__ import annotations

import asyncio
import ipaddress
import logging
from collections.abc import Callable
from typing import Any
from urllib.parse import urljoin, urlsplit

import aiohttp
from yarl import URL

_LOGGER = logging.getLogger(__name__)

# The largest cover taken from another site, and the most redirects one cover may take
ART_MAX = 2 * 1024 * 1024
ART_HOPS = 5
# The longest one cover fetch takes, its redirects and body included (seconds)
ART_WAIT = 20
_REDIRECTS = (301, 302, 303, 307, 308)


class NodeError(Exception):
    """The node answered with an error, or could not be reached."""


class AuthError(NodeError):
    """The node refused the API token."""


def base_url(host: str, port: int) -> str:
    """`http://host:port`, with brackets for an IPv6 address."""
    if ":" in host and not host.startswith("["):
        host = f"[{host}]"
    return f"http://{host}:{port}"


def _origin(url: str) -> tuple[str, str, int] | None:
    """`(scheme, host, port)` of an http(s) URL without user info; `None` for anything else."""
    try:
        u = urlsplit(url)
        port = u.port
    except ValueError:
        return None
    scheme = u.scheme.lower()
    if scheme not in ("http", "https") or not u.hostname or "@" in u.netloc:
        return None
    return scheme, u.hostname.lower(), port or (443 if scheme == "https" else 80)


def same_origin(url: str, base: str) -> bool:
    """Whether `url` is on the node at `base`: scheme, host and port compared exactly, after
    parsing. A prefix check let a URL through whose user info was the node's address, so the
    host was another one (security review 2026-10-27, #4); URLs with user info never count as
    the node's."""
    a = _origin(url)
    return a is not None and a == _origin(base)


def off_node_art_allowed(url: str) -> bool:
    """Whether a cover may come from `url` when it is not the node's: over https only, without
    user info, and never from an address of Home Assistant's own networks (loopback, private,
    link-local, shared/CGNAT and the like) written in the URL. The host is read as aiohttp will
    connect to it (yarl's, after IDNA mapping), not as it is written. A name is fetched only from
    a server whose certificate holds it."""
    try:
        if "@" in urlsplit(url).netloc:
            return False
        u = URL(url)
        u.port
        host = u.raw_host
    except (ValueError, TypeError, UnicodeError):
        return False
    if u.scheme.lower() != "https" or not host or u.raw_user is not None or u.raw_password is not None:
        return False
    host = host.lower().rstrip(".")
    if not host or not host.isascii() or host == "localhost" or host.endswith(".localhost"):
        return False
    try:
        ip = ipaddress.ip_address(host)
    except ValueError:
        # A name. One whose last part is a number is an address in another spelling
        # (`2130706433`, `0x7f.1`), which the system's resolver would take as such
        last = host.rsplit(".", 1)[-1]
        return ":" not in host and "%" not in host and not last.isdigit() and not last.startswith("0x")
    if isinstance(ip, ipaddress.IPv6Address):
        if ip.ipv4_mapped:
            ip = ip.ipv4_mapped
        elif ip in ipaddress.ip_network("64:ff9b::/96"):
            ip = ipaddress.IPv4Address(int(ip) & 0xFFFFFFFF)
        elif int(ip) >> 32 == 0 or ip.is_site_local or ip in ipaddress.ip_network("64:ff9b:1::/48"):
            # `::/96` (with the old IPv4-compatible form), site-local, local-use NAT64
            return False
    return ip.is_global and not ip.is_multicast


async def _read_capped(resp: Any, cap: int) -> bytes | None:
    """The body, or `None` once it is longer than `cap` (by its length header or as read)."""
    if resp.content_length is not None and resp.content_length > cap:
        return None
    body = bytearray()
    async for chunk in resp.content.iter_chunked(64 * 1024):
        body += chunk
        if len(body) > cap:
            return None
    return bytes(body)


class PubkyMusicApi:
    """The node's REST API (`/v1/...`), authenticated with the API token."""

    def __init__(self, session: aiohttp.ClientSession, base: str, token: str) -> None:
        self._session = session
        self.base = base.rstrip("/")
        self._token = token

    @property
    def headers(self) -> dict[str, str]:
        return {"Authorization": f"Bearer {self._token}"} if self._token else {}

    async def request(self, method: str, path: str, *, json: Any = None, params: dict | None = None) -> Any:
        url = path if path.startswith("http") else f"{self.base}/v1{path}"
        try:
            async with self._session.request(
                method,
                url,
                json=json,
                params=params,
                headers=self.headers if same_origin(url, self.base) else {},
                timeout=aiohttp.ClientTimeout(total=30),
            ) as resp:
                # 401: the token itself is refused. 403 is the node saying no to this one call (an app
                # password can't do everything): its reason, not a sign-in to redo
                if resp.status == 401:
                    raise AuthError("the node refused the API token")
                if resp.status >= 400:
                    try:
                        detail = (await resp.json()).get("error")
                    except (aiohttp.ContentTypeError, ValueError):
                        detail = await resp.text()
                    raise NodeError(f"{method} {path}: {resp.status} {detail}")
                if resp.status == 204 or resp.content_length == 0:
                    return None
                if resp.content_type == "application/json":
                    return await resp.json()
                return await resp.read()
        except (aiohttp.ClientError, TimeoutError) as err:
            raise NodeError(f"can't reach the node: {err}") from err

    async def get(self, path: str, **params: Any) -> Any:
        return await self.request("GET", path, params={k: v for k, v in params.items() if v is not None} or None)

    async def post(self, path: str, body: Any = None) -> Any:
        return await self.request("POST", path, json=body if body is not None else {})

    async def put(self, path: str, body: Any) -> Any:
        return await self.request("PUT", path, json=body)

    async def delete(self, path: str) -> Any:
        return await self.request("DELETE", path)

    async def image(self, path: str) -> tuple[bytes | None, str | None]:
        """An image from the node (covers need the token), or another site's where
        `off_node_art_allowed` lets it, at most `ART_MAX` bytes. Redirects are followed here, each
        checked the same way: within the node while it has not left it, else only where another
        site's cover may come from."""
        url = path if path.startswith("http") else f"{self.base}{path}"
        at_node = True
        try:
            async with asyncio.timeout(ART_WAIT):
                return await self._image(url, at_node)
        except (aiohttp.ClientError, TimeoutError, ValueError):
            return None, None

    async def _image(self, url: str, at_node: bool) -> tuple[bytes | None, str | None]:
        for _ in range(ART_HOPS + 1):
            at_node = at_node and same_origin(url, self.base)
            if not at_node and not off_node_art_allowed(url):
                return None, None
            # Another site's cover as it is sent (no compression), so the cap counts its bytes
            headers = self.headers if at_node else {"Accept-Encoding": "identity"}
            async with self._session.get(
                url, headers=headers, allow_redirects=False, timeout=aiohttp.ClientTimeout(total=ART_WAIT)
            ) as resp:
                if resp.status in _REDIRECTS and resp.headers.get("Location"):
                    url = urljoin(url, resp.headers["Location"])
                    continue
                if resp.status != 200:
                    return None, None
                if at_node:
                    return await resp.read(), resp.content_type
                body = await _read_capped(resp, ART_MAX)
                return (body, resp.content_type) if body is not None else (None, None)
        return None, None


class NodeEvents:
    """Follows `/v1/events`: a snapshot of every zone, then one message per change. Reconnects with
    a back-off and asks for a fresh snapshot (`resync`) when it missed a message."""

    def __init__(
        self,
        api: PubkyMusicApi,
        session: aiohttp.ClientSession,
        on_message: Callable[[dict], None],
        on_connected: Callable[[bool], None],
    ) -> None:
        self._api = api
        self._session = session
        self._on_message = on_message
        self._on_connected = on_connected
        self._task: asyncio.Task | None = None
        self._stop = False

    def start(self) -> None:
        self._task = asyncio.get_running_loop().create_task(self._run())

    async def stop(self) -> None:
        self._stop = True
        if self._task:
            self._task.cancel()
            try:
                await self._task
            except asyncio.CancelledError:
                pass

    async def _run(self) -> None:
        delay = 1
        ws_url = self._api.base.replace("http", "ws", 1) + "/v1/events"
        while not self._stop:
            try:
                async with self._session.ws_connect(ws_url, headers=self._api.headers, heartbeat=30) as ws:
                    delay = 1
                    self._on_connected(True)
                    seq = None
                    async for msg in ws:
                        if msg.type != aiohttp.WSMsgType.TEXT:
                            if msg.type in (aiohttp.WSMsgType.CLOSED, aiohttp.WSMsgType.ERROR):
                                break
                            continue
                        data = msg.json()
                        s = data.get("seq")
                        if s is not None and seq is not None and data.get("type") != "snapshot" and s != seq + 1:
                            await ws.send_json({"type": "resync"})
                        if s is not None:
                            seq = s
                        self._on_message(data)
            except asyncio.CancelledError:
                raise
            except (aiohttp.ClientError, TimeoutError, ValueError) as err:
                _LOGGER.debug("events socket: %s", err)
            self._on_connected(False)
            if self._stop:
                return
            await asyncio.sleep(delay)
            delay = min(delay * 2, 60)

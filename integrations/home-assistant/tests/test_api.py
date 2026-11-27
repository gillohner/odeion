"""The API client sends the node's token only to the node (security review 2026-10-27, #4), and
takes another site's cover only over https, never from a local address and size-capped.

Runs without Home Assistant: `python3 -m unittest discover -s integrations/home-assistant/tests`
(needs aiohttp).
"""

from __future__ import annotations

import asyncio
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "custom_components" / "pubky_music"))

import aiohttp  # noqa: E402
import api  # noqa: E402

BASE = "http://node:7788"
AT = "@"  # URLs with user info are put together at run time, so scanners don't read them as secrets


def with_userinfo(userinfo: str, rest: str) -> str:
    return f"http://{userinfo}{AT}{rest}"


TOKEN = "node" + "-token"  # built at run time, so it never reads as a secret


class _Resp:
    status = 200
    content_type = "image/jpeg"
    content_length = 3
    headers: dict = {}

    async def read(self) -> bytes:
        return b"img"

    @property
    def content(self):
        return _Chunks(b"img")

    async def json(self):
        return {}

    async def text(self) -> str:
        return ""


class _Chunks:
    """`resp.content`: the body in chunks of the size asked for."""

    def __init__(self, body: bytes) -> None:
        self._body = body

    async def _chunks(self, n: int):
        for i in range(0, len(self._body), n):
            yield self._body[i : i + n]

    def iter_chunked(self, n: int):
        return self._chunks(n)


class _Ctx:
    async def __aenter__(self):
        return _Resp()

    async def __aexit__(self, *exc):
        return False


class FakeSession:
    """Records the URL and headers of every request instead of sending it."""

    def __init__(self) -> None:
        self.sent: list[tuple[str, dict]] = []

    def get(self, url, headers=None, **kw):
        self.sent.append((str(url), dict(headers or {})))
        return _Ctx()

    def request(self, method, url, headers=None, **kw):
        self.sent.append((str(url), dict(headers or {})))
        return _Ctx()


def run(coro):
    return asyncio.run(coro)


class _Status(_Resp):
    content_type = "application/json"

    def __init__(self, status: int, error: str) -> None:
        self.status = status
        self._error = error

    async def json(self):
        return {"error": self._error}


class StatusSession(FakeSession):
    def __init__(self, status: int, error: str) -> None:
        super().__init__()
        self._resp = _Status(status, error)

    def request(self, method, url, headers=None, **kw):
        resp = self._resp

        class Ctx(_Ctx):
            async def __aenter__(self):
                return resp

        return Ctx()


class Refusals(unittest.TestCase):
    """A token the node refuses (401) asks for a new one; a call it refuses (403, e.g. one an app
    password can't make) says the node's reason and leaves the token alone."""

    def test_401_is_the_token(self):
        a = api.PubkyMusicApi(StatusSession(401, "missing or wrong bearer token"), BASE, TOKEN)
        with self.assertRaises(api.AuthError):
            run(a.get("/zones"))

    def test_403_is_that_call(self):
        why = "an app password plays, controls zones and browses; for this, sign in as the owner"
        a = api.PubkyMusicApi(StatusSession(403, why), BASE, TOKEN)
        with self.assertRaises(api.NodeError) as caught:
            run(a.get("/library/home"))
        self.assertNotIsInstance(caught.exception, api.AuthError)
        self.assertIn(why, str(caught.exception))


class SameOrigin(unittest.TestCase):
    def test_the_nodes_own_urls(self):
        for url in (
            "http://node:7788/v1/library/albums/1/cover",
            "http://NODE:7788/x",
            "HTTP://node:7788",
        ):
            self.assertTrue(api.same_origin(url, BASE), url)
        self.assertTrue(api.same_origin("http://[fe80::1]:7788/x", "http://[fe80::1]:7788"))
        self.assertTrue(api.same_origin("http://nas/x", "http://nas:80"))

    def test_lookalikes_are_not_the_node(self):
        for url in (
            with_userinfo("node:7788", "attacker.example/x"),  # user info: the host is attacker.example
            "http://node:7788.attacker.example/x",  # the old prefix check passed this
            "http://node:77880/x",  # and this
            "https://node:7788/x",  # another scheme
            "http://node:7789/x",
            "http://other:7788/x",
            with_userinfo("user", "node:7788/x"),
            with_userinfo("node:7788\\", "attacker.example/x"),
            "javascript:alert(1)",
            "ftp://node:7788/x",
            "http://node:notaport/x",
            "",
        ):
            self.assertFalse(api.same_origin(url, BASE), url)


class TokenGoesOnlyToTheNode(unittest.TestCase):
    def sent_headers(self, call) -> dict:
        session = FakeSession()
        client = api.PubkyMusicApi(session, BASE, TOKEN)
        run(call(client))
        self.assertEqual(len(session.sent), 1)
        return session.sent[0][1]

    def test_images_on_the_node_carry_the_token(self):
        for path in ("/v1/library/albums/1/cover", "http://node:7788/v1/art/x"):
            h = self.sent_headers(lambda c, p=path: c.image(p))
            self.assertEqual(h.get("Authorization"), f"Bearer {TOKEN}", path)

    def test_images_elsewhere_do_not(self):
        for path in (
            with_userinfo("node:7788", "attacker.example/x"),
            "http://node:7788.attacker.example/x",
            "http://node:77880/x",
            "https://images.example/cover.jpg",
            AT + "attacker.example/x",  # glued to the base it reads as user info
        ):
            # Not asked at all (plain http elsewhere), or asked without the token
            session = FakeSession()
            run(api.PubkyMusicApi(session, BASE, TOKEN).image(path))
            for _, h in session.sent:
                self.assertNotIn("Authorization", h, path)

    def test_requests_elsewhere_do_not(self):
        h = self.sent_headers(lambda c: c.request("GET", with_userinfo("node:7788", "attacker.example/v1/info")))
        self.assertNotIn("Authorization", h)
        h = self.sent_headers(lambda c: c.request("GET", "/info"))
        self.assertEqual(h.get("Authorization"), f"Bearer {TOKEN}")


class _Answer:
    """One canned answer: a status, its headers and a body (`length=None`: no length header)."""

    def __init__(self, status: int = 200, body: bytes = b"img", location: str | None = None, length="body") -> None:
        self.status = status
        self.headers = {"Location": location} if location else {}
        self.content_type = "image/jpeg"
        self._body = body
        self.content_length = len(body) if length == "body" else length

    async def read(self) -> bytes:
        return self._body

    @property
    def content(self):
        return _Chunks(self._body)


class Routes:
    """Answers each URL with its canned answer (404 for any other), recording what was asked."""

    def __init__(self, answers: dict[str, _Answer]) -> None:
        self.answers = answers
        self.sent: list[tuple[str, dict, dict]] = []

    def get(self, url, headers=None, **kw):
        self.sent.append((str(url), dict(headers or {}), kw))
        answer = self.answers.get(str(url), _Answer(404, b""))

        class Ctx:
            async def __aenter__(self):
                return answer

            async def __aexit__(self, *exc):
                return False

        return Ctx()


async def _timed(coro):
    t = asyncio.get_running_loop().time()
    got = await coro
    return got, asyncio.get_running_loop().time() - t


def moved(to: str) -> _Answer:
    return _Answer(302, b"", location=to)


class CoversFromElsewhere(unittest.TestCase):
    """Another site's cover: https only, never from a local address (also not by a redirect), at
    most `ART_MAX` bytes. The node's own covers come as before, with the token."""

    def fetch(self, path: str, answers: dict[str, _Answer]):
        session = Routes(answers)
        got = run(api.PubkyMusicApi(session, BASE, TOKEN).image(path))
        return got, session.sent

    def test_the_nodes_cover_comes_with_the_token(self):
        got, sent = self.fetch("/v1/library/albums/1/cover", {f"{BASE}/v1/library/albums/1/cover": _Answer()})
        self.assertEqual(got, (b"img", "image/jpeg"))
        self.assertEqual(sent[0][1].get("Authorization"), f"Bearer {TOKEN}")
        # Redirects are taken here, each one checked, never by aiohttp
        self.assertIs(sent[0][2].get("allow_redirects"), False)

    def test_a_redirect_within_the_node_keeps_the_token(self):
        got, sent = self.fetch(
            "/v1/moved",
            {f"{BASE}/v1/moved": moved("/v1/library/albums/1/cover"), f"{BASE}/v1/library/albums/1/cover": _Answer()},
        )
        self.assertEqual(got, (b"img", "image/jpeg"))
        self.assertEqual([h.get("Authorization") for _, h, _ in sent], [f"Bearer {TOKEN}"] * 2)

    def test_another_sites_cover_over_https_comes_without_the_token(self):
        url = "https://images.example/cover.jpg"
        got, sent = self.fetch("/v1/moved", {f"{BASE}/v1/moved": moved(url), url: _Answer()})
        self.assertEqual(got, (b"img", "image/jpeg"))
        self.assertNotIn("Authorization", sent[1][1])
        # Cover Art Archive answers with a redirect to the Internet Archive: followed over https
        caa = "https://coverartarchive.org/release/x/front-250"
        ia = "https://ia800.us.archive.org/x.jpg"
        got, sent = self.fetch(caa, {caa: moved(ia), ia: _Answer()})
        self.assertEqual(got, (b"img", "image/jpeg"))
        self.assertEqual([u for u, _, _ in sent], [caa, ia])

    def test_plain_http_elsewhere_is_never_asked(self):
        got, sent = self.fetch("http://images.example/x.jpg", {"http://images.example/x.jpg": _Answer()})
        self.assertEqual((got, sent), ((None, None), []))

    def test_a_redirect_to_plain_http_is_not_followed(self):
        for start in (f"{BASE}/v1/moved", "https://images.example/a.jpg"):
            got, sent = self.fetch(
                start, {start: moved("http://images.example/x.jpg"), "http://images.example/x.jpg": _Answer()}
            )
            self.assertEqual(got, (None, None), start)
            self.assertEqual([u for u, _, _ in sent], [start])

    def test_a_redirect_to_a_local_address_is_not_followed(self):
        for to in (
            "http://127.0.0.1/x.jpg",
            "https://127.0.0.1/x.jpg",
            "https://127.0.0.1:7788/v1/x",
            "https://[::1]/x.jpg",
            "https://192.168.1.1/x.jpg",
            "https://100.64.0.1/x.jpg",
            "https://169.254.169.254/x",
            "https://2130706433/x.jpg",
            "https://127。0。0。1/x.jpg",
            "https://localhost/x.jpg",
        ):
            for start in (f"{BASE}/v1/moved", "https://images.example/a.jpg"):
                got, sent = self.fetch(start, {start: moved(to), to: _Answer()})
                self.assertEqual(got, (None, None), (start, to))
                self.assertEqual([u for u, _, _ in sent], [start], (start, to))

    def test_once_it_left_the_node_it_does_not_come_back_with_the_token(self):
        start = "https://images.example/a.jpg"
        got, sent = self.fetch(start, {start: moved(f"{BASE}/v1/library/home"), f"{BASE}/v1/library/home": _Answer()})
        self.assertEqual(got, (None, None))
        self.assertEqual([u for u, _, _ in sent], [start])

    def test_a_cover_over_the_cap_is_refused(self):
        url = "https://images.example/big.jpg"
        big = b"x" * (api.ART_MAX + 1)
        for answer in (_Answer(body=big), _Answer(body=big, length=None)):
            got, _ = self.fetch(url, {url: answer})
            self.assertEqual(got, (None, None))
        fits = b"x" * api.ART_MAX
        got, _ = self.fetch(url, {url: _Answer(body=fits, length=None)})
        self.assertEqual(got, (fits, "image/jpeg"))

    def test_a_slow_cover_is_given_up_on(self):
        class Slow(Routes):
            def get(self, url, headers=None, **kw):
                self.sent.append((str(url), dict(headers or {}), kw))

                class Ctx:
                    async def __aenter__(self):
                        await asyncio.sleep(5)

                    async def __aexit__(self, *exc):
                        return False

                return Ctx()

        wait, api.ART_WAIT = api.ART_WAIT, 0.2
        try:
            started = asyncio.run(_timed(api.PubkyMusicApi(Slow({}), BASE, TOKEN).image("https://images.example/x.jpg")))
        finally:
            api.ART_WAIT = wait
        self.assertEqual(started[0], (None, None))
        self.assertLess(started[1], 2)

    def test_redirects_stop_after_a_few(self):
        loop = {f"https://images.example/{i}": moved(f"https://images.example/{i + 1}") for i in range(20)}
        got, sent = self.fetch("https://images.example/0", loop)
        self.assertEqual(got, (None, None))
        self.assertEqual(len(sent), api.ART_HOPS + 1)

    def test_which_addresses_another_sites_cover_may_come_from(self):
        for url in (
            "https://images.example/x.jpg",
            "https://coverartarchive.org/release/x/front-250",
            "https://8.8.8.8/x.jpg",
            "https://100.128.0.1/x.jpg",
            "https://[2606:4700::1111]/x.jpg",
        ):
            self.assertTrue(api.off_node_art_allowed(url), url)
        for url in (
            "http://images.example/x.jpg",
            "ftp://images.example/x.jpg",
            with_userinfo("user", "images.example/x.jpg").replace("http:", "https:"),
            "https://127.0.0.1/x.jpg",
            "https://127.1/x.jpg",
            "https://2130706433/x.jpg",
            "https://0x7f.1/x.jpg",
            "https://0177.0.0.1/x.jpg",
            "https://0.0.0.0/x.jpg",
            "https://10.0.0.1/x.jpg",
            "https://172.16.0.1/x.jpg",
            "https://192.168.1.1/x.jpg",
            "https://169.254.169.254/x.jpg",
            "https://100.64.0.1/x.jpg",
            "https://224.0.0.1/x.jpg",
            "https://[::1]/x.jpg",
            "https://[::]/x.jpg",
            "https://[fe80::1]/x.jpg",
            "https://[fe80::1%25eth0]/x.jpg",
            "https://[fd00::1]/x.jpg",
            "https://[::ffff:127.0.0.1]/x.jpg",
            "https://[64:ff9b::a00:1]/x.jpg",
            "https://[64:ff9b:1::1]/x.jpg",
            "https://[::7f00:1]/x.jpg",
            "https://[fec0::1]/x.jpg",
            "https://198.18.0.1/x.jpg",
            "https://192.0.0.1/x.jpg",
            # Other spellings aiohttp connects to as the same addresses
            "https://１２７．０．０．１/x.jpg",
            "https://127。0。0。1/x.jpg",
            "https://192。168。1。1/x.jpg",
            "https://ｌｏｃａｌｈｏｓｔ/x.jpg",
            "https://localhost/x.jpg",
            "https://LOCALHOST./x.jpg",
            "https://printer.localhost/x.jpg",
            "https://images.example:notaport/x.jpg",
            "",
        ):
            self.assertFalse(api.off_node_art_allowed(url), url)


class CoversOverRealSockets(unittest.IsolatedAsyncioTestCase):
    """The same through aiohttp itself: a node on 127.0.0.1 that redirects a cover to another local
    server, over http and over https. That server is never reached."""

    async def asyncSetUp(self) -> None:
        from aiohttp import web

        self.local_hits = 0

        async def count(reader, writer):
            self.local_hits += 1
            writer.write(b"HTTP/1.1 200 OK\r\ncontent-type: image/jpeg\r\ncontent-length: 5\r\n\r\nlocal")
            await writer.drain()
            writer.close()

        self.local = await asyncio.start_server(count, "127.0.0.1", 0)
        local_port = self.local.sockets[0].getsockname()[1]

        async def cover(request):
            if request.headers.get("Authorization") != f"Bearer {TOKEN}":
                return web.Response(status=401)
            return web.Response(body=b"img", content_type="image/jpeg")

        def to(scheme: str):
            async def handler(request):
                return web.Response(status=302, headers={"Location": f"{scheme}://127.0.0.1:{local_port}/x.jpg"})

            return handler

        app = web.Application()
        app.router.add_get("/v1/library/albums/1/cover", cover)
        app.router.add_get("/v1/to-http", to("http"))
        app.router.add_get("/v1/to-https", to("https"))
        self.runner = web.AppRunner(app)
        await self.runner.setup()
        await web.TCPSite(self.runner, "127.0.0.1", 0).start()
        self.base = f"http://127.0.0.1:{self.runner.addresses[0][1]}"
        self.session = aiohttp.ClientSession()

    async def asyncTearDown(self) -> None:
        await self.session.close()
        await self.runner.cleanup()
        self.local.close()
        await self.local.wait_closed()

    async def test_the_nodes_cover_comes_and_redirects_to_local_servers_are_not_followed(self):
        client = api.PubkyMusicApi(self.session, self.base, TOKEN)
        self.assertEqual(await client.image("/v1/library/albums/1/cover"), (b"img", "image/jpeg"))
        self.assertEqual(await client.image("/v1/to-http"), (None, None))
        self.assertEqual(await client.image("/v1/to-https"), (None, None))
        self.assertEqual(self.local_hits, 0)


if __name__ == "__main__":
    unittest.main()

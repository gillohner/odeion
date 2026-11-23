"""The API client sends the node's token only to the node (security review 2026-10-27, #4).

Runs without Home Assistant: `python3 -m unittest discover -s integrations/home-assistant/tests`
(needs aiohttp).
"""

from __future__ import annotations

import asyncio
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "custom_components" / "pubky_music"))

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

    async def read(self) -> bytes:
        return b"img"

    async def json(self):
        return {}

    async def text(self) -> str:
        return ""


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
            h = self.sent_headers(lambda c, p=path: c.image(p))
            self.assertNotIn("Authorization", h, path)

    def test_requests_elsewhere_do_not(self):
        h = self.sent_headers(lambda c: c.request("GET", with_userinfo("node:7788", "attacker.example/v1/info")))
        self.assertNotIn("Authorization", h)
        h = self.sent_headers(lambda c: c.request("GET", "/info"))
        self.assertEqual(h.get("Authorization"), f"Bearer {TOKEN}")


if __name__ == "__main__":
    unittest.main()

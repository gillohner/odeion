#!/usr/bin/env python3
"""Network-free tests for the advanced account recipes embedded in the UI.

All credentials and responses here are SYNTHETIC fixtures. No request reaches a node,
provider, account or device. User-facing recipe output is captured, never presented as
an actual successful sign-in.
"""
import contextlib
import io
import json
from pathlib import Path
import re
import subprocess
import sys
import urllib.error
from unittest.mock import patch

ROOT = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parent.parent
extract = r"""
const fs=require('node:fs'),vm=require('node:vm');
const s=fs.readFileSync(process.argv[1]+'/crates/pm-api/static/app.js','utf8');
const b=/\/\/ LOGIN-GUIDES-BEGIN\n([\s\S]*?)\/\/ LOGIN-GUIDES-END/.exec(s);const box={};
vm.runInNewContext(b[1]+'\nbox.g=LOGIN_GUIDES;box.s=LOGIN_SOURCES;',{box});
console.log(JSON.stringify({guides:Object.fromEntries(Object.entries(box.g).map(([k,g])=>[k,{codes:g.steps.map(x=>typeof x==='function'?x({}):x).filter(x=>typeof x==='object'&&x.code).map(x=>x.code)}])),sources:box.s}));
"""
data = json.loads(subprocess.check_output(["node", "-e", extract, str(ROOT)], text=True))
KEYS = ["plex:server", "plex:pin", "audiobookshelf:server", "audiobookshelf:password", "podcasts:gpodder", "podcasts:nextcloud"]
SECRET = 'synthetic-secret-"quoted"-\\n-\\path'
NODE_TOKEN = "synthetic-node-token"
USERNAME = 'synthetic-"user"-\\path'
SERVER = "https://media.synthetic.test"
problems = []

def check(ok, message):
    print(("ok " if ok else "FAIL ") + message)
    if not ok:
        problems.append(message)

class Response(io.BytesIO):
    def __init__(self, value):
        super().__init__(json.dumps(value).encode())

for key in KEYS:
    g = data["guides"].get(key)
    check(g is not None, key + ": a separate offered authentication variant has a guide")
    if not g:
        continue
    code = next((m.group(1) for raw in g["codes"] if (m := re.fullmatch(r"python3 - <<'PY'\n([\s\S]*)\nPY", raw))), None)
    check(code is not None, key + ": recipe uses Python JSON encoding and hidden prompts, not shell interpolation")
    if code is None:
        continue
    check("getpass.getpass" in code and "curl " not in code and "subprocess" not in code, key + ": authorization and account secrets never become child-process arguments")
    calls = []
    redirects_blocked = []

    class Opener:
        def open(self, request, timeout=None):
            check(request.full_url.startswith("http://127.0.0.1:7788/v1/"), key + ": only the local fixture API would be contacted")
            headers = dict(request.header_items())
            assert headers.get("Authorization") == "Bearer " + NODE_TOKEN
            assert NODE_TOKEN not in request.full_url
            body = json.loads(request.data) if request.data else None
            calls.append((request.get_method(), request.full_url, body))
            if request.full_url.endswith("/plex/pin"):
                return Response({"id": 123, "auth_url": "https://app.plex.tv/auth#SYNTHETIC-private-pin"})
            if "/plex/pin/" in request.full_url:
                return Response({"status": "connected", "server": {"connected": True}})
            return Response({"connected": True})

    def build(*handlers):
        for h in handlers:
            if hasattr(h, "redirect_request"):
                redirects_blocked.append(h.redirect_request(None, None, None, None, None) is None)
        return Opener()

    def prompt(text=""):
        return USERNAME if "user" in text.lower() else SERVER

    def hidden(text=""):
        label = text.lower()
        if "action" in label:
            return "connect"
        if "node api" in label:
            return NODE_TOKEN
        if "server" in label and ("url" in label or "address" in label):
            return SERVER
        if "username" in label:
            return USERNAME
        return SECRET

    output = io.StringIO()
    try:
        with patch("builtins.input", side_effect=prompt), patch("getpass.getpass", side_effect=hidden), patch("urllib.request.build_opener", side_effect=build), patch("time.sleep"), contextlib.redirect_stdout(output):
            exec(compile(code, "<synthetic-ui-recipe>", "exec"), {"__name__": "__main__"})
        check(bool(calls), key + ": recipe reaches the mocked local transport")
        check(bool(redirects_blocked) and all(redirects_blocked), key + ": authenticated requests cannot follow redirects")
        check(NODE_TOKEN not in output.getvalue() and SECRET not in output.getvalue(), key + ": recipe output excludes credentials")
        if key == "plex:pin":
            check(calls[0][0] == "POST" and any("/plex/pin/123" in u for _, u, _ in calls), key + ": starts a PIN and polls its returned id without exposing account tokens")
        else:
            body = calls[0][2]
            field = "token" if key == "plex:server" else "api_key" if key == "audiobookshelf:server" else "password"
            check(body.get(field) == SECRET, key + ": quoted/backslash-containing credentials round-trip exactly")
            if key in ("audiobookshelf:password", "podcasts:gpodder", "podcasts:nextcloud"):
                user_field = "user" if key.startswith("audiobookshelf") else "username"
                check(body.get(user_field) == USERNAME, key + ": user-entered identity round-trips exactly")
            if key.startswith("podcasts"):
                check(body.get("kind") == key.split(":")[1], key + ": exact runtime server kind is submitted")
    except Exception as e:
        check(False, key + ": recipe execution failed in synthetic transport (" + type(e).__name__ + ")")

    calls.clear()
    logout_output = io.StringIO()

    def disconnect_prompt(text=""):
        if "node api" in text.lower():
            return NODE_TOKEN
        if "action" in text.lower():
            return "disconnect"
        raise AssertionError("Disconnect must not ask for provider credentials")

    try:
        with patch("getpass.getpass", side_effect=disconnect_prompt), patch("urllib.request.build_opener", side_effect=build), contextlib.redirect_stdout(logout_output):
            exec(compile(code, "<synthetic-ui-recipe-signout>", "exec"), {"__name__": "__main__"})
        check(len(calls) == 1 and calls[0][0] == "DELETE" and NODE_TOKEN not in logout_output.getvalue(), key + ": full sign-out uses one authenticated DELETE and no provider credentials")
    except Exception as e:
        check(False, key + ": sign-out failed in synthetic transport (" + type(e).__name__ + ")")

    class RefusingOpener:
        def open(self, request, timeout=None):
            raise urllib.error.HTTPError(request.full_url, 503, "SYNTHETIC", None, io.BytesIO(SECRET.encode()))

    refused = io.StringIO()
    try:
        with patch("builtins.input", side_effect=prompt), patch("getpass.getpass", side_effect=hidden), patch("urllib.request.build_opener", return_value=RefusingOpener()), contextlib.redirect_stdout(refused):
            exec(compile(code, "<synthetic-ui-recipe-refusal>", "exec"), {"__name__": "__main__"})
        check(False, key + ": refusal must exit nonzero")
    except SystemExit as e:
        check(e.code != 0 and SECRET not in refused.getvalue() and NODE_TOKEN not in refused.getvalue(), key + ": refusal exits nonzero without printing raw upstream body or credentials")
    except Exception as e:
        check(False, key + ": refusal handling failed (" + type(e).__name__ + ")")

if problems:
    print(f"{len(problems)} recipe regression(s)")
    sys.exit(1)
print(f"All {len(KEYS)} synthetic account recipe variants passed; no network or live authentication performed")

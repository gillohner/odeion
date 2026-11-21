#!/usr/bin/env python3
"""Phone mode on an Android emulator (CI: android.yml, phone mode): the app with no node.

The APK is built with the `phone-mode-fixture` feature (never in releases): phone mode is already
chosen (Pubky Ring is skipped; the real sign-in is the node's Ring code, checked by the ring
probe), a TIDAL full-tracks session is seeded on the phone, and TIDAL is reached at a stand-in on
this runner (127.0.0.1:18490 on the emulator, through `adb reverse`). The stand-in answers TIDAL's
v1 API the way TIDAL does (search, album, playback info with a BTS manifest) and serves a 30 s FLAC
it writes itself.

The app must open phone mode by itself (no connect form, no node anywhere), show its own web UI,
find the song by searching in the page, and play it when its row is tapped: the app's own player
logs `playing /v1/sources/tidal/stream/<id>` with a path, then plays in real time. The stand-in
must have been asked with the seeded session only (bearer), and the log must carry no token.

Usage: android-phone-mode.py <apk> <out dir>
       android-phone-mode.py --self-test   (the FLAC writer and the stand-in; no adb)
"""
import base64
import html
import http.server
import json
import math
import re
import struct
import subprocess
import sys
import threading
import time
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

PKG = "to.synonym.pubkymusic"
PORT = 18490
# What the fixture build seeds as the person's TIDAL session (crates/pm-desktop/src/phone.rs)
TOKEN = "phone-mode-fixture-token"
TRACK = 1001
ALBUM = 501
TITLE = "Fixture Tone"
SELF_TEST = sys.argv[1:] == ["--self-test"]
if not SELF_TEST:
    apk, out = sys.argv[1], Path(sys.argv[2])
    out.mkdir(parents=True, exist_ok=True)


def log(msg):
    print(f"[{time.strftime('%H:%M:%S')}] {msg}", flush=True)


# ---- a FLAC file, written here (VERBATIM subframes: plain, valid, no encoder needed) ----

def crc8(data):
    c = 0
    for b in data:
        c ^= b
        for _ in range(8):
            c = ((c << 1) ^ 0x07) & 0xFF if c & 0x80 else (c << 1) & 0xFF
    return c


def crc16(data):
    c = 0
    for b in data:
        c ^= b << 8
        for _ in range(8):
            c = ((c << 1) ^ 0x8005) & 0xFFFF if c & 0x8000 else (c << 1) & 0xFFFF
    return c


def utf8_number(n):
    """FLAC's frame number: UTF-8's encoding of an integer (31 bits at most)."""
    if n < 0x80:
        return bytes([n])
    out = []
    while True:
        out.insert(0, 0x80 | (n & 0x3F))
        n >>= 6
        lead_bits = 7 - len(out) - 1
        if n < (1 << lead_bits):
            mark = (0xFF << (8 - len(out) - 1)) & 0xFF
            out.insert(0, mark | n)
            return bytes(out)


def flac(seconds=30, rate=44100, block=4096, freq=440, level=2000):
    """16-bit stereo FLAC of a quiet sine, fixed blocks of `block` samples."""
    total = seconds * rate
    info = struct.pack(">HH", block, block) + b"\x00\x00\x00" + b"\x00\x00\x00"
    # sample rate (20) | channels-1 (3) | bits-1 (5) | total samples (36)
    packed = (rate << 44) | (1 << 41) | (15 << 36) | total
    info += packed.to_bytes(8, "big") + bytes(16)
    head = b"fLaC" + bytes([0x80]) + len(info).to_bytes(3, "big") + info
    frames = []
    for number, start in enumerate(range(0, total, block)):
        n = min(block, total - start)
        code = 0xC if n == 4096 else 0x7
        hdr = bytes([0xFF, 0xF8, (code << 4) | 0x9, (0x1 << 4) | (0x4 << 1)]) + utf8_number(number)
        if code == 0x7:
            hdr += struct.pack(">H", n - 1)
        hdr += bytes([crc8(hdr)])
        samples = [int(level * math.sin(2 * math.pi * freq * (start + i) / rate)) for i in range(n)]
        body = struct.pack(f">{n}h", *samples)
        frame = hdr + b"\x02" + body + b"\x02" + body
        frames.append(frame + struct.pack(">H", crc16(frame)))
    return head + b"".join(frames)


# ---- the stand-in TIDAL --------------------------------------------------------------------------

ASKED = []


def answers(path, query, auth):
    """TIDAL's v1 answers for one album with one song; `None` for anything else (404)."""
    ASKED.append((path, auth))
    if auth != f"Bearer {TOKEN}":
        return 401, {"userMessage": "not signed in"}
    base = f"http://127.0.0.1:{PORT}"
    album = {"id": ALBUM, "title": "Phone Fixture", "cover": None, "releaseDate": "2026-11-21",
             "artists": [{"id": 7, "name": "Fixture Artist"}], "type": "ALBUM"}
    song = {"id": TRACK, "title": TITLE, "duration": 30, "trackNumber": 1, "isrc": "QZFIX2600001",
            "album": album, "artists": [{"id": 7, "name": "Fixture Artist"}], "streamReady": True,
            "audioQuality": "LOSSLESS"}
    if path == "/v1/sessions":
        return 200, {"userId": 1, "countryCode": "US", "sessionId": "fixture"}
    if path == "/v1/search":
        return 200, {"artists": {"items": []}, "albums": {"items": [album]}, "tracks": {"items": [song]}}
    if path == f"/v1/albums/{ALBUM}":
        return 200, album
    if path == f"/v1/albums/{ALBUM}/tracks":
        return 200, {"items": [song], "totalNumberOfItems": 1}
    if path == f"/v1/tracks/{TRACK}":
        return 200, song
    if path == f"/v1/tracks/{TRACK}/playbackinfopostpaywall":
        manifest = {"mimeType": "audio/flac", "codecs": "flac", "encryptionType": "NONE",
                    "urls": [f"{base}/media/tone.flac"]}
        return 200, {"trackId": TRACK, "assetPresentation": "FULL", "audioQuality": "LOSSLESS",
                     "manifestMimeType": "application/vnd.tidal.bts", "bitDepth": 16, "sampleRate": 44100,
                     "manifest": base64.b64encode(json.dumps(manifest).encode()).decode()}
    return None


def serve(audio):
    class H(http.server.BaseHTTPRequestHandler):
        def log_message(self, *a):
            pass

        def do_GET(self):
            u = urlsplit(self.path)
            if u.path == "/media/tone.flac":
                self.send_response(200)
                self.send_header("Content-Type", "audio/flac")
                self.send_header("Content-Length", str(len(audio)))
                self.end_headers()
                self.wfile.write(audio)
                return
            r = answers(u.path, parse_qs(u.query), self.headers.get("Authorization", ""))
            status, body = r if r else (404, {"userMessage": "not found"})
            data = json.dumps(body).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

    srv = http.server.ThreadingHTTPServer(("127.0.0.1", PORT), H)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv


# ---- the emulator --------------------------------------------------------------------------------

def adb(*args, check=True):
    r = subprocess.run(["adb", *args], capture_output=True, text=True, timeout=120)
    if check and r.returncode != 0:
        raise RuntimeError(f"adb {' '.join(args)}: {r.stderr.strip() or r.stdout.strip()}")
    return r.stdout


def until(what, fn, timeout=60, every=1.0):
    end = time.time() + timeout
    last = None
    while True:
        try:
            v = fn()
            if v:
                return v
            last = None
        except Exception as e:  # noqa: BLE001 (retried until the deadline)
            last = e
        if time.time() > end:
            raise RuntimeError(f"timed out waiting for {what}" + (f" (last error: {last})" if last else ""))
        time.sleep(every)


def logcat():
    p = out / "logcat.txt"
    return p.read_text(errors="replace") if p.exists() else ""


PROMPT = re.compile(r'text="(Don.t allow|Deny)"', re.I)


def ui_nodes():
    for _ in range(3):
        adb("shell", "uiautomator", "dump", "/sdcard/ui.xml")
        nodes = re.findall(r"<node [^>]*>", adb("exec-out", "cat", "/sdcard/ui.xml"))
        prompt = next((n for n in nodes if PROMPT.search(n)), None)
        if not prompt:
            return nodes
        adb("shell", "input", "tap", *map(str, center(prompt)))
        time.sleep(1)
    return nodes


def center(node):
    x1, y1, x2, y2 = map(int, re.search(r'bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', node).groups())
    return (x1 + x2) // 2, (y1 + y2) // 2


def find(pred):
    return next((n for n in ui_nodes() if pred(n)), None)


def labelled(text):
    """A node whose text or name is `text` or starts with it (a row says its title first)."""
    def pred(n):
        return any(v == text or v.startswith(text + " ") or v.startswith(text + ",")
                   for v in map(html.unescape, re.findall(r'\s(?:content-desc|text)="([^"]*)"', n)))
    return pred


def tap(pred, what, timeout=90):
    def steady():
        a = find(pred)
        if not a:
            return None
        time.sleep(1)
        b = find(pred)
        return b if b and center(a) == center(b) and center(b) != (0, 0) else None
    x, y = center(until(what, steady, timeout=timeout))
    adb("shell", "input", "tap", str(x), str(y))
    log(f"tapped {what} at {x},{y}")


def shot(name):
    png = subprocess.run(["adb", "exec-out", "screencap", "-p"], capture_output=True, timeout=60).stdout
    (out / f"{name}.png").write_bytes(png)


NATIVE = re.compile(r"the app's own player: (\w+) (\S*)(.*)")
NATIVE_AT = re.compile(r"^(\d\d-\d\d \d\d:\d\d:\d\d\.\d+).*the app's own player at position_ms=(\d+)", re.M)


def native_pace(after=0):
    pts = []
    for m in NATIVE_AT.finditer(logcat()[after:]):
        stamp, ms = m.group(1), int(m.group(2))
        t = time.mktime(time.strptime("2000-" + stamp.split(".")[0], "%Y-%m-%d %H:%M:%S")) + float("0." + stamp.split(".")[1])
        pts.append((t, ms))
    if len(pts) < 3 or pts[-1][0] - pts[0][0] < 9:
        return None
    return (pts[-1][1] - pts[0][1]) / 1000 / (pts[-1][0] - pts[0][0])


def run():
    audio = flac()
    (out / "tone.flac").write_bytes(audio)
    srv = serve(audio)
    sdk = int(adb("shell", "getprop", "ro.build.version.sdk").strip() or 0)
    log(f"Android API {sdk}; stand-in TIDAL on {PORT}, {len(audio)} bytes of FLAC")
    log(adb("install", "-r", *([] if sdk >= 33 else ["-g"]), apk).strip())
    adb("reverse", f"tcp:{PORT}", f"tcp:{PORT}")
    adb("logcat", "-c")
    lc = subprocess.Popen(["adb", "logcat", "-v", "time", "pubky-music:V", "AndroidRuntime:E", "ActivityManager:I", "*:S"],
                          stdout=open(out / "logcat.txt", "w"), stderr=subprocess.STDOUT)
    try:
        adb("shell", "am", "start", "-W", "-n", f"{PKG}/.MainActivity")
        # Phone mode opens by itself: its search page, never the connect form
        until("phone mode started", lambda: "phone mode: started" in logcat(), timeout=90)
        # The search page's one text field (its id and hint differ between WebViews)
        search = lambda n: "EditText" in n
        until("the phone page's search box", lambda: find(search), timeout=120)
        assert not find(labelled("Or connect by address")), "the connect page shows instead of phone mode"
        shot("1-phone-mode")
        tap(search, "the search box")
        adb("shell", "input", "text", "Fixture")
        adb("shell", "input", "keyevent", "66")
        mark = len(logcat())
        tap(labelled(TITLE), f"the song “{TITLE}”")
        shot("2-search")

        def started():
            return next((m for m in NATIVE.finditer(logcat()[mark:]) if m.group(1) == "playing"), None)
        try:
            m = until("the app's own player playing the TIDAL song", started, timeout=90)
        except RuntimeError:
            # A row tap opens the song's page on some layouts: play it from there
            tap(labelled("Play"), "the page's Play")
            m = until("the app's own player playing the TIDAL song", started, timeout=90)
        log(f"the app's own player: {m.group(0).strip()}")
        assert m.group(2).startswith(f"/v1/sources/tidal/stream/{TRACK}"), f"it plays the phone's TIDAL stream: {m.group(2)}"
        assert "path " in m.group(3), f"it says the path: {m.group(3)}"
        assert "phone mode: TIDAL full track fetched" in logcat(), "the full track came through the phone's backend"
        rate = until("the app's own player's pace", lambda: native_pace(mark), timeout=60, every=5)
        log(f"phone mode plays at {rate:.2f}x real time")
        assert 0.8 < rate < 1.2, f"phone mode: {rate:.2f}x"
        shot("3-playing")
        asked = [p for p, _ in ASKED]
        log(f"the stand-in TIDAL was asked: {sorted(set(asked))}")
        assert f"/v1/tracks/{TRACK}/playbackinfopostpaywall" in asked, asked
        assert "/v1/search" in asked, asked
        assert all(a == f"Bearer {TOKEN}" for _, a in ASKED), "every TIDAL call carried the person's session"
        assert TOKEN not in logcat(), "the log carries no token"
        assert "token=" not in "".join(m.group(0) for m in NATIVE.finditer(logcat())), "player lines carry no key"
        log("PASS: phone mode searched, opened and played a TIDAL song with no node")
    except Exception:
        # What the screen showed when it failed, for the artifact
        try:
            shot("failure")
            (out / "failure-ui.xml").write_text("\n".join(ui_nodes()))
        except Exception as e:  # noqa: BLE001
            log(f"no failure screen: {e}")
        raise
    finally:
        lc.terminate()
        srv.shutdown()
        subprocess.run(["adb", "logcat", "-d", "-v", "time"], stdout=open(out / "logcat-all.txt", "w"),
                       stderr=subprocess.STDOUT, check=False)


def self_test():
    # CRCs against FLAC's reference values ("123456789")
    assert crc8(b"123456789") == 0xF4, hex(crc8(b"123456789"))
    assert crc16(b"123456789") == 0xFEE8, hex(crc16(b"123456789"))
    assert utf8_number(0) == b"\x00" and utf8_number(0x7F) == b"\x7f"
    assert utf8_number(0x80) == "\u0080".encode() and utf8_number(0x7FF) == "\u07ff".encode()
    assert utf8_number(0x800) == "\u0800".encode()
    f = flac(seconds=1, block=4096)
    assert f[:4] == b"fLaC" and f[4] == 0x80 and int.from_bytes(f[5:8], "big") == 34
    si = f[8:42]
    rate = (si[10] << 12) | (si[11] << 4) | (si[12] >> 4)
    assert rate == 44100 and ((si[12] >> 1) & 7) + 1 == 2 and (((si[12] & 1) << 4) | (si[13] >> 4)) + 1 == 16
    total = ((si[13] & 0xF) << 32) | int.from_bytes(si[14:18], "big")
    assert total == 44100, total
    # 44100 = 10 full blocks + one of 3140: 11 frames, each starting with the sync code
    first = f[42:]
    assert first[:2] == b"\xff\xf8"
    assert len(f) == 42 + 10 * (4 + 1 + 1 + 2 * (1 + 2 * 4096) + 2) + (4 + 1 + 2 + 1 + 2 * (1 + 2 * 3140) + 2), len(f)
    # The stand-in: only the seeded session gets answers
    assert answers("/v1/search", {}, "Bearer nope")[0] == 401
    status, body = answers(f"/v1/tracks/{TRACK}/playbackinfopostpaywall", {}, f"Bearer {TOKEN}")
    m = json.loads(base64.b64decode(body["manifest"]))
    assert status == 200 and m["encryptionType"] == "NONE" and m["urls"][0].endswith("/media/tone.flac")
    assert answers("/v1/nothing", {}, f"Bearer {TOKEN}") is None
    print("android-phone-mode self-test: ok")


if __name__ == "__main__":
    if SELF_TEST:
        self_test()
    else:
        run()

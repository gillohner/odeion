"""Populated Album and Confirm matches evidence from the installed Android APK.

Called only after the existing strict player/gesture tour. Add synthetic rows to that
run's fresh SQLite library, navigate with adb taps, and recognize the actual native
screencaps with Tesseract. No WebView debugging, asset overlay or provider lookup.
OCR failure is fatal: a page title or an empty/skeleton page is not acceptance.
"""
import csv
import contextlib
import hashlib
import io
import json
import os
import re
import sqlite3
import struct
import subprocess
import time
from pathlib import Path

ALBUM_REQUIRED = ("Native Fixture", "Morning Light", "Evening Glow")
MATCHES_REQUIRED = ("Morning Light", "Studio Candidate", "Radio Candidate")


def seed_fixture(root):
    """Append only to the one-tone library created by android-endpoint.py, once."""
    root = Path(root).resolve()
    tone = root / "music" / "tone.wav"
    db = root / "data" / "library.db"
    with sqlite3.connect(f"{db.as_uri()}?mode=rw", uri=True, timeout=10) as c:
        c.row_factory = sqlite3.Row
        c.execute("PRAGMA foreign_keys=ON")
        c.execute("BEGIN IMMEDIATE")
        tracks = c.execute("SELECT * FROM tracks").fetchall()
        files = c.execute("SELECT * FROM files").fetchall()
        if len(tracks) != 1 or len(files) != 1 or files[0]["path"] != str(tone) or not tone.is_file():
            raise RuntimeError("native pages require the fresh one-tone fixture; refusing this library")
        artist = c.execute("INSERT INTO artists(name) VALUES('Fixture Artist')").lastrowid
        album = c.execute("INSERT INTO albums(title,album_artist,year) VALUES('Native Fixture','Fixture Artist',2026)").lastrowid
        music = root / "native-fixture"
        music.mkdir()  # no overwrite/reuse of a previous run's data
        ids = []
        for no, title in enumerate(("Morning Light", "Evening Glow", "Quiet River"), 1):
            tid = c.execute("INSERT INTO tracks(title,album_id,track_no,duration_ms) VALUES(?,?,?,120000)",
                            (title, album, no)).lastrowid
            ids.append(tid)
            c.execute("INSERT INTO track_artists(track_id,artist_id,position) VALUES(?,?,0)", (tid, artist))
            path = music / f"{no:02d} {title}.wav"
            os.link(tone, path)
            copied = dict(files[0])
            copied.pop("id")
            copied.update(track_id=tid, path=str(path))
            columns = ",".join(copied)
            c.execute(f"INSERT INTO files({columns}) VALUES({','.join('?' for _ in copied)})", tuple(copied.values()))
            c.execute("INSERT INTO tracks_fts(track_id,title,artist,album) VALUES(?,?,?,?)",
                      (tid, title, "Fixture Artist", "Native Fixture"))
        # Two conflicting candidates, no release ID, no canonical identity and no lookup job.
        for number, title in enumerate(("Studio Candidate", "Radio Candidate"), 1):
            c.execute("INSERT INTO identify_suggestions(track_id,recording_mbid,title,artist,score,source,created_at) VALUES(?,?,?,?,?,'search',1)",
                      (ids[0], f"00000000-0000-4000-8000-{number:012d}", title, "Fixture Artist", 0.91))
        c.execute("INSERT INTO identify_state(track_id,state,checked_at) VALUES(?,'suggested',1)", (ids[0],))
    return {"album_id": album, "track_ids": ids, "titles": list(ALBUM_REQUIRED[1:]) + ["Quiet River"]}


def normalized(text):
    return " ".join(re.findall(r"[a-z0-9]+", text.lower()))


def require_text(text, phrases):
    text = " " + normalized(text) + " "
    missing = [p for p in phrases if " " + normalized(p) + " " not in text]
    assert not missing, f"native screen missing rendered content: {', '.join(missing)}"


def phrase_center(words, phrase):
    target = normalized(phrase).split()
    for i in range(len(words) - len(target) + 1):
        group = words[i:i + len(target)]
        if [normalized(w["text"]) for w in group] != target or len({w["line"] for w in group}) != 1:
            continue
        left, top = min(w["left"] for w in group), min(w["top"] for w in group)
        right = max(w["left"] + w["width"] for w in group)
        bottom = max(w["top"] + w["height"] for w in group)
        return (left + right) // 2, (top + bottom) // 2
    return None


def read_screen(path):
    data = path.read_bytes()
    if not data.startswith(b"\x89PNG\r\n\x1a\n"):
        raise RuntimeError("adb did not produce a native PNG")
    result = subprocess.run(["tesseract", str(path), "stdout", "--psm", "11", "tsv"],
                            check=True, capture_output=True, text=True, timeout=30)
    words = []
    for row in csv.DictReader(io.StringIO(result.stdout), delimiter="\t"):
        if row["level"] != "5" or not row["text"].strip():
            continue
        words.append({"text": row["text"], **{k: int(row[k]) for k in ("left", "top", "width", "height")},
                      "line": tuple(int(row[k]) for k in ("block_num", "par_num", "line_num"))})
    return words


@contextlib.contextmanager
def capture_display(adb):
    """360x800 dp at 2x density gives legible native pixels; restore both overrides."""
    size = re.search(r"Override size:\s*(\d+x\d+)", adb("shell", "wm", "size"))
    density = re.search(r"Override density:\s*(\d+)", adb("shell", "wm", "density"))
    sized = dense = False
    try:
        adb("shell", "wm", "size", "720x1600")
        sized = True
        adb("shell", "wm", "density", "320")
        dense = True
        time.sleep(2)
        yield
    finally:
        try:
            if dense:
                adb("shell", "wm", "density", density.group(1) if density else "reset")
        finally:
            if sized:
                adb("shell", "wm", "size", size.group(1) if size else "reset")


def run(h):
    # The original tour, including its recording and stop, has already completed.
    with capture_display(h["adb"]):
        # Density changes recreate this APK's Activity/WebView. Start from a fresh
        # saved-node launch and fresh accessibility tree at the final geometry.
        # This is after, not a retry of, the original earliest-registration gate.
        h["adb"]("shell", "am", "force-stop", h["package"])
        h["adb"]("shell", "am", "start", "-W", "-n", f"{h['package']}/.MainActivity")
        h["tap"](lambda n: 'text="Library"' in n or 'content-desc="Library"' in n,
                 "Library after the capture display restart")
        capture_pages(h)


def capture_pages(h):
    adb, api, shot, until, log, out = (h[k] for k in ("adb", "api", "shot", "until", "log", "out"))
    assert not api("GET", "/v1/node/settings").get("contact"), "native fixture must not enable provider lookups"
    fixture = seed_fixture(out)
    album = api("GET", f"/v1/item/album/lib-{fixture['album_id']}")
    assert album["title"] == "Native Fixture" and len(album["songs"]) == 3, "fixture album is not populated"
    suggestions = api("GET", "/v1/library/suggestions/albums?limit=50")
    song = next((s for a in suggestions["albums"] for s in a["songs"] if s["track"]["id"] == fixture["track_ids"][0]), None)
    assert song and {o["title"] for o in song["options"]} == {"Studio Candidate", "Radio Candidate"}, "fixture candidates are missing"
    W, H = 720, 1600
    observations = []

    def screen(name, required=()):
        shot(name)
        path = out / f"{name}.png"
        assert struct.unpack(">II", path.read_bytes()[16:24]) == (W, H), "native capture display size was not applied"
        words = read_screen(path)
        # Only page content: exclude the status/top bar and the mini player/navigation.
        content = [w for w in words if H * .08 < w["top"] < H * .80]
        text = " ".join(w["text"] for w in content)
        (out / f"{name}-ocr.txt").write_text(text + "\n")
        require_text(text, required)
        return words, text

    def capture(name, required):
        words, text = until(name, lambda: screen(name, required), timeout=45, every=2)
        observations.append({"name": name, "required": list(required), "text": text,
                             "sha256": hashlib.sha256((out / f"{name}.png").read_bytes()).hexdigest()})
        log(f"native page captured: {name}; rendered {', '.join(required)}")
        return words

    def tap_phrase(phrase):
        def target():
            words, _ = screen("ui-native-navigation")
            return phrase_center([w for w in words if H * .08 < w["top"] < H * .80], phrase)
        point = until(f"native {phrase} tap target", target, timeout=30, every=2)
        adb("shell", "input", "tap", str(point[0]), str(point[1]))
        time.sleep(2)

    def library_ready():
        words, _ = screen("ui-native-library-ready")
        # The heading is below the chips, well above the footer. Library waits on
        # API reads before drawing; tapping its nav item is not a render barrier.
        return phrase_center([w for w in words if H * .18 < w["top"] < H * .35], "Your library")
    until("rendered Library heading", library_ready, timeout=45, every=2)

    # The second Library chip is Albums at this fixed display size (154dp, 120dp,
    # including Android's status bar). OCR confuses outlined chip borders with letters.
    # The next target must be this fixture's actual album tile, so a missed tap fails.
    adb("shell", "input", "tap", "308", "240")
    time.sleep(2)
    tap_phrase("Native Fixture")
    capture("ui-10-album", ALBUM_REQUIRED)
    tap_phrase("Match")  # navigates to existing suggestions; never starts Identify
    # Match focuses the album section. Capture both populated choices, then the page heading.
    capture("ui-11-confirm-matches-candidates", MATCHES_REQUIRED)
    adb("shell", "input", "swipe", str(W // 2), str(H // 4), str(W // 2), str(H * 3 // 4), "350")
    time.sleep(2)
    capture("ui-12-confirm-matches", ("Confirm matches", "Native Fixture", "Morning Light"))
    def digest(path):
        with open(path, "rb") as f:
            return hashlib.file_digest(f, "sha256").hexdigest()
    manifest = {"fixture": fixture, "captures": observations, "source": "adb exec-out screencap -p from the installed APK",
                "display": {"width": W, "height": H, "density": 320, "width_dp": 360, "height_dp": 800},
                "apk_sha256": digest(h["apk"]), "node_sha256": digest(h["node_bin"]),
                "checkout": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
                "tree": subprocess.check_output(["git", "rev-parse", "HEAD^{tree}"], text=True).strip()}
    (out / "ui-native-pages.json").write_text(json.dumps(manifest, indent=2) + "\n")
    log("PASS: populated native Album and Confirm matches pages")

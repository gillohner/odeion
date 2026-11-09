"""The phone UI with a finger, on the Android emulator (android-endpoint.py runs it with PM_UI_TOUR=1).

With the app connected to the node and its own zone registered, three songs are queued on that zone
and the page is pointed at it (Zones › Play to). Then, as a finger does it (`adb shell input swipe`,
which the WebView turns into touch Pointer Events): the mini player, a swipe left (the node's queue
moves to the next song) and right (back), a swipe up (Now playing), Up next as a sheet, a swipe
sideways on it (Lyrics), More, Back (the sheet closes, Now playing stays), Back again (Now playing
closes), the album page and Confirm matches. A screenshot of each (ui-*.png) and a recording of the
whole run (ui-tour.mp4, when the emulator can record).

Not a module of its own: android-endpoint.py passes its helpers in (`h`), so it runs inside that
script's node, app and adb session.
"""
import re
import subprocess
import time


def run(h):
    adb, api, shot, ui_nodes, tap, center, until, log, out = (h[k] for k in ("adb", "api", "shot", "ui_nodes", "tap", "center", "until", "log", "out"))

    def nodes_text():
        return "\n".join(ui_nodes())

    def find(pred):
        return next((n for n in ui_nodes() if pred(n)), None)

    def bounds(node):
        m = re.search(r'bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', node)
        return tuple(map(int, m.groups()))

    def swipe(x1, y1, x2, y2, ms=180):
        adb("shell", "input", "swipe", str(x1), str(y1), str(x2), str(y2), str(ms))
        time.sleep(1.2)

    def has(label):
        return f'"{label}"' in nodes_text()

    # The phone's own zone, three songs on it, playing
    track = api("GET", "/v1/library/tracks")
    items = track if isinstance(track, list) else track.get("items") or track.get("tracks") or []
    tid = items[0]["id"]
    zone = until("the phone's zone", lambda: next((z for z in api("GET", "/v1/zones") if str(z.get("output", "")).startswith("net:")), None), timeout=60)
    zid = zone["id"]
    api("POST", f"/v1/zones/{zid}/queue", {"track_ids": [tid, tid, tid], "replace": True, "play": True})
    until("playback on the phone's zone", lambda: api("GET", f"/v1/zones/{zid}").get("transport") == "playing", timeout=30)

    rec = None
    try:
        rec = subprocess.Popen(["adb", "shell", "screenrecord", "--time-limit", "120", "/sdcard/ui-tour.mp4"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    except Exception as e:  # noqa: BLE001
        log(f"no recording: {e}")
    try:
        # The page plays to that zone: Zones › its card's Play to
        tap(lambda n: 'text="Zones"' in n or 'content-desc="Zones"' in n, "the Zones tab")
        time.sleep(2)
        tap(lambda n: ('text="Play to"' in n or 'content-desc="Play to"' in n) and 'clickable="true"' in n, "Play to on the phone's zone card")
        time.sleep(2)
        tap(lambda n: 'text="Home"' in n or 'content-desc="Home"' in n, "the Home tab")
        time.sleep(2)
        mini = until("the mini player", lambda: find(lambda n: 'content-desc="Now playing"' in n or ('text="Now playing"' in n and 'clickable="true"' in n)), timeout=30)
        x1, y1, x2, y2 = bounds(mini)
        shot("ui-1-mini-player")
        # Swipe left: the next song; right: the previous one (the node's queue says so)
        idx0 = api("GET", f"/v1/zones/{zid}").get("queue_index") or 0
        cy = (y1 + y2) // 2
        swipe(x1 + (x2 - x1) * 3 // 4, cy, x1 + (x2 - x1) // 6, cy)
        idx1 = until("the next song after a swipe left", lambda: (lambda i: i if i is not None and i > idx0 else None)(api("GET", f"/v1/zones/{zid}").get("queue_index")), timeout=15)
        log(f"swipe left on the mini player: queue {idx0} -> {idx1}")
        shot("ui-2-after-swipe-left")
        time.sleep(3)
        pos1 = ((api("GET", f"/v1/zones/{zid}").get("now_playing") or {}).get("position_ms")) or 0
        swipe(x1 + (x2 - x1) // 6, cy, x1 + (x2 - x1) * 3 // 4, cy)
        # (previous goes back a song, or to the start of this one when it has played a few seconds:
        # either way the queue goes back or the position does)
        def went_back():
            z = api("GET", f"/v1/zones/{zid}")
            i = z.get("queue_index"); pos = ((z.get("now_playing") or {}).get("position_ms")) or 0
            # (a tuple: index 0 is an answer too; a restart only counts on the same song)
            return (i,) if i is not None and (i < idx1 or (i == idx1 and pos < pos1 - 1000)) else None
        idx2 = until("the previous song (or this one's start) after a swipe right", went_back, timeout=15)[0]
        log(f"swipe right on the mini player: queue {idx1} -> {idx2}")
        # A vertical scroll of the page doesn't change the song
        w = int(re.search(r"(\d+)x(\d+)", adb("shell", "wm", "size")).group(1))
        swipe(w // 2, y1 - 200, w // 2, max(300, y1 - 900), 300)
        swipe(w // 2, max(300, y1 - 900), w // 2, y1 - 200, 300)
        idx3 = api("GET", f"/v1/zones/{zid}").get("queue_index")
        assert idx3 == idx2, f"a vertical scroll changed the song ({idx2} -> {idx3})"
        log("a vertical scroll of the page leaves the song alone")
        # Swipe up: Now playing
        swipe((x1 + x2) // 2, cy, (x1 + x2) // 2, max(100, y1 - 600), 250)
        until("Now playing", lambda: has("Up next") and has("More") and has("Close Now playing"), timeout=15)
        shot("ui-3-now-playing")
        log("swipe up on the mini player opens Now playing")
        tap(lambda n: 'text="Up next"' in n and 'clickable="true"' in n, "Up next")
        until("the Up next sheet", lambda: has("Close the sheet"), timeout=15)
        time.sleep(1)
        shot("ui-4-sheet-up-next")
        # Sideways on the sheet: Lyrics
        sw, sh = int(re.search(r"(\d+)x(\d+)", adb("shell", "wm", "size")).group(1)), int(re.search(r"(\d+)x(\d+)", adb("shell", "wm", "size")).group(2))
        swipe(sw * 3 // 4, sh * 2 // 3, sw // 6, sh * 2 // 3)
        # (the test tone has no lyrics: the Lyrics tab says so)
        until("the Lyrics tab after a sideways swipe on the sheet", lambda: has("No lyrics found for this song.") or "Synced" in nodes_text(), timeout=20)
        log("a sideways swipe on the sheet moves to Lyrics")
        shot("ui-5-sheet-lyrics")
        # (the sheet's tab: the topmost "More"; the button under the player is behind the sheet)
        more = sorted((n for n in ui_nodes() if 'text="More"' in n), key=lambda n: bounds(n)[1])
        assert more, "no More tab"
        x, y = center(more[0])
        adb("shell", "input", "tap", str(x), str(y))
        time.sleep(2)
        shot("ui-6-sheet-more")
        # Back: the sheet closes, Now playing stays; Back again: Now playing closes
        adb("shell", "input", "keyevent", "4")
        until("the sheet closed by Back", lambda: not has("Close the sheet") and has("Close Now playing"), timeout=15)
        log("Back closes the sheet, Now playing stays")
        shot("ui-7-back-closes-sheet")
        adb("shell", "input", "keyevent", "4")
        until("Now playing closed by Back", lambda: not has("Close Now playing"), timeout=15)
        log("Back again closes Now playing")
        # Swipe left on the cover: the next song
        tap(lambda n: 'content-desc="Now playing"' in n or ('text="Now playing"' in n and 'clickable="true"' in n), "the mini player")
        until("Now playing again", lambda: has("Close Now playing"), timeout=15)
        idx4 = api("GET", f"/v1/zones/{zid}").get("queue_index") or 0
        swipe(sw * 3 // 4, sh // 3, sw // 6, sh // 3)
        idx5 = until("the next song after a swipe on the cover", lambda: (lambda i: i if i is not None and i != idx4 else None)(api("GET", f"/v1/zones/{zid}").get("queue_index")), timeout=15)
        log(f"swipe left on the cover: queue {idx4} -> {idx5}")
        # Swipe down: Now playing closes
        swipe(sw // 2, sh // 4, sw // 2, sh * 3 // 4, 250)
        until("Now playing closed by a swipe down", lambda: not has("Close Now playing"), timeout=15)
        log("swipe down closes Now playing")
        # The album page and Confirm matches (best effort: the CI library is one test tone)
        try:
            tap(lambda n: 'text="Library"' in n or 'content-desc="Library"' in n, "the Library tab")
            time.sleep(2)
            tap(lambda n: 'text="Albums"' in n and 'clickable="true"' in n, "Albums")
            time.sleep(2)
            tile = find(lambda n: 'clickable="true"' in n and ('Album' in n or 'music' in n) and 'text="Albums"' not in n)
            if tile:
                x, y = center(tile)
                adb("shell", "input", "tap", str(x), str(y))
                time.sleep(3)
                shot("ui-8-album")
            match = find(lambda n: 'Match this album' in n)
            if match:
                x, y = center(match)
                adb("shell", "input", "tap", str(x), str(y))
                time.sleep(4)
                shot("ui-9-confirm-matches")
        except Exception as e:  # noqa: BLE001
            log(f"album page / Confirm matches not reached here: {e}")
        log("PASS: the phone UI tour (mini player and cover swipes, sheets, Back order)")
    finally:
        api("POST", f"/v1/zones/{zid}/transport", {"action": "stop"})
        if rec:
            # (SIGINT lets screenrecord finish the file)
            adb("shell", "pkill", "-2", "screenrecord", check=False)
            time.sleep(4)
            rec.terminate()
            try:
                mp4 = subprocess.run(["adb", "exec-out", "cat", "/sdcard/ui-tour.mp4"], capture_output=True, timeout=120).stdout
                if mp4:
                    (out / "ui-tour.mp4").write_bytes(mp4)
            except Exception as e:  # noqa: BLE001
                log(f"no recording saved: {e}")

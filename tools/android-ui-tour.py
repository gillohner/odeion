"""The phone UI with a finger, on the Android emulator (android-endpoint.py runs it with PM_UI_TOUR=1).

The app is connected to the node and its own zone registered. Three songs are queued on that zone and
the page is pointed at it (Zones, its card's Play to). Then everything goes as a finger does it
(`adb shell input swipe`, which the WebView turns into touch Pointer Events) and as Back does it:
- the mini player: a swipe left (next song), a vertical scroll of the page (nothing), a swipe right
  (previous song), a swipe up (Now playing);
- Now playing: a swipe up (Up next as a sheet), sideways on the sheet (Lyrics, then More), Back (the
  sheet closes, Now playing stays), Back again (Now playing closes);
- the mini player tapped (Now playing), the cover swiped left (next song), a swipe down (it closes).

The WebView's accessibility tree, which `uiautomator` reads, is not kept up to date after the page
changes by itself (Chrome turns its accessibility off again when no service uses it), so it is read
only on the first screen: the places of the mini player and the Home and Zones tabs. Play to is
tapped where the Zones page has it with the CI's two zones (its card first). What the gestures did is
checked with the node (its queue moves on and back) and on the screen itself, comparing raw
screenshots of the part that must change (or must not). Each
step's screenshot is ui-*.png; the whole run is recorded in ui-tour.mp4 when the emulator can.

Not a module of its own: android-endpoint.py passes its helpers in (`h`), so it runs inside that
script's node, app and adb session.
"""
import re
import struct
import subprocess
import time


def run(h):
    adb, api, shot, ui_nodes, until, log, out = (h[k] for k in ("adb", "api", "shot", "ui_nodes", "until", "log", "out"))

    def bounds(node):
        m = re.search(r'bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', node)
        return tuple(map(int, m.groups()))

    def swipe(x1, y1, x2, y2, ms=180, settle=1.5):
        adb("shell", "input", "swipe", str(int(x1)), str(int(y1)), str(int(x2)), str(int(y2)), str(ms))
        time.sleep(settle)

    def raw():
        """The screen as (width, height, RGBA bytes): screencap's raw form (a 12- or 16-byte header)."""
        b = subprocess.run(["adb", "exec-out", "screencap"], capture_output=True, timeout=60).stdout
        w, hh = struct.unpack("<II", b[:8])
        off = 16 if len(b) - 16 == w * hh * 4 else 12
        return w, hh, b[off:off + w * hh * 4]

    def changed(a, b, box):
        """The share of pixels in box (x0, y0, x1, y1) that differ between two raw screens."""
        w = a[0]
        x0, y0, x1, y1 = box
        n = diff = 0
        for y in range(y0, y1, 2):
            ra = a[2][(y * w + x0) * 4:(y * w + x1) * 4]
            rb = b[2][(y * w + x0) * 4:(y * w + x1) * 4]
            for i in range(0, min(len(ra), len(rb)) - 3, 8):
                n += 1
                if max(abs(ra[i] - rb[i]), abs(ra[i + 1] - rb[i + 1]), abs(ra[i + 2] - rb[i + 2])) > 24:
                    diff += 1
        return diff / max(1, n)

    def settles(ref, box, over=None, under=None, timeout=12):
        """The screen once box differs from ref by more than `over` (or by less than `under`), polled:
        the emulator draws slowly (Now playing's first frame took over 2 s in CI). Returns (screen, share)."""
        end = time.time() + timeout
        while True:
            now = raw()
            share = changed(ref, now, box)
            if (over is not None and share > over) or (under is not None and share < under) or time.time() > end:
                return now, share
            time.sleep(1)

    W, H = map(int, re.search(r"(\d+)x(\d+)", adb("shell", "wm", "size")).group(1, 2))
    zid = None
    rec = None
    try:
        rec = subprocess.Popen(["adb", "shell", "screenrecord", "--time-limit", "170", "/sdcard/ui-tour.mp4"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    except Exception as e:  # noqa: BLE001
        log(f"no recording: {e}")
    try:
        # The first screen, read once: the mini player's place, the Home and Zones tabs
        nodes = ui_nodes()
        minis = [n for n in nodes if 'content-desc="Now playing"' in n or 'text="Now playing"' in n]
        assert minis, "no mini player on the first screen"
        _, y1, _, y2 = bounds(max(minis, key=lambda n: bounds(n)[1]))
        cy = (y1 + y2) // 2
        def tab(word):
            n = max((n for n in nodes if f'text="{word}"' in n or f'content-desc="{word}"' in n), key=lambda n: bounds(n)[1])
            x0, t0, x1, t1 = bounds(n)
            return (x0 + x1) // 2, (t0 + t1) // 2
        home, zones = tab("Home"), tab("Zones")
        # (the song's words: right of the cover, left of play/pause, under the progress line)
        words = (W * 22 // 100, y1 + (y2 - y1) // 5, W * 72 // 100, y2 - (y2 - y1) // 5)
        before = raw()
        # Three songs on the phone's own zone, playing
        t = api("GET", "/v1/library/tracks")
        items = t if isinstance(t, list) else t.get("items") or t.get("tracks") or []
        tid = items[0]["id"]
        zone = until("the phone's zone", lambda: next((z for z in api("GET", "/v1/zones") if str(z.get("output", "")).startswith("net:")), None), timeout=60)
        zid = zone["id"]
        api("POST", f"/v1/zones/{zid}/queue", {"track_ids": [tid, tid, tid], "replace": True, "play": True})
        until("playback on the phone's zone", lambda: api("GET", f"/v1/zones/{zid}").get("transport") == "playing", timeout=30)
        qi = lambda: api("GET", f"/v1/zones/{zid}").get("queue_index") or 0  # noqa: E731
        # The page to that zone: Zones, its card's Play to (the first card), then Home
        adb("shell", "input", "tap", str(zones[0]), str(zones[1]))
        time.sleep(3)
        shot("ui-0-zones")
        adb("shell", "input", "tap", str(W * 644 // 1000), str(H * 556 // 1000))
        time.sleep(2)
        adb("shell", "input", "tap", str(home[0]), str(home[1]))
        time.sleep(3)
        b = raw()
        picked = changed(before, b, words)
        shot("ui-1-mini-player")
        assert picked > 0.02, f"the mini player doesn't show the phone's zone after Play to ({picked:.3f} of its words changed); see ui-0-zones.png"
        log(f"the page plays to the phone's zone ({picked:.3f} of the mini player's words changed)")
        # Swipe left on it: the next song (the node's queue moves on)
        i0 = qi()
        swipe(W * 7 // 8, cy, W // 8, cy, settle=2.5)
        i1 = until("the next song after a swipe left", lambda: (qi(),) if qi() > i0 else None, timeout=15)[0]
        c = raw()
        log(f"swipe left on the mini player: queue {i0} -> {i1}")
        shot("ui-2-after-swipe-left")
        shot("ui-2-after-swipe-left")
        # A vertical scroll of the page: the same song
        swipe(W // 2, y1 - H // 10, W // 2, y1 - H // 2, 300)
        swipe(W // 2, y1 - H // 2, W // 2, y1 - H // 10, 300, settle=2)
        d = raw()
        assert qi() == i1, f"a vertical scroll of the page changed the song ({i1} -> {qi()})"
        log(f"a vertical scroll of the page leaves the song ({changed(c, d, words):.3f} of its words changed)")
        # Swipe right: the previous song (or this one's start: the node's queue goes back, or its position)
        pos = lambda: ((api("GET", f"/v1/zones/{zid}").get("now_playing") or {}).get("position_ms")) or 0  # noqa: E731
        time.sleep(3)
        p1 = pos()
        swipe(W // 8, cy, W * 7 // 8, cy, settle=2.5)
        until("the previous song (or this one's start) after a swipe right", lambda: (1,) if qi() < i1 or (qi() == i1 and pos() < p1 - 1000) else None, timeout=15)
        e = raw()
        log(f"swipe right on the mini player: queue {i1} -> {qi()}")
        # Swipe up on it: Now playing covers the bar
        swipe(W // 2, cy, W // 2, H // 4, 250, settle=1)
        np, cover = settles(e, (0, y1, W, y2), over=0.2)
        time.sleep(1.5)
        np = raw()
        assert cover > 0.2, f"a swipe up on the mini player didn't open Now playing ({cover:.3f})"
        log(f"swipe up on the mini player opens Now playing ({cover:.3f} of the bar's place changed)")
        shot("ui-3-now-playing")
        # Swipe up on the player: the Up next sheet covers its top part
        swipe(W // 2, H * 55 // 100, W // 2, H * 20 // 100, 250, settle=1)
        top = (0, H // 20, W, H // 2)
        s1, up = settles(np, top, over=0.2)
        time.sleep(1)
        s1 = raw()
        assert up > 0.2, f"a swipe up on Now playing didn't open the sheet ({up:.3f})"
        log(f"swipe up on Now playing opens Up next ({up:.3f})")
        shot("ui-4-sheet-up-next")
        # Sideways on the sheet: Lyrics, then More (the list below the tabs changes)
        body = (0, H * 30 // 100, W, H * 80 // 100)
        swipe(W * 7 // 8, H * 60 // 100, W // 8, H * 60 // 100, settle=1)
        s2, t1 = settles(s1, body, over=0.02)
        time.sleep(1.5)
        s2 = raw()
        assert t1 > 0.02, f"a sideways swipe on the sheet didn't change its tab ({t1:.3f})"
        log(f"a sideways swipe on the sheet: Lyrics ({t1:.3f})")
        shot("ui-5-sheet-lyrics")
        swipe(W * 7 // 8, H * 60 // 100, W // 8, H * 60 // 100, settle=1)
        s3, t2 = settles(s2, body, over=0.02)
        assert t2 > 0.02, f"a second sideways swipe didn't change the tab ({t2:.3f})"
        log(f"again: More ({t2:.3f})")
        shot("ui-6-sheet-more")
        # Back: the sheet closes and Now playing stays (its top looks as before the sheet); Back again: it closes
        adb("shell", "input", "keyevent", "4")
        b1, kept = settles(np, top, under=0.15)
        assert kept < 0.15, f"Back didn't close the sheet to Now playing ({kept:.3f} of its top differs from Now playing)"
        log(f"Back closes the sheet, Now playing stays ({kept:.3f})")
        shot("ui-7-back-closes-sheet")
        adb("shell", "input", "keyevent", "4")
        b2, bar = settles(e, (0, y1, W, y2), under=0.15)
        assert bar < 0.15, f"Back again didn't close Now playing ({bar:.3f} of the bar's place differs)"
        log(f"Back again closes Now playing ({bar:.3f})")
        # The mini player tapped: Now playing; the cover swiped left: the next song; a swipe down: closed
        adb("shell", "input", "tap", str(W // 2), str(cy))
        settles(b2, (0, y1, W, y2), over=0.2)
        time.sleep(1.5)
        i4 = qi()
        swipe(W * 7 // 8, H * 30 // 100, W // 8, H * 30 // 100, settle=3)
        n2 = raw()
        i5 = until("the next song after a swipe on the cover", lambda: (qi(),) if qi() != i4 else None, timeout=15)[0]
        log(f"swipe left on the cover: queue {i4} -> {i5}")
        shot("ui-8-cover-swiped")
        swipe(W // 2, H * 25 // 100, W // 2, H * 80 // 100, 250, settle=1)
        c2, shut = settles(n2, (0, y1, W, y2), over=0.2)
        assert shut > 0.2, f"a swipe down didn't close Now playing ({shut:.3f})"
        log(f"swipe down closes Now playing ({shut:.3f})")
        shot("ui-9-closed")
        log("PASS: the phone UI tour (mini player and cover swipes, sheets and their tabs, Back order)")
    except Exception:
        # (what the screen offered, for the next attempt)
        try:
            (out / "ui-nodes-fail.txt").write_text("\n".join(ui_nodes()))
        except Exception:  # noqa: BLE001
            pass
        raise
    finally:
        if zid:
            try:
                api("POST", f"/v1/zones/{zid}/transport", {"action": "stop"})
            except Exception:  # noqa: BLE001
                pass
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

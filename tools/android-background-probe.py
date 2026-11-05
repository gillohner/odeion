#!/usr/bin/env python3
"""Does the Android app keep playing in the background? (TASK-android-background-playback-1105;
owner: playback stops a few seconds after leaving the app and resumes when it is opened again.)

On an emulator (CI: android-background.yml), with a node on the host as tools/android-endpoint.py
sets it up (that script's helpers are reused), both ways the app can make sound:

  A. the phone's own zone (the app's native player, AAudio through cpal, kept alive by
     PlayerService): a track is queued on it through the node's API, the app is sent to the
     background (HOME) and the screen turned off; the node's position must keep advancing in real
     time for 60 s, then 30 s more with the screen off;
  B. the in-app player (the WebView's <audio>, the zone named "This browser" / "This phone"): the
     zone is picked and a song tapped in the app; `dumpsys audio` must still list the app's player
     as started 20 s and 60 s after HOME, and the mini player's time must have moved on when the
     app is opened again.

Along the way: logcat (the app, ActivityManager, Chromium), `dumpsys activity services` (is the
service in the foreground), `dumpsys activity processes` (is the app frozen), `dumpsys audio`
(players and their states), screenshots. Everything lands in the out dir; the exit code is non-zero
when either way stops, and the report says which.

Usage: android-background-probe.py <apk> <pm-node binary> <out dir>
"""
import importlib.util
import math
import os
import re
import struct
import subprocess
import sys
import time
import wave
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("android_endpoint", HERE / "android-endpoint.py")
ep = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ep)  # reads the same argv: <apk> <pm-node> <out>
adb, api, until, log, ui_nodes, tap, center = ep.adb, ep.api, ep.until, ep.log, ep.ui_nodes, ep.tap, ep.center
PKG, out = ep.PKG, ep.out
apk, node_bin = sys.argv[1], sys.argv[2]
results = {}


def shot(name):
    try:
        png = subprocess.run(["adb", "exec-out", "screencap", "-p"], capture_output=True, timeout=60).stdout
        (out / f"{name}.png").write_bytes(png)
    except Exception as e:  # noqa: BLE001
        log(f"no screenshot {name}: {e}")


def dump(name, *args):
    text = adb("shell", "dumpsys", *args, check=False)
    (out / f"{name}.txt").write_text(text)
    return text


def players():
    """The app's audio players as `dumpsys audio` lists them: (type, state) pairs."""
    text = dump("audio", "audio")
    uid = adb("shell", "cmd", "package", "list", "packages", "-U", PKG, check=False)
    m = re.search(r"uid:(\d+)", uid)
    want = m.group(1) if m else None
    found = []
    for line in text.splitlines():
        if "AudioPlaybackConfiguration" in line or "PlayerBase" in line or "state:" in line:
            if want and f"u/pid:{want}" not in line and f"uid:{want}" not in line and want not in line:
                continue
            st = re.search(r"state:\s*(\w+)", line)
            ty = re.search(r"type:\s*([\w ]+?)\s", line)
            if st:
                found.append((ty.group(1).strip() if ty else "?", st.group(1)))
    return found


def frozen():
    text = dump("processes", "activity", "processes", PKG)
    m = re.search(r"(mFrozen|isFrozen|frozen)=?\s*:?\s*(true|false)", text)
    return m.group(2) if m else "unknown"


def service_foreground():
    text = dump("services", "activity", "services", PKG)
    return "PlayerService" in text and "isForeground=true" in text


def screen(on):
    adb("shell", "input", "keyevent", "KEYCODE_WAKEUP" if on else "KEYCODE_SLEEP")
    time.sleep(2)


def node_toml():
    (out / "music").mkdir(parents=True, exist_ok=True)
    (out / "data").mkdir(exist_ok=True)
    with wave.open(str(out / "music" / "tone.wav"), "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(44100)
        frame = [struct.pack("<hh", v, v) for v in (int(2000 * math.sin(2 * math.pi * 440 * n / 44100)) for n in range(100))]
        w.writeframes(b"".join(frame) * (441 * 180))
    (out / "node.toml").write_text(f"""data_dir = "{out}/data"
[library]
folders = ["{out}/music"]
[api]
listen = "{os.environ.get('PM_TEST_LISTEN', '127.0.0.1:7788')}"
announce = false
[outputs]
discover = false
upnp = false
cast = false
airplay = false
slimproto_port = 0
[remote]
enabled = false
""")


def connect_app():
    """Install, start, connect by address, wait for the page, the service and the registration
    (as android-endpoint.py does)."""
    sdk = int(adb("shell", "getprop", "ro.build.version.sdk").strip() or 0)
    log(f"Android API {sdk}")
    log(adb("install", "-r", *([] if sdk >= 33 else ["-g"]), apk).strip())
    ep.relay(ep.GUEST)
    adb("logcat", "-c")
    lc = subprocess.Popen(["adb", "logcat", "-v", "time", "pubky-music:V", "AndroidRuntime:E", "ActivityManager:I", "chromium:I", "cr_MediaSession:V", "cr_AwContents:V", "*:S"],
                          stdout=open(out / "logcat.txt", "w"), stderr=subprocess.STDOUT)
    adb("shell", "am", "start", "-W", "-n", f"{PKG}/.MainActivity")
    tap(lambda n: "connect by address" in n, "“Or connect by address”")
    tap(lambda n: 'resource-id="url"' in n or "EditText" in n, "the address field")
    adb("shell", "input", "text", "10.0.2.2:7788")
    adb("shell", "input", "keyevent", "66")
    until("the node's page in the app", lambda: any("Now playing" in n for n in ui_nodes()), timeout=120, every=3)
    until("the player service", lambda: "player service started" in ep.logcat(), timeout=90)

    def registered():
        text = ep.logcat()
        ok = re.search(r"player registered with the node address=(\S+)", text)
        if ok:
            return (True, ok.group(1))
        bad = re.search(r"player not registered[^\n]*address=(\S+)", text)
        if bad:
            return (False, bad.group(1))
        return None

    done, address = until("the player's registration", registered, timeout=90)
    if not done:
        log(f"the player registered as {address}, not {ep.GUEST}: relaying that and starting the app again")
        ep.relay(address)
        adb("shell", "am", "force-stop", PKG)
        adb("shell", "am", "start", "-W", "-n", f"{PKG}/.MainActivity")
        until("the node's page again", lambda: any("Now playing" in n for n in ui_nodes()), timeout=120, every=3)
        done, address = until("the player's registration", registered, timeout=90)
        assert done, f"the node cannot reach the player at {address}"
    log(f"app connected; player registered as {address}")
    shot("0-connected")
    return lc, address


def part_a(track, address):
    """The phone's own zone, from the API."""
    outs = until("the phone's outputs on the node", lambda: [o for o in api("GET", "/v1/outputs") if o["id"].startswith(f"net:{address}/")], timeout=30)
    want = os.environ.get("PM_TEST_OUTPUT", "Speaker or headphones")
    phone = next((o for o in outs if want.lower() in o["name"].lower()), None)
    assert phone, f"no output named like {want!r}: {[o['name'] for o in outs]}"
    zone = until("the phone's zone", lambda: next((z for z in api("GET", "/v1/zones") if z["output"] == phone["id"]), None))
    zid = zone["id"]
    api("POST", f"/v1/zones/{zid}/queue", {"track_ids": [track["id"]], "replace": True, "play": True})
    until("playback", lambda: ep.position(zid)[1], timeout=30)
    rate, tr = ep.rate_over(zid, 10)
    log(f"A foreground: {rate:.2f}x ({tr}); players {players()}")
    adb("shell", "input", "keyevent", "3")
    log("A: HOME")
    time.sleep(5)
    log(f"A after 5 s: service foreground {service_foreground()}, frozen {frozen()}, players {players()}")
    rate1, tr1 = ep.rate_over(zid, 55)
    log(f"A background 60 s: {rate1:.2f}x ({tr1}); service foreground {service_foreground()}, frozen {frozen()}, players {players()}")
    screen(False)
    log("A: screen off")
    rate2, tr2 = ep.rate_over(zid, 30)
    log(f"A screen off 30 s: {rate2:.2f}x ({tr2}); frozen {frozen()}, players {players()}")
    screen(True)
    api("POST", f"/v1/zones/{zid}/transport", {"action": "stop"})
    ok = tr1 == "playing" and 0.8 < rate1 < 1.2 and tr2 == "playing" and 0.8 < rate2 < 1.2
    results["A native zone, background + screen off"] = ok
    return ok


def mini_time():
    """The mini player's elapsed time as the screen shows it (m:ss), or None."""
    for n in ui_nodes():
        m = re.search(r'text="(\d+:\d\d)"', n)
        if m:
            return m.group(1)
    return None


def part_b():
    """The in-app player: pick the zone and tap a song in the app."""
    adb("shell", "am", "start", "-W", "-n", f"{PKG}/.MainActivity")
    time.sleep(2)
    tap(lambda n: 'resource-id="zonebtn"' in n or 'content-desc="Playing on' in n, "the zone button")
    tap(lambda n: re.search(r'text="This (browser|phone|computer)"', n), "the in-app player in the picker")
    time.sleep(1)
    tap(lambda n: 'text="Library"' in n and 'resource-id="main"' not in n, "Library")
    time.sleep(2)
    tapped = False
    for pred, what in [(lambda n: 'text="Songs"' in n, "Songs"), (lambda n: re.search(r'text="tone"', n), "the song")]:
        try:
            tap(pred, what)
            tapped = what == "the song"
            time.sleep(2)
        except Exception as e:  # noqa: BLE001
            log(f"B: {what} not found ({e})")
    shot("b-0-tapped")
    if not tapped:
        screen_text = " / ".join(t for t in re.findall(r'text="([^"]+)"', "\n".join(ui_nodes())) if t.strip())[:400]
        log(f"B inconclusive: could not start a song in the app; on screen: {screen_text!r}")
        results["B in-app player (inconclusive: no song started)"] = None
        return None
    time.sleep(5)
    t0 = mini_time()
    p0 = players()
    log(f"B foreground: mini player {t0}, players {p0}")
    adb("shell", "input", "keyevent", "3")
    log("B: HOME")
    time.sleep(20)
    p1 = players()
    log(f"B after 20 s: players {p1}, service foreground {service_foreground()}, frozen {frozen()}")
    time.sleep(40)
    p2 = players()
    log(f"B after 60 s: players {p2}, frozen {frozen()}")
    adb("shell", "am", "start", "-W", "-n", f"{PKG}/.MainActivity")
    time.sleep(3)
    t1 = mini_time()
    shot("b-1-back")
    log(f"B back in the app: mini player {t1} (was {t0})")

    def secs(t):
        if not t:
            return None
        m, s = t.split(":")
        return int(m) * 60 + int(s)

    moved = secs(t0) is not None and secs(t1) is not None and secs(t1) - secs(t0) >= 50
    started = any(st == "started" for _, st in p2)
    ok = moved and started
    results["B in-app player, background 60 s"] = ok
    return ok


def main():
    node_toml()
    node = subprocess.Popen([node_bin, "--config", str(out / "node.toml"), "serve"], stdout=open(out / "node.log", "w"), stderr=subprocess.STDOUT)
    lc = None
    try:
        until("the node", lambda: api("GET", "/v1/info"))

        def tracks():
            t = api("GET", "/v1/library/tracks")
            return t if isinstance(t, list) else t.get("items") or t.get("tracks") or []

        track = until("the scan", tracks)[0]
        lc, address = connect_app()
        try:
            part_a(track, address)
        except Exception as e:  # noqa: BLE001
            log(f"A failed: {e}")
            results["A native zone, background + screen off"] = False
        try:
            part_b()
        except Exception as e:  # noqa: BLE001
            log(f"B failed: {e}")
            results["B in-app player, background 60 s"] = False
        dump("services-final", "activity", "services", PKG)
        dump("processes-final", "activity", "processes", PKG)
        dump("audio-final", "audio")
    finally:
        if lc:
            lc.terminate()
        node.terminate()
    log("RESULTS: " + "; ".join(f"{k}: {'PASS' if v else 'inconclusive' if v is None else 'FAIL'}" for k, v in results.items()))
    (out / "results.txt").write_text("\n".join(f"{k}: {v}" for k, v in results.items()))
    sys.exit(0 if all(v for v in results.values() if v is not None) and results else 1)


if __name__ == "__main__":
    main()

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
  C. (diagnostic) B with the player service stopped while away: from Android 12 the frozen app stops;
  D. back on screen the app starts the service again, and B keeps playing (C must have stopped it);
  E. the service as the app starts it when it plays the music itself (through the tunnel, offline):
     it holds its wake lock and B keeps playing; paused and out of sight it ends after 5 min, and
     starts again when the app is opened.

Along the way: logcat (the app, ActivityManager, Chromium), `dumpsys activity services` (is the
service in the foreground), `dumpsys activity processes` (is the app frozen), `dumpsys audio`
(players and their states), screenshots. Everything lands in the out dir; the exit code is non-zero
when a gating part (A, B, D, E) fails, and the report says which.

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
# Shown, never gating (part C's outcome depends on the Android version's freezer)
results_info = {}
lc_proc = None
# Whether part C stopped the service: part D's restart means something only then
c_stopped = None


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
        w.writeframes(b"".join(frame) * (441 * 600))
    (out / "node.toml").write_text(f"""data_dir = "{out}/data"
[library]
folders = ["{out}/music"]
[api]
listen = "{os.environ.get('PM_TEST_LISTEN', '127.0.0.1:7788')}"
token = "{ep.TOKEN}"
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
    global lc_proc
    lc = lc_proc = subprocess.Popen(["adb", "logcat", "-v", "time", "pubky-music:V", "AndroidRuntime:E", "ActivityManager:I", "chromium:I", "Tauri/Console:V", "cr_MediaSession:V", "cr_AwContents:V", "*:S"],
                                    stdout=open(out / "logcat.txt", "w"), stderr=subprocess.STDOUT)
    adb("shell", "am", "start", "-W", "-n", f"{PKG}/.MainActivity")
    tap(lambda n: "connect by address" in n, "“Or connect by address”")
    # The address form opens under the link: let it draw before looking for its fields
    time.sleep(2)
    tap(lambda n: 'resource-id="url"' in n or "EditText" in n, "the address field")
    # The runner as 10.0.2.2, except on the API 37 image, which can't reach it there ("Can't reach
    # http://10.0.2.2:7788" on 6226c456): there through `adb reverse`, as the update and Ring probes do
    if sdk >= 37:
        adb("reverse", "tcp:7788", "tcp:7788")
        adb("shell", "input", "text", "127.0.0.1:7788")
    else:
        adb("shell", "input", "text", "10.0.2.2:7788")
    # The node has a token (as android-endpoint.py sets it up): a node without one answers only at
    # its loopback names, and the app reaches it as 10.0.2.2
    tap(lambda n: 'resource-id="tok"' in n or ("EditText" in n and 'password="true"' in n), "the token field")
    adb("shell", "input", "text", ep.TOKEN)
    adb("shell", "input", "keyevent", "66")
    try:
        until("the node's page in the app", lambda: any("Now playing" in n for n in ui_nodes()), timeout=120, every=3)
    except RuntimeError:
        # What the screen showed instead (kept with the run's artifacts)
        subprocess.run(["adb", "exec-out", "screencap", "-p"], stdout=open(out / "connect-timeout.png", "wb"), check=False)
        (out / "connect-timeout-ui.txt").write_text("\n".join(ui_nodes()))
        raise
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
    """Now playing's elapsed time as the screen shows it (the first m:ss), or None. The screen's
    nodes are kept (ui-time-<n>.xml) so a miss can be read afterwards."""
    nodes = ui_nodes()
    k = len(list(out.glob("ui-time-*.xml")))
    (out / f"ui-time-{k}.xml").write_text("\n".join(nodes))
    for n in nodes:
        m = re.search(r'(?:text|content-desc)="(\d+:\d\d)', n)
        if m:
            return m.group(1)
    return None


def secs(t):
    if not t:
        return None
    m, s_ = t.split(":")
    return int(m) * 60 + int(s_)


def flinger(name):
    """`dumpsys media.audio_flinger`, kept: each track's server frame count shows whether its audio
    advanced (the WebView's track has no UI to read the time from)."""
    text = adb("shell", "dumpsys", "media.audio_flinger", check=False)
    (out / f"flinger-{name}.txt").write_text(text)
    return text


def track_frames(text):
    """Server frame counts of the active tracks in a flinger dump (best effort across versions)."""
    return [int(x) for x in re.findall(r"(?:Server|server)\s*[:=]?\s*(?:0x)?([0-9]{4,})", text)]


def away(seconds, screen_off=False, during=None):
    """Leave the app (HOME, and the screen off if asked) for `seconds`, come back, and return how far
    Now playing's time moved (None when it can't be read)."""
    tag = f"{seconds}{'-off' if screen_off else ''}"
    f0 = track_frames(flinger(f"before-{tag}"))
    # (the time can be missing for a moment while the page redraws: read it again before leaving)
    t0 = None
    for _ in range(6):
        t0 = secs(mini_time())
        if t0 is not None:
            break
        time.sleep(1)
    w0 = time.time()
    adb("shell", "input", "keyevent", "3")
    if screen_off:
        screen(False)
    if during:
        during()
    time.sleep(seconds / 2)
    log(f"B away {seconds} s{' (screen off)' if screen_off else ''}: halfway players {players()}, frozen {frozen()}")
    time.sleep(seconds / 2)
    f1 = track_frames(flinger(f"after-{tag}"))
    log(f"B away {tag}: track server frames before {f0[:6]} after {f1[:6]}")
    if screen_off:
        screen(True)
        adb("shell", "wm", "dismiss-keyguard", check=False)
    adb("shell", "am", "start", "-W", "-n", f"{PKG}/.MainActivity")
    time.sleep(1)
    t1 = secs(mini_time()); w1 = time.time()
    moved = None if t0 is None or t1 is None else t1 - t0
    log(f"B away {seconds} s{' (screen off)' if screen_off else ''}: Now playing {t0} -> {t1} s over {w1 - w0:.0f} s of wall time (moved {moved})")
    return moved, w1 - w0


def ui_dump(name):
    """The screen's nodes, kept in the out dir (what B looked for and didn't find)."""
    nodes = ui_nodes()
    (out / f"ui-{name}.xml").write_text("\n".join(nodes))
    return nodes


def tap_text(words, what, timeout=30):
    """Tap the first node whose text or content-desc contains one of `words`."""
    def find():
        for n in ui_nodes():
            for w in words:
                if f'text="{w}' in n or f'content-desc="{w}' in n or (w.startswith('~') and re.search(w[1:], n)):
                    return n
        return None
    try:
        n = until(what, find, timeout=timeout, every=2)
    except RuntimeError:
        ui_dump(what.replace(" ", "-"))
        raise
    x, y = center(n)
    adb("shell", "input", "tap", str(x), str(y))
    log(f"B: tapped {what}")


def part_b():
    """The in-app player (the WebView's <audio>, the app's default zone on a fresh install): play from
    Home's "Shuffle my music" or a song in the library, then HOME. Runs first, before A, so the app's
    zone is still its own player."""
    time.sleep(2)
    shot("b-0-home")
    try:
        tap_text(["Shuffle my music"], "Shuffle my music", timeout=40)
    except RuntimeError:
        tap_text(["Library"], "Library")
        time.sleep(2)
        tap_text(["Songs"], "Songs")
        time.sleep(2)
        tap_text(["tone"], "the song")
    time.sleep(4)
    # Now playing shows the elapsed time: open it from the mini player
    # (the song's title in the mini player: "Tone"; not the sound pill beside it)
    try:
        tap_text(["Now playing"], "the mini player (Now playing)", timeout=20)
    except RuntimeError:
        log("B: no mini player title to open Now playing with")
    time.sleep(3)
    shot("b-1-playing")
    log(f"B foreground: Now playing {mini_time()}, players {players()}")
    moved1, wall1 = away(60)
    shot("b-2-back")
    moved2, wall2 = away(30, screen_off=True)
    shot("b-3-back-screen")
    ok1 = moved1 is not None and moved1 >= wall1 - 8
    ok2 = moved2 is not None and moved2 >= wall2 - 8
    log(f"B verdict: kept playing in the background {ok1} ({moved1} of {wall1:.0f} s); with the screen off {ok2} ({moved2} of {wall2:.0f} s)")
    results["B in-app player, background 60 s"] = ok1
    results["B in-app player, screen off 30 s"] = ok2
    return ok1 and ok2


def restart_logcat():
    """`adb root` restarts adbd, which ends the running logcat: start it again, appending."""
    global lc_proc
    try:
        lc_proc.terminate()
    except Exception:  # noqa: BLE001
        pass
    lc_proc = subprocess.Popen(["adb", "logcat", "-v", "time", "pubky-music:V", "AndroidRuntime:E", "ActivityManager:I", "*:S"],
                               stdout=open(out / "logcat.txt", "a"), stderr=subprocess.STDOUT)


def part_c():
    """Diagnostic (not gating): the in-app player with the player service stopped while the app is
    away, as when nothing started it (the tunnel and offline cases before the fix). From Android 12 a
    cached app is frozen, so the music is expected to stop there; API 30 has no freezer."""
    r = subprocess.run(["adb", "root"], capture_output=True, text=True, timeout=60)
    log(f"C: adb root: {(r.stdout + r.stderr).strip()}")
    time.sleep(5)
    subprocess.run(["adb", "wait-for-device"], timeout=120)
    restart_logcat()
    adb("shell", "am", "start", "-W", "-n", f"{PKG}/.MainActivity")
    time.sleep(2)
    try:
        tap_text(["Now playing"], "the mini player (Now playing)", timeout=10)
    except RuntimeError:
        pass
    time.sleep(2)
    for n in ui_nodes():
        if re.search(r'(text|content-desc)="Play"', n):
            x, y = center(n)
            adb("shell", "input", "tap", str(x), str(y))
            log("C: pressed Play")
            break
    time.sleep(4)
    log(f"C foreground: Now playing {mini_time()}, players {players()}")

    def stop_service():
        global c_stopped
        out_ = adb("shell", "am", "stopservice", "-n", f"{PKG}/.PlayerService", check=False)
        time.sleep(2)
        c_stopped = not service_foreground()
        log(f"C: stopservice while away: {out_.strip()[:100]}; service stopped {c_stopped}")

    moved, wall = away(60, during=stop_service)
    shot("c-back")
    stops = moved is not None and moved < wall - 20
    log(f"C (diagnostic): without the service the in-app player {'STOPPED' if stops else 'kept playing'} in the background ({moved} of {wall:.0f} s)")
    results_info["C without the service (diagnostic)"] = "stopped" if stops else "kept playing" if moved is not None else "unknown"
    return stops


def part_d():
    """The fix: back on screen, the app starts the service again (Resumed), and the in-app player
    keeps playing in the background. Before the fix nothing started it again after C."""
    if not c_stopped:
        # Without a stopped service there is no restart to see: a setup failure, not a pass
        log(f"D: C did not stop the service (stopped {c_stopped}): nothing to restart, D fails")
        results["D service back on resume, in-app player keeps playing"] = False
        return False
    time.sleep(3)
    fg = service_foreground()
    log(f"D: back on screen after C: service foreground {fg}")
    # Where C's stopped service let the player stop (API 37), it stays paused: D is about the
    # service coming back and carrying playback from here, so play again first if it isn't playing
    a = secs(mini_time()); time.sleep(3); b = secs(mini_time())
    if a is None or b is None or b <= a:
        try:
            tap_text(['~(?:text|content-desc)="Play"'], "Play (C left the player paused)", timeout=10)
            time.sleep(3)
        except RuntimeError:
            log("D: no Play to tap")
    moved, wall = away(60)
    ok = fg and moved is not None and moved >= wall - 8
    log(f"D verdict: service back {fg}; kept playing {moved} of {wall:.0f} s")
    results["D service back on resume, in-app player keeps playing"] = ok
    return ok


def wake_lock_held():
    """Whether the player service's partial wake lock ("pubky-music:player") is held."""
    text = adb("shell", "dumpsys", "power", check=False)
    (out / f"power-{len(list(out.glob('power-*.txt')))}.txt").write_text(text)
    return any("pubky-music:player" in line and "PARTIAL_WAKE_LOCK" in line for line in text.splitlines())


def app_mode():
    """Start the service as the app does when it plays the music itself (through the tunnel, the
    offline app): `zone` false. The probe's app reaches its node directly, so it never picks this
    mode itself (the unit test the_player_service_mode_for_each_connection covers the choice)."""
    # (quoted: `adb shell` hands the words to the device's shell, which split the text and took
    # the rest, the zone extra too, for other arguments; the service then started as a zone)
    r = adb("shell", "am", "start-foreground-service", "-n", f"{PKG}/.PlayerService",
            "--es", "text", "'Plays music from the probe'", "--ez", "zone", "false", check=False)
    time.sleep(3)
    fg = service_foreground()
    starts = [l for l in ep.logcat().splitlines() if "player service: start" in l]
    app = bool(starts) and starts[-1].rstrip().endswith("start app mode")
    log(f"E: app mode: {r.strip()[:120]}; service foreground {fg}; the service says {starts[-1].split('player service: ')[-1].strip() if starts else 'nothing'}")
    return fg and app and "Error" not in r


def part_e():
    """The app playing the music itself (the owner's case: through the tunnel, the offline app).
    E1: the service in app mode holds its wake lock and keeps the in-app player going 60 s away.
    E2: paused and out of sight, it ends after 5 min with nothing playing (and its lock with it),
    and starts again when the app comes back."""
    # E1 (the app is on screen and playing, after D): from a stopped service, so app mode takes
    # its wake lock itself (D's resume left it running as a zone, with the zone's lock)
    adb("shell", "am", "stopservice", "-n", f"{PKG}/.PlayerService", check=False)
    time.sleep(2)
    stopped = not service_foreground() and not wake_lock_held()
    started = app_mode()
    lock1 = wake_lock_held()
    if not (stopped and started):
        log(f"E1: setup failed: stopped first {stopped}, app mode started {started}")
        lock1 = False
    moved, wall = away(60)
    ok1 = lock1 and moved is not None and moved >= wall - 8
    log(f"E1 verdict: fresh app mode holds the wake lock {lock1}; kept playing {moved} of {wall:.0f} s")
    results["E1 app mode: wake lock, in-app player keeps playing"] = ok1
    # E2: pause on Now playing, app mode again (coming back above restarted it as a zone), HOME
    try:
        tap_text(["Now playing"], "the mini player (Now playing)", timeout=10)
    except RuntimeError:
        pass
    time.sleep(2)
    tap_text(["Pause"], "Pause", timeout=20)
    time.sleep(3)
    if not app_mode():
        raise RuntimeError("E2: app mode did not start")
    adb("shell", "input", "keyevent", "3")
    t0 = time.time()
    ended = None
    while time.time() - t0 < 9 * 60:
        time.sleep(30)
        # What the service's watch saw (its debug lines) and who holds audio, for a failure's cause
        lines = [l for l in ep.logcat().splitlines() if "player service: watch" in l or "player service: start" in l]
        log(f"E2 +{round(time.time() - t0)} s: {lines[-1].split('player service: ')[-1] if lines else 'no watch line yet'}; players {players()}")
        if not service_foreground():
            ended = time.time() - t0
            break
    lock2 = wake_lock_held()
    log(f"E2: idle and out of sight: service ended after {ended if ended is None else round(ended)} s; wake lock held {lock2}")
    adb("shell", "am", "start", "-W", "-n", f"{PKG}/.MainActivity")
    time.sleep(4)
    back = service_foreground()
    # The watch ticks every 60 s and stops at 5 min idle: about 5 min after HOME (polled every 30 s)
    ok2 = ended is not None and 4.5 * 60 <= ended <= 7 * 60 and not lock2 and back
    log(f"E2 verdict: ended when idle {ended is not None} ({ended if ended is None else round(ended)} s), lock released {not lock2}, back on resume {back}")
    results["E2 app mode: ends when idle out of sight, back on resume"] = ok2
    return ok1 and ok2


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
        # B first: on a fresh install the app's zone is its own player
        try:
            part_b()
        except Exception as e:  # noqa: BLE001
            log(f"B failed: {e}")
            results["B in-app player, background 60 s"] = False
        try:
            part_a(track, address)
        except Exception as e:  # noqa: BLE001
            log(f"A failed: {e}")
            results["A native zone, background + screen off"] = False
        try:
            part_c()
        except Exception as e:  # noqa: BLE001
            log(f"C failed: {e}")
            results_info["C without the service (diagnostic)"] = f"error: {e}"
        try:
            part_d()
        except Exception as e:  # noqa: BLE001
            log(f"D failed: {e}")
            results["D service back on resume, in-app player keeps playing"] = False
        try:
            part_e()
        except Exception as e:  # noqa: BLE001
            log(f"E failed: {e}")
            results.setdefault("E1 app mode: wake lock, in-app player keeps playing", False)
            results.setdefault("E2 app mode: ends when idle out of sight, back on resume", False)
        dump("services-final", "activity", "services", PKG)
        dump("processes-final", "activity", "processes", PKG)
        dump("audio-final", "audio")
    finally:
        for p_ in (lc, lc_proc):
            if p_:
                p_.terminate()
        node.terminate()
    log("RESULTS: " + "; ".join(f"{k}: {'PASS' if v else 'inconclusive' if v is None else 'FAIL'}" for k, v in results.items()))
    log("INFO: " + "; ".join(f"{k}: {v}" for k, v in results_info.items()))
    (out / "results.txt").write_text("\n".join(f"{k}: {v}" for k, v in results.items()))
    sys.exit(0 if all(v for v in results.values() if v is not None) and results else 1)


if __name__ == "__main__":
    main()

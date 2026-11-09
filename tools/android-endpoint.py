#!/usr/bin/env python3
"""The phone as a zone, on an Android emulator (CI: release.yml, android-emulator).

The release APK is installed and connected by address to a node on the host
(10.0.2.2 from the emulator) over plain http, and must show the node's page.
Its built-in player (AAudio through cpal)
must become a zone that plays a track in real time, and keep playing with the
app in the background: the foreground service (PlayerService.kt) keeps the
process alive.

The emulator is behind its own NAT, so the node cannot reach the address the
app registers itself under (the emulator's own, 10.0.2.15). This script makes
that address reach the phone: it adds it to the runner's loopback and relays it
through `adb forward` to the player's port. The app's own registration (its
address and the player's key, which only it and the node know) then works as on
a phone on a home network; the node plays only with that key (protocol 2).

Then, as a phone does on later days:
- on Android 13+ the app is installed without granting anything, and the notification
  permission is refused, as a cautious owner would;
- the app is stopped and started again: it reconnects to the saved node by itself and its
  player service starts again (the path a first-launch test never takes);
- the app is made to crash (`am crash`): on the next launch it shows the crash report
  ("closed unexpectedly", tools/release/android/CrashReport.kt), and Continue carries on.

While the phone's zone plays, the system's media controls (tools/release/android/MediaControl.kt,
crates/pm-desktop/src/media.rs): `dumpsys media_session` shows the app's session active with the
track's title and PLAYING; KEYCODE_MEDIA_NEXT moves the node's queue; one volume: Android's media
volume is the zone's (set at 0, KEYCODE_VOLUME_UP raises it one step and the node reads it back,
the node's set moves it back to 0); a network zone (the node's null output with a percent volume)
chosen in the page gets the volume keys as its own volume, Android's media volume untouched.
Screenshots of the media notification and the lock screen. Volumes stay at 0-5 %.

Usage: android-endpoint.py <apk> <pm-node binary> <out dir>
"""
import json
import os
import re
import secrets
import socket
import subprocess
import sys
import threading
import time
import urllib.request
import wave
from pathlib import Path

PKG = "to.synonym.pubkymusic"
NODE = os.environ.get("PM_TEST_NODE", "http://127.0.0.1:7788")
# The node gets an API token: the app reaches it as 10.0.2.2 (the emulator's name for the runner),
# and a node without one answers only at its loopback names. Made per run.
TOKEN = secrets.token_hex(16)
apk, node_bin, out = sys.argv[1], sys.argv[2], Path(sys.argv[3])


def log(msg):
    print(f"[{time.strftime('%H:%M:%S')}] {msg}", flush=True)


def adb(*args, check=True):
    r = subprocess.run(["adb", *args], capture_output=True, text=True, timeout=120)
    if check and r.returncode != 0:
        raise RuntimeError(f"adb {' '.join(args)}: {r.stderr.strip() or r.stdout.strip()}")
    return r.stdout


def api(method, path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(
        NODE + path,
        data=data,
        method=method,
        headers={"content-type": "application/json", "authorization": f"Bearer {TOKEN}"},
    )
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            raw = r.read()
    except urllib.error.HTTPError as e:
        raise RuntimeError(f"{method} {path}: {e.code} {e.read().decode(errors='replace')}") from None
    return json.loads(raw) if raw else None


def until(what, fn, timeout=60, every=1.0):
    end = time.time() + timeout
    while True:
        try:
            v = fn()
            if v:
                return v
        except Exception as e:  # noqa: BLE001 (retried until the deadline)
            last = e
        else:
            last = None
        if time.time() > end:
            raise RuntimeError(f"timed out waiting for {what}" + (f" (last error: {last})" if last else ""))
        time.sleep(every)


def fresh_logcat():
    """Start the log over for the next attempt, keeping what came before (logcat-<n>.txt in the
    artifact), so a failure after a retry still shows why the first attempt ended."""
    log_file = out / "logcat.txt"
    if log_file.exists():
        n = len(list(out.glob("logcat-*.txt"))) + 1
        (out / f"logcat-{n}.txt").write_bytes(log_file.read_bytes())
    log_file.write_text("")


def logcat():
    return (out / "logcat.txt").read_text(errors="replace") if (out / "logcat.txt").exists() else ""


# ActivityManager killing the app itself, which is not a crash: e.g. "depends on provider
# com.google.android.gms/.fonts.provider.FontsProvider in dying proc com.google.android.gms.persistent"
# when Play services restarts on the emulator (release run 167, API 33)
KILLED = re.compile(r"Killing \d+:" + re.escape(PKG) + r"/\S+ \(adj -?\d+\): (.*)")


def system_kills():
    return KILLED.findall(logcat())


# (any case: the API 37 image says "DON’T ALLOW")
PROMPT = re.compile(r'text="(Don.t allow|Deny)"', re.I)


def ui_nodes():
    """The screen's nodes; Android 13+'s notification prompt, whenever it shows, is refused."""
    for _ in range(3):
        adb("shell", "uiautomator", "dump", "/sdcard/ui.xml")
        xml = adb("exec-out", "cat", "/sdcard/ui.xml")
        nodes = re.findall(r"<node [^>]*>", xml)
        prompt = next((n for n in nodes if PROMPT.search(n)), None)
        if not prompt:
            return nodes
        x, y = center(prompt)
        adb("shell", "input", "tap", str(x), str(y))
        log("notifications refused")
        time.sleep(1)
    return nodes


def center(node):
    m = re.search(r'bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', node)
    x1, y1, x2, y2 = map(int, m.groups())
    return (x1 + x2) // 2, (y1 + y2) // 2


def tap(pred, what):
    def find():
        for n in ui_nodes():
            if pred(n):
                return n
        return None

    node = until(what, find, timeout=90, every=2)
    x, y = center(node)
    if (x, y) == (0, 0):  # off screen: scroll it into view and look again
        adb("shell", "input", "swipe", "500", "1500", "500", "500", "300")
        node = until(what, find, timeout=30, every=2)
        x, y = center(node)
    adb("shell", "input", "tap", str(x), str(y))
    log(f"tapped {what} at {x},{y}")


def shot(name):
    png = subprocess.run(["adb", "exec-out", "screencap", "-p"], capture_output=True, timeout=60).stdout
    (out / f"{name}.png").write_bytes(png)


def check_status_bar():
    """The page starts below the status bar (Tauri draws edge to edge; android-patch.py pads)."""
    win = adb("shell", "dumpsys", "window")
    bars = [int(m.group(1)) for m in re.finditer(r"type=statusBars[^\n]*?frame=\[\d+,\d+\]\[\d+,(\d+)\]", win)]
    menu = next((n for n in ui_nodes() if 'content-desc="Menu"' in n or 'text="Menu"' in n), None)
    if not bars or not menu:
        log(f"status bar check skipped (status bar frame found: {bool(bars)}, menu found: {bool(menu)})")
        return
    top = int(re.search(r'bounds="\[\d+,(\d+)\]', menu).group(1))
    assert top >= max(bars), f"the status bar (to y={max(bars)}) covers the node's menu (top y={top})"
    log(f"the node's menu (y={top}) is below the status bar (y<{max(bars)})")


def zone_state(zid):
    return api("GET", f"/v1/zones/{zid}")


def stream_music():
    """Android's media volume as (index, max): `cmd media_session volume --get` where it answers,
    else `dumpsys audio` (the STREAM_MUSIC block: its Max, and Current for the device it plays on).
    What each said goes to volume-cmd.txt."""
    seen = []
    for cmd in (["cmd", "media_session", "volume", "--stream", "3", "--get"], ["media", "volume", "--stream", "3", "--get"]):
        r = subprocess.run(["adb", "shell", *cmd], capture_output=True, text=True, timeout=30)
        seen.append(f"$ {' '.join(cmd)}\n{r.stdout}{r.stderr}")
        m = re.search(r"volume is (\d+) in range \[(\d+)\.\.(\d+)\]", r.stdout + r.stderr)
        if m:
            return int(m.group(1)), int(m.group(3))
    d = adb("shell", "dumpsys", "audio", check=False)
    block = re.search(r"- STREAM_MUSIC:(.*?)(?:\n- STREAM_|\Z)", d, re.S)
    found = None
    if block:
        b = block.group(1)
        top = re.search(r"Max:\s*(\d+)", b)
        cur = re.search(r"Current:\s*([^\n]*)", b)
        dev = re.search(r"Devices:\s*([^\s(]+)", b)
        if top and cur:
            pairs = re.findall(r"\(([^)]*)\):\s*(\d+)", cur.group(1))
            pick = next((v for n, v in pairs if dev and n == dev.group(1)), pairs[0][1] if pairs else None)
            if pick is not None:
                found = (int(pick), int(top.group(1)))
        seen.append("dumpsys audio STREAM_MUSIC:" + b[:600])
    with open(out / "volume-cmd.txt", "a") as f:
        f.write("\n".join(seen) + f"\n=> {found}\n\n")
    return found


def set_stream_music(index):
    """Android's media volume to `index`; down to 0 by the volume-down key where the shell can't set
    it (the app on screen: its local session takes the key, as a phone's own button)."""
    for cmd in (["cmd", "media_session", "volume", "--stream", "3", "--set", str(index)], ["media", "volume", "--stream", "3", "--set", str(index)]):
        subprocess.run(["adb", "shell", *cmd], capture_output=True, text=True, timeout=30)
        s = stream_music()
        if s and s[0] == index:
            return s
    if index == 0:
        for _ in range(60):
            s = stream_music()
            if s and s[0] == 0:
                return s
            adb("shell", "input", "keyevent", "KEYCODE_VOLUME_DOWN")
            time.sleep(0.3)
    raise AssertionError(f"cannot set Android's media volume to {index} (now {stream_music()})")


def percent_of(index, top):
    """pm-out-local's media_volume::percent_of: 100 only at the top index."""
    return 100 if index >= top else min(99, (index * 100 + top // 2) // top)


def our_session():
    """The app's block in `dumpsys media_session` (kept in the artifact), or ""."""
    text = adb("shell", "dumpsys", "media_session")
    (out / "media_session.txt").write_text(text)
    at = text.find(f"package={PKG}")
    if at < 0:
        return ""
    return "\n".join(text[at:].splitlines()[:30])


def zone_percent(zid):
    return (zone_state(zid).get("volume") or {}).get("percent")


def media_controls(zid, track):
    """The system's media controls while the phone's zone plays (see the module's doc)."""
    title = track.get("title") or ""
    def playing():
        s = our_session()
        on = re.search(r"state=PlaybackState \{state=(?:3\b|PLAYING)", s)
        return s if ("active=true" in s and on and (not title or title in s)) else None
    s = until("the app's media session, active and playing, with the track's title", playing, timeout=30)
    log("media session: " + " | ".join(l.strip() for l in s.splitlines() if "state=" in l or "metadata" in l or "active=" in l))

    # One volume: Android's media volume is the zone's. Paused meanwhile: one step up from 0 can
    # be more than 5 % on a phone with few steps, so nothing plays while it is above 0
    api("POST", f"/v1/zones/{zid}/transport", {"action": "pause"})
    until("the zone paused", lambda: zone_state(zid).get("transport") == "paused", timeout=15)
    _, top = set_stream_music(0)
    until("the zone's level at 0 %", lambda: zone_percent(zid) == 0, timeout=20)
    adb("shell", "input", "keyevent", "KEYCODE_VOLUME_UP")
    until("KEYCODE_VOLUME_UP: Android's media volume one step up", lambda: (stream_music() or (None,))[0] == 1, timeout=10)
    want = percent_of(1, top)
    until(f"the zone's level read back at {want} %", lambda: zone_percent(zid) == want, timeout=20)
    log(f"volume key: Android's media volume 1 of {top}, the zone reads {want} %")
    api("POST", f"/v1/zones/{zid}/volume", {"volume": {"percent": 0}})
    until("the node's set: Android's media volume back at 0", lambda: (stream_music() or (None,))[0] == 0, timeout=15)
    log("the zone's volume set from the node moves Android's media volume")
    api("POST", f"/v1/zones/{zid}/transport", {"action": "play"})
    until("the zone playing again (at 0)", lambda: zone_state(zid).get("transport") == "playing", timeout=15)

    # The notification and the lock screen (Android 13+: notifications were refused above, as a
    # cautious owner would; allowed now so the media notification shows)
    adb("shell", "pm", "grant", PKG, "android.permission.POST_NOTIFICATIONS", check=False)
    time.sleep(2)
    adb("shell", "cmd", "statusbar", "expand-notifications", check=False)
    time.sleep(3)
    shot("6-media-notification")
    adb("shell", "cmd", "statusbar", "collapse", check=False)
    adb("shell", "input", "keyevent", "KEYCODE_SLEEP", check=False)
    time.sleep(2)
    adb("shell", "input", "keyevent", "KEYCODE_WAKEUP", check=False)
    time.sleep(3)
    shot("7-lock-screen")
    adb("shell", "wm", "dismiss-keyguard", check=False)
    adb("shell", "input", "keyevent", "82", check=False)
    time.sleep(2)

    # A media button: the node's queue moves on
    at = zone_state(zid).get("queue_index") or 0
    adb("shell", "input", "keyevent", "KEYCODE_MEDIA_NEXT")
    until("KEYCODE_MEDIA_NEXT: the node's queue moves on", lambda: (zone_state(zid).get("queue_index") or 0) == at + 1, timeout=20)
    log("KEYCODE_MEDIA_NEXT reached the node")

    # A network zone chosen in the page: the volume keys are its own volume
    nz = next((z for z in api("GET", "/v1/zones") if z["output"] == "null:percent"), None)
    assert nz, "no zone on the null output with a percent volume (PM_DEV_OUTPUTS)"
    api("POST", f"/v1/zones/{nz['id']}/volume", {"volume": {"percent": 3}})
    api("POST", f"/v1/zones/{nz['id']}/queue", {"track_ids": [track["id"]], "replace": True, "play": False})
    def diagnose(what):
        (out / f"ui-{what}.txt").write_text("\n".join(ui_nodes()))
        lines = [l for l in logcat().splitlines() if "media controls" in l or "message port" in l]
        (out / f"media-{what}.txt").write_text("\n".join(lines[-80:]))

    def pick(name, what):
        """Choose a zone in the page's picker. The WebView's accessibility dump can lag behind
        the screen: the picker is closed and opened again (three tries) before giving up."""
        for attempt in range(3):
            tap(lambda n: "choose where to play" in n, "the zone button")
            end = time.time() + 20
            while time.time() < end:
                node = next((n for n in ui_nodes() if name in n), None)
                if node:
                    x, y = center(node)
                    if (x, y) != (0, 0):
                        adb("shell", "input", "tap", str(x), str(y))
                        log(f"tapped {what} at {x},{y}")
                        return
                    adb("shell", "input", "swipe", "500", "1400", "500", "700", "300")
                time.sleep(2)
            log(f"{what} not in the screen's dump (try {attempt + 1}): the picker again")
            adb("shell", "input", "keyevent", "KEYCODE_BACK")
            time.sleep(2)
        diagnose("picker")
        raise RuntimeError(f"timed out waiting for {what}")

    pick("Null output (% volume)", "the null output's zone")
    def remote():
        s = our_session()
        # Android 11 prints the type as a number, 15 by name
        return s if re.search(r"volumeType=(?:2\b|REMOTE)", s) else None
    try:
        until("the session on the network zone, with its own volume (remote)", remote, timeout=20)
    except RuntimeError:
        diagnose("network-zone")
        raise
    before = stream_music()
    adb("shell", "input", "keyevent", "KEYCODE_VOLUME_UP")
    until("KEYCODE_VOLUME_UP: the network zone at 4 %", lambda: zone_percent(nz["id"]) == 4, timeout=20)
    assert stream_music() == before, f"Android's media volume moved too: {before} -> {stream_music()}"
    log("volume key on the network zone: its own volume 3 -> 4 %, Android's media volume untouched")
    shot("8-network-zone")
    # Back to the phone's zone in the page
    pick(zone_state(zid)["name"][:20], "the phone's zone")


def position(zid):
    z = zone_state(zid)
    if z.get("error"):
        raise RuntimeError(f"zone error: {z['error']}")
    np = z.get("now_playing") or {}
    return z.get("transport"), np.get("position_ms")


def rate_over(zid, seconds):
    t0, (_, p0) = time.time(), position(zid)
    time.sleep(seconds)
    t1, (tr, p1) = time.time(), position(zid)
    if p0 is None or p1 is None:
        raise RuntimeError(f"no position (transport {tr})")
    rate = (p1 - p0) / 1000 / (t1 - t0)
    log(f"position {p0} -> {p1} ms over {t1 - t0:.1f} s: {rate:.2f}x real time ({tr})")
    return rate, tr


# The address the app registers: the emulator's own, on the player's usual port
GUEST = "10.0.2.15:7790"
# The host end of `adb forward` to the player: one port per relay, so a second relay never
# re-points the first
FORWARD = 17790
relays = {}


def relay(address):
    """Make `address` (the emulator's, as the app registers it) reach the phone's player: add its
    IP to the runner's loopback, forward a host port to the player's port with adb, and relay
    connections to the address there."""
    if address in relays:
        return
    ip, port = address.rsplit(":", 1)
    host_port = FORWARD + len(relays)
    r = subprocess.run(["sudo", "ip", "addr", "add", f"{ip}/32", "dev", "lo"], capture_output=True, text=True)
    if r.returncode != 0 and "exists" not in r.stderr:
        raise RuntimeError(f"cannot add {ip} to the loopback: {r.stderr.strip()}")
    adb("forward", f"tcp:{host_port}", f"tcp:{port}")
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind((ip, int(port)))
    srv.listen(16)

    def pipe(a, b):
        try:
            while True:
                data = a.recv(65536)
                if not data:
                    break
                b.sendall(data)
        except OSError:
            pass
        finally:
            for x in (a, b):
                try:
                    x.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass

    def serve():
        while True:
            c, _ = srv.accept()
            try:
                up = socket.create_connection(("127.0.0.1", host_port), timeout=10)
                up.settimeout(None)
            except OSError:
                c.close()
                continue
            threading.Thread(target=pipe, args=(c, up), daemon=True).start()
            threading.Thread(target=pipe, args=(up, c), daemon=True).start()

    threading.Thread(target=serve, daemon=True).start()
    relays[address] = srv
    log(f"{address} relays to the phone's player (adb forward tcp:{host_port} -> tcp:{port})")


def main():
    (out / "music").mkdir(parents=True, exist_ok=True)
    (out / "data").mkdir(exist_ok=True)
    # Two minutes of a quiet 440 Hz tone, 16-bit 44.1 kHz stereo.
    import math
    import struct

    with wave.open(str(out / "music" / "tone.wav"), "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(44100)
        frame = [struct.pack("<hh", v, v) for v in (int(2000 * math.sin(2 * math.pi * 440 * n / 44100)) for n in range(100))]
        w.writeframes(b"".join(frame) * (441 * 120))
    (out / "node.toml").write_text(f"""data_dir = "{out}/data"
[library]
folders = ["{out}/music"]
[api]
listen = "{os.environ.get('PM_TEST_LISTEN', '127.0.0.1:7788')}"
token = "{TOKEN}"
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
    # The developer outputs (a null output with a percent volume: a network zone for the volume
    # keys), and the volume read back every second
    env = {**os.environ, "PM_DEV_OUTPUTS": "1", "PM_VOLUME_POLL_MS": "1000"}
    node = subprocess.Popen([node_bin, "--config", str(out / "node.toml"), "serve"],
                            stdout=open(out / "node.log", "w"), stderr=subprocess.STDOUT, env=env)
    try:
        run(node)
    finally:
        node.terminate()


def run(node):
    until("the node", lambda: api("GET", "/v1/info"))

    def tracks():
        t = api("GET", "/v1/library/tracks")
        items = t if isinstance(t, list) else t.get("items") or t.get("tracks") or []
        return items

    track = until("the scan", tracks)[0]
    log(f"node up, track {track['id']}")
    # The page lists the developer outputs (the network zone the volume keys are tried on)
    api("PUT", "/v1/node/settings", {"dev_outputs": True})

    sdk = int(adb("shell", "getprop", "ro.build.version.sdk").strip() or 0)
    log(f"Android API {sdk}")
    # Android 13+: nothing granted up front; the notification prompt is refused below
    log(adb("install", "-r", *([] if sdk >= 33 else ["-g"]), apk).strip())
    # Before the app starts: the address it will register reaches its player
    relay(GUEST)
    adb("logcat", "-c")
    lc = subprocess.Popen(["adb", "logcat", "-v", "time", "pubky-music:V", "AndroidRuntime:E", "ActivityManager:I", "*:S"],
                          stdout=open(out / "logcat.txt", "w"), stderr=subprocess.STDOUT)
    try:
        adb("shell", "am", "start", "-W", "-n", f"{PKG}/.MainActivity")
        tap(lambda n: "connect by address" in n, "“Or connect by address”")
        # The address field is the page's first text field (its id and hint differ between WebViews)
        tap(lambda n: 'resource-id="url"' in n or "EditText" in n, "the address field")
        adb("shell", "input", "text", "10.0.2.2:7788")
        # The node's token in the form's token field
        tap(lambda n: 'resource-id="tok"' in n or ("EditText" in n and 'password="true"' in n), "the token field")
        adb("shell", "input", "text", TOKEN)
        adb("shell", "input", "keyevent", "66")
        shot("1-connecting")
        def node_page():
            nodes = "\n".join(ui_nodes())
            if "Webpage not available" in nodes or "ERR_" in nodes:
                err = re.search(r'text="(net::ERR_[A-Z_]+)"', nodes)
                raise SystemExit(f"FAIL: the app cannot load the node's page ({err.group(1) if err else 'Webpage not available'})")
            return "Now playing" in nodes

        def diagnose():
            """What the app shows, and whether the node still answers the runner."""
            try:
                screen = " / ".join(t for t in re.findall(r'text="([^"]+)"', "\n".join(ui_nodes())) if t.strip())[:300]
            except Exception as e:  # noqa: BLE001
                screen = f"(no screen: {e})"
            try:
                api("GET", "/v1/info")
                node = "the node answers the runner"
            except Exception as e:  # noqa: BLE001
                node = f"the node does not answer the runner either ({e})"
            return f"on screen: {screen!r}; {node}"

        def wait_page(what, relaunch):
            """The node's page; with `relaunch`, the app is started again (twice at most) when Android
            kills it, which is the system's doing and not a crash."""
            seen = len(system_kills())
            for _ in range(3):
                def ready():
                    if relaunch and len(system_kills()) > seen:
                        return "killed"
                    return "page" if node_page() else None
                try:
                    got = until(what, ready, timeout=90, every=3)
                except RuntimeError as e:
                    raise RuntimeError(f"{e}; {diagnose()}") from None
                if got == "page":
                    return
                kills = system_kills()
                seen = len(kills)
                log(f"Android killed the app, not a crash ({kills[-1]}); starting it again")
                adb("shell", "am", "start", "-W", "-n", f"{PKG}/.MainActivity")
            raise SystemExit(f"FAIL: Android kept killing the app: {system_kills()[-1]}")

        wait_page("the node's page in the app", relaunch=False)
        log("the node's page is showing in the app")
        check_status_bar()
        shot("2a-light")
        adb("shell", "cmd", "uimode", "night", "yes")  # the bars and the page follow to dark
        time.sleep(3)
        shot("2b-dark")
        adb("shell", "cmd", "uimode", "night", "no")
        until("the player service", lambda: "player service started" in logcat(), timeout=90)

        def registered():
            """`(True, address)` once the app registered its player with the node, `(False,
            address)` when the node could not reach it."""
            text = logcat()
            ok = re.search(r"player registered with the node address=(\S+)", text)
            if ok:
                return (True, ok.group(1))
            bad = re.search(r"player not registered[^\n]*address=(\S+)", text)
            if bad:
                return (False, bad.group(1))
            # No address to relay: the player can't be registered at all; say why
            stuck = re.search(r"[^\n]*(player not registered|no player:)[^\n]*", text)
            if stuck:
                raise SystemExit(f"FAIL: {stuck.group(0).strip()}")
            return None

        done, address = until("the player's registration", registered, timeout=60)
        if not done:
            # The emulator gave it another address or port: relay that one, and start the app
            # again so it registers once more (with a new key)
            log(f"the player registered as {address}, not {GUEST}: relaying that and starting the app again")
            relay(address)
            adb("shell", "am", "force-stop", PKG)
            fresh_logcat()
            adb("logcat", "-c")
            adb("shell", "am", "start", "-W", "-n", f"{PKG}/.MainActivity")
            wait_page("the node's page after starting again", relaunch=True)
            done, address = until("the player's registration", registered, timeout=90)
            assert done, f"the node cannot reach the player at {address}"
        log(f"app connected; player registered with the node as {address}; service started")
        shot("2-connected")

        def phone_outputs():
            return [o for o in api("GET", "/v1/outputs") if o["id"].startswith(f"net:{address}/")]

        def play_here():
            outs = until("the phone's outputs on the node", phone_outputs, timeout=30)
            log("phone outputs: " + ", ".join(o["name"] for o in outs))
            want = os.environ.get("PM_TEST_OUTPUT", "Speaker or headphones")
            phone = next((o for o in outs if want.lower() in o["name"].lower()), None)
            assert phone, f"no output named like {want!r}"
            zone = until("the phone's zone", lambda: next((z for z in api("GET", "/v1/zones") if z["output"] == phone["id"]), None))
            zid = zone["id"]
            # Three times: KEYCODE_MEDIA_NEXT has somewhere to go, however long the checks before it take
            api("POST", f"/v1/zones/{zid}/queue", {"track_ids": [track["id"]] * 3, "replace": True, "play": True})
            until("playback", lambda: position(zid)[1], timeout=30)
            z = zone_state(zid)
            np = z["now_playing"]
            log(f"playing on {z['name']}: {np['source_format']} -> {np['output_format']}")

            rate, tr = rate_over(zid, 10)
            assert tr == "playing" and 0.8 < rate < 1.2, f"foreground: {rate:.2f}x ({tr})"
            media_controls(zid, track)

            adb("shell", "input", "keyevent", "3")  # HOME: the app goes to the background
            log("app in the background")
            time.sleep(15)
            rate, tr = rate_over(zid, 15)
            services = adb("shell", "dumpsys", "activity", "services", PKG)
            (out / "services.txt").write_text(services)
            assert "PlayerService" in services and "isForeground=true" in services, "the player service is not in the foreground"
            assert tr == "playing" and 0.8 < rate < 1.2, f"background: {rate:.2f}x ({tr})"
            shot("3-background")
            api("POST", f"/v1/zones/{zid}/transport", {"action": "stop"})

        # Android may kill the app while it plays when Play services restarts on the emulator
        # (the WebView depends on its fonts provider): that is the system's doing, not the app's,
        # so the app is started again, registers again (a new key) and plays once more
        for attempt in (1, 2):
            seen = len(system_kills())
            try:
                play_here()
                break
            except (AssertionError, RuntimeError) as e:
                kills = system_kills()
                if attempt == 2 or len(kills) == seen:
                    raise
                log(f"Android killed the app while it played, not a crash ({kills[-1]}); starting it again ({e})")
                fresh_logcat()
                adb("logcat", "-c")
                adb("shell", "am", "start", "-W", "-n", f"{PKG}/.MainActivity")
                wait_page("the node's page after Android killed the app", relaunch=True)
                done, address = until("the player's registration", registered, timeout=90)
                assert done, f"the node cannot reach the player at {address}"
        log("the phone is a zone, plays in real time, and keeps playing in the background")

        # Started again with the node saved: it reconnects and the service starts again
        adb("shell", "am", "force-stop", PKG)
        fresh_logcat()
        adb("logcat", "-c")
        adb("shell", "am", "start", "-W", "-n", f"{PKG}/.MainActivity")
        wait_page("the node's page after a restart", relaunch=True)
        until("the player service after a restart", lambda: "player service started" in logcat(), timeout=90)
        shot("4-restarted")
        log("restarted: reconnected to the saved node, player service started")

        # A crash: the next launch shows the report, and Continue carries on
        crashed = subprocess.run(["adb", "shell", "am", "crash", PKG], capture_output=True, text=True, timeout=60)
        if crashed.returncode != 0 or "Unknown command" in crashed.stdout + crashed.stderr:
            log(f"crash report check skipped: `am crash` is not available here ({crashed.stderr.strip()})")
        else:
            time.sleep(3)
            # Android may kill the app again right after (Play services restarting on the emulator:
            # "depends on provider …FontsProvider in dying proc", release run for fd2d80e0, API 33).
            # The report stays until Continue, so the app is started again and shows it once more.
            # Kills are counted before the start, so one logged while `am start -W` waits counts too.
            seen = len(system_kills())
            adb("shell", "am", "start", "-W", "-n", f"{PKG}/.MainActivity")
            for attempt in (1, 2, 3):
                def report_or_killed():
                    if len(system_kills()) > seen:
                        return "killed"
                    return "report" if any("closed unexpectedly" in n for n in ui_nodes()) else None
                got = until("the crash report", report_or_killed, timeout=60, every=2)
                if got == "report":
                    break
                kills = system_kills()
                seen = len(kills)
                if attempt == 3:
                    raise SystemExit(f"FAIL: Android kept killing the app: {kills[-1]}")
                log(f"Android killed the app, not a crash ({kills[-1]}); starting it again")
                adb("shell", "am", "start", "-W", "-n", f"{PKG}/.MainActivity")
            shot("5-crash-report")
            # A release build cannot be read with run-as: read the report off the screen
            report = "\n".join(re.findall(r'text="([^"]*)"', "\n".join(ui_nodes())))
            (out / "crash-report.txt").write_text(report)
            # The newest entry's title shows in words above the details
            assert "Java exception" in report or "crashed" in report or "Rust panic" in report, f"the report says: {report[:600]!r}"
            log("the crash report shows on the next launch")
            tap(lambda n: 'text="Continue"' in n, "Continue")
            wait_page("the node's page after the report", relaunch=True)
            log("Continue reconnects to the node")
        log("PASS: the phone is a zone, plays in real time and in the background, survives a restart, and reports a crash")
    finally:
        lc.terminate()


if __name__ == "__main__":
    try:
        main()
    except Exception as e:
        log(f"FAIL: {e!r}")
        try:
            shot("fail")
        except Exception:  # noqa: BLE001
            pass
        sys.exit(1)

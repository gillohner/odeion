#!/usr/bin/env python3
"""The phone as a zone, on an Android emulator (CI: android.yml, phone-as-zone; release.yml, android-emulator).

The release APK is installed and connected by address to a node on the host
(127.0.0.1:17788 through adb reverse) over plain http, and must show the node's page.
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

Usage: android-endpoint.py <apk> <pm-node binary> <out dir>
"""
import json
import os
import re
import secrets
import selectors
import socket
import subprocess
import sys
import threading
import time
import urllib.request
import wave
from pathlib import Path
from urllib.parse import urlsplit

PKG = "to.synonym.pubkymusic"
NODE = os.environ.get("PM_TEST_NODE", "http://127.0.0.1:7788")
# An API token made per run, required even over the emulator's adb-reverse connection.
TOKEN = secrets.token_hex(16)
apk, node_bin, out = sys.argv[1], sys.argv[2], Path(sys.argv[3])


def log(msg):
    print(f"[{time.strftime('%H:%M:%S')}] {str(msg).replace(TOKEN, '[redacted]')}", flush=True)


def adb(*args, check=True):
    try:
        r = subprocess.run(["adb", *args], capture_output=True, text=True, timeout=120)
    except subprocess.TimeoutExpired:
        raise RuntimeError(f"adb {' '.join(args)} timed out".replace(TOKEN, "[redacted]")) from None
    if check and r.returncode != 0:
        raise RuntimeError(f"adb {' '.join(args)}: {r.stderr.strip() or r.stdout.strip()}".replace(TOKEN, "[redacted]"))
    return r.stdout


def guest_http_status(host, port):
    """Probe from Android without credentials; retain only the HTTP status, never the body."""
    request = f"GET /v1/info HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n\r\n"
    proc = subprocess.Popen(["adb", "shell", "toybox", "nc", "-w", "5", host, str(port)],
                            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    try:
        # Keep stdin open until the response: nc half-closes its socket on EOF, and adb
        # reverse can then tear down both directions before the node's reply arrives.
        proc.stdin.write(request.encode())
        proc.stdin.flush()
        deadline = time.monotonic() + 10
        response = b""
        with selectors.DefaultSelector() as ready:
            ready.register(proc.stdout, selectors.EVENT_READ)
            while len(response) < 4096:
                if not ready.select(max(0, deadline - time.monotonic())):
                    return "timeout"
                chunk = os.read(proc.stdout.fileno(), 4096 - len(response))
                if not chunk:
                    code = proc.wait(timeout=2)
                    return f"exit {code}" if code else "no HTTP response"
                response += chunk
                if b"\n" in response:
                    status = re.match(rb"HTTP/1\.[01] (\d{3})\b", response)
                    return status.group(1).decode() if status else "no HTTP response"
        return "invalid HTTP status"
    except BrokenPipeError:
        return "request pipe closed"
    finally:
        # This is the subprocess we just created, not the adb server or any other client.
        try:
            proc.stdin.close()
        except BrokenPipeError:
            pass
        if proc.poll() is None:
            proc.terminate()
        try:
            proc.wait(timeout=2)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()
        proc.stdout.close()


def prepare_node_transport():
    """Wire the emulator to this runner's fixture before starting the app, without a fallback.

    Run 42581763874 reached connect_manual but timed out at 10.0.2.2 while the host node
    answered. Record that alias separately; use adb reverse for a stable fixture route, as
    the update/Ring probes do. The real app still submits the token and connects over HTTP.
    """
    node = urlsplit(NODE)
    if node.scheme != "http" or node.hostname not in ("127.0.0.1", "localhost"):
        raise RuntimeError("PM_TEST_NODE must name the HTTP fixture on the runner's loopback")
    port = node.port or 80
    log(f"diagnostic host alias without a token: {guest_http_status('10.0.2.2', port)}")
    adb("reverse", "--no-rebind", "tcp:17788", f"tcp:{port}")
    status = guest_http_status("127.0.0.1", 17788)
    if status != "401":
        raise RuntimeError(f"emulator fixture preflight must refuse an absent token with 401; got {status}")
    log("emulator fixture reachable through adb reverse; an absent token is refused (401)")
    return "127.0.0.1:17788"


def run_ui_tour():
    """Every enabled tour is an acceptance gate; no report-only mode."""
    if not os.environ.get("PM_UI_TOUR"):
        return
    import importlib.util
    spec = importlib.util.spec_from_file_location("android_ui_tour", Path(__file__).with_name("android-ui-tour.py"))
    tour = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(tour)
    tour.run({"adb": adb, "api": api, "shot": shot, "ui_nodes": ui_nodes, "tap": tap,
              "center": center, "until": until, "log": log, "out": out})


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


def find(pred):
    return next((n for n in ui_nodes() if pred(n)), None)


def tap(pred, what):
    """Tap a node once the screen holds still: the same node at the same place in two dumps a second
    apart. The connect page lays itself out again while the app looks for nodes on the network
    ("Search again", "None found"), and a tap made during that lands on whatever moved under it
    (CI: the address field never opened, run 42368568855)."""
    def steady():
        a = find(pred)
        if not a:
            return None
        time.sleep(1)
        b = find(pred)
        return b if b and center(a) == center(b) else None

    node = until(what, steady, timeout=90, every=1)
    x, y = center(node)
    if (x, y) == (0, 0):  # off screen: scroll it into view and look again
        adb("shell", "input", "swipe", "500", "1500", "500", "500", "300")
        node = until(what, steady, timeout=30, every=1)
        x, y = center(node)
    adb("shell", "input", "tap", str(x), str(y))
    log(f"tapped {what} at {x},{y}")


def type_into(pred, what, text, readable=True):
    """Type into a text field once it has the focus, and (when the field shows what it holds) check
    that all of it landed. Keys sent before a WebView field takes the focus, or while the page
    redraws the form, are lost: the address field once held "10" of "10.0.2.2:7788" (run
    42536847482). This is the harness typing, not the check: the field is cleared and typed into
    again, three times at most."""
    for attempt in range(3):
        tap(pred, what)
        try:
            until(f"{what} to take the focus", lambda: (find(pred) or "").find('focused="true"') >= 0, timeout=10, every=1)
        except RuntimeError as e:
            log(f"{e}; typing all the same")
        adb("shell", "input", "keyevent", "KEYCODE_MOVE_END", *["KEYCODE_DEL"] * 40)
        adb("shell", "input", "text", text)
        if not readable:
            return
        try:
            until(f"{what} to hold what was typed", lambda: f'text="{text}"' in (find(pred) or ""), timeout=10, every=1)
            return
        except RuntimeError:
            held = re.search(r' text="([^"]*)"', find(pred) or "")
            log(f"{what} holds {held.group(1) if held else None!r}, not {text!r} (attempt {attempt + 1}): typing again")
    raise RuntimeError(f"{what} never held {text!r} after three tries")


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
    node = subprocess.Popen([node_bin, "--config", str(out / "node.toml"), "serve"],
                            stdout=open(out / "node.log", "w"), stderr=subprocess.STDOUT)
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

    sdk = int(adb("shell", "getprop", "ro.build.version.sdk").strip() or 0)
    log(f"Android API {sdk}")
    # Android 13+: nothing granted up front; the notification prompt is refused below
    log(adb("install", "-r", *([] if sdk >= 33 else ["-g"]), apk).strip())
    # Before the app starts: the address it will register reaches its player
    relay(GUEST)
    app_node = prepare_node_transport()
    adb("logcat", "-c")
    lc = subprocess.Popen(["adb", "logcat", "-v", "time", "pubky-music:V", "AndroidRuntime:E", "ActivityManager:I", "*:S"],
                          stdout=open(out / "logcat.txt", "w"), stderr=subprocess.STDOUT)
    try:
        adb("shell", "am", "start", "-W", "-n", f"{PKG}/.MainActivity")
        tap(lambda n: "connect by address" in n, "“Or connect by address”")
        # The address field is the page's first text field (its id and hint differ between WebViews)
        type_into(lambda n: 'resource-id="url"' in n or "EditText" in n, "the address field", app_node)
        # The node's token in the form's token field (a password field shows no text to check)
        type_into(lambda n: 'resource-id="tok"' in n or ("EditText" in n and 'password="true"' in n), "the token field",
                  TOKEN, readable=False)
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
                nodes = "\n".join(ui_nodes()).replace(TOKEN, "[redacted]")
                (out / "connect-failure-ui.txt").write_text(nodes)
                # Keep the error at the bottom of the form, which the old 300-character prefix
                # cut off. The full, redacted accessibility dump is retained in the artifact.
                screen = " / ".join(t for t in re.findall(r'text="([^"]+)"', nodes) if t.strip())[-1000:]
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
            api("POST", f"/v1/zones/{zid}/queue", {"track_ids": [track["id"]], "replace": True, "play": True})
            until("playback", lambda: position(zid)[1], timeout=30)
            z = zone_state(zid)
            np = z["now_playing"]
            log(f"playing on {z['name']}: {np['source_format']} -> {np['output_format']}")

            rate, tr = rate_over(zid, 10)
            assert tr == "playing" and 0.8 < rate < 1.2, f"foreground: {rate:.2f}x ({tr})"

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
        run_ui_tour()

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

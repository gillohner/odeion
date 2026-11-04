#!/usr/bin/env python3
"""The Pubky Ring sign-in transport, inside the app on an Android emulator (CI:
android-ring-transport.yml).

The APK is built with the `ring-transport-probe` feature (crates/pm-desktop/src/ring_probe.rs):
right after the app sets up its TLS verifier it routes requests the way the Ring grant exchange
does and logs one `RINGPROBE {json}` line per step. This script serves the fixture homeserver's
ICANN endpoint on the runner (HTTPS on localhost, a CA made for this run), makes it the phone's
`localhost:18443` with `adb reverse`, starts the app and checks:

- fixture: with a cached, signed PKARR packet whose direct endpoint refuses and whose ICANN
  endpoint answers, `GET /info` and `POST /auth/grant/session` go to the ICANN endpoint, and the
  grant request reaches it exactly once. A request sent to the raw key (PubkyTLS) instead is the
  reported failure ("error sending request for url (https://<key>/auth/grant/session)").
- public homeserver (the one in the report): when a lookup finds both endpoints and the direct
  one does not answer, the default client must not pick the raw key either.

Usage: android-ring-probe.py <apk> <fixture dir with ca.pem, leaf.pem, leaf.key> <out dir>
"""
import http.server
import json
import ssl
import subprocess
import sys
import threading
import time
from pathlib import Path

PKG = "to.synonym.pubkymusic"
PORT = 18443
apk, fixture_dir, out = sys.argv[1], Path(sys.argv[2]), Path(sys.argv[3])
out.mkdir(parents=True, exist_ok=True)
seen = []


def log(msg):
    print(f"[{time.strftime('%H:%M:%S')}] {msg}", flush=True)


def adb(*args, check=True, timeout=180):
    r = subprocess.run(["adb", *args], capture_output=True, text=True, timeout=timeout)
    if check and r.returncode != 0:
        raise RuntimeError(f"adb {' '.join(args)}: {r.stderr.strip() or r.stdout.strip()}")
    return r.stdout


class Fixture(http.server.BaseHTTPRequestHandler):
    """The fixture homeserver's ICANN endpoint: answers /info, refuses every grant."""

    def _record(self):
        n = int(self.headers.get("content-length") or 0)
        if n:
            self.rfile.read(n)
        seen.append({"method": self.command, "path": self.path, "host": self.headers.get("host"),
                     "pubky_host": self.headers.get("pubky-host")})

    def _send(self, code, body):
        data = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        self._record()
        if self.path == "/info":
            self._send(200, {"public_key": "fixture", "features": []})
        else:
            self._send(404, {"error": "not found"})

    def do_POST(self):
        self._record()
        # Never a session: the probe only checks where the grant goes.
        self._send(400, {"error": "fixture: grants are not accepted here"})

    def log_message(self, *a):
        pass


def serve():
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", PORT), Fixture)
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx.load_cert_chain(fixture_dir / "leaf.pem", fixture_dir / "leaf.key")
    srv.socket = ctx.wrap_socket(srv.socket, server_side=True)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv


def probe_lines(text):
    steps = {}
    for line in text.splitlines():
        i = line.find("RINGPROBE {")
        if i < 0:
            continue
        try:
            v = json.loads(line[i + len("RINGPROBE "):])
        except json.JSONDecodeError:
            continue
        steps[v.get("step")] = v
    return steps


def main():
    serve()
    api = adb("shell", "getprop", "ro.build.version.sdk").strip()
    release = adb("shell", "getprop", "ro.build.version.release").strip()
    log(f"Android {release} (API {api})")
    adb("reverse", f"tcp:{PORT}", f"tcp:{PORT}")
    adb("install", "-r", apk, timeout=600)
    # The emulator's network comes up after boot: start the app only once a name resolves.
    for _ in range(90):
        if "1 received" in adb("shell", "ping", "-c", "1", "-W", "2", "pkarr.pubky.app", check=False):
            break
        time.sleep(2)
    else:
        log("the emulator has no network after 3 min")
    adb("logcat", "-c")
    logfile = out / "logcat.txt"
    lc = subprocess.Popen(["adb", "logcat", "-v", "time", "pubky-music:V", "AndroidRuntime:E", "ActivityManager:I", "*:S"],
                          stdout=open(logfile, "w"), stderr=subprocess.STDOUT)
    states = {}

    def wait_for(step, timeout):
        deadline = time.time() + timeout
        while time.time() < deadline:
            steps = probe_lines(logfile.read_text(errors="replace"))
            if step in steps:
                return steps
            time.sleep(2)
        return probe_lines(logfile.read_text(errors="replace"))

    def process_state():
        text = adb("shell", "dumpsys", "activity", "processes", PKG, check=False)
        keep = [l.strip() for l in text.splitlines() if any(k in l.lower() for k in ("frozen", "procstate", "cached", "oom adj", "setprocstate"))]
        return keep[:12]

    try:
        launch = ["shell", "monkey", "-p", PKG, "-c", "android.intent.category.LAUNCHER", "1"]
        adb(*launch)
        steps = wait_for("phase1_done", 420)
        if "phase1_done" in steps:
            # As when the user switches to Ring to approve: the app goes to the background.
            adb("shell", "input", "keyevent", "KEYCODE_HOME")
            time.sleep(45)
            states["background_45s"] = process_state()
            time.sleep(55)
            states["background_100s"] = process_state()
            adb(*launch)
            steps = wait_for("done", 180)
    finally:
        time.sleep(1)
        lc.terminate()
    report = {"android": release, "api": api, "steps": steps, "fixture_requests": seen, "process_states": states}
    (out / "ring-probe.json").write_text(json.dumps(report, indent=1))
    print(json.dumps(report, indent=1))

    failures = []
    if "done" not in steps:
        failures.append("the probe did not finish (see logcat.txt)")
    fx = steps.get("fixture", {})
    for name in ("info", "grant"):
        r = fx.get(name) or {}
        if r.get("host") != "localhost":
            failures.append(f"fixture {name}: sent to {r.get('sent_to') or r.get('error')!r}, "
                            f"not the ICANN endpoint localhost:{PORT} (direct refused)")
    grants = [s for s in seen if s["method"] == "POST" and s["path"] == "/auth/grant/session"]
    if len(grants) != 1:
        failures.append(f"fixture: the grant reached the ICANN endpoint {len(grants)} times, not once")

    direct = steps.get("direct_tcp", {})
    default = steps.get("resolve_default", {})
    def eps(step):
        return ((steps.get(step, {}).get("lookup") or {}).get("endpoints") or {}).get("endpoints") or []

    found_icann = any(ep.get("domain") for s in ("resolve_relays", "resolve_dht", "resolve_default") for ep in eps(s))
    for name, info in (("default client", default.get("info") or {}),
                       ("default client after the background", steps.get("after_background", {}).get("info") or {})):
        if found_icann and direct.get("reachable") is False and info.get("host") != "homeserver.riginode.xyz":
            failures.append(f"public homeserver, {name}: direct endpoint unreachable and an ICANN endpoint exists, "
                            f"but it sent to {info.get('sent_to')!r} ({info.get('outcome') or info.get('error')})")
    if not found_icann:
        failures.append("public homeserver: no lookup (relays, DHT, default) found its ICANN endpoint "
                        f"(relays: {steps.get('resolve_relays', {}).get('lookup')}, "
                        f"DHT: {steps.get('resolve_dht', {}).get('lookup')}, relay TLS: {steps.get('relay_tls')})")
    for f in failures:
        log("FAIL " + f)
    if failures:
        sys.exit(1)
    log("ok: the grant exchange takes the ICANN endpoint when the direct one refuses")


main()

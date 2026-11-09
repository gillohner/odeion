#!/usr/bin/env python3
"""The Android app updating itself, on an emulator (CI: android.yml, update).

Two x86_64 builds signed with one throwaway key, both with the `update-feed-fixture` feature
(crates/pm-desktop/src/update.rs): build N (versionCode 1000) and build N+1 (1001). Their feed
is http://127.0.0.1:18480/latest.json, served here on the runner (written by
tools/release/updater-manifest.py, as a release does) and made the phone's localhost with
`adb reverse`. Nothing reaches GitHub or any account.

1. Tampered: N is installed and starts; the feed offers N+1 but the APK served is not the file
   the feed names. The app asks, Install is tapped; the app must refuse the download and Android
   must keep N.
2. Update: the right APK is served. Install is tapped; "Install unknown apps" is off, so the app
   explains and opens its settings page; the setting is turned on (appops) and the app continues
   to Android's own confirmation, which is accepted. N+1 must be installed, and when it starts
   it must find the file N wrote to the app's data (and offer nothing: it is the newest).

Usage: android-update-probe.py <apk N> <apk N+1> <out dir> [--settings-optional]
"""
import hashlib
import http.server
import json
import re
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

PKG = "to.synonym.pubkymusic"
PORT = 18480
OLD_CODE, NEW_CODE = 1000, 1001
OLD_NAME, NEW_NAME = "0.0.1-1", "0.0.1-2"
apk_old, apk_new, out = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
settings_optional = "--settings-optional" in sys.argv[4:]
out.mkdir(parents=True, exist_ok=True)
served = []
state = {"tampered": True}


def log(msg):
    print(f"[{time.strftime('%H:%M:%S')}] {msg}", flush=True)


def adb(*args, check=True, timeout=180):
    r = subprocess.run(["adb", *args], capture_output=True, text=True, timeout=timeout)
    if check and r.returncode != 0:
        raise RuntimeError(f"adb {' '.join(args)}: {r.stderr.strip() or r.stdout.strip()}")
    return r.stdout


def sh(*args, timeout=30):
    """An adb shell command whose failure is an answer, not an error."""
    try:
        r = subprocess.run(["adb", "shell", *args], capture_output=True, text=True, timeout=timeout)
        return r.stdout + r.stderr
    except subprocess.TimeoutExpired:
        return ""


def save_diagnostics(name):
    for buf in ("crash", "system"):
        try:
            r = subprocess.run(["adb", "logcat", "-d", "-v", "time", "-b", buf], capture_output=True,
                               text=True, timeout=60)
            (out / f"{name}-{buf}.txt").write_text(r.stdout[-400000:])
        except subprocess.TimeoutExpired:
            pass


def settle(steady=45, limit=600):
    """Boot complete, the package service answers, and system_server kept its pid for `steady` s
    (the API 37 image restarts it once after boot; tools/android-ring-probe.py)."""
    adb("wait-for-device", check=False, timeout=300)
    deadline = time.time() + limit
    pid, since, pids = None, time.time(), []
    while time.time() < deadline:
        booted = sh("getprop", "sys.boot_completed").strip() == "1"
        now = sh("pidof", "system_server").strip() or None
        pm_ok = "package:android" in sh("pm", "path", "android")
        if now != pid:
            if now:
                pids.append(now)
            pid, since = now, time.time()
        if booted and pm_ok and pid and time.time() - since >= steady:
            log(f"system steady: system_server pid {pid} for {steady} s (pids seen {pids})")
            return
        time.sleep(3)
    save_diagnostics("settle")
    log(f"the system did not settle in {limit} s (system_server pids {pids}); going on")


def version_code():
    m = re.search(r"versionCode=(\d+)", sh("dumpsys", "package", PKG))
    return int(m.group(1)) if m else None


def install(apk, tries=4):
    for attempt in range(tries):
        try:
            adb("install", "-r", str(apk), timeout=600)
            if version_code() is not None:
                return
            raise RuntimeError("adb install returned, but the package is not there")
        except (RuntimeError, subprocess.TimeoutExpired) as e:
            log(f"install attempt {attempt + 1} failed: {e}")
            save_diagnostics(f"install{attempt + 1}")
            if attempt == tries - 1:
                raise
            time.sleep(10)
            settle(steady=30)


def build_feed(work):
    """latest.json as the release workflow writes it, for N+1."""
    dist = work / "dist"
    dist.mkdir()
    (dist / "pubky-music-linux-x86_64.AppImage").write_bytes(b"desktop bundle")
    (dist / "pubky-music-linux-x86_64.AppImage.sig").write_text("desktop signature\n")
    (dist / "pubky-music-android-universal.apk").write_bytes(apk_new.read_bytes())
    script = Path(__file__).parent / "release" / "updater-manifest.py"
    r = subprocess.run([sys.executable, "-I", str(script), str(dist), NEW_NAME, f"http://127.0.0.1:{PORT}",
                        str(NEW_CODE)], capture_output=True, text=True, check=True)
    feed = json.loads(r.stdout)
    (out / "latest.json").write_text(r.stdout)
    want = hashlib.sha256(apk_new.read_bytes()).hexdigest()
    assert feed["android"]["apks"]["universal"]["sha256"] == want, feed
    return r.stdout.encode()


class Fixture(http.server.BaseHTTPRequestHandler):
    feed = b""

    def do_GET(self):
        served.append({"path": self.path, "tampered": state["tampered"], "at": time.strftime("%H:%M:%S")})
        if self.path == "/latest.json":
            body, kind = self.feed, "application/json"
        elif self.path == "/pubky-music-android-universal.apk":
            body = apk_new.read_bytes()
            if state["tampered"]:
                # Another file than the one the feed names: one byte changed in the middle.
                i = len(body) // 2
                body = body[:i] + bytes([body[i] ^ 0xFF]) + body[i + 1:]
            kind = "application/vnd.android.package-archive"
        else:
            self.send_response(404)
            self.end_headers()
            return
        self.send_response(200)
        self.send_header("content-type", kind)
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *a):
        pass


def ui_nodes():
    adb("shell", "uiautomator", "dump", "/sdcard/ui.xml", check=False)
    xml = adb("exec-out", "cat", "/sdcard/ui.xml", check=False)
    return re.findall(r"<node [^>]*>", xml)


def center(node):
    m = re.search(r'bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', node)
    x1, y1, x2, y2 = map(int, m.groups())
    return (x1 + x2) // 2, (y1 + y2) // 2


def find(pred, timeout, what):
    deadline = time.time() + timeout
    while time.time() < deadline:
        nodes = ui_nodes()
        # Android 13+'s notification prompt, whenever it shows, is refused.
        deny = next((n for n in nodes if re.search(r'text="(Don.t allow|Deny)"', n)), None)
        if deny:
            adb("shell", "input", "tap", *map(str, center(deny)))
            log("notifications refused")
            time.sleep(1)
            continue
        hit = next((n for n in nodes if pred(n)), None)
        if hit:
            return hit
        time.sleep(2)
    shot(f"missing-{re.sub(r'[^a-z0-9]+', '-', what.lower())}")
    raise RuntimeError(f"timed out waiting for {what}")


def tap(node, what):
    x, y = center(node)
    adb("shell", "input", "tap", str(x), str(y))
    log(f"tapped {what} at {x},{y}")


def shot(name):
    png = subprocess.run(["adb", "exec-out", "screencap", "-p"], capture_output=True, timeout=60).stdout
    (out / f"{name}.png").write_bytes(png)


def text_is(*words):
    pat = re.compile(r'text="(' + "|".join(re.escape(w) for w in words) + r')"', re.I)
    return lambda n: bool(pat.search(n))


def launch():
    adb("shell", "monkey", "-p", PKG, "-c", "android.intent.category.LAUNCHER", "1")


def main():
    work = Path(tempfile.mkdtemp())
    Fixture.feed = build_feed(work)
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", PORT), Fixture)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    api = adb("shell", "getprop", "ro.build.version.sdk").strip()
    release = adb("shell", "getprop", "ro.build.version.release").strip()
    log(f"Android {release} (API {api})")
    settle()
    install(apk_old)
    adb("reverse", f"tcp:{PORT}", f"tcp:{PORT}")
    report = {"android": release, "api": api, "steps": {}}
    failures = []
    logfile = out / "logcat.txt"
    adb("logcat", "-c")
    lc = subprocess.Popen(["adb", "logcat", "-v", "time", "pubky-music:V", "PackageInstaller:V",
                           "AndroidRuntime:E", "ActivityManager:I", "*:S"],
                          stdout=open(logfile, "w"), stderr=subprocess.STDOUT)

    def logtext():
        return logfile.read_text(errors="replace")

    def wait_log(pattern, timeout):
        deadline = time.time() + timeout
        while time.time() < deadline:
            m = re.search(pattern, logtext())
            if m:
                return m
            time.sleep(2)
        return None

    def probes():
        found = []
        for line in logtext().splitlines():
            i = line.find("UPDATEPROBE {")
            if i >= 0:
                try:
                    found.append(json.loads(line[i + len("UPDATEPROBE "):]))
                except json.JSONDecodeError:
                    pass
        return found

    question = f"Odeion {NEW_NAME} is available (this is {OLD_NAME})."
    try:
        # 1. tampered
        if version_code() != OLD_CODE:
            failures.append(f"build N installed as versionCode {version_code()}, not {OLD_CODE}")
        launch()
        node = find(lambda n: question in n, 300, "the update question")
        shot("1-question")
        report["steps"]["question"] = re.search(r'text="([^"]*)"', node).group(1)
        tap(find(text_is("Install"), 30, "Install"), "Install")
        refused = wait_log(r"updates: not installed: .*SHA-256", 240)
        shot("1-refused")
        report["steps"]["tampered_refused"] = bool(refused)
        if not refused:
            failures.append("tampered: the app did not refuse the download (no SHA-256 refusal in logcat)")
        if "handed to the installer" in logtext() or "checked; handing it to Android" in logtext():
            failures.append("tampered: the app handed the download to Android")
        if not any(s["path"].endswith(".apk") and s["tampered"] for s in served):
            failures.append("tampered: the app never downloaded the APK")
        time.sleep(10)
        report["steps"]["tampered_version_code"] = version_code()
        if version_code() != OLD_CODE:
            failures.append(f"tampered: versionCode is {version_code()} afterwards, not {OLD_CODE}")
        first = probes()
        report["steps"]["first_run_probe"] = first[:1]
        if not first or first[0].get("marker") is not None:
            failures.append(f"first run: expected no marker yet, got {first[:1]}")

        # 2. the real update
        state["tampered"] = False
        adb("shell", "am", "force-stop", PKG)
        time.sleep(2)
        launch()
        find(lambda n: question in n, 300, "the update question again")
        tap(find(text_is("Install"), 30, "Install"), "Install")
        node = find(text_is("Open settings"), 240, "the explanation for Install unknown apps")
        shot("2-allow-explained")
        tap(node, "Open settings")
        time.sleep(4)
        focus = sh("dumpsys", "window", timeout=60)
        focus = "\n".join(l.strip() for l in focus.splitlines() if "mCurrentFocus" in l or "mFocusedApp" in l)
        shot("2-settings")
        opened = "settings" in focus.lower() or "ManageAppExternalSources" in focus
        report["steps"]["settings_opened"] = {"opened": opened, "focus": focus}
        if not opened and not settings_optional:
            failures.append(f"Install unknown apps: its settings page did not open ({focus!r})")
        sh("appops", "set", PKG, "REQUEST_INSTALL_PACKAGES", "allow")
        log("Install unknown apps: allowed for the app (appops)")
        if opened:
            adb("shell", "input", "keyevent", "KEYCODE_BACK")
        confirm = find(lambda n: "packageinstaller" in n and re.search(r'text="(Update|Install|UPDATE|INSTALL)"', n),
                       240, "Android's install confirmation")
        shot("2-confirm")
        tap(confirm, "Android's Update")
        deadline = time.time() + 240
        while time.time() < deadline and version_code() != NEW_CODE:
            time.sleep(3)
        report["steps"]["updated_version_code"] = version_code()
        if version_code() != NEW_CODE:
            failures.append(f"update: versionCode is {version_code()}, not {NEW_CODE}")
        else:
            launch()
            if not wait_log(rf'UPDATEPROBE \{{"installed":{NEW_CODE}', 180):
                failures.append("update: build N+1 did not start (no UPDATEPROBE from it)")
            after = [p for p in probes() if p.get("installed") == NEW_CODE]
            report["steps"]["after_update_probe"] = after[:1]
            if not after or after[0].get("marker") != f"written by versionCode {OLD_CODE}":
                failures.append(f"update: the app's data did not survive: {after[:1]}")
            if not wait_log(r"this is the newest version", 120):
                failures.append("update: build N+1 did not find itself the newest")
            shot("3-after")
    except Exception as e:  # noqa: BLE001 (reported with the evidence)
        failures.append(f"harness: {e}")
    finally:
        time.sleep(1)
        lc.terminate()
        save_diagnostics("end")
        srv.shutdown()
    report["served"] = served
    report["failures"] = failures
    (out / "update-probe.json").write_text(json.dumps(report, indent=1))
    print(json.dumps(report, indent=1))
    for f in failures:
        log("FAIL " + f)
    if failures:
        sys.exit(1)
    log("ok: a tampered download is refused; the update installs over the app and keeps its data")


main()

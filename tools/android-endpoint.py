#!/usr/bin/env python3
"""The phone as a zone, on an Android emulator (CI: android.yml, phone-as-zone; release.yml, android-emulator).

The release APK is installed and connected by address to a node on the host
(10.0.2.2 from the emulator) over plain http, and must show the node's page.
Its built-in player (AAudio through cpal)
must become a zone that plays a track in real time, and keep playing with the
app in the background: the foreground service (PlayerService.kt) keeps the
process alive.

The in-app player ("This phone" in the page) is the app's own player (crates/pm-app-player,
src/native.rs): a song started in the page plays through the phone's output, not the WebView's
<audio>. The app logs what it plays and the path ("the app's own player: playing ..., path ...")
and where it is every 5 s; it must play in real time, in the background too, while the node's zone
on the same output stays silent; the node then playing that zone pauses the app's player (one
stream per output).

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
Screenshots of the media notification and the lock screen; the lock screen's is taken only once the
keyguard is showing (`dumpsys window policy`, the screen awake) with the track on it (the screen's
uiautomator dump; where uiautomator cannot dump the lock screen because the playing track's progress
never lets System UI go idle, System UI's lock-screen window holding the focus and the app's media
session playing that track), else the run fails. Volumes stay at 0-5 %.

Usage: android-endpoint.py <apk> <pm-node binary> <out dir>
       android-endpoint.py --self-test   (the lock-screen decision on synthetic dumps; no adb)
"""
import html
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
SELF_TEST = sys.argv[1:] == ["--self-test"]
if not SELF_TEST:
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


def session_block(text, pkg=PKG):
    """The app's own block in `dumpsys media_session` (from its `package=` line up to the next
    session's header, at most 30 lines), or "": never a line of another app's session."""
    at = text.find(f"package={pkg}")
    if at < 0:
        return ""
    lines = text[text.rfind("\n", 0, at) + 1:].splitlines()
    indent = len(lines[0]) - len(lines[0].lstrip())
    block = [lines[0]]
    for line in lines[1:30]:
        if line.strip() and len(line) - len(line.lstrip()) < indent:
            break
        block.append(line)
    return "\n".join(block)


def our_session():
    """The app's block in `dumpsys media_session` (kept in the artifact), or ""."""
    text = adb("shell", "dumpsys", "media_session")
    (out / "media_session.txt").write_text(text)
    return session_block(text)


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
    lock_screen(title)

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


# The lock screen. Its screenshot counts only with the keyguard showing (not covered by an app, not
# dreaming) on an awake screen, with the track on it: a sleep and a wake alone once gave the app's
# own page under that name (#329's first 7-lock-screen.png). The decision is in pure functions, tried
# offline by --self-test on synthetic dumps.
#
# The keyguard's flags as `dumpsys window policy` prints them across Android versions:
# KeyguardServiceDelegate's `showing=` / `occluded=` / `dreaming=` lines, its KeyguardStateMonitor's
# `mIsShowing=`, older PhoneWindowManager's `mShowingLockscreen=` / `mDreamingLockscreen=` /
# `mKeyguardOccluded=`, and `isKeyguardShowing=` / `mKeyguardShowing=`. No known flag at all, or
# flags that disagree, is not a lock screen.
KEYGUARD_SHOWING = ("showing", "mIsShowing", "mShowingLockscreen", "isKeyguardShowing", "mKeyguardShowing")
# True on any of these: something is over the keyguard (an activity shown when locked, a dream)
KEYGUARD_COVERED = ("occluded", "dreaming", "mKeyguardOccluded", "mDreamingLockscreen")
KEYGUARD_NAMED = re.compile(r"\b(mIsShowing|mShowingLockscreen|isKeyguardShowing|mKeyguardShowing|mKeyguardOccluded|mDreamingLockscreen)=(true|false)\b")
SYSTEMUI = "com.android.systemui"


def keyguard_flags(policy):
    """[(flag, bool)] for every known keyguard flag in `dumpsys window policy`. The bare `showing=`,
    `occluded=` and `dreaming=` are read only in the KeyguardServiceDelegate block (the lines
    indented under its heading), not anywhere else they may appear."""
    flags = []
    m = re.search(r"^([ \t]*)KeyguardServiceDelegate[^\n]*\n((?:\1[ \t]+[^\n]*(?:\n|$))*)", policy, re.M)
    if m:
        for name, v in re.findall(r"^[ \t]*(showing|occluded|dreaming)=(true|false)[ \t]*$", m.group(2), re.M):
            flags.append((name, v == "true"))
    flags += [(name, v == "true") for name, v in KEYGUARD_NAMED.findall(policy)]
    return flags


def keyguard_state(policy):
    """("showing" | "hidden" | "covered" | "unknown" | "conflict", why) from `dumpsys window policy`."""
    flags = keyguard_flags(policy)
    shows = [(n, v) for n, v in flags if n in KEYGUARD_SHOWING]
    covered = [n for n, v in flags if n in KEYGUARD_COVERED and v]
    said = ", ".join(f"{n}={str(v).lower()}" for n, v in flags)
    if not shows:
        return "unknown", f"no known keyguard flag in dumpsys window policy ({said or 'none at all'})"
    if all(v for _, v in shows):
        if covered:
            return "covered", f"the keyguard is covered ({said})"
        return "showing", f"keyguard showing ({said})"
    if not any(v for _, v in shows):
        return "hidden", f"no keyguard ({said})"
    return "conflict", f"the keyguard flags disagree ({said})"


def screen_awake(power):
    """(True, why) when `dumpsys power` says the screen is awake (mWakefulness=Awake)."""
    m = re.search(r"\bmWakefulness=(\w+)", power)
    if not m:
        return False, "no mWakefulness in dumpsys power"
    return m.group(1) == "Awake", f"mWakefulness={m.group(1)}"


def media_on_lock_screen(xml, title, pkg=PKG):
    """(True, why) when the screen's uiautomator dump is System UI's with the track's title on it
    (the lock screen's media player or the media notification) and none of the app's own nodes."""
    nodes = re.findall(r"<node [^>]*>", xml)
    if not nodes:
        return False, "no nodes in the screen's dump"
    if not title:
        return False, "no track title to look for on the lock screen"
    ours = [n for n in nodes if f'package="{pkg}"' in n]
    if ours:
        return False, f"the app is on screen ({len(ours)} of its nodes), not the lock screen"
    want = title.casefold()
    for n in nodes:
        if f'package="{SYSTEMUI}"' not in n:
            continue
        for attr in re.findall(r'\s(?:text|content-desc)="([^"]*)"', n):
            if want in html.unescape(attr).casefold():
                return True, f"the track's title {title!r} in System UI's node {html.unescape(attr)!r}"
    return False, f"the track's title {title!r} is not on the screen in System UI's nodes"


# `uiautomator dump` waits for the screen to go idle and gives up ("ERROR: could not get idle
# state.", AOSP DumpCommand) while System UI keeps sending accessibility events: on the lock screen
# the playing track's progress bar does so all the time, so the dump of a lock screen with music on
# it can be empty on some images (API 35's). Only then the lock screen is read from what Android
# says instead: the focused window is System UI's lock-screen window (from Android 11 the keyguard
# is drawn in NotificationShade, which takes the focus while the keyguard shows), and the app's media
# session, active and playing with the track's title, is what that window's media player shows.
DUMP_UNAVAILABLE = re.compile(r"could not get idle state|null root node", re.I)
LOCK_WINDOW = re.compile(r"Window\{[^}\n]*\sNotificationShade\}")


def lock_window(focus, pkg=PKG):
    """(True, why) when `dumpsys window`'s mCurrentFocus is System UI's lock-screen window. Every
    mCurrentFocus line must be (one per display): one on the app, or on anything else, is not."""
    said = [s.strip() for s in re.findall(r"\bmCurrentFocus=([^\n]*)", focus)]
    if not said:
        return False, "no mCurrentFocus in dumpsys window"
    for s in said:
        if pkg in s:
            return False, f"the app holds the focus ({s})"
        if not LOCK_WINDOW.fullmatch(s):
            return False, f"the focus is not System UI's lock-screen window ({s})"
    return True, f"System UI's lock-screen window has the focus ({said[0]})"


def media_session_playing(session, title):
    """(True, why) when the app's block in `dumpsys media_session` is active and playing the track."""
    if not session:
        return False, "the app has no media session"
    if not title:
        return False, "no track title to look for in the media session"
    if "active=true" not in session:
        return False, "the app's media session is not active"
    if not re.search(r"state=PlaybackState \{state=(?:3\b|PLAYING)", session):
        return False, "the app's media session is not playing"
    if title.casefold() not in session.casefold():
        return False, f"the track's title {title!r} is not in the app's media session"
    return True, f"the app's media session is active and playing {title!r}"


def lock_screen_verdict(policy, power, xml, title, pkg=PKG, dump_said="", focus="", session=""):
    """(True, why) only for an awake screen whose keyguard shows, uncovered, with the track on it:
    in the uiautomator dump, or, only when uiautomator says it could not dump the screen, with
    System UI's lock-screen window focused and the app's session playing the track."""
    awake, why_awake = screen_awake(power)
    if not awake:
        return False, f"the screen is not awake ({why_awake})"
    state, why_kg = keyguard_state(policy)
    if state != "showing":
        return False, why_kg
    media, why_media = media_on_lock_screen(xml, title, pkg)
    if media:
        return True, f"{why_awake}; {why_kg}; {why_media}"
    if re.search(r"<node ", xml) or not DUMP_UNAVAILABLE.search(dump_said):
        return False, f"{why_kg}, but {why_media}" + (f" (uiautomator: {dump_said.strip()})" if dump_said.strip() else "")
    window, why_window = lock_window(focus, pkg)
    if not window:
        return False, f"{why_kg}, uiautomator could not dump the screen ({dump_said.strip()}), and {why_window}"
    playing, why_playing = media_session_playing(session, title)
    if not playing:
        return False, f"{why_kg}, uiautomator could not dump the screen ({dump_said.strip()}), and {why_playing}"
    return True, f"{why_awake}; {why_kg}; uiautomator could not dump the screen ({dump_said.strip()}); {why_window}; {why_playing}"


def screen_dumps():
    """`dumpsys window policy`, `dumpsys power`, the screen's uiautomator dump and what uiautomator
    said, the focused window and the app's media session, kept in the artifact (keyguard-policy.txt,
    keyguard-power.txt, ui-lock-screen.xml.txt, ui-lock-screen-dump.txt, window-focus.txt,
    media_session.txt). A dump from an earlier try is removed first, never read again."""
    policy = adb("shell", "dumpsys", "window", "policy")
    power = adb("shell", "dumpsys", "power")
    (out / "keyguard-policy.txt").write_text(policy)
    (out / "keyguard-power.txt").write_text(power)
    xml, dump_said, focus, session = "", "", "", ""
    if screen_awake(power)[0]:
        adb("shell", "rm", "-f", "/sdcard/lock.xml", check=False)
        r = subprocess.run(["adb", "shell", "uiautomator", "dump", "/sdcard/lock.xml"], capture_output=True, text=True, timeout=120)
        dump_said = f"{r.stdout}{r.stderr}".strip() or f"exit {r.returncode}, nothing said"
        xml = adb("exec-out", "cat", "/sdcard/lock.xml", check=False)
        (out / "ui-lock-screen.xml.txt").write_text(xml)
        (out / "ui-lock-screen-dump.txt").write_text(dump_said + "\n")
        focus = "\n".join(l.strip() for l in adb("shell", "dumpsys", "window").splitlines() if "mCurrentFocus=" in l or "mFocusedApp=" in l)
        (out / "window-focus.txt").write_text(focus + "\n")
        session = our_session()
    return policy, power, xml, dump_said, focus, session


def lock_screen_now(title):
    """The lock-screen verdict on the screen as it is now (see lock_screen_verdict)."""
    policy, power, xml, dump_said, focus, session = screen_dumps()
    return lock_screen_verdict(policy, power, xml, title, dump_said=dump_said, focus=focus, session=session)


def lock_screen(title):
    """The lock screen with the track on it, screenshot 7-lock-screen, then unlocked again. The
    emulator's keyguard is enabled for it when `locksettings` says it is disabled (an insecure
    keyguard, no credential), and disabled again after. Fails when no such lock screen shows."""
    said = adb("shell", "locksettings", "get-disabled", check=False).strip()
    enabled_here = said == "true"
    if enabled_here:
        adb("shell", "locksettings", "set-disabled", "false")
        log("the emulator's keyguard was disabled (locksettings get-disabled: true): enabled for the lock screen")
    elif said != "false":
        log(f"locksettings get-disabled said {said!r}: the keyguard left as it is")
    done = False
    try:
        last = "not looked at"
        for attempt in (1, 2):
            adb("shell", "input", "keyevent", "KEYCODE_SLEEP", check=False)
            try:
                until("the screen off", lambda: not screen_awake(adb("shell", "dumpsys", "power"))[0], timeout=15)
            except RuntimeError as e:
                log(f"{e} (try {attempt})")
            time.sleep(1)
            adb("shell", "input", "keyevent", "KEYCODE_WAKEUP", check=False)

            def ready():
                nonlocal last
                ok, last = lock_screen_now(title)
                return ok

            try:
                until("the lock screen with the track on it", ready, timeout=30, every=2)
                break
            except RuntimeError:
                log(f"no lock screen with the track (try {attempt}): {last}")
        else:
            raise AssertionError(f"no lock screen with the track on it after sleep and wake (locksettings get-disabled: "
                                 f"{said!r}): {last}; see keyguard-policy.txt, keyguard-power.txt, ui-lock-screen.xml.txt, "
                                 f"ui-lock-screen-dump.txt, window-focus.txt, media_session.txt")
        shot("7-lock-screen")
        ok, why = lock_screen_now(title)
        assert ok, f"the lock screen went while its screenshot was taken: {why}"
        log(f"lock screen: {why}")
        done = True
    finally:
        try:
            unlock()
        except Exception as e:  # noqa: BLE001 (the lock screen's own failure is the one reported)
            if done:
                raise
            log(f"unlocking after the failure: {e}")
        finally:
            if enabled_here:
                adb("shell", "locksettings", "set-disabled", "true", check=False)
                log("the emulator's keyguard disabled again (as it was)")


def unlock():
    """The keyguard dismissed (no credential is set) and the app in front again, as before the lock."""
    steps = [("wm", "dismiss-keyguard"), ("input", "keyevent", "82"), ("input", "swipe", "500", "1700", "500", "400", "300")]
    n = [0]

    def gone():
        state, _ = keyguard_state(adb("shell", "dumpsys", "window", "policy"))
        if state == "hidden":
            return True
        adb("shell", "input", "keyevent", "KEYCODE_WAKEUP", check=False)
        adb("shell", *steps[n[0] % len(steps)], check=False)
        n[0] += 1
        return False

    try:
        until("the keyguard dismissed", gone, timeout=30, every=2)
    except RuntimeError:
        raise AssertionError(f"the keyguard does not go away: {keyguard_state(adb('shell', 'dumpsys', 'window', 'policy'))[1]}") from None
    adb("shell", "am", "start", "-W", "-n", f"{PKG}/.MainActivity")
    log("unlocked: the keyguard dismissed, the app in front again")


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


# The app's own player in logcat (src/native.rs): a transition, and where it is every 5 s
NATIVE = re.compile(r"the app's own player: (\w+) (\S*)(.*)")
NATIVE_AT = re.compile(r"^(\d\d-\d\d \d\d:\d\d:\d\d\.\d+).*the app's own player at position_ms=(\d+)", re.M)


def native_pace(after=0):
    """Real-time rate of the app's own player from its logged positions (logcat's own clock), and
    how many were logged; only lines past offset `after` of the log."""
    pts = []
    for m in NATIVE_AT.finditer(logcat()[after:]):
        t = time.mktime(time.strptime("2000-" + m.group(1).split(".")[0], "%Y-%m-%d %H:%M:%S")) + float("0." + m.group(1).split(".")[1])
        pts.append((t, int(m.group(2))))
    if len(pts) < 3 or pts[-1][0] - pts[0][0] < 9:
        return None, len(pts)
    return (pts[-1][1] - pts[0][1]) / 1000 / (pts[-1][0] - pts[0][0]), len(pts)


def in_app_player(address, track):
    """The page's "This phone", played by the app itself (see the module's doc)."""
    adb("shell", "am", "start", "-W", "-n", f"{PKG}/.MainActivity")
    time.sleep(3)
    start = len(logcat())
    try:
        tap(lambda n: "Shuffle my music" in n, "“Shuffle my music”")
    except RuntimeError:
        tap(lambda n: 'text="Library"' in n or 'content-desc="Library"' in n, "Library")
        tap(lambda n: 'text="Songs"' in n or 'content-desc="Songs"' in n, "Songs")
        tap(lambda n: re.search(r'(text|content-desc)="[Tt]one', n), "the song")
    def started():
        for m in NATIVE.finditer(logcat()[start:]):
            if m.group(1) == "playing":
                return m
        return None
    m = until("the app's own player playing the song", started, timeout=60)
    log(f"the app's own player: {m.group(0).strip()}")
    assert m.group(2).startswith("/v1/library/tracks/"), f"it plays the node's file: {m.group(2)}"
    assert "path " in m.group(3), f"it says the path: {m.group(3)}"
    assert "token" not in m.group(0), "the log line carries no token"
    def native_session(playing):
        s = our_session()
        state = r"(?:3\b|PLAYING)" if playing else r"(?:2\b|PAUSED)"
        if "active=true" in s and re.search(r"state=PlaybackState \{state=" + state, s):
            return s
        return None
    def native_session_moving():
        s = native_session(True)
        return s if s and re.search(r"position=[1-9][0-9]*", s) else None
    session = until("native playback in the single framework MediaSession", native_session_moving, timeout=30)
    assert re.search(r"position=[1-9][0-9]*", session), f"native session has no position: {session}"
    (out / "native-media-session-playing.txt").write_text(session)
    shot("6-in-app-player")
    rate, n = until("the app's own player's pace", lambda: native_pace(start)[0] and native_pace(start), timeout=60, every=5)
    log(f"the app's own player: {rate:.2f}x real time over {n} logged positions")
    assert 0.8 < rate < 1.2, f"in the app: {rate:.2f}x"
    # Not the WebView: the page's <audio> has no source while the app plays
    phone = next((z for z in api("GET", "/v1/zones") if z["output"].startswith(f"net:{address}/")), None)
    assert phone is None or phone.get("transport") != "playing", "the node's zone on the phone plays too"
    # In the background, the screen off
    adb("shell", "input", "keyevent", "3")
    adb("shell", "input", "keyevent", "KEYCODE_SLEEP", check=False)
    away = len(logcat())
    time.sleep(25)
    rate, n = native_pace(away)
    adb("shell", "input", "keyevent", "KEYCODE_WAKEUP", check=False)
    adb("shell", "wm", "dismiss-keyguard", check=False)
    assert rate is not None and 0.8 < rate < 1.2, f"in the background, screen off: {rate} over {n} positions"
    log(f"the app's own player in the background, screen off: {rate:.2f}x real time")
    # These buttons must reach the engine with the WebView in the background, and the same
    # session must follow the real engine state. No second player or node-zone transport.
    adb("shell", "input", "keyevent", "KEYCODE_MEDIA_PAUSE")
    session = until("background native session paused", lambda: native_session(False), timeout=15)
    (out / "native-media-session-paused.txt").write_text(session)
    adb("shell", "input", "keyevent", "KEYCODE_MEDIA_PLAY")
    session = until("background native session resumed", lambda: native_session(True), timeout=15)
    (out / "native-media-session-resumed.txt").write_text(session)
    log("native session pause/play works in background; original pace/handover assertions retained")
    # One stream per output: the node plays its zone on the phone, the app's player pauses
    if phone:
        mark = len(logcat())
        api("POST", f"/v1/zones/{phone['id']}/queue", {"track_ids": [track["id"]], "replace": True, "play": True})
        until("the node's zone playing on the phone", lambda: position(phone["id"])[1], timeout=30)
        def paused():
            return next((m for m in NATIVE.finditer(logcat()[mark:]) if m.group(1) == "paused"), None)
        m = until("the app's own player paused for the node's zone", paused, timeout=30)
        log(f"handed over: {m.group(0).strip()}")
        rate, tr = rate_over(phone["id"], 8)
        assert tr == "playing" and 0.8 < rate < 1.2, f"the node's zone after the handover: {rate:.2f}x ({tr})"
        api("POST", f"/v1/zones/{phone['id']}/transport", {"action": "stop"})
    adb("shell", "am", "start", "-W", "-n", f"{PKG}/.MainActivity")


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
        type_into(lambda n: 'resource-id="url"' in n or "EditText" in n, "the address field", "10.0.2.2:7788")
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
        in_app_player(address, track)
        log("the in-app player is the app's own: real time, in the background, and it gives the output to the node's zone")

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
        log("PASS: the phone is a zone, plays in real time and in the background, the in-app player is the app's own, survives a restart, and reports a crash")
    finally:
        lc.terminate()


# --self-test: the lock-screen decision on SYNTHETIC samples, written here by hand in the formats
# Android prints (not captured from a device or CI run). No adb, no emulator, no arguments.
def _policy(showing, occluded="false", dreaming="false", monitor=None, indent="    "):
    """A made-up `dumpsys window policy` tail with a KeyguardServiceDelegate block."""
    i2 = indent + "  "
    text = (f"WINDOW MANAGER POLICY STATE (dumpsys window policy)\n{indent}mSafeMode=false mSystemReady=true\n"
            f"{indent}KeyguardServiceDelegate\n{i2}showing={showing}\n{i2}inputRestricted={showing}\n"
            f"{i2}occluded={occluded}\n{i2}secure=false\n{i2}dreaming={dreaming}\n{i2}systemIsReady=true\n"
            f"{i2}deviceHasKeyguard=true\n{i2}enabled=true\n{i2}screenState=SCREEN_STATE_ON\n"
            f"{i2}interactiveState=INTERACTIVE_STATE_AWAKE\n")
    if monitor is not None:
        text += f"{i2}KeyguardStateMonitor\n{i2}  mIsShowing={monitor}\n{i2}  mSimSecure=false\n{i2}  mTrusted=false\n"
    return text + f"{indent}mHomePressed=false\n"


def _ui(*nodes):
    """A made-up uiautomator dump: (package, resource-id, text, content-desc) per node."""
    body = "".join(f'<node index="{k}" text="{t}" resource-id="{r}" class="android.widget.TextView" package="{p}" '
                   f'content-desc="{d}" bounds="[0,{k * 100}][1080,{k * 100 + 90}]" />' for k, (p, r, t, d) in enumerate(nodes))
    return f"<?xml version='1.0' encoding='UTF-8' standalone='yes' ?><hierarchy rotation=\"0\">{body}</hierarchy>"


SAMPLE_AWAKE = "POWER MANAGER (dumpsys power)\n  mWakefulness=Awake\n  mWakefulnessChanging=false\n"
SAMPLE_ASLEEP = "POWER MANAGER (dumpsys power)\n  mWakefulness=Asleep\n  mWakefulnessChanging=false\n"
SAMPLE_APP_HOME = _ui((PKG, "", "Home", ""), (PKG, "", "Nothing playing", ""), (PKG, "", "", "Library"))
SAMPLE_SHADE = _ui((SYSTEMUI, f"{SYSTEMUI}:id/clock", "12:00", ""), (SYSTEMUI, f"{SYSTEMUI}:id/header_title", "Tone", ""),
                   (SYSTEMUI, f"{SYSTEMUI}:id/header_artist", "This phone", ""), (SYSTEMUI, "", "", "Pause"),
                   (SYSTEMUI, f"{SYSTEMUI}:id/clear_all", "Clear all", ""))
SAMPLE_KEYGUARD_BARE = _ui((SYSTEMUI, f"{SYSTEMUI}:id/keyguard_status_view", "", ""), (SYSTEMUI, f"{SYSTEMUI}:id/lockscreen_clock_view", "12:00", ""),
                           (SYSTEMUI, f"{SYSTEMUI}:id/keyguard_indication_text", "Swipe up to open", ""))
SAMPLE_KEYGUARD_MEDIA = _ui((SYSTEMUI, f"{SYSTEMUI}:id/keyguard_status_view", "", ""), (SYSTEMUI, f"{SYSTEMUI}:id/lockscreen_clock_view", "12:00", ""),
                            (SYSTEMUI, f"{SYSTEMUI}:id/header_title", "Tone", ""), (SYSTEMUI, f"{SYSTEMUI}:id/header_artist", "This phone", ""),
                            (SYSTEMUI, "", "", "Pause"), (SYSTEMUI, f"{SYSTEMUI}:id/keyguard_indication_text", "Swipe up to open", ""))


def self_test():
    failed = []

    def check(cond, what):
        print(("ok   " if cond else "FAIL ") + what)
        if not cond:
            failed.append(what)

    def verdict(policy, power, xml, title="Tone"):
        return lock_screen_verdict(policy, power, xml, title)[0]

    unlocked = _policy("false", monitor="false")
    locked = _policy("true", monitor="true")
    # The four the lock screen's screenshot turns on
    check(not verdict(unlocked, SAMPLE_AWAKE, SAMPLE_APP_HOME), "REJECT: the app's Home page, unlocked (#329's first 7-lock-screen.png)")
    check(not verdict(unlocked, SAMPLE_AWAKE, SAMPLE_SHADE), "REJECT: the notification shade with the track, no keyguard")
    check(not verdict(locked, SAMPLE_AWAKE, SAMPLE_KEYGUARD_BARE), "REJECT: the keyguard without the track")
    check(verdict(locked, SAMPLE_AWAKE, SAMPLE_KEYGUARD_MEDIA), "ACCEPT: the keyguard with the track on it")
    # Fail closed
    check(not verdict(_policy("true", occluded="true", monitor="true"), SAMPLE_AWAKE, SAMPLE_APP_HOME),
          "REJECT: the keyguard covered by the app (occluded=true)")
    check(not verdict(_policy("true", occluded="true", monitor="true"), SAMPLE_AWAKE, SAMPLE_KEYGUARD_MEDIA),
          "REJECT: occluded=true, whatever the dump shows")
    check(not verdict(_policy("true", dreaming="true", monitor="true"), SAMPLE_AWAKE, SAMPLE_KEYGUARD_MEDIA), "REJECT: dreaming over the keyguard")
    check(not verdict(locked, SAMPLE_ASLEEP, SAMPLE_KEYGUARD_MEDIA), "REJECT: the screen asleep")
    check(not verdict(locked, "POWER MANAGER\n  mScreenOn=?\n", SAMPLE_KEYGUARD_MEDIA), "REJECT: dumpsys power without mWakefulness")
    check(not verdict("WINDOW MANAGER POLICY STATE\n    mSafeMode=false\n", SAMPLE_AWAKE, SAMPLE_KEYGUARD_MEDIA),
          "REJECT: no known keyguard flag (an unknown format)")
    check(not verdict(_policy("true", monitor="false"), SAMPLE_AWAKE, SAMPLE_KEYGUARD_MEDIA), "REJECT: keyguard flags that disagree")
    check(not verdict(locked, SAMPLE_AWAKE, SAMPLE_KEYGUARD_MEDIA, title=""), "REJECT: no track title to look for")
    check(not verdict(locked, SAMPLE_AWAKE, SAMPLE_KEYGUARD_MEDIA, title="Another track"), "REJECT: another title on the lock screen")
    check(not verdict(locked, SAMPLE_AWAKE, ""), "REJECT: an empty screen dump")
    check(not verdict(locked, SAMPLE_AWAKE, _ui((PKG, "", "Tone", ""))), "REJECT: the title only in the app's own nodes")
    check(not verdict("    showing=true\n    occluded=false\n", SAMPLE_AWAKE, SAMPLE_KEYGUARD_MEDIA),
          "REJECT: a bare showing=true outside the KeyguardServiceDelegate block")
    # The other spellings
    check(verdict(_policy("true"), SAMPLE_AWAKE, SAMPLE_KEYGUARD_MEDIA), "ACCEPT: KeyguardServiceDelegate showing=true alone")
    check(verdict("  mShowingLockscreen=true mShowingDream=false mDreamingLockscreen=false\n", SAMPLE_AWAKE, SAMPLE_KEYGUARD_MEDIA),
          "ACCEPT: the older mShowingLockscreen=true")
    check(not verdict("  mShowingLockscreen=true mShowingDream=true mDreamingLockscreen=true\n", SAMPLE_AWAKE, SAMPLE_KEYGUARD_MEDIA),
          "REJECT: the older mDreamingLockscreen=true")
    check(verdict("  isKeyguardShowing=true\n", SAMPLE_AWAKE, SAMPLE_KEYGUARD_MEDIA), "ACCEPT: isKeyguardShowing=true")
    check(not verdict("  mKeyguardShowing=false\n", SAMPLE_AWAKE, SAMPLE_KEYGUARD_MEDIA), "REJECT: mKeyguardShowing=false")
    check(verdict(locked, SAMPLE_AWAKE, SAMPLE_KEYGUARD_MEDIA, title="tone"), "ACCEPT: the title in another case")
    check(verdict(locked, SAMPLE_AWAKE, _ui((SYSTEMUI, "", "R&amp;B", ""), (SYSTEMUI, "", "Swipe up to open", "")), title="R&B"),
          "ACCEPT: an XML-escaped title")
    # What unlock() waits for
    check(keyguard_state(unlocked)[0] == "hidden", "unlock: no keyguard is seen as hidden")
    check(keyguard_state(locked)[0] == "showing", "unlock: the keyguard is seen as showing")
    check(keyguard_state("")[0] == "unknown", "unlock: nothing known is not hidden")
    # uiautomator unable to dump the lock screen (the playing track's progress never lets it idle):
    # read from the focused window and the app's media session, only then
    idle = "ERROR: could not get idle state."
    shade = f"mCurrentFocus=Window{{4f1c2a7 u0 NotificationShade}}\nmFocusedApp=ActivityRecord{{91d3 u0 {PKG}/.MainActivity t9}}"
    app_focus = f"mCurrentFocus=Window{{8e2b11 u0 {PKG}/{PKG}.MainActivity}}\nmFocusedApp=ActivityRecord{{91d3 u0 {PKG}/.MainActivity t9}}"
    playing = (f"    {PKG} {PKG}/MediaControl (userId=0)\n      package={PKG}\n      active=true\n"
               "      state=PlaybackState {state=PLAYING(3), position=10962, speed=1.0, actions=823}\n"
               "      metadata: size=5, description=Tone, This phone, null\n")
    paused = playing.replace("state=PLAYING(3)", "state=PAUSED(2)")

    def fallback(policy=locked, power=SAMPLE_AWAKE, xml="", said=idle, focus=shade, session=playing, title="Tone"):
        return lock_screen_verdict(policy, power, xml, title, dump_said=said, focus=focus, session=session)[0]

    check(fallback(), "ACCEPT: no dump (could not get idle state), System UI's lock-screen window focused, the session playing the track")
    check(fallback(said="ERROR: null root node returned by UiTestAutomationBridge."), "ACCEPT: no dump (null root node), the same")
    check(not fallback(said=""), "REJECT: an empty dump uiautomator gave no reason for")
    check(not fallback(said="UI hierchary dumped to: /sdcard/lock.xml"), "REJECT: an empty dump uiautomator says it wrote")
    check(not fallback(focus=app_focus), "REJECT: no dump, the app holding the focus")
    check(not fallback(focus="mCurrentFocus=null\nmFocusedApp=null"), "REJECT: no dump, nothing focused")
    check(not fallback(focus=""), "REJECT: no dump, no mCurrentFocus at all")
    check(not fallback(session=paused), "REJECT: no dump, the session paused")
    check(not fallback(session=playing.replace("active=true", "active=false")), "REJECT: no dump, the session inactive")
    check(not fallback(session=""), "REJECT: no dump, no session of the app")
    check(not fallback(title="Another track"), "REJECT: no dump, the session playing another track")
    check(not fallback(title=""), "REJECT: no dump, no track title to look for")
    check(not fallback(policy=unlocked), "REJECT: no dump, the shade over an unlocked screen (no keyguard)")
    check(not fallback(policy=_policy("true", occluded="true", monitor="true")), "REJECT: no dump, the keyguard covered")
    check(not fallback(power=SAMPLE_ASLEEP), "REJECT: no dump, the screen asleep")
    check(not fallback(xml=SAMPLE_APP_HOME), "REJECT: a dump of the app's Home, whatever uiautomator also said")
    check(not fallback(xml=SAMPLE_KEYGUARD_BARE), "REJECT: a dump of the keyguard without the track, whatever the session says")
    check(not fallback(focus=shade + "\n" + app_focus), "REJECT: no dump, the app focused on another display line")
    check(not fallback(focus="mCurrentFocus=Window{77aa u0 StatusBar}"), "REJECT: no dump, the status bar focused (not the lock screen)")
    # The app's own block of `dumpsys media_session`, never another app's session after it
    other = ("    com.other.player com.other.player/Session (userId=0)\n      package=com.other.player\n      active=true\n"
             "      state=PlaybackState {state=PLAYING(3), position=1, speed=1.0}\n      metadata: size=3, description=Tone, x, y\n")
    dump = "  Sessions Stack - have 2 sessions:\n" + paused + other
    ours = session_block(dump)
    check(ours.lstrip().startswith(f"package={PKG}") and "com.other.player" not in ours and "PAUSED(2)" in ours,
          "session block: the app's own lines only, up to the next session")
    check(not media_session_playing(ours, "Tone")[0], "REJECT: the app paused while another app plays the same title")
    check(session_block(other) == "", "session block: none for an app without a session")
    if failed:
        print(f"self-test FAILED: {len(failed)} check(s)")
        return 1
    print("self-test ok: the lock screen's screenshot is taken only on an awake, uncovered keyguard with the track on it")
    return 0


if __name__ == "__main__":
    if SELF_TEST:
        sys.exit(self_test())
    try:
        main()
    except Exception as e:
        log(f"FAIL: {e!r}")
        try:
            shot("fail")
        except Exception:  # noqa: BLE001
            pass
        sys.exit(1)

#!/usr/bin/env python3
"""Checks tools/release/updater-manifest.py on fixture release files: the desktop entries are
what they were before the Android entry existed, the Android entry names each signed APK with its
SHA-256, and unsigned or missing APKs give no Android entry. CI: the `desktop` job in ci.yml.

Usage: updater-manifest-check.py
"""
import hashlib
import json
import subprocess
import sys
import tempfile
from pathlib import Path

SCRIPT = Path(__file__).with_name("updater-manifest.py")
BASE = "https://github.com/gillohner/pubky-music-node/releases/download/edge"
failures = []


def run(dist, *extra):
    r = subprocess.run([sys.executable, "-I", str(SCRIPT), str(dist), "0.3.0-12", BASE + "/", *extra],
                       capture_output=True, text=True)
    return r.returncode, r.stdout, r.stderr


def expect(cond, what):
    if not cond:
        failures.append(what)


with tempfile.TemporaryDirectory() as d:
    dist = Path(d)
    for name in ("pubky-music-linux-x86_64.AppImage", "pubky-music-windows-x86_64-setup.exe",
                 "pubky-music-macos-universal.app.tar.gz"):
        (dist / name).write_bytes(b"bundle " + name.encode())
        (dist / f"{name}.sig").write_text(f"sig-of-{name}\n")

    # Before any APK: exactly the desktop feed (no android key), also with a versionCode.
    code, before, err = run(dist)
    expect(code == 0, f"desktop-only feed failed: {err}")
    before = json.loads(before)
    expect("android" not in before, "a feed without APKs has an android entry")
    expect(sorted(before) == ["notes", "platforms", "pub_date", "version"], f"top-level keys: {sorted(before)}")
    expect(before["platforms"]["linux-x86_64"] == {"signature": "sig-of-pubky-music-linux-x86_64.AppImage",
                                                   "url": f"{BASE}/pubky-music-linux-x86_64.AppImage"},
           f"linux entry: {before['platforms'].get('linux-x86_64')}")
    code, out, _ = run(dist, "3000012")
    expect(code == 0 and "android" not in json.loads(out), "no APKs but an android entry")

    # Unsigned APKs (no key in the build): no android entry.
    (dist / "pubky-music-android-universal-unsigned.apk").write_bytes(b"u")
    (dist / "pubky-music-android-arm64-unsigned.apk").write_bytes(b"a")
    code, out, _ = run(dist, "3000012")
    expect(code == 0 and "android" not in json.loads(out), "unsigned APKs got an android entry")

    # Signed APKs: the android entry, and the desktop part byte for byte as before.
    arm, uni = b"arm64 apk bytes", b"universal apk bytes"
    (dist / "pubky-music-android-arm64.apk").write_bytes(arm)
    (dist / "pubky-music-android-universal.apk").write_bytes(uni)
    code, out, err = run(dist, "3000012")
    expect(code == 0, f"feed with APKs failed: {err}")
    after = json.loads(out)
    expect(json.dumps(after["platforms"], indent=2) == json.dumps(before["platforms"], indent=2),
           "the desktop entries changed")
    for k in ("version", "notes"):
        expect(after[k] == before[k], f"{k} changed")
    expect(list(after)[:4] == ["version", "notes", "pub_date", "platforms"], f"key order: {list(after)}")
    expect(after.get("android") == {
        "version": "0.3.0-12",
        "versionCode": 3000012,
        "apks": {
            "arm64-v8a": {"url": f"{BASE}/pubky-music-android-arm64.apk", "sha256": hashlib.sha256(arm).hexdigest()},
            "universal": {"url": f"{BASE}/pubky-music-android-universal.apk", "sha256": hashlib.sha256(uni).hexdigest()},
        },
    }, f"android entry: {after.get('android')}")

    # Without a versionCode (an older workflow): no android entry.
    code, out, _ = run(dist)
    expect(code == 0 and "android" not in json.loads(out), "android entry without a versionCode")
    # A versionCode that is not a number is an error, not a feed that never updates.
    code, _, _ = run(dist, "edge")
    expect(code != 0, "a non-numeric versionCode was accepted")

    # Only the arm64 APK: no android entry (other phones would get nothing).
    (dist / "pubky-music-android-universal.apk").unlink()
    code, out, _ = run(dist, "3000012")
    expect(code == 0 and "android" not in json.loads(out), "an android entry without the universal APK")

for f in failures:
    print("FAIL " + f)
if failures:
    sys.exit(1)
print("updater-manifest: ok")

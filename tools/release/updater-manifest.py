#!/usr/bin/env python3
"""The app's update feed (latest.json) from the release files in dist/:

- `platforms` (Tauri updater format, read by the desktop apps): one entry per platform with the
  signed bundle's URL and the contents of its .sig (tools/release/collect-app.sh);
- `android` (read by the Android app, crates/pm-desktop/src/update_apk.rs), when a versionCode is
  given and the signed APKs are there: the version, the versionCode and per APK its URL and
  SHA-256. Desktops ignore it; their entries stay exactly as they were.

Usage: updater-manifest.py <dist> <version> <download base URL> [<Android versionCode>]  > latest.json
Checked by tools/release/updater-manifest-check.py.
"""
import datetime
import hashlib
import json
import sys
from pathlib import Path

dist, version, base = Path(sys.argv[1]), sys.argv[2], sys.argv[3].rstrip("/")
version_code = sys.argv[4] if len(sys.argv) > 4 else ""
# Tauri's platform keys -> the bundle the updater installs there.
BUNDLES = {
    "darwin-aarch64": "pubky-music-macos-universal.app.tar.gz",
    "darwin-x86_64": "pubky-music-macos-universal.app.tar.gz",
    "linux-x86_64": "pubky-music-linux-x86_64.AppImage",
    "linux-aarch64": "pubky-music-linux-arm64.AppImage",
    "windows-x86_64": "pubky-music-windows-x86_64-setup.exe",
}
# The app's ABI names -> the APK for them. Signed APKs only: tools/release/android-sign.sh names
# unsigned ones -unsigned, and Android would not install those over the app anyway.
APKS = {
    "arm64-v8a": "pubky-music-android-arm64.apk",
    "universal": "pubky-music-android-universal.apk",
}
platforms = {}
for key, name in BUNDLES.items():
    sig = dist / f"{name}.sig"
    if not (dist / name).exists() or not sig.exists():
        print(f"updater-manifest: no {name} with its .sig; {key} gets no updates", file=sys.stderr)
        continue
    platforms[key] = {"signature": sig.read_text().strip(), "url": f"{base}/{name}"}
if not platforms:
    sys.exit("updater-manifest: no signed bundles in " + str(dist))


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


android = None
if version_code:
    if not version_code.isdigit() or int(version_code) <= 0:
        sys.exit(f"updater-manifest: the Android versionCode {version_code!r} is not a positive number")
    apks = {abi: {"url": f"{base}/{name}", "sha256": sha256(dist / name)}
            for abi, name in APKS.items() if (dist / name).exists()}
    # Every phone can take the universal APK; without it some would never be offered an update.
    if "universal" in apks:
        android = {"version": version, "versionCode": int(version_code), "apks": apks}
    else:
        print("updater-manifest: no signed universal APK; Android gets no updates", file=sys.stderr)
feed = {
    "version": version,
    "notes": "See the release notes on GitHub.",
    "pub_date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "platforms": platforms,
}
if android:
    feed["android"] = android
json.dump(feed, sys.stdout, indent=2)
print()

#!/usr/bin/env python3
"""The .aab (for Google Play) has no updater: neither REQUEST_INSTALL_PACKAGES in its manifest nor
the AppUpdate class (tools/release/android-patch.py --without-updater). The APK given for
comparison must have the class, so the check can see it at all. Used by release.yml and
android-update.yml.

Usage: aab-check.py <gradle outputs dir> <an APK built with the updater>
"""
import sys
import zipfile
from pathlib import Path

outputs, apk = Path(sys.argv[1]), Path(sys.argv[2])
CLASS = b"pubkymusic/AppUpdate;"


def die(msg):
    print(f"::error::aab-check: {msg}")
    sys.exit(1)


aabs = sorted(outputs.glob("bundle/**/*release*.aab"))
if len(aabs) != 1:
    die(f"expected one release .aab under {outputs}/bundle, found {[str(a) for a in aabs]}")
with zipfile.ZipFile(aabs[0]) as z:
    manifest = z.read("base/manifest/AndroidManifest.xml")
    dex = b"".join(z.read(n) for n in z.namelist() if n.startswith("base/dex/") and n.endswith(".dex"))
if b"android.permission.INTERNET" not in manifest:
    die("cannot read the .aab's manifest (no INTERNET permission found)")
if b"REQUEST_INSTALL_PACKAGES" in manifest:
    die("the .aab asks for REQUEST_INSTALL_PACKAGES")
if not dex:
    die("the .aab has no dex files")
if CLASS in dex:
    die("the .aab has the AppUpdate class")
with zipfile.ZipFile(apk) as z:
    apk_dex = b"".join(z.read(n) for n in z.namelist() if n.endswith(".dex"))
    apk_manifest = z.read("AndroidManifest.xml")
if CLASS not in apk_dex:
    die(f"{apk} has no AppUpdate class either: this check cannot see it")
if "REQUEST_INSTALL_PACKAGES".encode("utf-16-le") not in apk_manifest and b"REQUEST_INSTALL_PACKAGES" not in apk_manifest:
    die(f"{apk} does not ask for REQUEST_INSTALL_PACKAGES either: this check cannot see it")
print(f"aab-check: {aabs[0].name} has neither REQUEST_INSTALL_PACKAGES nor AppUpdate ({apk.name} has both)")

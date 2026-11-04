#!/usr/bin/env python3
"""Runs tools/release/android-patch.py on a small, fictional stand-in for the project
`tauri android init` generates, and checks the Odeion names without changing an identity
(docs/branding-compatibility.md):

- the launcher label and the activity title are Odeion;
- the application id (`to.synonym.pubkymusic`) and the Kotlin package stay, so an installed app
  updates in place, keeping its sign-ins and downloads;
- the player notification says Odeion; the Application class keeps its name.

No Android SDK needed; a real build is release.yml's android job.

    python3 -I tools/release/android-label-check.py [scratch dir]
"""

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
problems = []


def check(ok, what):
    print(f"{'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        problems.append(what)


base = Path(tempfile.mkdtemp(prefix="odeion-android-", dir=sys.argv[1] if len(sys.argv) > 1 else None))
try:
    root = base / "pm-desktop"
    app = root / "gen" / "android" / "app"
    java = app / "src" / "main" / "java" / "to" / "synonym" / "pubkymusic"
    values = app / "src" / "main" / "res" / "values"
    for d in (java, values):
        d.mkdir(parents=True)
    # Fictional fixture in the shape of Tauri 2's Android template
    shutil.copy(REPO / "crates" / "pm-desktop" / "Cargo.lock", root / "Cargo.lock")
    (app / "build.gradle.kts").write_text(
        'android {\n    defaultConfig {\n        applicationId = "to.synonym.pubkymusic"\n'
        '        manifestPlaceholders["usesCleartextTraffic"] = "false"\n    }\n}\n'
    )
    (app / "src" / "main" / "AndroidManifest.xml").write_text(
        '<manifest xmlns:android="http://schemas.android.com/apk/res/android">\n'
        '    <uses-permission android:name="android.permission.INTERNET" />\n'
        '    <application\n        android:label="@string/app_name"\n'
        '        android:usesCleartextTraffic="${usesCleartextTraffic}">\n'
        '        <activity android:name=".MainActivity" android:label="@string/main_activity_title" />\n'
        "    </application>\n</manifest>\n"
    )
    (java / "MainActivity.kt").write_text(
        "package to.synonym.pubkymusic\n\nclass MainActivity : TauriActivity() {\n"
        "  override fun onCreate(savedInstanceState: android.os.Bundle?) {\n"
        "    super.onCreate(savedInstanceState)\n  }\n}\n"
    )
    (values / "strings.xml").write_text(
        "<resources>\n    <string name=\"app_name\">Pubky Music</string>\n"  # compat: pre-Odeion template output
        "    <string name=\"main_activity_title\">Pubky Music</string>\n</resources>\n"  # compat: pre-Odeion
    )
    r = subprocess.run([sys.executable, "-I", str(HERE / "android-patch.py"), str(root)], capture_output=True, text=True)
    check(r.returncode == 0, f"android-patch.py runs ({r.stderr.strip()[:200]})")
    strings = (values / "strings.xml").read_text()
    check('<string name="app_name">Odeion</string>' in strings, "the launcher label is Odeion")
    check('<string name="main_activity_title">Odeion</string>' in strings, "the activity title is Odeion")
    check("Pubky Music" not in strings, "no old name in strings.xml")  # compat: pre-Odeion name must be gone
    check('applicationId = "to.synonym.pubkymusic"' in (app / "build.gradle.kts").read_text(),
          "the application id is unchanged (an installed app updates in place)")
    service = (java / "PlayerService.kt").read_text()
    check(service.startswith("// ") and "package to.synonym.pubkymusic" in service, "the service keeps its package")
    check('.setContentTitle("Odeion")' in service, "the player notification says Odeion")
    crash = (java / "CrashReport.kt").read_text()
    check('"Odeion crash report"' in crash, "the crash report says Odeion")
    check('android:name=".PubkyMusicApp"' in (app / "src" / "main" / "AndroidManifest.xml").read_text(),
          "the Application class keeps its name")
    again = subprocess.run([sys.executable, "-I", str(HERE / "android-patch.py"), str(root)], capture_output=True, text=True)
    check(again.returncode == 0 and "already patched" in again.stdout, "a second run changes nothing")
finally:
    shutil.rmtree(base, ignore_errors=True)

print(f"\n{len(problems)} problem(s)")
sys.exit(1 if problems else 0)

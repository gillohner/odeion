#!/usr/bin/env python3
"""Adjust the Android project `tauri android init` generates (gen/android is
not committed; CI generates it on every build). Each change checks its anchor
and fails loudly when Tauri's template moves, instead of building an app that
silently cannot connect.

1. Cleartext HTTP in release builds: nodes on a LAN are http://192.168.x.y:7788.
2. rustls-platform-verifier's Kotlin component (Pubky Ring sign-in needs it;
   see crates/pm-desktop/src/android.rs), at the version Cargo.lock pins.
3. ProGuard keep rules for it and the player service: R8 cannot see JNI use.
4. CHANGE_WIFI_MULTICAST_STATE: the Wi-Fi multicast lock that LAN discovery
   (mDNS) needs while it looks for nodes (src/android.rs).
5. The player's foreground service (tools/release/android/PlayerService.kt),
   its permissions and manifest entry: it keeps the phone's zone playing with
   the app in the background (src/android.rs starts it).
6. Our own network security config, allowing cleartext: the verifier's Android
   library (2.) brings one that allows http only to certificate-revocation
   hosts, and a network security config overrides usesCleartextTraffic (1.),
   so the WebView refused every http:// node (ERR_CLEARTEXT_NOT_PERMITTED).
7. Room for the system bars: Tauri's MainActivity draws edge to edge, and the
   WebView (before Chromium 136) reports no safe-area insets to the page, so
   the status bar covered the node's menu and search. The content view is
   padded by the bars' (and the keyboard's) insets, on the UI's background
   (dark or light with the system, like the web UI's theme-color).
8. A crash report readable without adb (tools/release/android/CrashReport.kt): an
   Application class installs the handler before any activity or native library
   loads; the player's notification gets a monochrome status-bar icon (an adaptive
   launcher icon there crashes some phones).
9. The launcher label and the activity title say Odeion. Tauri writes them from productName, which
   stays at its pre-Odeion value because it also names the desktop installs (docs/branding-compatibility.md);
   the application id is not touched, so an installed app updates in place.
10. App updates (tools/release/android/AppUpdate.kt, src/update.rs): the class and
   REQUEST_INSTALL_PACKAGES. `--without-updater` takes both out again of a patched project, for
   the .aab: an app from Google Play may update only through Play. Without the class the app
   never checks for updates.
11. USB DACs (tools/release/android/UsbAudio.kt, src/android.rs): an AudioDeviceCallback reports the
   USB audio outputs (name, rates, encodings, Android 14's mixer attributes) to the player, which
   tells the node. It never asks for USB permission. Bit-perfect playback to the DAC
   (tools/release/android/BitPerfect.kt) sets the DAC's mixer attributes, which needs
   MODIFY_AUDIO_SETTINGS (a normal permission, granted at install). Exclusive USB
   (tools/release/android/UsbExclusive.kt) asks USB permission for the DAC (Auto once per DAC on
   screen, again on a replug when it was granted before; Settings' "Allow Exclusive USB"), and
   hands the device to the app's own driver. ProGuard keep rules for all three.
12. The window's own colour before the first page paints: the pages' background (light, and dark
   in values-night) as the theme's windowBackground and Android 12's splash screen background, so a
   cold start shows no black frame (owner's recording, 2026-11-06).
13. The system's media controls (tools/release/android/MediaControl.kt, src/media.rs): one media
   session for the zone the app controls (the player service's notification becomes the media
   notification; lock screen, quick settings, headset and Bluetooth buttons, the volume keys), the
   page's message port (androidx.webkit, in Tauri's template) and Android's media volume. ProGuard
   keep rule: its static methods are called over JNI.
14. A USB DAC's permission that lasts: MainActivity takes USB_DEVICE_ATTACHED for audio-class devices
   (res/xml/usb_audio_devices.xml, class 1), so Android's dialog offers to always open the DAC with
   Odeion and the grant survives replugs (without it, a permission from requestPermission ends at
   the unplug and Exclusive USB is first only once). Side effects: Android asks "Open Odeion?" at
   every plug-in of any USB audio device (dongles and headsets too), even with Odeion not running,
   and with "always" opens Odeion at each one.

Usage: android-patch.py <crates/pm-desktop> [--without-updater]
"""
import re
import sys
from pathlib import Path

args = [a for a in sys.argv[1:] if a != "--without-updater"]
without_updater = "--without-updater" in sys.argv[1:]
root = Path(args[0] if args else "crates/pm-desktop")
app = root / "gen" / "android" / "app"
gradle = app / "build.gradle.kts"
proguard = app / "proguard-rules.pro"
MARK = "// pubky-music: patched"


def die(msg: str) -> None:
    print(f"android-patch: {msg}", file=sys.stderr)
    sys.exit(1)


if not gradle.exists():
    die(f"{gradle} missing: run `tauri android init` first")
text = gradle.read_text()
INSTALL_PERM = '<uses-permission android:name="android.permission.REQUEST_INSTALL_PACKAGES" />'
if without_updater:
    # 10, undone: for the .aab
    if MARK not in text:
        die("not patched yet: run android-patch.py without --without-updater first")
    manifest = app / "src" / "main" / "AndroidManifest.xml"
    mtext = manifest.read_text()
    if mtext.count(INSTALL_PERM) != 1:
        die(f"expected one `{INSTALL_PERM}` in {manifest}")
    manifest.write_text(mtext.replace("\n    " + INSTALL_PERM, "").replace(INSTALL_PERM, ""))
    found = list((app / "src" / "main" / "java").rglob("AppUpdate.kt"))
    if len(found) != 1:
        die(f"expected one AppUpdate.kt, found {len(found)}")
    found[0].unlink()
    print("android-patch: without the updater (no AppUpdate, no REQUEST_INSTALL_PACKAGES)")
    sys.exit(0)
if MARK in text:
    print("android-patch: already patched")
    sys.exit(0)

# 1. cleartext
old = 'manifestPlaceholders["usesCleartextTraffic"] = "false"'
if text.count(old) != 1:
    die(f"expected one `{old}` in {gradle}")
text = text.replace(old, 'manifestPlaceholders["usesCleartextTraffic"] = "true"')

# 2. the verifier's Kotlin component, version from Cargo.lock
lock = (root / "Cargo.lock").read_text()
m = re.search(r'name = "rustls-platform-verifier-android"\nversion = "([^"]+)"', lock)
if not m:
    die("rustls-platform-verifier-android not in Cargo.lock")
version = m.group(1)
text += f"""
{MARK}
repositories {{
    maven {{
        url = uri("https://github.com/rustls/rustls-platform-verifier/raw/maven-archive/android-release-support/maven/")
    }}
}}
dependencies {{
    implementation("org.rustls:rustls-platform-verifier:{version}")
}}
"""
gradle.write_text(text)

# 3. keep rule
rule = "-keep, includedescriptorclasses class org.rustls.platformverifier.** { *; }"
existing = proguard.read_text() if proguard.exists() else ""
if rule not in existing:
    existing = existing.rstrip("\n") + "\n\n# rustls-platform-verifier (called over JNI)\n" + rule + "\n"
player = "-keep class **.PlayerService { public static *; }"
crash = "-keep class **.CrashReport { public static *; }\n-keep class **.PubkyMusicApp { *; }"
updater = "-keep class **.AppUpdate { public static *; }"
usb_audio = "-keep class **.UsbAudio { public static *; }"
bit_perfect = "-keep class **.BitPerfect { public static *; }"
exclusive = "-keep class **.UsbExclusive { public static *; }"
media_control = "-keep class **.MediaControl { public static *; }"
if player not in existing:
    existing = existing.rstrip("\n") + "\n\n# The player service's start/stop (called over JNI from src/android.rs)\n" + player + "\n"
if crash.split("\n")[0] not in existing:
    existing = existing.rstrip("\n") + "\n\n# The crash report (Application class; share() is called over JNI)\n" + crash + "\n"
if updater not in existing:
    existing = existing.rstrip("\n") + "\n\n# App updates (called over JNI from src/android.rs)\n" + updater + "\n"
if usb_audio not in existing:
    existing = existing.rstrip("\n") + "\n\n# USB DACs: start() and next() are called over JNI from src/android.rs\n" + usb_audio + "\n"
if bit_perfect not in existing:
    existing = existing.rstrip("\n") + "\n\n# Bit-perfect playback: its static methods are called over JNI from src/android.rs\n" + bit_perfect + "\n"
if exclusive not in existing:
    existing = existing.rstrip("\n") + "\n\n# Exclusive USB: its static methods are called over JNI from src/android.rs\n" + exclusive + "\n"
if media_control not in existing:
    existing = existing.rstrip("\n") + "\n\n# The media controls: their static methods are called over JNI from src/android.rs\n" + media_control + "\n"
proguard.write_text(existing)

# 4. multicast permission
manifest = app / "src" / "main" / "AndroidManifest.xml"
mtext = manifest.read_text()
perm = '<uses-permission android:name="android.permission.CHANGE_WIFI_MULTICAST_STATE" />'
if perm not in mtext:
    anchor = '<uses-permission android:name="android.permission.INTERNET" />'
    if mtext.count(anchor) != 1:
        die(f"expected one `{anchor}` in {manifest}")
    manifest.write_text(mtext.replace(anchor, anchor + "\n    " + perm))

# 5. the player's foreground service, in MainActivity's package
activities = list((app / "src" / "main" / "java").rglob("MainActivity.kt"))
if len(activities) != 1:
    die(f"expected one MainActivity.kt, found {len(activities)}")
pkg = re.search(r"^package ([\w.]+)", activities[0].read_text(), re.M)
if not pkg:
    die(f"no package line in {activities[0]}")
service = (Path(__file__).parent / "android" / "PlayerService.kt").read_text()
service, n = re.subn(r"^package [\w.]+", f"package {pkg.group(1)}", service, count=1, flags=re.M)
if n != 1:
    die("PlayerService.kt has no package line")
(activities[0].parent / "PlayerService.kt").write_text(service)
mtext = manifest.read_text()
anchor = '<uses-permission android:name="android.permission.INTERNET" />'
perms = [
    "android.permission.FOREGROUND_SERVICE",
    "android.permission.FOREGROUND_SERVICE_MEDIA_PLAYBACK",
    "android.permission.POST_NOTIFICATIONS",
    "android.permission.WAKE_LOCK",
    "android.permission.ACCESS_WIFI_STATE",
]
for p in perms:
    line = f'<uses-permission android:name="{p}" />'
    if line not in mtext:
        mtext = mtext.replace(anchor, anchor + "\n    " + line)
entry = (
    '<service android:name=".PlayerService" android:exported="false"'
    ' android:foregroundServiceType="mediaPlayback" />'
)
if entry not in mtext:
    if mtext.count("</application>") != 1:
        die(f"expected one </application> in {manifest}")
    mtext = mtext.replace("</application>", "    " + entry + "\n    </application>")
manifest.write_text(mtext)

# 6. cleartext for every host (nodes on a LAN), replacing the verifier library's config
res_xml = app / "src" / "main" / "res" / "xml"
res_xml.mkdir(parents=True, exist_ok=True)
(res_xml / "pubky_music_network.xml").write_text("""<?xml version="1.0" encoding="utf-8"?>
<!-- tools/release/android-patch.py: nodes on a LAN are http://192.168.x.y:7788. -->
<network-security-config>
    <base-config cleartextTrafficPermitted="true" />
</network-security-config>
""")
mtext = manifest.read_text()
if "pubky_music_network" not in mtext:
    tools_ns = 'xmlns:tools="http://schemas.android.com/tools"'
    if tools_ns not in mtext:
        root = '<manifest xmlns:android="http://schemas.android.com/apk/res/android"'
        if mtext.count(root) != 1:
            die(f"expected one `{root}` in {manifest}")
        mtext = mtext.replace(root, root + "\n    " + tools_ns)
    anchor = 'android:usesCleartextTraffic="${usesCleartextTraffic}"'
    if mtext.count(anchor) != 1:
        die(f"expected one `{anchor}` in {manifest}")
    mtext = mtext.replace(
        anchor,
        anchor + '\n        android:networkSecurityConfig="@xml/pubky_music_network"'
        '\n        tools:replace="android:networkSecurityConfig"',
    )
    manifest.write_text(mtext)

# 7. pad the content view by the system bars, keyboard and cutout, on the UI's background
activity = activities[0]
atext = activity.read_text()
if "pubky-music: insets" not in atext:
    call = "super.onCreate(savedInstanceState)"
    if atext.count(call) != 1 or atext.count("\n}") < 1:
        die(f"expected one `{call}` in {activity}")
    atext = atext.replace(call, call + """
    // pubky-music: insets (tools/release/android-patch.py)
    val root = findViewById<android.view.View>(android.R.id.content)
    androidx.core.view.ViewCompat.setOnApplyWindowInsetsListener(root) { v, insets ->
      val t = androidx.core.view.WindowInsetsCompat.Type.systemBars() or
        androidx.core.view.WindowInsetsCompat.Type.displayCutout() or
        androidx.core.view.WindowInsetsCompat.Type.ime()
      val bars = insets.getInsets(t)
      v.setPadding(bars.left, bars.top, bars.right, bars.bottom)
      androidx.core.view.WindowInsetsCompat.CONSUMED
    }
    pubkyMusicBars()""")
    # The bars follow the system theme, as the web UI does unless Settings picks one
    # (theme-color in crates/pm-api/static/index.html). uiMode changes do not recreate the activity.
    tail = atext.rstrip().rfind("}")
    atext = atext[:tail] + """
  override fun onConfigurationChanged(newConfig: android.content.res.Configuration) {
    super.onConfigurationChanged(newConfig)
    pubkyMusicBars()
  }

  private fun pubkyMusicBars() {
    val root = findViewById<android.view.View>(android.R.id.content)
    val night = (resources.configuration.uiMode and android.content.res.Configuration.UI_MODE_NIGHT_MASK) ==
      android.content.res.Configuration.UI_MODE_NIGHT_YES
    root.setBackgroundColor(if (night) 0xFF0F1014.toInt() else 0xFFF5F5F8.toInt())
    androidx.core.view.WindowCompat.getInsetsController(window, root).apply {
      isAppearanceLightStatusBars = !night
      isAppearanceLightNavigationBars = !night
    }
  }
}
"""
    activity.write_text(atext)

# 8. the crash report: its Application class, and the player notification's icon
crash_kt = (Path(__file__).parent / "android" / "CrashReport.kt").read_text()
crash_kt, n = re.subn(r"^package [\w.]+", f"package {pkg.group(1)}", crash_kt, count=1, flags=re.M)
if n != 1:
    die("CrashReport.kt has no package line")
(activities[0].parent / "CrashReport.kt").write_text(crash_kt)
mtext = manifest.read_text()
if ".PubkyMusicApp" not in mtext:
    m = re.search(r"<application\b[^>]*>", mtext, re.S)
    if not m:
        die(f"no <application> in {manifest}")
    if "android:name=" in m.group(0):
        die(f"the template's <application> already names a class: {m.group(0)[:120]}")
    mtext = mtext[: m.start()] + m.group(0).replace("<application", '<application\n        android:name=".PubkyMusicApp"', 1) + mtext[m.end():]
    manifest.write_text(mtext)
drawable = app / "src" / "main" / "res" / "drawable"
drawable.mkdir(parents=True, exist_ok=True)
(drawable / "ic_stat_pubky_music.xml").write_text("""<?xml version="1.0" encoding="utf-8"?>
<!-- tools/release/android-patch.py: the player notification's status-bar icon (a note). -->
<vector xmlns:android="http://schemas.android.com/apk/res/android"
    android:width="24dp" android:height="24dp" android:viewportWidth="24" android:viewportHeight="24">
    <path android:fillColor="#FFFFFFFF"
        android:pathData="M12,3v10.55A4,4 0,1 0,14 17V7h4V3h-6z" />
</vector>
""")

# 9. the label people see: Odeion (productName, and so the template's value, is an install identity)
strings = app / "src" / "main" / "res" / "values" / "strings.xml"
stext = strings.read_text() if strings.exists() else ""
for name in ("app_name", "main_activity_title"):
    stext, n = re.subn(rf'(<string name="{name}">)[^<]*(</string>)', r"\1Odeion\2", stext)
    if name == "app_name" and n != 1:
        die(f"expected one app_name string in {strings}")
strings.write_text(stext)

# 10. app updates: the class next to MainActivity, and the permission to hand an APK to the installer
updater_kt = (Path(__file__).parent / "android" / "AppUpdate.kt").read_text()
updater_kt, n = re.subn(r"^package [\w.]+", f"package {pkg.group(1)}", updater_kt, count=1, flags=re.M)
if n != 1:
    die("AppUpdate.kt has no package line")
(activities[0].parent / "AppUpdate.kt").write_text(updater_kt)
mtext = manifest.read_text()
if INSTALL_PERM not in mtext:
    anchor = '<uses-permission android:name="android.permission.INTERNET" />'
    if mtext.count(anchor) != 1:
        die(f"expected one `{anchor}` in {manifest}")
    manifest.write_text(mtext.replace(anchor, anchor + "\n    " + INSTALL_PERM))

# 11. USB DACs: the watcher next to MainActivity
usb_kt = (Path(__file__).parent / "android" / "UsbAudio.kt").read_text()
usb_kt, n = re.subn(r"^package [\w.]+", f"package {pkg.group(1)}", usb_kt, count=1, flags=re.M)
if n != 1:
    die("UsbAudio.kt has no package line")
(activities[0].parent / "UsbAudio.kt").write_text(usb_kt)
bp_kt = (Path(__file__).parent / "android" / "BitPerfect.kt").read_text()
bp_kt, n = re.subn(r"^package [\w.]+", f"package {pkg.group(1)}", bp_kt, count=1, flags=re.M)
if n != 1:
    die("BitPerfect.kt has no package line")
(activities[0].parent / "BitPerfect.kt").write_text(bp_kt)
ex_kt = (Path(__file__).parent / "android" / "UsbExclusive.kt").read_text()
ex_kt, n = re.subn(r"^package [\w.]+", f"package {pkg.group(1)}", ex_kt, count=1, flags=re.M)
if n != 1:
    die("UsbExclusive.kt has no package line")
(activities[0].parent / "UsbExclusive.kt").write_text(ex_kt)

# 13. the media controls, next to MainActivity; the page's message port is androidx.webkit's
if "androidx.webkit:webkit" not in gradle.read_text():
    die(f"androidx.webkit is not in {gradle}: the media controls' message port needs it")
mc_kt = (Path(__file__).parent / "android" / "MediaControl.kt").read_text()
mc_kt, n = re.subn(r"^package [\w.]+", f"package {pkg.group(1)}", mc_kt, count=1, flags=re.M)
if n != 1:
    die("MediaControl.kt has no package line")
(activities[0].parent / "MediaControl.kt").write_text(mc_kt)
mtext = manifest.read_text()
audio_perm = '<uses-permission android:name="android.permission.MODIFY_AUDIO_SETTINGS" />'
if audio_perm not in mtext:
    anchor = '<uses-permission android:name="android.permission.INTERNET" />'
    if mtext.count(anchor) != 1:
        die(f"expected one `{anchor}` in {manifest}")
    manifest.write_text(mtext.replace(anchor, anchor + "\n    " + audio_perm))

# 12. the window's colour before the first paint, as the pages' --bg (app.css)
res = app / "src" / "main" / "res"
for qual, colour in (("values", "#f5f5f8"), ("values-night", "#0f1014")):
    (res / qual).mkdir(parents=True, exist_ok=True)
    (res / qual / "odeion_colors.xml").write_text(
        '<?xml version="1.0" encoding="utf-8"?>\n<resources>\n'
        f'    <color name="odeion_bg">{colour}</color>\n</resources>\n'
    )
themes = sorted(res.glob("values*/themes.xml"))
if not themes:
    die(f"no themes.xml under {res}")
bg_items = (
    '    <item name="android:windowBackground">@color/odeion_bg</item>\n'
    '        <item name="android:windowSplashScreenBackground" tools:targetApi="31">@color/odeion_bg</item>\n'
    "    "
)
for t in themes:
    ttext = t.read_text()
    if "</style>" not in ttext:
        die(f"no style in {t}")
    ttext = ttext.replace("</style>", bg_items + "</style>")
    if "xmlns:tools=" not in ttext:
        ttext = re.sub(r"<resources(\s|>)", r'<resources xmlns:tools="http://schemas.android.com/tools"\1', ttext, count=1)
    t.write_text(ttext)

# 14. USB DACs: MainActivity takes USB_DEVICE_ATTACHED for audio-class devices, so the permission lasts
res_xml.mkdir(parents=True, exist_ok=True)
(res_xml / "usb_audio_devices.xml").write_text("""<?xml version="1.0" encoding="utf-8"?>
<!-- tools/release/android-patch.py: USB audio devices (class 1, on the device or an interface). -->
<resources>
    <usb-device class="1" />
</resources>
""")
mtext = manifest.read_text()
usb_action = "android.hardware.usb.action.USB_DEVICE_ATTACHED"
if usb_action not in mtext:
    acts = list(re.finditer(r'<activity\b[^>]*?android:name="\.MainActivity"[^>]*?(?:/>|>.*?</activity>)', mtext, re.S))
    if len(acts) != 1:
        die(f"expected one MainActivity <activity> in {manifest}, found {len(acts)}")
    a = acts[0]
    usb_entry = (
        "            <intent-filter>\n"
        f'                <action android:name="{usb_action}" />\n'
        "            </intent-filter>\n"
        f'            <meta-data android:name="{usb_action}"\n'
        '                android:resource="@xml/usb_audio_devices" />\n'
        "        "
    )
    block = a.group(0)
    if block.endswith("/>"):
        block = block[:-2].rstrip() + ">\n" + usb_entry + "</activity>"
    else:
        # before </activity>, on its indentation
        block = block[: -len("</activity>")].rstrip() + "\n" + usb_entry + "</activity>"
    mtext = mtext[: a.start()] + block + mtext[a.end():]
    manifest.write_text(mtext)

print(
    f"android-patch: multicast permission, cleartext on, rustls-platform-verifier {version}, keep rule, "
    f"player service ({pkg.group(1)}), network config, insets, crash report added, label Odeion, app updates, USB DACs (bit-perfect, exclusive USB), window colour, media controls, USB attach filter"
)

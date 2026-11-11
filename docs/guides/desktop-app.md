# The desktop and Android app

The Odeion app is the node's web page in its own window, for macOS, Windows, Linux and
Android. It connects to a node you have installed (see the [README](../../README.md#get-started));
it doesn't hold your library itself. What it adds over a browser:

- it finds your node on the home network, and from anywhere once the node is signed in to Pubky;
- on a computer, it plays through that computer's sound outputs and DACs;
- on Android, it plays on the phone (speaker, headphones, Bluetooth, a USB DAC) and can keep
  music for offline listening.

## Install

Download from the [latest release](https://github.com/gillohner/pubky-music-node/releases/latest):

| System | File | First start |
| :-- | :-- | :-- |
| macOS | `pubky-music-macos-universal.dmg` | Drag it to Applications. The app isn't signed by Apple: the first time, right-click it and choose **Open**. If macOS still refuses, open **System Settings › Privacy & Security** and click **Open Anyway**. |
| Windows | `pubky-music-windows-x86_64-setup.exe` | The installer isn't signed: choose **More info**, then **Run anyway**. |
| Linux | `.deb` (Debian, Ubuntu) or `.AppImage` (others), x86_64 or arm64 | `sudo apt install ./pubky-music-linux-x86_64.deb`, or make the AppImage executable and run it. |
| Android, Google TV | `pubky-music-android-arm64.apk` | Open the file on the device and allow installs from that source. |

On a computer the installed app still carries its earlier name for now: "Pubky Music" in Applications,
the Dock, the Start menu and the Linux app menu (compat: pre-Odeion name; it is what lets updates
replace the installed app). Its window says Odeion. On Android the app is called Odeion.

## Connect to your node

The app opens on a connect page. Use one of these:

- **On this network:** nodes on your home network are listed. Click yours and enter the access
  token once.
- **Or connect by address:** type the node's address (for example `192.168.1.10:7788`) and the token.
- **Sign in with Pubky Ring:** once the node is signed in to Pubky (**Settings › My accounts**
  on the node), the app finds it through your Pubky account, at home or away.

The app remembers the node. If the node's address changes (a new IP from the router), the app
finds it again on the network by itself.

## Playing on this computer or phone

While connected at home, the app adds this device as a speaker on the node: "<computer name> ·
<sound output>" on a computer, "<phone model> · Speaker or headphones" on Android. Pick it with the
zone button in the player bar, like any other speaker. On Linux it plays bit-perfect to a DAC; on
macOS and Windows it goes through the system's sound output. Computers that play to a DAC this way
can be grouped in sync, like squeezelite players (Settings › Zones and outputs › Group outputs).

Only the node the app is connected to can play to this device: each time the app starts it makes
a new key and gives it to that node, with nothing for you to set up. With the node on the same
computer (`localhost`), the player is reachable from this computer only. Otherwise the node
connects to it on TCP port 7790: allow that if your firewall asks.

Away from home the app still controls the node and its speakers, but this device isn't offered as
a speaker then.

### This phone (Android)

On Android, **This phone** in the zone button plays the music in the app itself: the app opens the
song from your node (at home, or through remote access away from home) and plays it through the
phone's output the same way the phone's own zone does. With a USB DAC that means Auto, bit-perfect
or Exclusive USB as chosen in Settings › This phone, and the DAC gets the song's own rate (a
44.1 kHz FLAC arrives at 44.1 kHz, not at Android's 48 kHz). Now playing › Details › Output says
what the DAC gets, as for any zone. It keeps playing with the screen off and goes on to the next
song by itself.

A few streams the app can't play itself (a radio playlist, a format it doesn't decode) play through
Android's mixer instead; Details › Output then says "Android mixer". The phone's own zone stays on
the node for other controllers; while the app plays, the node starting that zone pauses the app
(one stream at a time on the phone's output), and the app playing again takes it back.

## Updates

On macOS, Windows and the Linux AppImage, the app checks for a new version when it starts and
asks before installing it. The Linux `.deb` is updated like other packages: install the new
`.deb` over the old one.

On Android (the `.apk` from the release page), the app checks too and asks "Odeion X is available
(this is Y). Install it now?". After **Install** it downloads the update; Android then asks you to
confirm. The first time, Android needs your permission for Odeion to install apps: the app says so
and opens that setting; turn on **Allow from this source** and go back to Odeion. Your settings and
downloaded music stay.

## Subsonic apps instead

Any OpenSubsonic app (Symfonium and Amperfy on phones, Feishin on desktops) works with the node
too: server `http://<node address>:7788`, user `pubky`, and the access token as the password.

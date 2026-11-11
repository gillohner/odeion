# Device test prompt: Chromecast, Google TV, smart TVs, Sonos, AirPlay, Snapcast, Squeezebox, Android phones, USB DACs on Android

The multiroom outputs (infra brief §3) were built against mocks and open-source receivers on a machine
with no speakers or TVs. Paste the section for each device you have into Claude Code on a laptop on
the same LAN as the node, or follow it by hand. Each section lists what was already checked without
the device, so you know what is new.

```
Use a node from main (or the edge image) with [api] token set, as in a real setup. In node.toml:
[outputs] upnp = true, cast = true. Report pass/fail with a screenshot or log line per check, and
each device's brand, model, year and firmware. Run the node with RUST_LOG=info,pm_out_cast=debug,
pm_out_upnp=debug and attach the log for failures.
```

## Chromecast, Google TV, Android TV, Nest speakers, Cast groups

Checked without devices (`crates/pm-out-cast` tests against a fake receiver):
- launch, load and stop;
- keepalive pings both ways;
- volume through the live link;
- resuming the same session after the connection drops;
- stepping aside when another app takes the device;
- the title, artist, album and art the device is sent;
- stream URLs that carry a per-stream key, not the token.

```
1. Settings → Outputs → Scan the network: every Cast device appears as a zone named after the
   device (model in brackets). Cast groups from the Google Home app appear as "<name> (Cast group)".
2. Play an album with covers to a Chromecast on a TV (or a Google TV): the TV shows the title,
   artist, album and cover; audio plays; the zone's position advances. Note the delay between
   pressing play and hearing it.
3. Pause and resume, next, previous, seek to the middle of a track: each is heard within a few
   seconds; the TV keeps playing (no "nothing is playing" screen) across a seek.
4. Volume slider in the zone: the TV's volume overlay moves. Mute.
5. Leave it playing 10 minutes: it does not stop by itself (the keepalive works).
6. Turn the router's Wi-Fi off for 5 s and on again while it plays: playback comes back without
   touching anything (log: "Cast session resumed").
7. While the node plays, cast YouTube to the same TV from a phone: the node's zone stops quietly
   and does not stop YouTube (log: "another sender took the Cast device").
8. A Cast group of two Nest/Home speakers: play to the group zone; both speakers play in sync.
9. A 24/96 FLAC: the signal path shows the resample to 48 kHz (Cast's ceiling), not bit-perfect.
10. Gapless album (live album or a DJ mix): no gaps between tracks. The TV keeps the first
    track's title for the whole run, a known limitation: a new title would need a reload, which
    breaks the gap.
```

## Smart TVs and other DLNA renderers (Samsung, LG, Sony, Philips)

Checked without devices, with gmediarender (a real DLNA renderer) on a token-protected node:
- Found over SSDP; fetched the keyed stream URL and played in real time.
- Limited to LPCM only (`--mime-filter audio/L16`, like many LG and Samsung TVs): offered DLNA LPCM.
  The zone converted to 16-bit 48 kHz and the signal path says so.
- Limited to MP3 only: no zone, and the log says why.
- The answer carries `transferMode.dlna.org` and `contentFeatures.dlna.org`, and HEAD does not take
  the stream away from the GET that follows.

```
1. The TV appears as a zone after a scan. TVs often need "DLNA", "Media sharing" or "Allow remote
   control" turned on, and must be on, not in standby. The node log line "UPnP renderer … stream="
   says what it chose (Wav or L16); "accepts no PCM stream" means the TV takes only compressed
   files, and no zone appears.
2. Play an album with covers: audio from the TV speakers; the TV shows title, artist, album and
   cover (photo it).
3. Pause/resume, next, seek, volume. Note anything the TV ignores or answers with an error (log
   lines from pm_out_upnp: "renderer Play refused, retrying" is fine once or twice).
4. A 24/96 file: on a WAV TV the signal path stays bit-perfect up to what the TV listed; on an
   LPCM TV it shows the conversion to 16-bit 44.1/48 kHz.
5. Report the TV's GetProtocolInfo list (in the log at debug level, or from
   `curl` on its ConnectionManager) so the device database can record it.
```

## Sonos

Checked without devices (`crates/pm-out-upnp/src/sonos.rs` tests against two fake players):
- A player in a Sonos group plays through its coordinator. The topology is read from
  `GetZoneGroupState`, escaped as Sonos sends it.
- A node zone made of Sonos players joins them with `x-rincon:` and they leave again on stop. The
  lead stops before the others leave.
- Sonos's ceiling is 16-bit up to 48 kHz.

```
1. Every Sonos player appears as a zone after a scan (UPnP on, same LAN; Sonos S1 and S2 both).
2. Play an album to one player: audio plays, the Sonos app shows the title, artist and cover.
   Check the signal path: anything above 16/48 shows the conversion.
3. Group two players in the Sonos app, then play from the node to the member (not the first
   one): the whole Sonos group plays (it went to the coordinator).
4. Ungroup them in the Sonos app. In the node, make a zone of both (Zones → new zone with two
   outputs) and play: the Sonos app shows them grouped, in sync. Stop: they are separate again.
5. Leave a zone playing for an hour: Sonos does not stop the stream by itself (it is a live WAV
   of unknown length; if Sonos stops near 4 GB or after some time, report when).
6. Volume per player.
```

## Snapcast (Raspberry Pis, old phones, any snapclient)

Checked here with real snapserver 0.34 and snapclient:
- Every Snapcast group became a zone, labelled by its clients.
- Playing routed the group to the node's stream; the client received the tone in real time.
- A test pattern came out of snapclient bit-exact at 16 and 24 bits (FLAC on the wire).
- Stopping put the group back on its previous stream and removed the node's.

```
1. Install snapserver on the node's machine (or any machine on the LAN) and snapclient on two
   Pis. In node.toml: [outputs] snapcast = "<snapserver host>". Restart the node: each Snapcast
   group is a zone "<client names> (Snapcast)".
2. Group both Pis in Snapcast (snapweb or the Snapcast app) and play to that zone: both play, in
   tight sync (stand between them: no echo).
3. Volume from the node changes both clients. Stop: the group goes back to what it played before.
4. A 24/96 album: the signal path stays lossless to 24/96; note if a Pi's sound card resamples
   (snapclient log).
```

## AirPlay (Apple TV, AirPort Express, AirPlay speakers, shairport-sync)

Checked here with shairport-sync 4.3.7, a real AirPlay 1 receiver:
- Found over mDNS (`_raop._tcp`) with its TXT; the node made a zone "<name> (AirPlay)".
- The zone played in real time (the planner converted to 44.1 kHz 16-bit, and the signal path
  says so).
- A test pattern came out of the receiver bit-exact.

What is sent: AirPlay 1 (RAOP), unencrypted, Apple Lossless, 44.1/16. Receivers that require
encryption (`et` without 0; older AirPort Express firmware) are skipped, and the log says why.
AirPlay 2 is not built: its open implementations rely on reverse-engineered pairing (handoff §2).
Most AirPlay 2 devices still take AirPlay 1.

```
1. Every AirPlay receiver appears as a zone after a scan, or is named in the log with the reason
   it was skipped (paste those lines).
2. Play an album: audio after about 2 s (AirPlay's buffer); the receiver shows title, artist and
   album where it has a screen (Apple TV).
3. Pause and resume, next, seek: each is heard within about 2 s.
4. Volume from the node moves the receiver's volume (Apple TV shows it).
5. An Apple TV (tvOS 17/18) and a HomePod: does it play, is it skipped (encryption), or does it
   refuse (log "the receiver refused …")? Note the model and OS version.
6. Two AirPlay zones grouped in the node: "loosely synced" (AirPlay 1 has no shared clock);
   note the audible offset.
```

## An Android phone or tablet as a zone (the app's built-in player)

Checked here on an Android 14 emulator in CI (release.yml, `android-emulator`), with the signed release APK:
- Connected by address, the app's player became a zone "<model> · Speaker or headphones".
- A 16/44.1 track played in real time through AAudio, and kept playing with the app in the background
  (the foreground service; `dumpsys` shows it in the foreground).

What it is: Android's shared audio path (AAudio through cpal), 44.1 or 48 kHz up to 24-bit; the node
converts anything higher (the signal path says so). Not bit-perfect: Android's mixer runs at 48 kHz on
nearly every phone. In a group it is "loosely synced": Android adds latency the node cannot see.

```
1. Install pubky-music-android-arm64.apk from the edge release. Connect to your node (Pubky Ring,
   "On this network", or by address). Allow notifications when asked: a "Odeion" notification
   says the phone is a zone. The node's Zones list shows "<model> · Speaker or headphones".
2. From another controller (the web UI on a laptop), play an album to the phone's zone: it plays from
   the phone's speaker within about a second. Pause, next and seek are heard at once.
3. Lock the phone and leave it for five minutes: playback goes on. Then switch to another app: it
   still goes on. Swipe the app away from recents: the zone stops and the notification goes.
4. Plug in wired headphones, then pair Bluetooth headphones, during playback: the sound follows to
   each (Android's routing). A USB DAC with an adapter: the same; note the rate the DAC shows.
5. Wi-Fi: walk to the edge of coverage. Note any dropouts and whether playback recovers by itself.
6. Battery: note the battery used over 30 minutes of playback with the screen off.
7. If anything fails, attach `adb logcat -s pubky-music` (USB debugging on) and the phone's model
   and Android version.
```

## Native in-app player and offline songs (Android)

"This phone" is the app's own player: the app opens the node's song and plays it through the phone's
output with the phone zone's path (Auto, bit-perfect, Exclusive USB), at home and through remote
access. Checked in CI on an emulator (`android-emulator`): a song started in the page plays in the
app in real time, in the background with the screen off, the node's zone on the phone takes the
output over (the app pauses), and a song downloaded in the app plays in airplane mode with the node
stopped. Not checked there: a USB DAC, remote access, a real phone's battery.
Downloads (Download in the ⋯ menu, the storage screen, the mark) are checked on a fixture page
(`tools/ui-app-downloads.js`) and the sync with a stand-in node (`crates/pm-offline/tests/offline.rs`).

```
Edge or CI APK of the top PR of the stack. Start with the KA17's volume low (0-5 %).
1. On mobile data (Wi-Fi off, so the app uses remote access), FiiO KA17 plugged in: play a library
   16/44.1 FLAC on "This phone". The KA17 shows 44.1k. Now playing > Details > Output says the path
   ("Bit-perfect · FiiO KA17 · 44.1 kHz / 16 bit", or "Exclusive USB · FiiO KA17 · …", or
   "Shared · FiiO KA17 · <why>" with Settings > This phone on Shared).
2. Screen off for 3 minutes while it plays: the next song starts by itself; the lock-screen
   buttons (pause, next) work.
3. Download an album (its page's ⋯ menu > Download). Your library > Downloads lists it with its
   size and the download under way; when it is done, its songs and its tile carry the download mark.
4. Airplane mode on, close the app (swipe it away) and open it again: it says your node can't be
   reached and opens what the phone keeps; the album plays, seek and next work, and it goes on
   with the screen off. Airplane mode off: once the music is paused or over, the app goes back to
   the node by itself (never in the middle of a song).
5. A TIDAL song (search for one, or a playlist with one): its ⋯ menu has no Download.
6. Your library > Downloads > Remove on the album: it goes from the list, the marks go, and the
   album still plays from the node.
7. If anything fails: `adb logcat -s pubky-music` (lines "the app's own player: …" say what it
   plays and the path), the phone's model and Android version.
```

## The Android app's screens (the owner's recording, 2026-11-06)

Checked here with a phone's viewport rules in Chromium (tools/ui-android-first.js at 360, 390 and
412 px, the app's bridge, six zones and the API token warning): every page and panel is the screen's
width; Play to, Sleep timer and the status panel are bottom sheets; Home keeps its headers in place
while it loads; the connecting page is centred and offers Try again after 5 s. Not checked without
the phone: the WebView itself, the keyboard, the first frames of a cold start.

```
1. Install the edge APK. Cold start (swipe the app away first): no black window; the app's colour,
   then the connecting page in the middle (not at the top first), then the app. Turn the node off
   once: after about 5 s the page says it hasn't answered and offers Try again.
2. With the status chip showing (the API token warning), swipe sideways on Home, Search, Zones,
   Settings, Now playing and its panels: nothing moves sideways and no left edge is cut off
   ("earch music", "ome On" before). Pinch: the page doesn't zoom out.
3. Open Now playing and tap the zone's name at its top (the mini player has no zone button since
   2026-11-08): Play to is a sheet from the bottom, the full width, with a handle. Swipe it
   down: it closes. Open it again, tap Sleep timer: a sheet; Back closes it and the page stays.
4. Tap the chip: it says "Change the API token" (not "not answering"); its sheet is "External
   resources".
5. Open Home twice: the second time the covers fill grey squares of the same size and the headers
   don't jump down.
6. Tap Search in the bottom bar, then the field: the keyboard opens, and the field and first
   results stay above it.
7. Report the phone's model and Android version, and a screen recording of 1 to 6.
```

## The phone UI: mini player, Now playing, lists and swipes (the owner's recording, 2026-11-08)

Checked here with a phone's viewport rules in Chromium (tools/ui-phone-simplify.js and
tools/ui-nowplaying-phone.js at 360, 390 and 412 px, swipes as a finger's Pointer Events) and in the
Android emulator (the android workflow's phone-as-zone job: its UI tour, screenshots and a
recording; every tour assertion must pass; the whole tour on Android 15, only up to Play to on
Android 11, whose emulator image has a WebView older than Chrome 88 that never updates). The emulator reaches its fixture node through
ADB; a real phone reaching its node over Wi-Fi is a separate acceptance check.
Not checked without the phone: how the swipes feel under a real finger (speed, the
40 px threshold), the keyboard over the tag field, and Back from the system's gesture navigation.

```
1. Library or an album with a song playing. The mini player: the cover, the title and artist on
   the whole width, one play/pause on the right, a thin line on top that moves with the song.
   Nothing else in it. Swipe it left: the next song slides in; right: the previous one. Swipe it
   down: nothing happens. Scroll the album's list up and down over it: the song doesn't change.
2. Swipe the mini player up (or tap it): Now playing, one screen with nothing to scroll: the zone's
   name at the top, the cover, title, quality, seek, the five transport buttons, volume, and Up
   next · Lyrics · More at the bottom. Swipe the cover left and right: next and previous.
3. Swipe up on the title: Up next as a sheet over the player, with a handle, the song and its
   pause, and the tabs Up next | Lyrics | More. Swipe sideways on it: the tabs change. Swipe down on
   the handle: it closes. Open Lyrics, press Back: the sheet closes, Now playing stays; Back again:
   Now playing closes.
4. More › Tags: tap Add a tag. The keyboard opens and the field stays in view above it, no black
   space. Add one: the message shows at the bottom, above the keyboard or the bars, not mid-screen.
5. An album not matched yet: the header is compact (cover beside the title), Play, Shuffle, then
   heart, Play to and ⋯ as plain icons; "Not matched to MusicBrainz · Match" is one small line under
   them. At least two songs show before the mini player. The playing song has a small equaliser
   and the accent colour. "✓ Select": the check and the word on one line. Long-press a song: it is
   selected (with the bar of actions), no menu.
6. Confirm matches (Match on that album, or from Identify in Settings): the title, Skip all unsure and
   "How confirming works" show at once with grey rows under them, and nothing jumps when the list
   comes; a long file name is one line, a tap shows all of it.
7. Anything loading (Starting on…, Saving…, the top line while a page loads): one round spinner,
   turning, centred, never a dash.
8. Report the phone's model and Android version, and a screen recording of 1 to 7.
```

## USB DAC (FiiO KA17) on Android

Checked without the DAC:
- The DAC's description, the choice of Android's mixer format for each track, the rule for
  "confirmed", the sample packing (24 bit as packed 24 or left-justified 32, values unchanged) and the
  node's plan (a 96/24 and a 192/24 track reach the DAC with no resample, no requantize) are
  unit-tested (crates/pm-out-local/src/usb_dac.rs and bit_perfect.rs, against recorded and fake
  mixer lists).
- The node learning it, and the zone pausing when the DAC goes, are tested on a fake phone
  (crates/pm-out-net/tests/device.rs, crates/pm-zones/tests/zone.rs). So are the page's words
  (tools/ui-usb-dac.js).

Not checked: a real DAC on a real phone, and Android's own behaviour (whether it really passes the
stream untouched). **The KA17's own display is the acceptance check**: it shows the rate it receives.

What it does:
- **Android 14 and later:** the app sets the DAC's mixer to the track's exact format in bit-perfect
  mode, and plays it with the same format. The node stops converting to 44.1/48 for the phone: a 96/24
  file reaches the DAC as 96/24. The app says "Bit-perfect · FiiO KA17 · 96 kHz / 24 bit" only when
  Android confirmed it for the format playing now; otherwise it says why not (for example "the DAC
  does not list 352.8 kHz for bit-perfect", or "Android refused bit-perfect for this format").
- **Volume, at any level:** Android scales nothing in bit-perfect mode; it hands its media volume to
  the DAC's own hardware volume (on phones whose USB audio follows Android's reference code). So the
  app's slider, the volume keys and Android's panel stay Android's media volume and the music stays
  bit-perfect at any volume; nothing switches to shared because of the volume. Before the first
  bit-perfect song on a DAC the app lowers that volume, only while Android plays media to that DAC
  alone, to the level you last played it at, else 20 % (it never raises it: lower already, it
  stays). Android then sets the DAC's own volume from it, so the KA17's knob does not limit the
  level in this mode. On a phone whose Android doesn't pass the volume to the DAC, it plays at the
  DAC's own level whatever the media volume. A DAC without a volume of its own: the app has no
  slider for it; set the level on the DAC or the amplifier.
- **Below Android 14,** without Exclusive USB (below), or when Android refuses, it plays shared as
  before (48 kHz on nearly every phone), and says why.
- **Unplugging mid-song pauses the zone** ("FiiO KA17 was unplugged: paused"); nothing plays on from
  the phone's speaker. Plug it in again and press Play: it carries on where it was, on the DAC.

```
1. Install the edge APK (pubky-music-android-arm64.apk), connect to your node, pick the phone's zone.
   Note the phone's model and Android version (Settings › About phone › Android version).
2. Safety first: no headphones or in-ear monitors on the KA17 for the first play of this section
   (a speaker or amplifier turned low instead), and the KA17's own volume down: in Android's
   bit-perfect mode Android sets the DAC's volume from its media volume, and a phone that doesn't
   may play at the DAC's full level. If Odeion was allowed the KA17 with "always" before, clear it
   first: Settings › Apps › Odeion › Open by default › Clear defaults (wording varies by phone).
   Plug in the FiiO KA17 (USB-C, no adapter), app on screen. Android may ask "Open Odeion to handle
   FiiO KA17?" and/or whether Odeion may access it: for this section answer Cancel / Deny to each,
   without "always" (Android's bit-perfect path; Exclusive USB is the next section). Within a few
   seconds:
   - The zone picker (the speaker button) shows under the phone's zone "FiiO KA17 · …".
   - Settings › This device › This phone: "FiiO KA17", what it takes, how it plays, who sets the
     volume ("Volume: the DAC’s own" once Android let the app read the DAC), and an "Allow Exclusive
     USB" button; nothing else to choose. On Android 13 or older a note says why it plays shared.
3. Android's media volume low (0-5 %): the app lowers it before the first song (to 20 % the first
   time) but never raises it, so it stays where you put it. Play in turn a 44.1/16, a 96/24 and a
   192/24 FLAC file. For each, report:
   - what the KA17's own display shows (expected on Android 14+: 44.1, 96, 192; below Android 14:
     48 each time);
   - what the app's Sound panel says under "Playing on" (expected: "Bit-perfect · FiiO KA17 ·
     44.1 kHz / 16 bit", and so on), and its Signal path (expected: no resample stage);
   - whether system sounds (a notification) are heard during playback (Android mutes them in
     bit-perfect mode).
   If the panel says "Shared · FiiO KA17 · <why>", report the exact words.
4. Volume: during bit-perfect playback press the phone's volume buttons (one step at a time, staying
   low). It stays bit-perfect (the panel keeps saying "Bit-perfect · FiiO KA17 · …", no gap), and the
   KA17's own level (its display) follows the buttons: Android hands its volume to the DAC. If the
   display doesn't change, note it: that phone's Android doesn't pass the volume to the DAC. Pause,
   change the volume, press Play: still bit-perfect. Only your keys and the slider change the level;
   report anything louder than you set.
5. Unplug the KA17 mid-song: the zone must pause (the bar says "FiiO KA17 was unplugged: paused"),
   and nothing must play from the phone's speaker. Also try at the end of a song, just as the next one
   (in another format) starts: it must pause too. Plug it in again: the DAC shows again; press Play:
   it carries on where it was, on the DAC (check its display). Report any sound from the speaker,
   however short.
6. Close the app (swipe it away): a notification sound plays from the DAC again.
7. If something is off, attach `adb logcat -s pubky-music` (USB debugging on): the lines
   "USB audio devices changed", "bit-perfect player" and "no bit-perfect player" show what the app
   saw and what Android answered.
```

### Exclusive USB (the app's own USB driver)

Checked without the DAC: the playback setting for each format, the clock and volume requests (the
bytes on the wire), the packet sizes at 44.1/48/96/192 kHz on full and high speed, the DAC's feedback
and the packet sizes it gives, the start level (the last one or -40 dB, never louder than the DAC
was) and the give-back, the quirks
table, and the driver's steps (open, play, rate change, unplug) against a fake DAC
(crates/pm-usb-audio, crates/pm-out-local/src/exclusive.rs); the page's words (tools/ui-usb-dac.js).
Not checked: anything on a real phone. The usbfs calls follow Linux's headers and libusb's Android
backend, but only this pass shows whether Android lets the app take the DAC, whether the KA17 takes
the packets, and whether Android gets the DAC back afterwards. Device-unverified until then.

What it does: on any Android (also below 14), the app takes the DAC from Android's driver, sets its
clock to the song's rate, reads the rate back, and sends the samples itself. The app says "Exclusive
USB · FiiO KA17 · 96 kHz / 24 bit" only when the DAC read back exactly that rate and the stream runs.
The KA17's own volume becomes the zone's volume (the slider, the keys and Android's panel set it);
before the first sound the app sets it itself to the level you last played this DAC at or -40 dB,
never louder than the DAC was (turned down or muted on its own buttons, it stays down: muted, it
starts at its quietest), never from Android's media volume (which stays where it was). So the
KA17's knob is no protection here: the app sets the level. Meanwhile other apps' sounds go to the
phone's speaker.

```
1. Same phone and files as above. Protection: no headphones or in-ear monitors on the KA17 for the
   first play, an amplifier or speaker turned low (the app sets the KA17's own level itself: -40 dB
   the first time). Settings › This phone › Allow Exclusive USB
   (Android asks by itself once per DAC, the first time it is plugged in with the app on screen; the
   button asks again). Android asks whether Odeion may access the FiiO KA17: tick "always open"
   (or similar) if the dialog offers it, then allow. Report whether the dialog appeared, its words,
   and whether it offered "always". With "always", Android may also open Odeion when the KA17 is
   plugged in (report it).
2. Play the 44.1/16, 96/24 and 192/24 files in turn. For each, report:
   - the KA17's display (expected: 44.1, 96, 192 on any Android version);
   - the Sound panel's "Playing on" line (expected: "Exclusive USB · FiiO KA17 · 96 kHz / 24 bit",
     and so on) and the signal path (no resample stage);
   - whether the first second was quiet (the last level or -40 dB, never louder than the KA17 was:
     turn it down to its lowest before one play and check it starts there);
   - any clicks, dropouts or a pitch that drifts over a few minutes (the DAC's feedback paces it).
   If the panel says "not confirmed", or the zone stops with a reason, report the exact words.
3. Volume: move the zone's volume in the app. The KA17's volume (its display) must follow, and the
   signal path must not show a volume stage. Report the steps you saw on the display.
4. Unplug the KA17 mid-song: the zone pauses ("FiiO KA17 was unplugged: paused"), nothing from the
   speaker. Plug it in again (app on screen) and press Play: Exclusive USB again, at the same level
   or lower, never higher. If "always" wasn't ticked, Android asks again once on this plug-in
   (allow it); a "no" holds until the next plug-in, and it plays bit-perfect through Android
   meanwhile. Report whether it asked.
5. Close the app (swipe it away) while paused: within a few seconds Android plays notification sounds
   from the DAC again, and the KA17's volume is where it was before step 1 or lower (never higher).
   Report if it needs a replug.
6. Open the app and play again (Exclusive USB at the level you left it).
   Then force-stop the app while it plays (Settings › Apps › Odeion › Force stop): Android has no
   sound from the DAC until the app starts again (it gives the DAC back at its next start) or the
   DAC is replugged. Report which of the two brought it back.
   Mute the zone during a 96/24 song, let a 44.1/16 song follow: it stays muted; unmute, and the
   next 96/24 song plays at your level again; close the app while muted: the DAC goes back muted.
   Turn the zone down during a 96/24 song: the next 44.1/16 song is no louder.
7. If an Android 13 (or older) phone is at hand: steps 1 to 5 there. It plays Exclusive USB once
   access was allowed (as on any Android version).
8. On problems: `adb logcat -s pubky-music`, lines with "Exclusive USB".
```

## Media controls, volume keys and full DAC control (Android)

Checked without a phone: what the system's media controls show for each kind of zone, the volume
steps (one key press is one step, never less than the device's own), Android's media volume as a
percent and back, Auto's order (Exclusive USB, then bit-perfect, then shared) and its one permission
ask, the page's two calls to the app (crates/pm-desktop/src/media.rs, crates/pm-out-local/src/
media_volume.rs and exclusive.rs; tools/ui-android-media.js); and on the emulator (Android 11, 15,
17: tools/android-endpoint.py) the session in `dumpsys media_session`, KEYCODE_MEDIA_NEXT, the volume
keys on the phone's zone and on a network zone, and screenshots of the notification and lock screen.
Not checked: a real phone's lock screen and Bluetooth buttons, the keys with the app in the
background, and the KA17. **Volumes at 0-5 % only; never play on the Linn or the NUC's zones for
this; start the KA17 test from its lowest volume.**

What it does:
- **The system's media player:** the notification, the lock screen and the quick-settings panel show
  the zone the app's page has chosen (title, artist, cover, position) with previous, play/pause and
  next; headset and Bluetooth buttons drive it. Network zones too (the Linn), and This phone.
- **One volume:** on a network zone the phone's volume keys and Android's volume panel set the zone's
  own volume (the same as the slider). On the phone's own zone and on This phone the slider is
  Android's media volume, which the keys move too; the app adds no volume of its own. On screen this
  works on every Android version; in the background Android sends the keys to a network zone from
  Android 16 on (Android 12L to 15 only on phones that allow it, Android 12 not at all).
- **A USB DAC, best path by itself:** with the DAC plugged in and the app on screen, Android asks once
  whether Odeion may use it; allowed, it plays Exclusive USB (the KA17's own volume is the zone's,
  the keys move it, nothing is scaled in the app); refused, bit-perfect on Android 14+ at any media
  volume (Android hands it to the DAC's own volume), else shared, and says why. "Play on this phone"
  uses the phone's own zone while the DAC is there (This phone, the page's own player, always goes
  through Android's mixer). Settings › This phone shows how it plays and who sets the volume, with
  nothing to choose ("Allow Exclusive USB" while Android's permission is missing).
- **Calls and other apps:** a call pauses the phone's zone and it plays on afterwards; another app
  playing pauses it; unplugging headphones pauses it; after the DAC is unplugged, Android's volume on
  the speaker is at most a quarter.

```
1. Install the edge APK, connect at home (same Wi-Fi as the node), Android media volume low.
2. Pick the phone's own zone in the app and play a song. Pull down the notification shade: the
   media notification shows the title, artist and cover, with previous / pause / next. Press them;
   report what happens. Lock the phone: the lock screen shows the same controls (report).
3. Bluetooth headphones or a headset with buttons: play/pause and next/previous (report each).
4. Volume keys, app on screen, phone's zone: the slider follows each press, and the node's web page
   on a computer shows the same level within a second or two. Move the slider: Android's own volume
   bar moves with it.
5. Pick This phone and play: same as 4 (the slider is Android's media volume; there is no second
   volume).
6. A network zone you may change (not the Linn: a test speaker, or the node's null output with
   developer outputs on): app on screen, the keys change that zone's volume and Android's panel shows
   the zone's name and level. Then the app in the background (home screen), keys again: report your
   Android version and whether they still changed the zone (expected from Android 16 on; Android 17
   on your phone).
7. KA17 at its lowest volume, plugged in with the app on screen: Android may first ask "Open Odeion
   to handle FiiO KA17?" (Cancel is fine; after OK there Android gives Odeion the KA17 without asking
   again, also fine); once that is gone, it asks once whether Odeion may access it (allow). Never two
   dialogs on top of each other: report it if they are. Play on the phone's zone: the Sound panel and
   Now playing › Details say "Exclusive USB · FiiO KA17 · …"; the KA17's display shows the file's
   rate. Volume keys: the KA17's own volume (its display) moves, one step per press; it never jumps
   up. Report the steps.
8. Unplug the KA17 mid-song: the zone pauses, nothing from the speaker; Android's media volume is
   now at most a quarter. Plug it in again: with "always" ticked, no permission question; without,
   Android asks once again on this plug-in (only while the app is on screen). Either way it plays
   Exclusive USB again once allowed.
9. A phone call (or an alarm) while the phone's zone plays: it pauses and plays on after the call.
10. On problems: `adb logcat -s pubky-music`, lines with "media controls", "audio focus" and
    "Exclusive USB".
```

## USB DAC: always bit-perfect, volume on the DAC (Android)

Checked without the DAC: one behaviour whatever Android's media volume is (40 %, 0 %, unknown: never
shared because of it); Exclusive USB's start level (the level last played on that DAC, kept by its
USB ids, else -40 dB within the DAC's range, never louder than the DAC was before the take, never
from Android's media volume, which it never sets);
Android's bit-perfect start level (lowered to the last level, else 20 %, never raised, set only while
Android routes media to that DAC alone); a UAC1 DAC whose volume the app's driver can't set left to
Android's paths; a DAC without a volume of its own staying bit-perfect with a fixed volume and a
plain note;
earlier settings (a mode, the agreement to full level) reading as the one behaviour; the system's
session volume per path (crates/pm-out-local, crates/pm-usb-audio/src/volume.rs,
crates/pm-desktop/src/media.rs); the page (tools/ui-usb-dac.js, tools/ui-android-media.js).
Not checked: a real phone and the KA17, and whether the phone's Android hands its media volume to
the DAC in bit-perfect mode (Android's reference code does; a phone maker's may not).

What it does: with a USB DAC as the phone's output it always plays bit-perfect: Exclusive USB (the
app's own USB driver) when Android allows it, else Android's bit-perfect path (Android 14+), else
shared only where neither is possible, and the Output line says why. In Exclusive USB the slider,
the volume keys and Android's panel set the DAC's own volume; in Android's bit-perfect path they set
Android's media volume, which Android hands to the DAC's own volume. Nothing is forced to 100 %.

```
1. Safety first: no headphones or in-ear monitors on the KA17 for the first play of each step; a
   speaker or amplifier turned low instead. Android's media volume at 0-5 % throughout. The KA17's
   own level is set by the app in Exclusive USB (the last level or -40 dB, never louder than the
   KA17 was) and by Android in step 5, so its knob is no limit: the protection is no headphones and
   a low amplifier.
2. Plug in the KA17 with the app on screen; when Android asks whether Odeion may access it, tick
   "always" if offered and Allow (if it doesn't ask, because it was denied earlier: Settings › This
   phone › Allow Exclusive USB).
   Play a 44.1/16 file, then a 96/24 file, on This phone. Expected: the KA17's display shows 44.1,
   then 96; Now playing › Details › Output says "Exclusive USB · FiiO KA17 · 44.1 kHz / 16 bit",
   then "… 96 kHz / 24 bit"; the first sound is quiet (the KA17 at the last level or -40 dB, never
   louder than it was before; report the level its display shows at the start).
3. Move the app's slider, press the volume keys (screen on, then screen off) and use Android's
   volume panel: each moves the KA17's own volume (its display). Android's media volume stays where
   it was (check it in Android's sound settings afterwards). Report the steps you saw.
4. Unplug the KA17 mid-song: the zone pauses ("FiiO KA17 was unplugged: paused") and nothing plays
   from the phone's speaker. Plug it back (app on screen): Exclusive USB again, at the same level or
   lower, never higher, on its display. If Android asks again (no "always" ticked in step 2), allow
   it; it asks once per plug-in, never in the background.
5. Back to Android's path: clear Odeion's "Open by default" for the KA17 (Settings › Apps › Odeion ›
   Open by default › Clear defaults), replug with the app on screen, and answer Cancel / Deny to
   Android's dialogs ("Open Odeion?", then whether Odeion may access the KA17; each at most once on
   this plug-in, never two at once). Details › Output says
   "Bit-perfect · FiiO KA17 · …" (Android 14+), the KA17 shows the file's rate, and the volume keys
   move the volume. If the KA17's display level doesn't change with the keys, note it: that phone's
   Android doesn't pass the volume to the DAC: it then plays at the KA17's own level whatever the
   media volume (report it). The app lowers Android's media volume before the first song to the
   level you last played it at, or 20 % the first time, never raising it: at 0-5 % it stays there.
6. A DAC or dongle without a volume of its own, if one is at hand: it plays bit-perfect, the app has
   no volume slider for it, and Settings › This phone says "Volume: set it on the DAC" (the note:
   "This DAC has no volume of its own: set the volume on the DAC or amplifier.").
7. A USB-C audio adapter or headset that is not the KA17 (a USB-C to 3.5 mm dongle, a USB headset),
   with Odeion not running, then with it on screen: plug it in and report the dialog Android shows
   (expected, because Odeion takes USB audio devices: "Open Odeion to handle …?"; Cancel is fine)
   and whether ticking "always" makes Odeion open at every plug-in of it. With the app on screen,
   after Cancel there, Android may then ask whether Odeion may access it: only after the first
   dialog is gone, never on top of it (after OK there, no second question).
8. On problems: `adb logcat -s pubky-music`, lines with "Exclusive USB", "bit-perfect" and
   "USB audio devices changed".
```

## Squeezebox players and squeezelite (Slimproto)

Checked here with real squeezelite 2.0 in CI (ci.yml, `slimproto`):
- 16/44.1 and 24/96 came out bit-exact from the first sample, with no click.
- The position followed real time; pause was silent within a second, and play resumed.
- A player that reconnected kept its one zone and its volume.

Players connect to the node on port 3483 (set `[outputs] slimproto_port` if LMS also runs on that machine).
Groups of Slimproto players are "loosely synced" until the node starts them together.

```
1. Point the player at the node: squeezelite `-s <node>`, or a Squeezebox's server setting (it may
   also find the node by itself). It appears as a zone "<model> (<MAC>)".
2. Play an album: audio within about a second; pause, next and seek take effect at once; the
   position in the UI follows the music.
3. A 24/96 or 24/192 album: the signal path stays bit-perfect up to the player's maximum rate
   (squeezelite `-r`), and the node converts above it.
4. Restart the player during playback: the zone stays and plays again on play; the volume stays.
5. Note the model, firmware (or squeezelite version and `-o` device), and anything audible.
```

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
- **Volume, never louder:** Android applies no volume of its own in bit-perfect mode: the level is
  the DAC's (the KA17's buttons) or the amplifier's. So on Auto (the default) the app plays
  bit-perfect only while Android's media volume is at full, where nothing is lost; turned down, it
  plays shared and says "Android's volume is at N%, and bit-perfect plays at full level". Choosing
  Preferred mode Bit-perfect below full volume asks first ("Bit-perfect plays at full level; Android's
  volume (N%) no longer applies… Use bit-perfect?"); with OK it plays bit-perfect for that DAC, and
  Settings and the Sound panel keep saying it plays at full level. The
  zone's software volume (Output and DSP) still works and the signal path then says it changes the
  samples.
- **Below Android 14,** or when Android refuses, it plays shared as before (48 kHz on nearly every
  phone), and says why.
- **Unplugging mid-song pauses the zone** ("FiiO KA17 was unplugged: paused"); nothing plays on from
  the phone's speaker. Plug it in again and press Play: it carries on where it was, on the DAC.

```
1. Install the edge APK (pubky-music-android-arm64.apk), connect to your node, pick the phone's zone.
   Note the phone's model and Android version (Settings › About phone › Android version).
2. Plug in the FiiO KA17 (USB-C, no adapter). The app never asks for USB access. Within a few seconds:
   - The zone picker (the speaker button) shows under the phone's zone "FiiO KA17 · …".
   - Settings › This device › This phone: "FiiO KA17", what it takes, and Preferred mode (Auto,
     Bit-perfect, Shared). On Android 13 or older, Bit-perfect can't be picked and a note says why.
3. Turn the KA17's own volume down, then Android's media volume up to full (Auto plays bit-perfect
   only from full). With Preferred mode on Auto, play in turn a 44.1/16, a 96/24 and a 192/24 FLAC
   file. For each, report:
   - what the KA17's own display shows (expected on Android 14+: 44.1, 96, 192; below Android 14:
     48 each time);
   - what the app's Sound panel says under "Playing on" (expected: "Bit-perfect · FiiO KA17 ·
     44.1 kHz / 16 bit", and so on), and its Signal path (expected: no resample stage);
   - whether system sounds (a notification) are heard during playback (Android mutes them in
     bit-perfect mode).
   If the panel says "Shared · FiiO KA17 · <why>", report the exact words.
4. Volume: during bit-perfect playback press the phone's volume buttons down a few steps; report
   whether the level changes (it should not get louder at any point). Play the next song: on Auto it
   now plays shared (the panel says why), at Android's volume. Report anything louder than before.
5. Unplug the KA17 mid-song: the zone must pause (the bar says "FiiO KA17 was unplugged: paused"),
   and nothing must play from the phone's speaker. Also try at the end of a song, just as the next one
   (in another format) starts: it must pause too. Plug it in again: the DAC shows again; press Play:
   it carries on where it was, on the DAC (check its display). Report any sound from the speaker,
   however short.
6. With Android's volume at about half, Settings › This phone › Preferred mode: Bit-perfect. The app
   must ask first; Cancel keeps Auto. Choose it again and press OK, turn the KA17 down, play: the
   Sound panel says it plays at full level. Report whether it got louder than you expected.
7. Settings › This phone › Preferred mode: Shared, then play the 96/24 file: the display shows 48 and
   the panel says "Shared · FiiO KA17 · shared, as chosen in Settings". Back to Auto.
8. Close the app (swipe it away): a notification sound plays from the DAC again.
9. If something is off, attach `adb logcat -s pubky-music` (USB debugging on): the lines
   "USB audio devices changed", "bit-perfect player" and "no bit-perfect player" show what the app
   saw and what Android answered.
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

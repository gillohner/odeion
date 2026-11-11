# Odeion overview

How Odeion fits together, what Pubky and Pinakes add, the first steps after installing, and what
works today. To install it, start from the [README](../../README.md#get-started).

## How it works

- **The node** is the server at home. It reads your music folders, signs in to your streaming
  services and sends the music to the speakers. Everything else talks to it.
- **To control it** you use the web page (it installs as an app on phones and computers), the
  Odeion app for macOS, Windows, Linux and Android, any OpenSubsonic app (Symfonium, Feishin,
  Amperfy), or Home Assistant.
- **Speakers:** most are found on your network by themselves: UPnP and DLNA streamers and TVs
  (Linn, WiiM, Naim and others), Sonos, HEOS, Bluesound, Chromecast, AirPlay and a DAC on the
  node's own machine. Squeezebox and squeezelite players are pointed at the node, and Snapcast and
  MPD are added in its config. The Odeion app and the browser can play too. Each speaker is a zone
  with its own queue, and zones can be grouped. Full list: [Speakers](speakers.md).
- **Sources** are your files (a local disk, a USB drive, a network share, Google Drive or
  OneDrive), files on your Pubky homeserver, internet radio and podcasts, and streaming services:
  TIDAL, Qobuz, Spotify, Deezer, the Internet Archive, Audius, Jamendo, SoundCloud, Bandcamp,
  YouTube, and your own Jellyfin, Plex, Subsonic or Audiobookshelf server. What each one needs:
  [Streaming services](../STREAMING-PROVIDERS.md).
- **Pubky** is your identity and your storage. You sign in with the Pubky Ring app. Playlists,
  favourites, tags and listening history are written to your Pubky homeserver and follow you to a
  new node, and the app uses it to find and reach your node from anywhere.
- **Pinakes** is a public index of the links people share on Pubky. Odeion asks it what people on
  Pubky play and tag, which public collections hold a song, and where else a song or album can be
  found. Artist photos and texts come from Wikipedia and Wikimedia Commons, and song and album
  facts from MusicBrainz.

## Pubky

Optional, recommended. [Pubky](https://pubky.org) is an open network where your identity is a key
you hold and your data lives on a homeserver you choose. For Odeion you need:

- **The Pubky Ring app** on your phone. It holds your Pubky identity, and you approve sign-ins
  with it: the node's own sign-in, and logging in to the web page and the app.
- **A homeserver for your account**, one of these:
  - **The node's own homeserver.** The USB-stick installer sets one up by default, and with Docker
    the bundle in [`deploy/bundle/`](../../deploy/bundle/) runs one next to the node. Your
    account, your playlists and any music files you copy there stay on your own machine. On the
    first start, **Settings › My accounts** offers **Create your Pubky account on this node’s
    homeserver**.
  - **An existing homeserver**, the one your Pubky account already uses. Sign the node in with
    **Settings › My accounts › Sign in with Pubky** and approve in Pubky Ring.

With Pubky you get: logging in with Pubky Ring instead of a token, inviting other people to your
node (each with their own sign-in), playlists, favourites, tags and history kept in your account,
public tags and shared playlists, and the Odeion app reaching the node away from home without any
router setup.

Without Pubky, the node still does all the music: library, speakers, streaming services, radio,
Subsonic apps and backups. You log in with the node's API token, playlists stay on the node, and
for access away from home you use Tailscale or a Cloudflare Tunnel ([Remote access](remote-access.md)).

## Pinakes

Optional. Pinakes is the music index Odeion reads for the social side. It is public, read-only
and needs no account. By default the node uses `https://8092--main--pinakes--gil.coder.riginode.xyz`,
a development instance run by the project's maintainer, so it can be slow or away at times.

These parts come from Pinakes:

- On Home, what people on Pubky play: trending songs, tags and your friends' picks.
- The **Pubky** results in search.
- **In collections** on song, album and artist pages: public collections that hold them.
- **Tagged alike on Pubky** under Similar.
- Part of the **Also on** links (the rest come from MusicBrainz), which also tell a playlist
  where else a song can play from.
- Titles and covers for web links kept in a playlist.
- Podcast search (when Pinakes has nothing, the node asks Apple's podcast directory).

Pinakes, MusicBrainz and Wikipedia each ask every app for a way to reach whoever runs it. So the
node asks for nothing from them until it has one: an email address or a website you add on Home's
**Get started** card or in **Settings › Discover**, or, when you sign the node in to Pubky and
gave none, your Pubky profile link. That also turns on Discover. It goes only to those services.

If Pinakes doesn't answer, everything else works; the app lists it under **External resources**
below the node's status. To use another Pinakes instance, add this to the node's config file
(`/data/node.toml` inside the Docker container, `/etc/pubky-music/node.toml` on Linux) and
restart the node:

```toml
[pubky]
pinakes_base = "https://pinakes.example.org"   # without /v1
```

Names and pictures of people, comments and who tagged what come from Nexus, Pubky's public index
(`https://nexus.staging.pubky.app` by default; `nexus_base` in the same section).

## First steps after installing

The app and the web app don't run a node themselves, and neither does a squeezelite speaker
([Speakers](speakers.md)): install the node first ([Get started](../../README.md#get-started)),
then connect them to it. Umbrel isn't supported right now.

1. **Open the web page.** In a browser on the same network, go to `http://<node address>:7788`
   (for example `http://192.168.1.10:7788`; the box from the USB stick is at
   `http://odeion.local:7788`).
2. **Log in.** With Pubky Ring, once the node is signed in to Pubky (step 5). Before that, open
   **Use the node’s API token instead**, paste the token and press **Log in with the token**. The
   token is:
   - Odeion box: log in on the box's own screen as `odeion` with the password you chose, then
     run `sudo sed -n 's/^PM_TOKEN=//p' /opt/odeion/.env`.
   - Docker: the output of `docker exec pubky-music cat /data/api-token` (or `PM_TOKEN` if you set
     one; with the bundle, `PM_TOKEN` in `.env`).
   - Linux without Docker: the token the install printed, also in `/etc/pubky-music/node.toml`.

   Home's **Get started** card then lists what is left to do on this node, and only that.
3. **Add your music.** With Docker the node reads the music folder (`/music`) by itself.
   Otherwise go to **Your library › Node library** (or the Get started card), type the folder's
   path and press **Add folder**. New files show up about 15 seconds after they land.
4. **Pick a speaker.** The zone button at the right of the player bar says **This browser** at
   first. Press it and choose a speaker, then press play. All speakers the node found are in
   **Settings › Zones and outputs**. If one is missing, see [Speakers](speakers.md).
5. **Optional:**
   - Sign the node in to Pubky (see [Pubky](#pubky) above).
   - Add an email address or website in **Settings › Discover** to turn on Discover.
   - Connect streaming services under **Sources**, and internet radio under **Radio**.
   - Invite people, or start a party link for guests, in **Settings › People**.

## Away from home

Once the node is signed in to Pubky, the Odeion app reaches it from anywhere through an encrypted
peer-to-peer link. Nothing to open on your router. For a browser or Subsonic apps away from home,
use Tailscale or a Cloudflare Tunnel. Don't forward port 7788 to the internet. See
[Remote access](remote-access.md).

## Updates and backups

- **Update:** with Docker, pull the new image and recreate the container. The box from the USB
  stick installs Debian's security updates by itself; Odeion is updated with a few commands on its
  console ([guide](install-iso.md#updating-odeion)). The desktop and Android app ask when a new
  version is out.
- **Back up:** **Settings › Backup** makes one file with your library data, playlists, favourites,
  history and settings (never your music files), and can do it every day or week. With the node's
  own homeserver, `deploy/bundle/backup.sh` backs up the node and the homeserver together.

Details: [Backups and updates](backup-and-update.md).

## Where it stands

As of November 2026. The last numbered release is **0.2.2** (30 October 2026), still under the
project's old name. **Edge** is built from every change to the code.

**Live in release 0.2.2**

- Library: folder scans, tags, covers, artist pages, search, play history, lyrics.
- Playback to every speaker kind listed above, grouped zones, per-zone EQ and other sound
  settings, internet radio. Bit-perfect by default, and the signal path shows every change made to
  the sound.
- Streaming sources, the desktop and Android app, Subsonic apps, Home Assistant.
- Pubky sign-in with Pubky Ring, playlists and favourites in your account, the bundle with its own
  homeserver, remote access through the app.

**Live on edge, not in a release yet**

- The Odeion name and logo, and the web page as an installable app.
- The USB-stick installer for an Odeion box.
- Each person signs in to their own TIDAL, Qobuz and Spotify; the node's owner chooses whose
  sign-ins people play with.
- People you invite keep their own bookmarks and playlists on their own homeserver.
- TIDAL in lossless and hi-res with **Sign in with TIDAL**; Qobuz with just email and password.
- Android app: keeps playing in the background, updates itself, and plays bit-perfect to a USB DAC
  (Android 14 and later, or with its own USB driver on older phones). The USB DAC modes are still
  being tested on real phones.
- Now playing on a phone as one screen, with the mini player on the tabs.
- Quicker song identification, and reviewing its matches is optional.
- Songs are matched automatically after each scan, and the node's owner corrects a match with
  **Wrong match?** in a song's or album's ⋯ menu. An album takes the release its matched songs
  agree on. It comes with the next release.
- Copying library folders to your homeserver in the background.
- Tags in the top card of song, album and artist pages, drawn as in pubky.app.
- Usage insights for the owner, the node's part: the switch, storage and the summary (off by
  default).
- **?** in the top bar: **Report a problem…** sends the node's owner a problem, an idea or a
  question, and **My reports** shows what became of yours.

**Proposed** (being built)

- Android: the phone plays in the app itself, also away from home, with the phone's media
  controls and volume keys.
- Android: picking songs, albums and playlists to keep on the phone from their ⋯ menu, a storage
  screen, and playing them through the app's own player with no network.
- A simpler mini player and list rows on phones, with more swipes.
- Simpler sign-in pages for streaming services.
- Usage insights, the browser part: the notice, its Settings pages and the recording itself
  (until then nothing is recorded). [How it works](usage-insights.md).
- Release images under the name `ghcr.io/gillohner/odeion` (until then releases are
  `ghcr.io/gillohner/pubky-music-node` and edge is `ghcr.io/gillohner/odeion:edge`), and the
  desktop app's installed name, which is still the old one.

What changed and when: [CHANGELOG](../CHANGELOG.md).

## Downloads

| | Latest release (0.2.2) | Edge (newest code) |
| :-- | :-- | :-- |
| All files | [releases/latest](https://github.com/gillohner/odeion/releases/latest) | [releases/tag/edge](https://github.com/gillohner/odeion/releases/tag/edge) |
| macOS (universal) | [.dmg](https://github.com/gillohner/odeion/releases/latest/download/pubky-music-macos-universal.dmg) | [.dmg](https://github.com/gillohner/odeion/releases/download/edge/pubky-music-macos-universal.dmg) |
| Windows | [setup .exe](https://github.com/gillohner/odeion/releases/latest/download/pubky-music-windows-x86_64-setup.exe) | [setup .exe](https://github.com/gillohner/odeion/releases/download/edge/pubky-music-windows-x86_64-setup.exe) |
| Linux x86_64 | [.AppImage](https://github.com/gillohner/odeion/releases/latest/download/pubky-music-linux-x86_64.AppImage), [.deb](https://github.com/gillohner/odeion/releases/latest/download/pubky-music-linux-x86_64.deb) | [.AppImage](https://github.com/gillohner/odeion/releases/download/edge/pubky-music-linux-x86_64.AppImage), [.deb](https://github.com/gillohner/odeion/releases/download/edge/pubky-music-linux-x86_64.deb) |
| Linux arm64 | [.AppImage](https://github.com/gillohner/odeion/releases/latest/download/pubky-music-linux-arm64.AppImage), [.deb](https://github.com/gillohner/odeion/releases/latest/download/pubky-music-linux-arm64.deb) | [.AppImage](https://github.com/gillohner/odeion/releases/download/edge/pubky-music-linux-arm64.AppImage), [.deb](https://github.com/gillohner/odeion/releases/download/edge/pubky-music-linux-arm64.deb) |
| Android and Google TV | [arm64 .apk](https://github.com/gillohner/odeion/releases/latest/download/pubky-music-android-arm64.apk) | [arm64 .apk](https://github.com/gillohner/odeion/releases/download/edge/pubky-music-android-arm64.apk) |
| Odeion box installer | not in a release yet | [odeion-edge-amd64.iso](https://github.com/gillohner/odeion/releases/download/edge/odeion-edge-amd64.iso) |
| The node, without Docker | `pubky-music-node-linux-{x86_64,arm64,armv7}.tar.gz`, `…-windows-x86_64.zip`, `…-macos-arm64.tar.gz` | the same names under edge |
| Docker image | `ghcr.io/gillohner/pubky-music-node:latest` | `ghcr.io/gillohner/odeion:edge` |

Every release has a `SHA256SUMS` file to check downloads against.

## For developers

- Build and run from source, every config option and all deployment shapes:
  [docs/deploy.md](../deploy.md).
- HTTP and WebSocket API: [docs/api.md](../api.md). Connection and discovery:
  [docs/CONNECTION.md](../CONNECTION.md). Zones and outputs: [docs/ZONES.md](../ZONES.md).
- Design decisions: [docs/adr/](../adr/). Releases: [docs/RELEASING.md](../RELEASING.md).
  The app: [crates/pm-desktop/README.md](../../crates/pm-desktop/README.md).
- Pinakes and Odeion: [docs/PINAKES-CONTEXT.md](../PINAKES-CONTEXT.md).
- Odeion had another name before; what kept the old name and why:
  [docs/branding-compatibility.md](../branding-compatibility.md).

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release -p pm-node --features alsa   # needs libasound2-dev; builds odeiond and pm-node
```

The workspace is Rust: `pm-node` builds the node (`odeiond`, and `pm-node`, its old name), `pm-api`
the HTTP API and web UI, `pm-library` the library, `pm-zones` and `pm-audio` playback, `pm-out-*`
the speaker protocols, `pm-pubky` the Pubky and Pinakes side, and `pm-desktop` the app (not a
workspace member). Every outbound request names the node and its contact; MusicBrainz is asked at
most once a second, and Pinakes lookups are batched. No copyrighted audio is ever written under a
public `/pub/` path.

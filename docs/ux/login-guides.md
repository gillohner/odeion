# Odeion login and setup guides: inventory

[Screen evidence and reproduction commands](login-guides-evidence.md).

Every offered method has an adjacent numbered guide: prerequisites, browser versus node-host steps, expected success, failure recovery and full sign-out/reconnect. Recommended routes are displayed before advanced own-app and existing-token alternatives. The backend’s chosen, remembered and recommended authentication default remains authoritative.

Guides are data in `LOGIN_GUIDES`, with per-source placement in `LOGIN_SOURCES`. No new login form or authentication mechanism is implied by a guide. Private user credentials and unauthorized third-party app secrets are never bundled; existing-token imports are restricted to your own authorized sign-in. Pasted credentials are masked with deliberate Show/Hide controls.

## Every source and the guides on its page (16 sources)

| Source | Guides on its page |
| :-- | :-- |
| `source:archive` | `source:open` |
| `source:audius` | `source:open` |
| `source:deezer` | `source:open` |
| `source:radio` | `source:open` |
| `source:podcasts` | `source:open`, `podcasts:gpodder`, `podcasts:nextcloud` |
| `source:jamendo` | `source:open`, `jamendo:signin` |
| `source:bandcamp` | `source:open`, `bandcamp:identity` |
| `source:tidal` | `tidal:app`, `tidal:import`, `tidal:device`, `tidal:paste` |
| `source:qobuz` | `qobuz:own`, `qobuz:import`, `qobuz:paste` |
| `source:spotify` | `spotify:app`, `spotify:librespot`, `spotify:librespot-file`, `spotify:soloist` |
| `source:youtube` | `youtube:key`, `youtube:cookies`, `youtube:ytdlp` |
| `source:soundcloud` | `soundcloud:client`, `soundcloud:token` |
| `source:subsonic` | `subsonic:server` |
| `source:jellyfin` | `jellyfin:server` |
| `source:plex` | `plex:pin`, `plex:server` |
| `source:audiobookshelf` | `audiobookshelf:password`, `audiobookshelf:server` |

## Every guide (48 guides)

| Key | Title | Level | Where it happens |
| :-- | :-- | :-- | :-- |
| `node:ring` | Log in with Pubky Ring | recommended | both |
| `node:token` | Log in with the node’s API token | advanced | server |
| `tidal:app` | Sign in at TIDAL | recommended | browser |
| `tidal:device` | Full tracks with a code (unofficial) | advanced | both |
| `tidal:import` | Use a sign-in already on this machine | advanced | server |
| `tidal:paste` | Paste your own token | advanced | browser |
| `qobuz:own` | Qobuz with your own app and account | only | browser |
| `qobuz:import` | Use a config already on this machine | advanced | server |
| `qobuz:paste` | Paste a config | advanced | browser |
| `spotify:app` | Connect your own Spotify app | advanced | browser |
| `spotify:librespot` | Pair with Spotify Connect (librespot) | recommended | both |
| `spotify:librespot-file` | librespot with your own credentials.json | advanced | server |
| `youtube:key` | Search with your YouTube Data API key | recommended | browser |
| `youtube:cookies` | yt-dlp with your own cookies file | advanced | server |
| `lastfm:own` | Connect Last.fm with your own API account | recommended | browser |
| `lastfm:import` | Use your scrobbler’s sign-in on this machine | advanced | server |
| `lastfm:paste` | Paste a key, secret and session key | advanced | browser |
| `subsonic:server` | Connect your Subsonic-compatible server | only | browser |
| `jellyfin:server` | Sign in to your Jellyfin | only | browser |
| `jamendo:signin` | Sign in to Jamendo (optional) | only | browser |
| `listenbrainz:token` | Connect ListenBrainz | only | browser |
| `webdav:share` | Add a WebDAV share | only | browser |
| `drive:google` | Sign in to Google Drive | only | browser |
| `drive:onedrive` | Sign in to OneDrive | only | browser |
| `app:spotify` | Spotify: your own developer app | only | browser |
| `app:tidal` | TIDAL: your own developer app | advanced | browser |
| `app:tidal-full` | TIDAL full tracks: a device client | advanced | browser |
| `app:qobuz` | Qobuz: an app Qobuz issued to you | advanced | browser |
| `app:soundcloud` | SoundCloud: your approved app | advanced | browser |
| `app:youtube` | YouTube Data: an API key | advanced | browser |
| `app:lastfm` | Last.fm: an API account | advanced | browser |
| `app:jamendo` | Jamendo: your own app with its secret | advanced | browser |
| `app:acoustid` | AcoustID: an application key | advanced | browser |
| `app:google-drive` | Google Drive: your own OAuth client | advanced | browser |
| `app:onedrive` | OneDrive: your own app registration | advanced | browser |
| `source:open` | Use it without signing in | recommended | browser |
| `soundcloud:client` | SoundCloud: enter a client ID | only | browser |
| `soundcloud:token` | Your likes and playlists: your own existing OAuth token | advanced | browser |
| `bandcamp:identity` | Your purchases: your own Bandcamp sign-in | advanced | browser |
| `spotify:soloist` | Play on this machine with Spotify Soloist | only | both |
| `youtube:ytdlp` | YouTube on your speakers: get yt-dlp | only | server |
| `acoustid:key` | Match by sound: an AcoustID key | only | browser |
| `plex:pin` | Sign in with Plex using a PIN | recommended | both |
| `plex:server` | connect Plex with your existing token | advanced | server |
| `audiobookshelf:password` | Sign in to Audiobookshelf with your account | recommended | server |
| `audiobookshelf:server` | Audiobookshelf with your API key | advanced | server |
| `podcasts:gpodder` | Sync subscriptions with gPodder | advanced | server |
| `podcasts:nextcloud` | Sync subscriptions with Nextcloud | advanced | server |

## No account setup needed

- `app:fanart`: Built in (artist images); nothing to set up.
- `app:audiodb`: Built in, or its public test key; nothing to set up.

## Coverage and account contracts

`node tools/login-guides-check.js` reads every streaming source, sign-in Way, Service app, drive provider, standalone UI credential control, Plex/Audiobookshelf account routes and credential-schema fields, and the gPodder server Kind registry. Every route for these account providers is explicitly classified as setup or a non-authenticated operation; unknown routes or fields fail instead of silently inheriting an unrelated guide. Each method must have a guide, inventory row and render hook on its own page.

`--self-test` mutates a source, render hook, sign-in Way, Service app, Plex auth route, Audiobookshelf credential field and podcast server Kind in memory; every mutation must be rejected. `python3 -I tools/login-guides-recipes-check.py .` executes the six local account recipes against synthetic transport only, testing quoted/backslash-containing credentials, redirect refusal, masked prompts and redacted errors. No live sign-in is inferred.

## Method details

- **Node token:** the documented Linux service reads `[api] token` from `/etc/pubky-music/node.toml`; a generated token is in the data directory. Ordinary Docker uses `/data/api-token` unless configured; the bundle uses `PM_TOKEN` in `.env`. Read locally, never in chat.
- **Google Drive / OneDrive:** secret-based Google OAuth returns to the node’s `/drives/google/callback`; OneDrive uses a public Mobile and desktop registration, not a SPA registration.
- **librespot:** pair in Spotify Connect first; credentials.json is only an existing own sign-in in the supported librespot format. Native and Docker installation are separate disclosures. The container name is discovered, not assumed; host/container architecture and in-container execution are checked. Its data/bin directory remains service-user writable. Rustls avoids an external TLS dependency; no claim about transitive OpenSSL packages is needed. Reload detection after installing into data/bin; do not restart a playing node unnecessarily.
- **Plex:** PIN authorization and an existing own-token connection are distinct methods. PIN polling uses the returned id; the node selects an owned server first and then a responding connection. The private authorization link is shown only to the person running it; account and administrator tokens are not printed.
- **Audiobookshelf:** own username/password sign-in and an existing API key are separate methods. The account must be authorized to create the key the node keeps.
- **Podcast sync:** gPodder API v2 and Nextcloud gpoddersync are distinct server kinds. Nextcloud uses a dedicated app password.
- **API-only methods:** Plex, Audiobookshelf and podcast sync have no forms here yet. Their advanced interactive Python recipes use hidden prompts, JSON serialization, in-memory Authorization headers and redirect refusal. No secret becomes a child-process argument; raw upstream bodies are not printed. Run the same recipe with `disconnect` to sign out, then `connect` to sign in again.

## Official pages behind the guides, and Advanced: use your own app (checked 2026-11-02)

HTTP status from `curl -L` on 2026-11-02.

| Status | URL |
| :-- | :-- |
| 200 | https://developer.spotify.com/dashboard |
| 200 | https://developer.spotify.com/documentation/web-api/concepts/quota-modes (five users in development mode, owner needs Premium) |
| 200 | https://developer.spotify.com/documentation/web-api/concepts/redirect_uri (HTTPS, or loopback 127.0.0.1 / [::1]) |
| 200 | https://github.com/librespot-org/librespot (Premium only; cache folder mode 700) |
| 200 | https://raw.githubusercontent.com/librespot-org/librespot/dev/COMPILING.md |
| 200 | https://developer.tidal.com |
| 200 | https://developer.tidal.com/documentation/api-sdk/api-sdk-authorization |
| 403 | https://link.tidal.com (refuses non-browser requests; named in steps, not linked) |
| 200 | https://www.last.fm/api/account/create |
| 200 | https://www.last.fm/api/authspec |
| 200 | https://console.cloud.google.com/apis/library/youtube.googleapis.com |
| 200 | https://developers.google.com/youtube/v3/getting-started |
| 200 | https://github.com/yt-dlp/yt-dlp |
| 200 | https://github.com/yt-dlp/yt-dlp/wiki/FAQ |
| 200 | https://developers.google.com/identity/protocols/oauth2/web-server |
| 200 | https://learn.microsoft.com/en-us/entra/identity-platform/quickstart-register-app |
| 200 | https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-auth-code-flow (SPA redirects need a browser Origin) |
| 200 | https://devportal.jamendo.com |
| 200 | https://listenbrainz.org/settings/ |
| 200 | https://acoustid.org/new-application |
| 200 | https://pubky.org |
| 200 | https://github.com/pubky/pubky-ring |
| 200 | https://www.qobuz.com |
| 200 | https://jellyfin.org/docs/ |
| 200 | https://www.navidrome.org/docs/ |
| 200 | https://www.subsonic.org/pages/api.jsp |
| 200 | https://developers.soundcloud.com |
| 200 | https://bandcamp.com |
| 200 | https://support.plex.tv/articles/204059436-finding-an-authentication-token-x-plex-token/ |
| 200 | https://www.audiobookshelf.org/docs |
| 200 | https://gpodder.net |
| 200 | https://apps.nextcloud.com/apps/gpoddersync |
| 200 | https://docs.nextcloud.com/server/latest/user_manual/en/files/access_webdav.html |

## Advanced: use your own app — current quota and Soloist references

Checked against primary documentation on 2026-11-02 (HTTP 200):

- https://developers.google.com/youtube/v3/determine_quota_cost — default 100 search calls per day, rather than the older per-search unit arithmetic.
- https://developer.spotify.com/dashboard/soloist — each person generates their own key.
- https://developer.spotify.com/documentation/soloist/tutorials/getting-started — supported downloads and setup.

The librespot instructions discover the actual Docker container name, work with the ordinary Docker install and the bundle, keep the persistent bin directory writable by the node, and reload detection without an unnecessary service restart. The build recipes have been checked against the runtime and upstream documentation, not executed against a real installation.

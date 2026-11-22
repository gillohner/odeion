# ADR 0018: Phone mode. One client, two backends

Status: **accepted** (2026-11-21, owner; PLAN-phone-standalone-1121). Amends [ADR 0012](0012-one-client-tauri-shell.md).

## Context

ADR 0012 made the app the node's web page in a Tauri shell: "the app shows nothing useful without a node.
That is the product." People who want to try Odeion on a phone, or who have no machine to run a node on,
had nothing to use. The owner decided that the Android app also works with no node, with these limits:

- Pubky sign-in is required, so what the person does on the phone is kept on their homeserver and a node
  they add later has it.
- The only sources are TIDAL and Qobuz, with the person's own accounts.
- Songs added to the phone are kept in the app's own storage, which is enough.

A second UI for the phone would bring back the double work that ADR 0012 ended.

## Decision

**One client, two backends.** The page is still the node's web UI (`crates/pm-api/static/`). In phone
mode the app serves that page from a backend inside the app (`pm-offline::phone`) instead of loading it
from a node.

- **The shared code.** `crates/pm-sources` holds what both backends run: TIDAL full tracks (sign-in,
  renewal, playback info), the TIDAL v1 catalogue, Qobuz (app discovery, sign-in, catalogue, stream URLs),
  segmented and DASH streams, the search shapes and page ids, and the vault form of a sign-in. pm-api
  uses it through thin wrappers (its database implements `SessionStore` and `qobuz::AppCache`). The phone
  uses it through a JSON file in the app's storage. The official TIDAL API sign-in stays in the node only,
  because it redirects back to the node's own address.
- **The page asks, it doesn't guess.** `/v1/info` carries `mode: "phone"` and a `capabilities` list
  (`search`, `items`, `sources`, `bookmarks`, `listens`, `songs`, `add-songs`, `homeserver-files`). The
  page shows only the views those capabilities cover, and a node keeps all of them. A capability gets
  added when the phone backend has the routes for it.
- **Same routes, same shapes.** The phone answers the node's routes for what it covers: search (NDJSON),
  service item and artist pages, `/v1/sources`, the TIDAL and Qobuz sign-ins, `/v1/favourites`,
  `/v1/library/history`, the upload routes (`/v1/library/uploads`), and the app's offline list (`/v1/offline`, the shape the
  app's downloads page reads).
  The one route only the phone has is `/v1/phone/storage`.
- **Locked to the app.** The backend listens on 127.0.0.1, on the port it had the last time for this person (so the page keeps
  its origin and what it keeps in the browser; another if that one is taken), and checks Host and Origin. It
  also requires a 256-bit key made at each start on every `/v1` request (Bearer or `?token=`, compared in
  constant time). Other apps on the phone can reach 127.0.0.1, and this backend holds streaming sign-ins.
  The app gives the key to the page through `localStorage`, only on the backend's origin.
- **The person's data on their homeserver.** Ring grants phone mode `PHONE_CAPABILITIES`. The phone is a
  device in the person's sources vault (ADR 0017, VAULT.md), named "Odeion on this phone" with device
  `android`. It writes sign-ins in the node's own form, so nodes take them. It takes a node's copy only
  when it was made with the app the phone uses. Where the vault holds the phone's TIDAL sign-in, it
  renews it under the blob's lock or lease (`Vault::refresh`, each homeserver step within a time limit),
  one renewal at a time, and never beside a renewal still running there or after a vault failure that
  leaves open whether it is being renewed elsewhere (the homeserver didn't answer, or TIDAL didn't under
  the lock; the current session plays meanwhile while it still works; an expired one waits up to 30 s);
  offline, it renews on the phone and the next sync writes
  the result. It follows "Sign out everywhere". It writes Bookmarks as social v1 bookmarks,
  listens as music v1 listen posts where the person's listen history setting puts them (off, private or
  public, read from the node's settings copy), and prefs to `prefs.json` (newest `updated_at` wins), as
  the node writes them. The streaming quality chosen on the phone goes into `prefs.json`.
- **Added songs.** Picked in the system's file picker, sent with the node's upload protocol and kept in the
  app's storage. Their tags are read with `pm-library`. They are named as the node names files
  (`pm_appdata::files`). Their canonical URL comes from their tags (recording MBID, else ISRC), as on a
  node: a bookmark or a listen of one points at it. A setting copies them to
  `/priv/pubky.music/files/`, where a node finds them, on a task of its own, found there by content
  hash, and counted as copied once the homeserver holds all of it (a short copy is sent again). A
  copy there whose size the homeserver doesn't give counts as there, as on a node, and one whose
  size it can't say now is never sent over or removed. The setting is on by default only when the
  person's homeserver is the one installed with their node (`node.json`'s `bundled_homeserver`).

## Consequences

- The amendment to ADR 0012: the Android app shows something useful without a node, namely TIDAL and
  Qobuz, songs added to the phone, and the person's Pubky data. On desktops the app still needs a node.
- What the phone covers grows one capability at a time. Collections, reviews, tags and the library views
  are not phone capabilities yet, and the page hides them there.
- The phone holds streaming sign-ins. They sit in a file that only the app can read (Android's Auto
  Backup may still copy the app's storage: a follow-up), and the key keeps
  other apps on the phone out. A sign-ins file or vault sync record that can't be read is never taken
  for an empty one (that would be a sign-out the vault sync carries out, or a first sync). The record
  stays as it is, and the phone doesn't sync with the vault until it reads. A sign-ins file that reads
  but isn't the store's JSON is kept whole under a dated name beside it
  (`sign-ins.json.<date>.unreadable`, readable by the app alone; the app never deletes it, so it keeps
  the tokens it held until the app's data is cleared), and phone mode starts without what couldn't
  be read (a value that doesn't read, such as a service's sign-in that isn't its JSON: only that
  one). Settings lost with it are noted, and copying added songs to the homeserver is off until
  the person chooses again (an "off" never turns into the default). A file that can't be read at
  all (a read error) stops phone mode's start as before.
  Those services are signed out on the phone and set aside: the vault sync never removes or writes
  them until the person signs in again (a copy in the vault the phone can use is taken, which is a
  sign-in again). The connect page says so before phone mode opens, until the person presses
  Continue. The phone's other files are kept the same way: the person's waiting writes (`data.json`), the record of
  songs copied (`files.json`) and the song list (`songs.json`) are left as they are when they can't be
  read, and nothing is saved over them (phone mode still starts). The page says they can't be read,
  never "none".
- The vault sync records each service's outcome before it looks at the next one, and a record it
  can't save ends the sync. A take from the vault or a write to it is noted as under way first: until
  it is recorded, a TIDAL renewal asks the vault whether it holds the phone's refresh token rather than
  renew beside it, and the next sync reads the vault's copy as if the step had been recorded (a new
  sign-in there is taken, one gone is signed out here), unless a write's copy is still the one it
  replaced, so it never landed: then it is written again. A write cut off where the vault had no copy
  before, with none there at the next sync, is written again: the phone can't tell a write that never
  landed from one removed since. A sign-out on the phone keeps the sign-in's id, so a sign-out of the
  very sign-in the vault has goes to the vault even when the sync stopped before recording it; the
  id is let go once neither side has the sign-in. "Sign out everywhere" is recorded only by a sync
  that read every service's copy. A TIDAL renewal request is given 40 s, well within the 60 s the
  vault waits for it under its lock, and a yes from TIDAL whose answer doesn't arrive whole is not
  taken for a refusal. A renewal written over a vault copy another device rewrote meanwhile keeps
  what the phone made with the new tokens, and once TIDAL renewed for the phone it never renews
  the session before again on its own. A renewal on the phone runs in the vault's turn, so a vault
  sync never writes the session being renewed.
- One backend per person on the phone: leaving phone mode ends its sync and waits, and opening it again
  while a step of the old one still runs gives back that one, so two never write the sign-ins over each
  other's. The app opens phone mode one open at a time, so it never starts two servers.
- A TIDAL song is fetched once into the app's cache (the newest 3 are kept), for the quality and the
  TIDAL account it was fetched with. It is sent to the player as
  it arrives when the fetch takes longer than 12 s, and a fetch that nobody reads for 5 s stops.
- The built-in TIDAL clients ship inside the APK, masked as they are in the node's binaries.
- CI starts phone mode on an emulator with no node and plays a song from a stand-in TIDAL
  (`android.yml`, job `phone-mode`).

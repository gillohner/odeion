# Who can do what on a node

One place decides: `crates/pm-api/src/access.rs`. Every `/v1` route behind the sign-in check names,
per method, the one capability it needs (`ROUTES`), every caller has a role, and `decide` answers. A
route that names no capability is the owner's alone, and the tests fail until it names one.

## Who is asking

| Caller | How the node knows them | Role |
| :-- | :-- | :-- |
| The owner's browser | logged in with Pubky Ring as the node's own identity | owner |
| The owner's app (desktop, Android) | the same, as a separate sign-in: Log out in a browser does not end it | owner |
| Anything with the API token | `PM_TOKEN`, `[api] token` or the token the node made (`<data>/api-token`): scripts, Home Assistant, a Subsonic app (as its password) | owner |
| A node without an API token (it then listens on its own machine only) | only requests from the node's own machine, addressed to `127.0.0.1` or `localhost` | owner |
| A person you gave music access | logged in with Pubky Ring as someone in People with music on | listener |
| A party guest | joined with a party's link | guest |

Remote access (iroh) and a tunnel carry the same sign-ins; they add no role.

The UPnP renderer (`/upnp/zone/{zone}/…`, on unless `[renderer] enabled = false`) answers devices on
the home network without a sign-in: play, pause, stop, seek, next, previous, volume, mute, and
replacing or adding to the queue. It never answers through remote access, nor through a tunnel or
proxy that marks what it forwards (a forwarding header).

## What each role can do

| Capability | What it covers | Owner | Listener | Guest |
| :-- | :-- | :-: | :-: | :-: |
| `artwork` | album covers | yes | yes | yes |
| `party_guest` | the party they joined: its search and its queue | yes | | yes |
| `session` | who am I, log out, the node's name and health, live updates | yes | yes | |
| `library` | browse, search and play the shared library and the public catalogues | yes | yes | |
| `zone_list` | the list of zones (each caller sees the ones they may use) | yes | yes | |
| `zone_use` | a zone's queue, transport, volume, sleep timer | yes | on the zones the owner opened | |
| `own` | their own bookmarks, collections, preferences, devices, presence, problem reports, sign-ins on this node | yes | yes | |
| `own_play` | play with their own streaming sign-ins | yes | yes | |
| `own_sign_in_state` | see their own streaming sign-ins, sign out of them | yes | yes | |
| `own_sign_in` | sign in to their own TIDAL, Qobuz or Spotify | yes | unless the node plays everyone through its own sign-ins | |
| `owner_music` | the owner's tags, reviews, history, smart collections, Home, radio stations, podcasts, follows, imports | yes | | |
| `zone_manage` | add, remove and tune zones and speakers | yes | | |
| `library_manage` | folders, scans, uploads, edits, hiding and deleting songs, identification, homeserver files | yes | | |
| `settings` | the node's settings and service apps | yes | | |
| `service_logins` | the node's own sign-ins at services and media servers | yes | | |
| `people` | invites, music access, sign-ins of people, the zones open to them | yes | | |
| `tokens` | the API token | yes | | |
| `vault` | the sources vault: its devices and keys | yes | | |
| `backup` | back up and restore | yes | | |
| `identity` | the node's Pubky identity and its homeserver | yes | | |
| `remote_access` | the tunnel | yes | | |
| `party` | start and end parties | yes | | |
| `diagnostics` | usage insights across the node, the speaker discovery report | yes | | |

`GET /v1/me` answers the caller's capabilities as `can`. The owner's list there leaves out
`party_guest`: the guest routes answer a guest's own sign-in only.

`own` also covers a person's play order, their usage choice, their own homeserver link and whose
sign-ins they play with. `owner_music` also covers browsing a streaming service with the node's
sign-ins.

The owner opens zones to the people they added in Settings › People & access: every zone (so it is
until a choice is made), only some, or none (they then play in their own browser or app only). The
choice is the same for everyone. While the stored choice can't be read, no zone is open to them.

A song plays with the sign-in of whoever queued it. A listener's songs use their own TIDAL, Qobuz
or Spotify sign-in, or the node's where the owner chose that in Settings › People & access; never
another listener's. What they read of the library shows file names, not the owner's folders.

## Sign-ins

- A browser or app that logs in with Pubky Ring gets a sign-in of its own. The node keeps only a
  hash of it, with what the browser or app said it is, when it signed in, the day it was last used and
  whether it came from outside the node's own network. The node keeps at most 20 browsers and 50
  apps for the owner and 10 sign-ins for each person; one more ends the oldest.
- `GET /v1/me/sign-ins` lists the caller's own; `DELETE /v1/me/sign-ins/{id}` ends one and
  `DELETE /v1/me/sign-ins` all but the one asking. The owner ends a person's with
  `DELETE /v1/people/{key}/sign-ins[/{id}]` (Settings › People & access).
- Switching a person's music off pauses them: their sign-ins are refused until it is switched back on
  (a browser then goes back to Log in), and the node forgets their devices and what those keep for
  offline. Removing a person ends their sign-ins and deletes what the node keeps for them; someone
  with an account on the node's homeserver stays listed, without music.
- Make a new token (when the node made its own API token) replaces the API token and ends every
  sign-in of the owner's. A token set in the config is changed there; the owner's sign-ins then end
  at the next start.
- The owner's sign-ins are never part of a backup, and a restore keeps the ones the node has.

## Without a sign-in

These routes sit outside the sign-in check (`OPEN` in `access.rs`; a test compares the list with
the routes the node serves). Each hands out nothing private, or checks a key or code of its own.

| Route | Why it is open |
| :-- | :-- |
| `/v1/auth/pubky/start`, `/v1/auth/pubky/poll`, `/v1/auth/pubky/owner/avatar` | logging in with Pubky Ring: this is how a sign-in is got |
| `/v1/party/join` | joining a party, with the code in its link |
| `/tidal/callback`, `/spotify/callback`, `/lastfm/callback`, `/jamendo/callback`, `/drives/{provider}/callback` | where a service sends the browser back after a sign-in someone started |
| `/v1/outputs/pull/{name}/{file}`, `/v1/art/{id}` | a stream or a cover for a speaker or TV, with that stream's or cover's own key |
| `/v1/upnp/events/{token}` | a speaker's volume events, from the home network only |
| `/upnp/node.xml` | discovery: the node's name, id and version |
| `/mb/ws/2/{*path}` | the MusicBrainz caching proxy: public data only, one upstream request a second |

The web page's own pages and files (`crates/pm-web`, a party's link `/party/{code}` among them)
and the UPnP renderer (above) need no sign-in either. A Subsonic app signs
in with the API token as its password (`/rest`). A node without an API token answers only its own
machine.

## Adding a route

Add the route in `v1_routes()` (`crates/pm-api/src/lib.rs`) and one line per method in
`access::ROUTES` under the capability it needs. `cargo test -p pm-api access::` fails for a route
or method without a line, and for a line without a route. A route that must work without a sign-in
goes into `open_routes()` and `access::OPEN`, with why. A handler that answers differently per
person asks `caller.can(Capability::…)`; one that takes a zone from the body asks
`accounts::may_use_zone`.

This page changes with them: a new capability needs its row in the table above, and an open route
its path in the list, or `access::tests::the_access_doc_matches_the_roles` fails.

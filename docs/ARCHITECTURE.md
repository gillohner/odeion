# Architecture: what lives where

A map of the workspace for people who change the code. What the node does is in `README.md` and
`docs/STATUS.md`; why things are the way they are is in `docs/adr/`.

## The crates

Dependencies point one way, from the top of this list down. A crate never depends on one below it.

| crate | what it is |
|---|---|
| `pm-model` | Shared types: audio formats, capabilities, the signal path, music identity, library entities. Depends on no other crate here. |
| `pm-zones` | Zones, grouping, the queue and transport, and the `Output`/`AudioSink` traits every output implements. |
| `pm-audio` | Decoding, DSD, the DSP pipeline, probing. |
| `pm-library` | Scanning, tags, matching and the SQLite catalogue. |
| `pm-stream` | The wire protocol between a node and an endpoint. |
| `pm-out-*` | One crate per kind of output: `local` (ALSA, and cpal for the apps), `net` (endpoints), `upnp`, `cast`, `airplay`, `bluos`, `mpd`, `slimproto`, `snapcast`. Each needs only `pm-model` and `pm-zones` (`local` also `pm-usb-audio`, `net` also `pm-stream`). |
| `pm-renderer`, `pm-discovery`, `pm-remote` | Zones as UPnP renderers; finding nodes on the LAN; the iroh tunnel for remote access. No dependency on other crates here. |
| `pm-appdata`, `pm-pubky`, `pubky-music-specs` | Odeion's own data on a homeserver and the sources vault; Pubky, Pinakes and Nexus clients; the music kinds on the social specs. |
| `pm-api` | The node's HTTP and WebSocket API (`/v1`), and today most features behind it: accounts, search, item pages, identification, copies to the homeserver, every streaming service. By far the largest crate. |
| `pm-web` | The web UI: its files (`static/`) and the routes that serve them. Depends on no other crate here. |
| `pm-node` | The daemon (`odeiond`): reads the configuration, builds the outputs, and serves `pm-api` with `pm-web`'s pages next to it. |
| `pm-offline`, `pm-app-player`, `pm-endpoint` | Parts of the apps: what a device keeps offline and its local server; the app's own player; the daemon that lends a machine's outputs to a node. |
| `pm-desktop` | The app (Tauri: desktop and Android). Its own workspace with its own `Cargo.lock`, because it needs WebKitGTK on Linux; it is built by its own CI jobs. |
| `pm-client` | A Rust client for `/v1`, used by its own tests of the tunnel. |

## The web UI

- The same page runs in the browser, the installed web app and the apps (`window.pmDesktop`).
- Its files are in `crates/pm-web/static/` and are compiled into `pm-web`. A change to the UI
  rebuilds `pm-web` and links the node again; it does not rebuild `pm-api`.
- `pm-api` does not depend on `pm-web`. `pm_api::router(state)` is the API alone;
  `pm-node` calls `router_with_pages(state, pm_web::router())` for its own listener and for the
  remote-access tunnel's. `pm-api` uses `pm-web` in tests only.
- The files have fixed names in four places, which have to agree: the routes in
  `crates/pm-web/src/lib.rs`, the two lists in `static/sw.js` (checked by `tools/pwa-check.js`),
  and what the apps keep for offline use (`pm_offline::sync::UI_FILES`, checked against the page
  by `pm-web`'s tests).

## Tests and CI

- A test whose result depends on the UI's files goes in `pm-web` (`src/tests.rs`), or in
  `crates/pm-api/tests/pages.rs` when it compares the page with a list of the API (the views a
  problem report may name, the usage schema). `pm-api`'s unit tests do not read those files.
- That is what lets CI skip the jobs that only build and test Rust for a pull request that
  changes nothing but `crates/pm-web/static`, `tools/ui-*.js` and docs (`changes` in
  `.github/workflows/ci.yml`); the `pages` job runs the tests above instead. Main runs everything.
- A page checked against fixtures is a script `tools/ui-<name>.js`; `tools/ui-fixture.js` serves
  the UI's files from the source tree, so these need no node.

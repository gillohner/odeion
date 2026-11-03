# Odeion local diagnostics

Local diagnostics help the node's owner find slow pages, failed requests and pages that break in
their own use of Odeion. Use them to decide which improvements to look into, or to attach
something concrete to a bug report. They show what happened, not why. They can't say what you
were trying to do, and they change nothing in the app by themselves.

Everything stays on your node. Nothing is uploaded or sent anywhere unless you download the file
and share it yourself.

## Before you start

- The node needs an API token (it makes one itself unless `[api] token` or `PM_TOKEN` is set).
  A node running without a token treats every direct request as the owner, so diagnostics are
  switched off for good there.
- Sign in as the owner, with the API token or your own Pubky Ring sign-in. Listeners and party
  guests can't see or use diagnostics.

## Start and stop

1. Open **Settings › About and updates › Diagnostics** and read what Start records.
2. Turn on **Allow diagnostics on this node**. This is permission for this node, not consent for
   any browser: turning it on records nothing.
3. In each of your browsers you want to include, choose **Start diagnostics for this session**.
   Other browsers never start by themselves.
4. While a browser records, a bar at the top of the page says **Recording diagnostics in this
   browser** and has **Stop**. **Stop diagnostics** in Settings does the same.

Turning **Allow diagnostics** off refuses new events at once and ends every browser's session. A
browser that is still open notices at its next send or status check, within about 30 seconds.

When it ends by itself:

- The node switch starts off whenever the node starts, and turns itself off after 24 hours.
- A browser's session lasts at most 60 minutes. It ends early when you reload or leave the page,
  log out, sign in as someone else, or make a new token. The browser keeps its own clock for the
  60 minutes, so it stops recording on time even when it can't reach the node.
- Turning the switch off and on again never restarts an old session. The page sends the switch
  state it showed when you pressed Start (or turned the switch on). If someone turned the switch
  off and on elsewhere since then, the node refuses the request and you need to press Start again.

## What is recorded

Only pages you open yourself are recorded: from the menu, a link, a search, or Back and Forward.
A page that redraws because something changed on the node is not recorded. That covers someone
else playing on a zone, a library scan, or a party guest adding a song.

| What | What is stored |
| :-- | :-- |
| Navigation | The kind of page, from a fixed list (Albums, Search, Settings, an item page…). Never what is on it or in its address |
| Page loading | The kind of page, how it ended (fine, refused, a 4xx or 5xx answer, no answer, broken) and a coarse duration (under 0.1 s, 0.3 s, 1 s, 3 s, 10 s, or longer) |
| Reads | The reads a page makes while it loads, matched against a fixed list of routes. Each is stored as an area (library, playlists, zones…), how it ended and a coarse duration. Never its address, query or body. Requests not on the list, and changes you make by clicking, are not recorded |
| Script errors | The class of an error that broke a page while it was drawn (type, reference, range or syntax error, or other). Never its message or stack. Errors elsewhere are not recorded |
| Session | A short-lived pseudonym, and whether the browser is a phone, tablet or desktop |
| Bundle | The schema version, the app version, the node's own timestamps in seconds, the limits, counts per category, and how many events were dropped |

Never recorded: anything you type or search, titles, names, addresses, URLs, Pubky keys, IP
addresses, user ids, cookies, tokens or other credentials, request or response bodies, the page's
HTML, screenshots, audio or keystrokes. There is no general click listener, screen recording,
console or log capture, or third-party service.

Not covered yet: playback and copy jobs, and anything listeners do. Listeners have no way to
consent, so turning diagnostics on is never consent to record another person using the node.

## Storage, limits and clearing

Events live in a bounded ring in the node's memory, not in a database or log file. A restart
clears them all and turns diagnostics off.

- **Size.** At most 50,000 events and 50 MiB, counted as they would be exported. When either cap
  is reached, the oldest events go first.
- **Age.** Events can be used for 72 hours. The node checks once a minute, so an event is removed
  from memory within about a minute after it turns 72 hours old. It goes sooner if diagnostics are
  used at that moment.
- **Rates.** At most 32 browser sessions at once. Each session may send 120 events a minute, and
  the node takes 2,000 a minute in all. Each batch holds at most 32 events or 16 KiB.
- **The browser's side.** Each browser keeps at most 64 events waiting and sends one batch at a
  time. A send that gets no answer within 8 seconds is given up and not repeated.

Events can be dropped at any of these limits, or when the node can't be reached. The bundle
counts drops in three ways: pushed out by the caps, refused for the rates, and dropped by the
browsers. The node keeps each count for 72 hours from when it received it, in hourly steps (so up
to an hour less). A browser reports its own drops with its next batch, which can be up to an hour
after the drops happened, so such a count can outlast the events it is about by up to about an
hour. The file is not a complete record of your use.

**Turning diagnostics off** stops new events but keeps the ones already recorded. **Clear**
deletes them, and the drop counts, at once. If a browser is still recording, new events arrive
after Clear: stop it first if you want the ring to stay empty.

## Download and share

**Download** saves a JSON file (`odeion-diagnostics.json`). Nothing is sent anywhere automatically.

- **Sessions.** In the file, sessions are named `s1`, `s2`… afresh for each download. Session ids,
  tokens, keys and the node's logs are not in it.
- **Totals.** Each kind of page and each read area has totals for how loads and reads ended, so
  frequently slow or failing parts show without anything about your music or searches.
- **Before you share it.** Check the file. Copies you download are outside the node's limits:
  delete them yourself when you no longer need them.
- **The node's own log** is separate and is not covered by this. It can contain addresses, names
  and error text, so don't attach it on the assumption that it is cleaned the same way.

## How the node protects it

- **Checked on every request.** Every diagnostics request needs the owner's sign-in, and the node
  checks the credential itself. If the request has a body, it checks again once the body arrives.
- **A sign-in that ends meanwhile.** Log out, a new token or another identity can end the sign-in
  while a request is still arriving. That request then changes nothing. The sessions started with
  that sign-in end at once. When the node takes on a Pubky identity (another one, the same one
  again, or none), every browser's session ends, even if saving the sign-in change fails.
- **Fixed answers.** Requests outside the fixed schema are refused whole, with a fixed answer that
  never repeats what was sent, and they are not written to the log.

## For developers

The routes are in [the API reference](../api.md) (`/v1/node/diagnostics`). The node side is
`crates/pm-api/src/diagnostics.rs` with its tests in `crates/pm-api/tests/diagnostics.rs`.
`tools/ui-diagnostics.js` runs the real web app against a made-up origin with a fake node and
checks the browser side. That covers consent, Stop and Off, stale requests, other people's
activity, expiry offline, a stalled node, phone, tablet and desktop layouts, and listeners. It
starts no node:

```sh
npm install playwright@1.49.1 && npx playwright install chromium
node /path/to/tools/ui-diagnostics.js /path/to/repository /tmp/diagnostics-shots
```

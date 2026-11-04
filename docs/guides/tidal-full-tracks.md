# TIDAL full tracks

TIDAL has several sign-ins on this node, in **Sources › TIDAL › Sign in**. Each person signs in to
their own TIDAL account; one person's sign-in never plays for another.

- **Sign in with TIDAL** (Recommended; full tracks, unofficial): you sign in at TIDAL in your
  browser and paste back the address of the page you land on. It is the only sign-in TIDAL serves
  **lossless and hi-res FLAC** to (up to 24-bit/192 kHz). Nothing to set up: a release build
  carries the client it needs.
- **Sign in with a code** (the fallback; full tracks, unofficial): the way TIDAL's own devices
  sign in, with a code you enter at link.tidal.com. TIDAL may send this sign-in **AAC only** (lossy),
  so use it only when the browser sign-in doesn't work for you.
- Under **Advanced**: the official app (the project's, or your own from TIDAL's developer site:
  search, albums, your collection and playlist imports; TIDAL plays **30-second previews** to
  apps on its third-party developer tier), **Your own device client** for the code sign-in, and
  **Paste a token** from another app. **A sign-in from another app on this machine** shows up only
  when the node finds one.

TIDAL has not approved the unofficial sign-ins for other apps: they can stop working at any time,
and they use your TIDAL account. Full tracks need a paid TIDAL plan.

The **Full tracks** status at the top of the Sign in group says where you are: Needs sign-in,
Waiting, Working, AAC only, Off, Problem, or (a build without its clients) Needs client.

## Sign in with TIDAL

1. Press **Sign in with TIDAL**. TIDAL's sign-in opens in a new tab. Sign in there. TIDAL then
   shows a page that may say "Oops" or look empty: that is the page you want, keep it open.
2. Copy the whole address of that page from the address bar (it starts with
   `https://tidal.com/` and has `?code=` in it), paste it into the field on the node's page and
   press **Done**. The code alone works too.

The status turns **Working** and TIDAL plays full tracks, up to 24-bit/192 kHz (the **Quality**
setting can cap it at 16-bit/44.1 kHz). The page says what went wrong in plain words: an address
that isn't TIDAL's (only `https://` addresses on tidal.com are read), one without a code, a code
TIDAL didn't accept (a code works once and only for a few minutes), or a sign-in that waited longer
than ten minutes. Then press **Sign in with TIDAL** again.

No password goes through the node. It keeps the tokens TIDAL gives it and renews them itself.

## Sign in with a code

Pick **Sign in with a code** and press the button. The page shows a code, big, and **Open
link.tidal.com**: on a phone or a computer that is signed in to TIDAL, that button opens the page
with the code filled in; confirm it there. If the code isn't filled in, type it, or use **Copy**.
The code is good for about five minutes; the time left is under the button. The page notices when
TIDAL says yes and continues by itself. If the code runs out first, the page says so and offers a
new one.

TIDAL often sends this sign-in AAC instead of FLAC. The TIDAL page then says **Sign in with TIDAL
again for lossless**, with the button; signing in that way replaces the code sign-in.

### Advanced: your own device client

A release build carries a device client for the code sign-in. A build from source without it
lists the code sign-in as unavailable until the owner adds one under **Advanced › Your own device
client**. A client entered there replaces the built-in one for everyone on the node, even if TIDAL
later refuses it (the node never switches clients by itself). Enter a device client you are
allowed to use.

An app from TIDAL's developer site is **not** a device client: TIDAL refuses it for the code sign-in.
That one goes in **Settings › This node › Service apps** and becomes your official app.

You can paste a whole `client_id=… client_secret=…` block or JSON into either field: it is split.
**Check** asks TIDAL, without saving anything and without your account:

| It says | It means |
| :-- | :-- |
| OK: TIDAL accepts this device client | go on |
| This client can't use the code sign-in | an app from the developer portal, not a device client |
| TIDAL doesn't know this client ID | a typo, or the client was withdrawn |
| TIDAL knows this client ID, but the secret doesn't go with it | the secret is wrong |

**Save and sign in with a code** keeps the client like a sign-in (the secret is never shown again)
and starts the sign-in. The config keys are `[tidal] unofficial_client_id` and
`unofficial_client_secret` (or `PM_TIDAL_UNOFFICIAL_CLIENT_ID` and `_SECRET`). The order is: a
client saved on the page, then node.toml, then the environment, then the build's own.

Collection and playlist imports still go through the official app: the page offers to sign in with
it too.

## What arrives

Each track the node opens is logged in one line with what TIDAL answered: the quality
(`HI_RES_LOSSLESS`, `LOSSLESS`, `HIGH`), the manifest kind, the codec, and the bit depth and sample
rate when TIDAL names them. Addresses and tokens are never logged. When TIDAL answers AAC where
lossless was asked for, the node asks once more for LOSSLESS before it plays the AAC. The player's
signal path shows what really arrives (`FLAC 24/96`, `AAC`), read from the stream itself.

## When it stops working

If TIDAL refuses the client or ends the session, the node switches full tracks off, the status
says **Problem** with what happened, and TIDAL plays previews until it works again. **Try again**
switches it back on; **Sign out** forgets the session. A session imported from another app's file
(tidal-dl-ng, a pasted token) can be replaced by a newer file the same way.

## What the node keeps

Your own session tokens, and the device client you entered, in the node's database, treated like a
sign-in: a backup carries them only under your passphrase. A session made before this version
keeps working and renews with the device client, as before; nobody is signed out by an upgrade.
The build's two clients stay in the binary, not in Settings, masked so they are not plain text in
it; that is not encryption: someone with the binary can recover them.

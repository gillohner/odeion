# Qobuz

The node plays full tracks from Qobuz, up to 24-bit/192 kHz, through the private API Qobuz's own
players use. This is unofficial and can stop working at any time. It needs a paid Qobuz streaming
plan (Studio or Sublime; download-only plans do not stream).

## Sign in

Open **Sources › Qobuz**. Under **Sign in with your Qobuz account**, enter your Qobuz email and
password and choose **Sign in**. That is the whole setup: no app ID, app secret, API application or
config import is needed.

On your first sign-in, the node reads the current Qobuz web player's public login page and bundle
to discover the app values its API expects. It selects a signing secret, then caches the app ID,
secret and bundle URL for the whole node. Later sign-ins reuse that cache. If Qobuz refuses the
signature or app ID, the node refreshes those values once and retries without signing anyone out.
It looks at the web player at most once every five minutes, or once a minute after a look that
could not reach Qobuz or was interrupted. What a look found is reused for at most ten minutes,
unless Qobuz refused those values; refused values are not tried again until the next look. Until
then, a sign-in or refresh that needs new values gets the last look's own answer. Only a 400 about
the app ID or the signature counts against the app values: any other 400 fails only that person's
sign-in. When such a 400 comes with the cached values, the sign-in tries once more with what the web
player offers (within the same limits), in case Qobuz retired the cached values without saying so;
the 400 stands when the web player offers no other values, or when they get it too. Every request
to Qobuz uses one fixed browser User-Agent, never the node's own.

Your email and password go to Qobuz once. The password is sent as the MD5 Qobuz's own login flow
expects; neither the password nor its MD5 is stored or logged. Each person signs in with their own
account, and the node keeps only that person's session token.

Errors are shown in place:

- **Wrong Qobuz email or password**: check them and try again.
- **This Qobuz account is free**: full tracks need a paid Qobuz plan.
- **Qobuz refused this sign-in**: Qobuz turned down this sign-in for a reason other than its app
  values (for example, a mistyped email address); check the email and try again. Nobody else is
  affected.
- **Qobuz would not stream for this account**: Qobuz accepted the sign-in but turned down the
  signed stream check for this account, for a reason other than its app values; try again later.
  Nobody else is affected.
- **Qobuz changed its web player**: automatic discovery failed, or Qobuz refused what it found;
  the node looks again after five minutes.
- **Could not reach Qobuz**: a network problem, a timeout or a busy Qobuz; "try again later" means
  in about a minute.
- **Qobuz setup was interrupted**: the last look at the web player did not finish (for example,
  the page was closed during sign-in); try again in a minute.

Signed in, the page shows the account, plan and available quality. **Quality** can cap playback at
Max (up to 24-bit/192 kHz), CD (16-bit/44.1 kHz), or MP3 320 kbps. **Sign out** forgets only that
person's session; the node-wide app cache stays.

## When it stops working

If Qobuz ends a session, or refuses the app values and the values a look at the web player finds
do not work either, Qobuz switches itself off and the page says why. A track Qobuz refuses for
another reason fails only that track. When Qobuz cannot be reached or is busy, also during a
refresh and in the minute after a look that could not reach the web player or was interrupted, the
track fails for now (a browser gets "service unavailable", not "not found") and Qobuz stays on. **Try again**
switches it back on. A rotated app secret is refreshed without removing anyone's user token. Qobuz links keep opening on qobuz.com meanwhile.

## Admin override and backups

Normally no configuration is necessary. An administrator can silently override discovery with
`[qobuz] app_id` and `app_secret` in `node.toml`, or `PM_QOBUZ_APP_ID` and
`PM_QOBUZ_APP_SECRET`. Both values are needed; one alone is ignored with a warning in the log.
When set, those values win and web-player discovery and refreshes are skipped.

The cached app values and each person's session token are treated like sign-ins: a backup carries
them only under your passphrase. Passwords and password MD5s are never included.

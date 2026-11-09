# Qobuz

The node plays full tracks from Qobuz, up to 24-bit/192 kHz, through the private API Qobuz's own
players use. Qobuz has not approved this for other apps: it can stop working at any time, and it
uses your Qobuz account. It needs a paid Qobuz streaming plan.

Everything is on one page: **Sources › Qobuz**. Its first line says what you get and what you
need; two steps do the rest. Signed in, one row says who, the plan and the quality.

## What you need

1. **streamrip's or qobuz-dl's config** (streamrip's `config.toml` or qobuz-dl's `config.ini`).
   It holds the Qobuz app values Qobuz's API needs, and often your sign-in too. Qobuz hands no app
   to people who ask, so the page doesn't ask you for one. An app set in the node's config
   (`[qobuz] app_id` and `app_secret`, or `PM_QOBUZ_APP_ID` and `PM_QOBUZ_APP_SECRET`) works too:
   step 1 is then done already.
2. **Your Qobuz account** with a streaming plan (Studio or Sublime; Qobuz's download-only plans
   don't stream).

## Step 1: streamrip's or qobuz-dl's config

If the node finds one on its machine, it is listed with **Use it**. Otherwise pick the file
(**Use this file**) or paste its contents (**Use it**); it can come from another machine. The node
reads the app values and the sign-in from it and does both steps at once. The file is only read,
and nothing of it is shown again. A file without usable values is refused in place, saying why.

**Use a newer config** under a done step 1 replaces the app (also one from the node config), and
**Forget** removes the app a config gave. A listener on a node whose owner hasn't set up Qobuz
sees just that; the owner sets it up first.

## Step 2: Your Qobuz account

Only needed when the config had no sign-in. Enter your Qobuz email and password. They go to
Qobuz once, to sign in; the node keeps only the session token Qobuz answers with. Two refusals are
said in place:

- *This Qobuz account has no streaming plan*: full tracks need a paid plan; the account signed in,
  but Qobuz won't stream to it.
- *Qobuz doesn't know that email and password*: check them and try again.

Signed in, the row says **Working** with the quality your plan allows, and a **Quality** setting
lets you cap it (Max, CD, or MP3 320 kbps). **Sign out** forgets the session; the app stays.

## When it stops working

If Qobuz refuses the app or ends the session, the node switches Qobuz off and the row says
**Problem** with what happened; **Try again** switches it back on. Qobuz links keep opening on
qobuz.com meanwhile. If the app stopped working, use a newer config from streamrip or qobuz-dl.

## What the node keeps

The app ID and secret, and the session token, in the node's database, treated like a sign-in: a
backup carries them only under your passphrase. The password is never stored or logged.

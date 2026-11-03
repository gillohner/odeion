# Install the web app

Your node's web page can be installed as an app called **Odeion**: it gets an icon on your home
screen, dock or taskbar and opens in its own window, without the browser's address bar. It is
still your node's page, so:

- it needs your node to be on and reachable from the device; music doesn't play offline in it
  (for songs kept on the device, use [the desktop and Android app](desktop-app.md) and its
  Downloads);
- on a computer and on Android the app shares the browser's sign-in. On iPhone and iPad an app
  added to the Home Screen keeps its own storage, apart from the browser's, so you log in once
  more inside it;
- an update of the node shows up the next time you open the app. Nothing reloads while music
  plays.

## Before you start: an HTTPS address

Browsers install apps only from an `https://` address, or from `localhost` on the node's own
computer. An address like `http://192.168.1.20:7788` or `http://nuc.local:7788` can't be
installed; the **Install app** help says so and names the address.

To get an HTTPS address, put the node behind HTTPS: a Cloudflare Tunnel (see
[Remote access](remote-access.md#cloudflare-tunnel-the-bundle), which gives you
`https://music.example.com`) or a reverse proxy with a certificate on your own network. The
Tailscale address in Remote access (`http://100.x.y.z:7788`) is plain HTTP and can't be
installed; Tailscale's own HTTPS certificates (`tailscale serve`, see Tailscale's documentation)
can put an HTTPS address in front of the node. Then open the node at that address and install from
there.

## Install

Open your node in the browser and use the **Install app** button in the top bar (the square with
an arrow, next to your account button), or, before you log in, **Install app** under the login.
Where the browser can install right away (Chrome, Edge
and other Chromium browsers once you've used the page for a little while), it asks you to
confirm. Otherwise the button shows the steps for your browser:

| Browser | Steps |
| :-- | :-- |
| Chrome on a computer | The install icon at the right end of the address bar, or the menu › **Install page as app** (in some versions under **Cast, save and share**) |
| Edge on a computer | **Settings and more** › **Apps** › **Install this site as an app** |
| Chrome on Android | Menu › **Install app** (or **Add to Home screen**) › **Install** |
| Samsung Internet | Menu › **Add page to** › **Home screen** |
| Safari on iPhone or iPad | **Share** › **Add to Home Screen** › **Add**. From iOS 16.4 this works in Chrome, Edge and Firefox on iOS too |
| Safari on a Mac (Safari 17, macOS Sonoma or later) | **File** › **Add to Dock** |
| Firefox on Windows (143 or later) | The web apps icon in the address bar › **Add to taskbar** |
| Firefox on Android | Menu › **Install** › **Add to Home Screen** (add it automatically, or place the icon yourself). If the menu has no Install, **Add to Home screen** adds a shortcut that opens in Firefox |
| Firefox on macOS or Linux | Can't install web apps: use Chrome, Edge or Safari |

Then open Odeion from your home screen, dock, Start menu or app launcher.

The button doesn't show inside the installed app's window, in the page you installed it from, or
inside the desktop and Android app. A normal browser tab opened later can't always tell that the
app is already installed, so it may offer the button again there. **Hide this button** in its help
hides it in that browser; to see it again, add `#install` to the address (for example
`https://music.example.org/#install`, or on the page you have open).

## When the node can't be reached

Once you have opened the node's page over HTTPS (installed or not), the browser keeps a copy of the
page (only the page, its script, styles and icons; nothing of your library, your sign-in or your
music). So the app still opens while the node is off, out of reach or answered for by an error
page of a proxy or tunnel in front of it, and says it can't reach the node. When the node answers
again the app connects by itself, or offers **Reconnect** if it was still waiting for the node.
The page is always asked from the node first, so an updated node is used at once.

The copy is fetched without your sign-in and kept only when it is exactly the node's public page.
Behind a sign-in proxy (Cloudflare Access, Authelia and the like) that asks everyone to sign in
first, the app still installs (the page sends its sign-in cookie for the app's description), but
nothing is kept: while the node is away (or the proxy answers with an error) the app shows
**Can't reach your node** after a few seconds. Nothing private from the proxy is ever kept. Tested in Chromium with a stand-in
cookie proxy, not with each proxy product.

## Uninstall

Remove it as any app on that device (long-press the icon, or the app window's menu ›
**Uninstall**). Your node and your library are not affected.

The kept copy stays in that browser until you clear the site's data in the browser's settings,
including after going back to a node version without the installable app: the browser keeps the
old copy's helper, which always asks the node first, so the older node's page is what you get.

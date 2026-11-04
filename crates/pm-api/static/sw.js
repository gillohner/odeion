// Odeion's service worker (pwa.js registers it; docs/guides/install-web-app.md).
// One job: the installed app still opens while the node is away, and says so, instead of the
// browser's error page. So it keeps the node's PUBLIC page files and nothing else: never the
// API, covers, audio, anything signed in or with a token. Always the network first, so an
// updated node's page is used at once; the copy is only for when the node doesn't answer.
// It never reloads a page or deletes a cache, so a new version can't stop the music.
// A node may sit behind a sign-in proxy: what the page itself gets (with its cookies) may be a
// sign-in page, a redirect or a page personalised for someone. So the worker never keeps what
// the page got. It keeps a file only by asking for it itself, with no cookies or other
// credentials and no redirects, and only when the answer is exactly the node's public file:
// 200, from this origin, at that path with no query, its own type, not private or no-store,
// and (for the page) the node's page itself.
'use strict';
const CACHE = 'odeion-shell';
// The page's files. The page itself (/) is the same for every path the app shows.
const FILES = ['/', '/app.js', '/app.css', '/pwa.js', '/pwa.css', '/manifest.webmanifest', '/icon.svg', '/icon-192.png', '/icon-512.png', '/icon-maskable-512.png', '/apple-touch-icon.png'];
// The app's own pages, exactly the node's page routes (lib.rs `web` router and WEB_VIEWS; checked
// by tools/pwa-check.js); a party link, a sign-in return or anything else is left to the network.
const PAGES = /^\/(?:|app\/?|now-playing|discover(?:\/.+)?|search|library|add-music|downloads|favourites|albums|artists|genres|tracks|playlists|radio|sources|queue|zones|settings|identify|(?:settings|library|sources|home)\/[^/]+|(?:song|album|artist|playlist|person|smart|genre|source|tag)\/[^/]+|shared\/[^/]+(?:\/[^/]+)?)$/;
// How long a launch waits for the node before it opens the kept page (a node that is off
// often doesn't refuse, it just never answers).
const WAIT = 4000;
// Each file's type, and what only the node's own page has
const TYPES = { js: 'text/javascript', css: 'text/css', png: 'image/png', svg: 'image/svg+xml', webmanifest: 'application/manifest+json' };
const typeOf = (path) => (path === '/' ? 'text/html' : TYPES[path.split('.').pop()]);
// (the manifest link carries the page's credentials: a sign-in proxy in front of the node needs them)
const PAGE_MARKS = ['<link rel="manifest" href="/manifest.webmanifest" crossorigin="use-credentials">', '<script src="/app.js"></script>'];

// Whether `r` is the node's public file at `path`, as anyone gets it
async function canonical(path, r) {
  if (!r || r.status !== 200 || r.type !== 'basic' || r.redirected) return false;
  const u = new URL(r.url);
  if (u.origin !== self.location.origin || u.pathname !== path || u.search) return false;
  if (/no-store|private/i.test(r.headers.get('cache-control') || '')) return false;
  if ((r.headers.get('content-type') || '').split(';')[0].trim().toLowerCase() !== typeOf(path)) return false;
  if (path === '/') { const text = await r.clone().text(); if (!PAGE_MARKS.every((m) => text.includes(m))) return false; }
  return true;
}

// Ask for the public file without credentials and keep it when it is canonical. Any other answer
// (a proxy's or portal's page, 401/403/404/429, an error) leaves a valid kept copy as it is; only an
// entry under that name that is itself no longer valid goes: that one entry, nothing more.
async function keep(path) {
  try {
    const r = await fetch(new Request(path, { credentials: 'omit', cache: 'no-cache', redirect: 'error' }));
    const c = await caches.open(CACHE);
    if (await canonical(path, r)) { await c.put(path, r); return true; }
    // Not kept: let its body go, or it holds the connection the other files need
    if (r.body) r.body.cancel().catch(() => {});
    const old = await c.match(path);
    if (old && !(await canonical(path, old))) await c.delete(path);
  } catch { /* the node is away: keep what there is */ }
  return false;
}

// Whether two answers have the same bytes
async function same(a, b) {
  const [x, y] = await Promise.all([a.arrayBuffer(), b.clone().arrayBuffer()]);
  if (x.byteLength !== y.byteLength) return false;
  const u = new Uint8Array(x); const v = new Uint8Array(y);
  for (let i = 0; i < u.length; i++) if (u[i] !== v[i]) return false;
  return true;
}

// The kept copy of `path`, if it is still the canonical file
async function copyOf(path) {
  const r = await caches.match(path, { cacheName: CACHE });
  return (await canonical(path, r)) ? r : null;
}

// The files the page can't start without (it waits for them in its head): a kept page is used only
// with all of them kept too, or it would wait for the node with no end and show nothing
const BLOCKING = ['/pwa.js', '/app.js', '/app.css', '/pwa.css'];
async function keptPage() {
  const root = await copyOf('/');
  if (!root) return null;
  for (const f of BLOCKING) if (!(await copyOf(f))) return null;
  return root;
}

self.addEventListener('install', (e) => {
  // The API, covers and streams go straight to the network, without even starting this worker
  // (Chromium's static routes, where there are any)
  try { if (e.addRoutes && self.URLPattern) e.addRoutes({ condition: { urlPattern: new URLPattern({ pathname: '/v1/*' }) }, source: 'network' }); } catch { /* not supported */ }
  e.waitUntil(Promise.all(FILES.map(keep)).then(() => self.skipWaiting()));
});
// Take over open pages for their next request; no page is reloaded.
self.addEventListener('activate', (e) => e.waitUntil(self.clients.claim()));

// No answer in time: null, as if the node were away (resolves, so nothing is left unhandled)
const timeout = (ms) => new Promise((ok) => setTimeout(() => ok(null), ms));
// A reverse proxy or tunnel in front of a stopped node answers with its own 5xx (502–504, and
// Cloudflare's 520–530): as good as no answer. The node's own page files never answer 5xx.
// A sign-in answer (401/403) is not "away": it goes to the page as it is.
const away = (r) => !r || r.status >= 500;

// Pages opened from the kept copy (their client ids): their files come from the copy at once,
// so a launch waits for the node once, not once per file. Forgotten when the worker stops.
const fromCopy = new Set();
const openedFromCopy = (id) => { if (!id) return; fromCopy.add(id); if (fromCopy.size > 50) fromCopy.delete(fromCopy.values().next().value); };

// A page asks whether it was opened from the kept copy: then the node didn't answer, and pwa.js says so
self.addEventListener('message', (e) => {
  if (e.data === 'pwa:opened-from-copy?' && e.source) e.source.postMessage({ pwa: 'opened-from-copy', value: fromCopy.has(e.source.id) });
});

// The node's answer to the page's own request (never kept), or null when it doesn't come: an
// error, or (when `bounded`) no answer within WAIT
async function fromNode(request, bounded) {
  const net = fetch(request);
  net.catch(() => {}); // a late failure after the kept copy was used is not an error
  try { return await (bounded ? Promise.race([net, timeout(WAIT)]) : net); } catch { return null; }
}

// The node's answer to a navigation within WAIT, null when it doesn't come in time. For the page
// itself (200, HTML) "in time" means all of it: a proxy can send the headers and then stall, so the
// body is read within the same deadline (a stalled download is cancelled). Anything else (a sign-in
// 401/403, a redirect, another type) goes to the page as it is. Never kept; a page bigger than
// PAGE_MAX that is still arriving is passed on as it comes.
const PAGE_MAX = 4 * 1024 * 1024;
async function pageFromNode(request) {
  const until = Date.now() + WAIT;
  const r = await fromNode(request, true);
  if (!r || r.status !== 200 || r.type !== 'basic' || !r.body || !/^text\/html/i.test(r.headers.get('content-type') || '')) return r;
  const reader = r.body.getReader();
  const parts = []; let size = 0;
  const init = { status: r.status, statusText: r.statusText, headers: r.headers };
  try {
    for (;;) {
      const step = await Promise.race([reader.read(), timeout(Math.max(0, until - Date.now()))]);
      if (!step) { reader.cancel().catch(() => {}); return null; }
      if (step.done) return new Response(new Blob(parts), init);
      parts.push(step.value); size += step.value.byteLength;
      if (size > PAGE_MAX) {
        return new Response(new ReadableStream({
          start(c) { for (const p of parts) c.enqueue(p); },
          async pull(c) { const n = await reader.read(); if (n.done) c.close(); else c.enqueue(n.value); },
          cancel() { reader.cancel().catch(() => {}); },
        }), init);
      }
    }
  } catch { reader.cancel().catch(() => {}); return null; }
}

const OFFLINE = '<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><meta name="color-scheme" content="light dark"><title>Odeion</title>'
  + '<body style="font-family:system-ui,sans-serif;margin:0;display:grid;place-items:center;min-height:100vh;padding:24px;box-sizing:border-box;background:#0f1014;color:#e8e8ee">'
  + '<main style="max-width:420px"><h1 style="font-size:22px">Can’t reach your node</h1><p>Odeion needs your node to be on and reachable from this device. Music doesn’t play offline here.</p><p><a href="/" style="color:#a48cff">Try again</a></p></main>';

self.addEventListener('fetch', (e) => {
  const req = e.request;
  if (req.method !== 'GET') return;
  const url = new URL(req.url);
  if (url.origin !== self.location.origin) return;
  if (req.mode === 'navigate') {
    if (!PAGES.test(url.pathname)) return;
    e.respondWith((async () => {
      const cached = await keptPage();
      // A launch waits for the node a bounded time, with or without a copy (behind a sign-in proxy
      // nothing may be kept): then the copy, or the "can't reach" page, never an endless wait
      const r = await pageFromNode(req);
      // The node answered: that answer goes to the page as it is (a sign-in 401/403 too); the copy is refreshed apart
      if (!away(r)) { e.waitUntil(keep('/')); return r; }
      if (cached) { openedFromCopy(e.resultingClientId); return cached; }
      // No copy: the same "can't reach your node" page for no answer and for a proxy's or tunnel's 5xx
      if (r && r.body) r.body.cancel().catch(() => {});
      return new Response(OFFLINE, { status: 503, headers: { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store' } });
    })());
    return;
  }
  if (url.search || url.pathname === '/' || !FILES.includes(url.pathname)) return;
  const path = url.pathname;
  e.respondWith((async () => {
    const cached = await copyOf(path);
    if (cached && fromCopy.has(e.clientId)) return cached;
    // A page that came from the node takes its files from the node too, however slow (never a new
    // page with an older kept script); the copy only when the node fails. A page opened from the
    // copy (the node away) waits for a file it has no copy of only a bounded time.
    const r = await fromNode(req, fromCopy.has(e.clientId));
    if (!away(r)) {
      // A file the node changed (or one not kept yet) is fetched again, without credentials. The
      // same length is not the same file: the bytes are compared (the page's answer is never kept).
      const seen = r.clone();
      e.waitUntil((async () => { if (!cached || !(await same(seen, cached))) await keep(path); })().catch(() => {}));
      return r;
    }
    return cached || r || Response.error();
  })());
});

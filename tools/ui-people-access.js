// Settings › People & access: the one screen for who can use this node, adding and removing people,
// what they can do, the browsers and apps signed in, and the password for apps. At 375 and 1280 px:
// - the owner: one sentence per person (music on, a homeserver account without music, music switched
//   off; every zone or the zones named; each choice of whose music services), the key behind "Show key";
//   a person's sign-ins with Sign out and Sign out everywhere; Rename, Music off and on, the storage
//   limit and Remove, each asked first where it takes something away and sent once; Add someone and
//   what to send them; the can / can't lists; zones "Every zone / Only these" (none ticked is refused);
//   the three choices of whose music services; the browsers and apps signed in as you, with Log out,
//   Sign out and Sign out all others; the password for apps; a party list that failed says so; a
//   people list that failed says so and leaves the rest of the screen working; a removal that did not
//   finish keeps Remove on the person; opened at an address of the node's own machine, the page says
//   to send the node's address on the network instead of offering its own to copy;
// - someone the owner added: "What you can do here" and "Your sign-ins on this node" in their own
//   Settings, and none of the owner's controls;
// - who the page takes the caller for: by what /v1/me says they can do (Ungroup, the status chip's
//   owner chores); a /v1/me that failed is the node out of reach, a 404 is the owner;
// - the words of the screen, its dialogs and its toasts: none of the old or inside names.
// Fixture only (no node): node tools/ui-people-access.js <repo> <screenshots dir>
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const { create, go, cleanCopy, ORIGIN } = require('./ui-fixture');
const OUT = process.argv[3] || '/tmp/ui-people-access';
fs.mkdirSync(OUT, { recursive: true });

const ANA = 'pxnu33x7jtpx9ar1ytsi4yxbp6a5o36gwhffs8zoxmbuptici1jy';
const BO = 'o3ftkuxjd9k6zgnc4hhbhbg5jusubr4zqyb1dtzi8hhm3hf5wbmy';
const CY = 'tbepfp8fisyiqpskbmxsmpub47gxb7xdsh5x9ut9qrb97gcq9phy';
const DI = 'ybndrfg8ejkmcpqxot1uwisza345h769ybndrfg8ejkmcpqxot1u';
const FAY = '8ejkmcpqxot1uwisza345h769ybndrfg8ejkmcpqxot1uybndrfg';
const GUS = 'za345h769ybndrfg8ejkmcpqxot1uwisza345h769ybndrfg8ejk';
const HS = 'h769ybndrfg8ejkmcpqxot1uwisza345h769ybndrfg8ejkmcpqx';
const QR = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" fill="#fff"/></svg>';
const DAY = 86400;
const T0 = Date.UTC(2026, 10, 20, 12) / 1000;
// The page's clock and words are pinned (UTC, en-US), so a date reads the same wherever this runs
const PINNED = { locale: 'en-US', timezoneId: 'UTC' };
const day = (t) => new Date(t * 1000).toLocaleDateString('en-US', { timeZone: 'UTC' });
const NEW_TOKEN = 'f'.repeat(64);
// What GET /v1/me lists for each caller (crates/pm-api/src/access.rs: the owner has every capability
// but a party guest's; someone the owner added has these, `own_sign_in` unless the node plays
// everyone through its own sign-ins)
const PERSON_CAN = ['artwork', 'session', 'library', 'zone_list', 'zone_use', 'own', 'own_play', 'own_sign_in_state', 'own_sign_in'];
const OWNER_CAN = [...PERSON_CAN, 'owner_music', 'zone_manage', 'library_manage', 'settings', 'service_logins', 'people', 'tokens', 'vault', 'backup', 'identity', 'remote_access', 'party', 'diagnostics'];
// The words a person reads: none of the old or inside names, wherever they show (the screen, a
// dialog, a toast). "token" only as the heading's "(API token)".
const plainWords = (text, where) => {
  assert.doesNotMatch(text, /\b(admins?|listeners?|sessions?|favou?rites?|playlists?|outside)\b/i, `${where}: the words`);
  assert.doesNotMatch(text.replace(/\(API token\)/gi, ''), /token/i, `${where}: “token” only in “(API token)”`);
  assert.doesNotMatch(text, /\bundefined\b|\[object Object\]|\bnull\b/, `${where}: no leftovers`);
};
// Every dialog and toast the page shows is kept (window.__said), to read their words at the end
const keepSaid = (context) => context.addInitScript(() => {
  window.__said = [];
  const look = () => {
    const t = document.querySelector('#toast');
    for (const x of [t && t.textContent, ...[...document.querySelectorAll('dialog.dialog')].map((d) => d.innerText)]) if (x && x.trim() && !window.__said.includes(x.trim())) window.__said.push(x.trim());
  };
  new MutationObserver(look).observe(document, { subtree: true, childList: true, characterData: true });
});
const saidWords = async (page, where, least) => {
  const said = await page.evaluate(() => window.__said);
  assert.ok(said.length >= least, `${where}: its dialogs and toasts were read (${said.length})`);
  for (const text of said) plainWords(text, `${where}: “${text.slice(0, 50)}”`);
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, what, ms = 6000) => { const end = Date.now() + ms; for (;;) { const v = await fn(); if (v) return v; if (Date.now() > end) assert.fail(`timed out waiting for: ${what}`); await sleep(40); } };
const zone = (id, name) => ({ id, name, output: `upnp:${id}`, transport: 'stopped', queue: [], queue_index: null, queue_version: 1, repeat: 'off', shuffle: false, radio: false, error: null, acked: 0, volume: { percent: 40 }, now_playing: null });
const ZONES = [zone('kitchen', 'Kitchen'), zone('den', 'Den'), zone('hall', 'Hall')];

// What the node keeps, as its routes answer (crates/pm-api/src/people.rs, accounts.rs)
const node = (over = {}) => ({
  policy: 'own', replaceable: true, exposed: null, zones: [], none: false, parties: [], fail: {}, gate: {}, calls: [], half: {}, leftover: new Set(),
  homeserver: { configured: true, reachable: true, version: '0.6.0', users: 3, disk_used_mb: 2, public_key: HS },
  people: [
    { key: ANA, access: 'music', name: 'Ana', music_enabled: true, homeserver_enabled: true, added_at: T0, storage_quota_mb: null, used_bytes: null, sessions: 2, sign_ins: [
      { id: 'a1', kind: 'person', label: 'Firefox on Linux', remote: false, created_at: T0, last_used: T0 - (T0 % DAY), current: false },
      { id: 'a2', kind: 'person', remote: true, created_at: T0 - 3 * DAY, current: false },
    ] },
    { key: BO, access: 'homeserver_music', name: null, music_enabled: true, homeserver_enabled: true, added_at: T0, storage_quota_mb: 500, used_bytes: 12e6, sessions: 0, sign_ins: [] },
    { key: CY, access: 'homeserver', name: 'Cy', music_enabled: false, homeserver_enabled: true, added_at: T0, storage_quota_mb: 'unlimited', used_bytes: 3e6, sessions: 0, sign_ins: [] },
    { key: DI, access: 'music', name: 'Di', music_enabled: false, homeserver_enabled: true, added_at: T0, storage_quota_mb: null, used_bytes: null, sessions: 1, sign_ins: [
      { id: 'd1', kind: 'person', label: 'Odeion app 0.2.4', remote: false, created_at: T0, current: false },
    ] },
  ],
  invites: [{ id: 'inv1', access: 'homeserver_music', name: 'Eve', token: 'AAAA-BBBB-CCCC', storage_quota_mb: 200, created_at: T0, url: `pubkyauth://direct_signup?hs=${HS}&st=AAAA-BBBB-CCCC`, qr_svg: QR }],
  mine: [
    { id: 'm1', kind: 'browser', label: 'Chromium on Linux', remote: false, created_at: T0, last_used: T0 - (T0 % DAY), current: true },
    { id: 'm2', kind: 'app', label: 'Odeion app 0.2.4', remote: true, created_at: T0 - DAY, current: false },
    { id: 'm3', kind: 'browser', remote: false, created_at: T0 - 9 * DAY, current: false },
  ],
  ...over,
});
// The node's routes for this screen, over the fixture's own. `n.fail['GET /people']`: that call
// answers 500 with this message; `n.gate['PUT /people/zones']`: it waits for this promise first.
const serve = async (context, n) => {
  await context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: ZONES })); });
  await context.route(/\/v1\/(people|party|me\/sign-ins|node\/settings|node\/token)(\/|\?|$)/, async (r) => {
    const req = r.request(); const v = new URL(req.url()).pathname.slice(3); const m = req.method();
    const body = req.postData() ? JSON.parse(req.postData()) : undefined;
    const call = `${m} ${v}`;
    n.calls.push({ call, body });
    const json = (b, status = 200) => r.fulfill({ status, contentType: 'application/json', body: JSON.stringify(b) });
    if (n.gate[call]) await n.gate[call];
    if (n.fail[call]) return json({ error: n.fail[call] }, 500);
    const seg = v.split('/').filter(Boolean).map(decodeURIComponent);
    if (call === 'GET /node/settings') return json({ contact: '', discovery: false, token_replaceable: n.replaceable, sources_policy: n.policy, ...(n.exposed ? { api_token_exposed_at: n.exposed } : {}) });
    if (call === 'PUT /node/settings') { if (body.sources_policy) n.policy = body.sources_policy; return json({}); }
    if (call === 'POST /node/token') return json({ token: NEW_TOKEN });
    if (call === 'GET /me/sign-ins') return json(n.mine);
    if (call === 'DELETE /me/sign-ins') { const before = n.mine.length; n.mine = n.mine.filter((s) => s.current); return json({ ended: before - n.mine.length }); }
    if (m === 'DELETE' && seg[0] === 'me' && seg.length === 3) { const had = n.mine.some((s) => s.id === seg[2]); n.mine = n.mine.filter((s) => s.id !== seg[2]); return had ? json({ ended: 1 }) : json({ error: 'no such sign-in' }, 404); }
    if (call === 'GET /party') return json(n.parties);
    if (call === 'POST /party') { const pt = { id: `p${n.parties.length + 1}`, name: body.name || 'Party', zone: body.zone, zone_name: (ZONES.find((z) => z.id === body.zone) || {}).name, created_at: T0, expires_at: T0 + body.hours * 3600, adds_per_hour: body.adds_per_hour, waiting_per_guest: body.waiting_per_guest, guests: [], added: 0, url: `${ORIGIN}/party/abcdefghijklmnopqrstuvwxyz0123456789`, qr_svg: QR }; n.parties.push(pt); return json(pt); }
    if (m === 'DELETE' && seg[0] === 'party') { n.parties = n.parties.filter((p) => p.id !== seg[1]); return json({}); }
    // (`unfinished`: the people whose removal did not finish, as the node remembers them; left out when none)
    if (call === 'GET /people') return json({ owner: null, people: n.people, invites: n.invites, homeserver: n.homeserver, ...(n.leftover.size ? { unfinished: [...n.leftover] } : {}) });
    // The zones as a node of this version keeps them (`none`: no zone at all; an empty id is refused
    // without it). `n.olderZones`: as a node before it does (no `none` in its answers, the list kept
    // as sent: the list of one zone without a name opens no zone there). `n.dropsNone`: answers
    // with `none`, but does not keep one sent to it.
    if (call === 'GET /people/zones') return json(n.olderZones ? { zones: n.zones } : { zones: n.zones, none: n.none });
    if (call === 'PUT /people/zones') {
      if (n.olderZones) { n.zones = body.zones; return json({ zones: n.zones }); }
      const ids = body.zones.filter((z) => z.trim());
      if (body.none && ids.length) return json({ error: 'either the zones people may play on, or none: not both' }, 400);
      if (!body.none && ids.length !== body.zones.length) return json({ error: 'a zone id is empty: send the zones’ ids, or none: true for no zone' }, 400);
      n.zones = ids; n.none = !!body.none && !n.dropsNone;
      return json({ zones: n.zones, none: n.none });
    }
    if (call === 'POST /people/invites') {
      if (body.access === 'music') { n.people.push({ key: body.key, access: 'music', name: body.name || null, music_enabled: true, homeserver_enabled: true, added_at: T0, storage_quota_mb: null, used_bytes: null, sessions: 0, sign_ins: [] }); return json({ person: body.key }); }
      const inv = { id: `inv${n.invites.length + 1}`, access: body.access, name: body.name || null, token: 'DDDD-EEEE-FFFF', storage_quota_mb: body.storage_quota_mb || null, created_at: T0, url: `pubkyauth://direct_signup?hs=${HS}&st=DDDD-EEEE-FFFF`, qr_svg: QR };
      n.invites.push(inv); return json(inv);
    }
    if (m === 'DELETE' && seg[1] === 'invites') { n.invites = n.invites.filter((i) => i.id !== seg[2]); return json({ forgotten: seg[2] }); }
    // (someone taken off the list by a removal that then failed: removing again finishes it)
    // (`n.nothingLeft`: finished elsewhere meanwhile: the node answers that there is nobody by that key)
    if (m === 'DELETE' && seg.length === 2 && n.leftover.has(seg[1])) { n.leftover.delete(seg[1]); return n.nothingLeft ? json({ error: 'no such person' }, 404) : json({ people: n.people, invites: n.invites }); }
    const p = n.people.find((x) => x.key === seg[1]);
    if (!p) return json({ error: 'no such person' }, 404);
    if (m === 'PATCH') {
      if ('name' in body) p.name = body.name || null;
      // (Music off is a pause on the node: their sign-ins stay)
      if ('music' in body) { p.music_enabled = body.music; if (body.music && p.access === 'homeserver') p.access = 'homeserver_music'; }
      if ('homeserver_enabled' in body) p.homeserver_enabled = body.homeserver_enabled;
      if ('storage_quota_mb' in body) p.storage_quota_mb = body.storage_quota_mb;
      return json(p);
    }
    if (m === 'DELETE' && seg.length === 2) {
      if (p.access === 'music') n.people = n.people.filter((x) => x !== p); else { p.access = 'homeserver'; p.music_enabled = false; }
      p.sign_ins = [];
      // `n.half[key]`: the node took their music access away, then its cleanup failed with this
      if (n.half[p.key]) { const why = n.half[p.key]; delete n.half[p.key]; if (!n.people.includes(p)) n.leftover.add(p.key); return json({ error: why }, 500); }
      return json({ people: n.people, invites: n.invites });
    }
    if (m === 'DELETE' && seg[2] === 'sign-ins' && seg.length === 4) { const had = p.sign_ins.some((s) => s.id === seg[3]); p.sign_ins = p.sign_ins.filter((s) => s.id !== seg[3]); return had ? json({ ended: 1 }) : json({ error: 'no such sign-in' }, 404); }
    if (m === 'DELETE' && seg[2] === 'sign-ins') { const ended = p.sign_ins.length; p.sign_ins = []; return json({ ended }); }
    return json({ error: `the fixture has no ${call}` }, 404);
  });
};
// The whole page in one picture: the page scrolls inside the app's shell, so the window is made tall for it
const shot = async (page, width, name) => {
  const tall = await page.evaluate(() => { const m = document.querySelector('#main'); return m ? m.scrollHeight - m.clientHeight : 0; });
  await page.setViewportSize({ width, height: 900 + tall });
  await page.screenshot({ path: path.join(OUT, name) });
  await page.setViewportSize({ width, height: 900 });
};
const calls = (n, call) => n.calls.filter((c) => c.call === call);
const owner = async (browser, width, n, opts = {}) => {
  const made = await create(browser, width, { touch: width < 600, fixtures: { '/zones': { body: ZONES }, '/node/apps': { body: [] }, ...(opts.fixtures || {}) }, ...opts.state });
  await keepSaid(made.context);
  if (opts.before) await opts.before(made.context);
  await serve(made.context, n);
  await made.page.goto(`${opts.origin || ORIGIN}${opts.at || '/settings/people'}`);
  return made;
};
// The app's own files and the few answers its start needs, at an address of the node's own machine
// (the fixture serves one made-up origin only): for what the page says when it is opened there
const LOOPBACK = 'http://127.0.0.1:7788';
const atLoopback = async (context) => {
  const STATIC = path.join(path.resolve(process.argv[2] || path.join(__dirname, '..')), 'crates/pm-api/static');
  const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml' };
  const quiet = { '/info': { name: 'Fixture node', offline: false, library: { tracks: 1, albums: 1, artists: 2, files: 1, missing_files: 0 } }, '/me': { role: 'owner', key: null, zones: null, can: OWNER_CAN }, '/zones': ZONES, '/outputs': [], '/sources': [], '/presence': [], '/node/apps': [], '/pubky': { user: null }, '/me/prefs': {} };
  await context.route(`${LOOPBACK}/**`, (r) => {
    const p = new URL(r.request().url()).pathname;
    if (!p.startsWith('/v1/')) { const f = path.join(STATIC, p === '/' || !path.extname(p) ? 'index.html' : p.slice(1)); return fs.existsSync(f) ? r.fulfill({ body: fs.readFileSync(f), contentType: types[path.extname(f)] || 'application/octet-stream' }) : r.fulfill({ status: 404, body: '' }); }
    const v = p.slice(3);
    return v in quiet ? r.fulfill({ contentType: 'application/json', body: JSON.stringify(quiet[v]) }) : r.fulfill({ status: 404, contentType: 'application/json', body: JSON.stringify({ error: 'Fixture endpoint unavailable' }) });
  });
};

(async () => {
  const launched = await chromium.launch();
  // (every page of this script gets the pinned clock and words)
  const browser = { newContext: (o) => launched.newContext({ ...o, ...PINNED }), close: () => launched.close() };
  try {
    for (const width of [375, 1280]) {
      const phone = width < 600;
      // ---- the owner ----
      {
        const n = node({ olderZones: phone });
        // At 1280 the node says what the owner can do, as a node of this version does (`can` in
        // /v1/me), and knows "no zone"; at 375 it does neither (an older node): the page then goes by
        // the role, and "no zone" reaches that node as a list it reads as no zone too
        const { context, page, state } = await owner(browser, width, n, { fixtures: phone ? {} : { '/me': { body: { role: 'owner', key: null, zones: null, can: OWNER_CAN } } } });
        const box = page.locator('#peoplebox');
        const person = (key) => box.locator(`[data-person="${key}"]`);
        const says = (key) => person(key).locator('[data-psays]').innerText();
        const dialog = page.locator('dialog.dialog');
        const toast = (text) => page.locator('#toast', { hasText: text }).waitFor();
        // On a phone each section folds: open the one a step works in
        const openSec = async (id) => { if (phone) await box.locator(`details[data-fold="${id}"]`).evaluate((d) => { d.open = true; }); };
        const settled = () => sleep(150); // (a second request, if one were sent, would be in by now)
        await person(ANA).waitFor();

        // The tab is named for all of it, and each section is there under its own heading
        if (!phone) assert.equal((await page.locator('#settab-people').innerText()).trim(), 'People & access');
        assert.equal(await page.locator('#set-people .set-title').innerText(), 'People & access');
        const heads = await box.locator('h3.set-h').allInnerTexts();
        assert.deepEqual(heads.map((h) => h.toLowerCase()), ['who can use this node', 'add someone', 'what people you add can do', 'zones they can play on', 'whose music services they play with', 'browsers and apps signed in as you', 'password for apps (api token)', 'party', 'also in settings'], 'the sections, in order');
        if (phone) {
          assert.equal(await box.locator('details[data-fold]').count(), 8, 'on a phone each section is a fold');
          assert.equal(await box.locator('details[data-fold="who"]').getAttribute('open'), '', 'the people are open');
          assert.equal(await box.locator('details[data-fold="party"][open]').count(), 0, 'Party is shut');
        } else assert.equal(await box.locator('[data-fold]').count(), 0, 'no folds off phones');

        // 1. Who can use this node: you first, then a sentence per person
        const first = box.locator('.set-rows').first().locator('> .set-row').first();
        assert.match(await first.innerText(), /^You\s+Owner: you can change everything on this node\.$/);
        assert.equal(await person(ANA).locator('.set-name').innerText(), 'Ana');
        assert.equal(await says(ANA), 'Plays music here, on every zone, with their own TIDAL, Qobuz or Spotify.', 'music on');
        assert.equal(await person(BO).locator('.set-name').innerText(), 'No name yet', 'the key is never the title');
        assert.equal(await says(BO), 'Plays music here, on every zone, with their own TIDAL, Qobuz or Spotify.');
        assert.match(await person(BO).locator('[data-pstore]').innerText(), /Storage on this node’s homeserver: 12 MB used, a limit of 500 MB\./);
        assert.equal(await says(CY), 'Has storage on this node’s homeserver. No music.', 'a homeserver account without music');
        assert.match(await person(CY).locator('[data-pstore]').innerText(), /3 MB used, no limit\./);
        assert.equal(await says(DI), 'Music is switched off for them.', 'music switched off');
        assert.equal(await person(ANA).locator('[data-pstore]').count(), 0, 'no storage line without an account here');
        // The key: behind "Show key", whole, with Copy
        assert.equal(await person(BO).locator('.pa-keyrow').isVisible(), false, 'the key is not shown until asked for');
        await person(BO).locator('summary', { hasText: 'Show key' }).click();
        assert.equal(await person(BO).locator('.pa-keyrow code').innerText(), BO);
        assert.equal(await person(BO).locator('[data-copykey]').getAttribute('data-copykey'), BO);
        // Only what works: no homeserver controls for someone whose account is elsewhere, no Remove
        // for someone who has nothing here to remove
        assert.deepEqual(await person(ANA).locator('.pa-acts .btn').allInnerTexts(), ['Rename', 'Switch music off', 'Remove…']);
        assert.deepEqual(await person(BO).locator('.pa-acts .btn').allInnerTexts(), ['Rename', 'Switch music off', 'Switch homeserver account off', 'Storage limit…', 'Remove…']);
        assert.deepEqual(await person(CY).locator('.pa-acts .btn').allInnerTexts(), ['Rename', 'Switch music on', 'Switch homeserver account off', 'Storage limit…']);
        // A list or a box that opens in a row is written after the row's <p>, not inside it. (Inside,
        // the parser ends the <p> early and leaves an empty <p> behind it: that is what shows it.)
        const strayP = async (when) => {
          assert.equal(await box.locator('p:empty:not(.field-err)').count(), 0, `no <p> the parser had to end early (${when})`);
          assert.equal(await box.locator('.pa-fold').count(), await box.locator(':is(.set-label, .set-body) > .pa-fold').count(), `each fold is its row’s own child (${when})`);
        };
        assert.ok(await box.locator('.pa-fold').count() >= 5, 'the people’s folds are there');
        await strayP('the people');
        await cleanCopy(page);
        await shot(page, width, `owner-${width}.png`);

        // Their sign-ins: how many, then the list with what each said it is, since when, and from where
        assert.equal(await person(BO).locator('[data-pnone]').innerText(), 'Not signed in anywhere.');
        assert.equal(await person(CY).locator('[data-pnone], [data-psi]').count(), 0, 'no sign-in line for someone without music');
        const anaList = person(ANA).locator('details[data-keepopen^="si:"]');
        assert.equal(await anaList.locator('summary').innerText(), 'Signed in on 2 browsers or apps');
        assert.equal(await anaList.locator('[data-psi]').first().isVisible(), false, 'the list opens when asked');
        await anaList.locator('summary').click();
        const a1 = await anaList.locator('[data-psi="a1"]').innerText();
        assert.equal(day(T0), '11/20/2026', 'the pinned date');
        assert.match(a1, /Firefox on Linux/); assert.ok(a1.includes('since 11/20/2026 · last used 11/20/2026'), a1);
        assert.doesNotMatch(a1, /away from home/, 'a sign-in from the node’s own network says nothing about where');
        const a2 = await anaList.locator('[data-psi="a2"]').innerText();
        assert.match(a2, /A browser or app/); assert.ok(a2.includes('since 11/17/2026 · signed in away from home'), a2); assert.doesNotMatch(a2, /last used/);
        // Music off: their sign-ins are listed (they can still be ended), with no promise that they work again
        const diFold = await person(DI).locator('details[data-keepopen^="si:"] summary').innerText();
        assert.equal(diFold, 'Signed in on 1 browser or app (not usable while music is off)');
        // Sign out one: the right call, once; the row goes, the list stays open, the focus is on the next one
        await anaList.locator('[data-psi-end="a2"]').click();
        await anaList.locator('[data-psi="a2"]').waitFor({ state: 'detached' });
        await settled();
        assert.equal(calls(n, `DELETE /people/${ANA}/sign-ins/a2`).length, 1);
        assert.equal(await anaList.locator('summary').innerText(), 'Signed in on 1 browser or app');
        assert.equal(await anaList.locator('[data-psi="a1"]').isVisible(), true, 'the list stays open');
        assert.equal(await page.evaluate(() => document.activeElement && document.activeElement.dataset.psiEnd), 'a1', 'the focus goes to the sign-in left');
        // Sign out everywhere asks first
        await anaList.locator('[data-psi-all]').click();
        assert.match(await dialog.innerText(), /Sign Ana out everywhere\?[\s\S]*log in again with Pubky Ring/);
        await dialog.locator('[data-ci="-1"]').click();
        await settled();
        assert.equal(calls(n, `DELETE /people/${ANA}/sign-ins`).length, 0, 'Cancel sends nothing');
        await anaList.locator('[data-psi-all]').click();
        await dialog.locator('.choice').click();
        await person(ANA).locator('[data-pnone]').waitFor();
        await settled();
        assert.equal(calls(n, `DELETE /people/${ANA}/sign-ins`).length, 1);

        // Music off asks first, in words that say what the node does: their pages go back to Log in and
        // their devices are forgotten. It neither says they are signed out everywhere (their sign-ins
        // are kept) nor promises that anything keeps working or works again. Sent once.
        await person(ANA).locator('[data-pmusic]').click();
        const offText = await dialog.innerText();
        assert.match(offText, /Switch music off for Ana\?/);
        assert.match(offText, /They can’t use this node until you switch it back on\. Their browsers and apps go back to Log in, and the node forgets their devices and what those keep for offline\./);
        assert.doesNotMatch(offText, /signed out everywhere|work again|keeps? working|sign-ins stay/i, 'no promise the node does not keep');
        await dialog.locator('[data-ci="-1"]').click();
        await settled();
        assert.equal(calls(n, `PATCH /people/${ANA}`).length, 0, 'Cancel sends nothing');
        assert.equal(await page.evaluate(() => document.activeElement && document.activeElement.dataset.pmusic), ANA, 'after Cancel the focus is back on the button');
        await person(ANA).locator('[data-pmusic]').click();
        await dialog.locator('.choice').click();
        await toast('Music is off for Ana');
        assert.equal((await page.locator('#toast').innerText()).trim(), 'Music is off for Ana', 'the result says that and no more');
        await until(async () => (await says(ANA)) === 'Music is switched off for them.', 'the sentence after Music off');
        await settled();
        assert.deepEqual(calls(n, `PATCH /people/${ANA}`).map((c) => c.body), [{ music: false }]);
        assert.equal(await page.evaluate(() => document.activeElement && document.activeElement.dataset.pmusic), ANA, 'the focus stays on the person’s Music button');
        // Music on needs no question; pressed twice, it is sent once
        await person(DI).locator('[data-pmusic]').dblclick();
        await until(async () => /^Plays music here/.test(await says(DI)), 'the sentence after Music on');
        await settled();
        assert.deepEqual(calls(n, `PATCH /people/${DI}`).map((c) => c.body), [{ music: true }], 'sent once');
        assert.equal(await dialog.count(), 0);

        // Rename: a field with the focus; Enter saves
        await person(BO).locator('[data-prename]').click();
        assert.equal(await page.evaluate(() => document.activeElement && document.activeElement.matches('dialog [data-field]')), true, 'the field has the focus');
        await page.keyboard.type('Bo'); await page.keyboard.press('Enter');
        await until(async () => (await person(BO).locator('.set-name').innerText()) === 'Bo', 'the new name');
        assert.deepEqual(calls(n, `PATCH /people/${BO}`).map((c) => c.body), [{ name: 'Bo' }]);
        // The storage limit: emptying the field is not "no limit"; "No limit" is its own choice
        await person(BO).locator('[data-pquota]').click();
        assert.deepEqual(await dialog.locator('.choice-label').allInnerTexts(), ['Set this limit', 'No limit', 'The homeserver’s usual limit']);
        assert.doesNotMatch(await dialog.innerText(), /is not a limit|No limit was typed/);
        await dialog.locator('[data-field]').fill('');
        await dialog.locator('.choice').first().click();
        // Asked again, and what was wrong is said in the dialog itself (not in a toast behind it)
        await dialog.locator('p', { hasText: 'No limit was typed: type a whole number of MB, like 500, or choose one of the other two.' }).waitFor();
        await dialog.locator('[data-field]').fill('lots');
        await dialog.locator('.choice').first().click();
        await dialog.locator('p', { hasText: '“lots” is not a limit: type a whole number of MB' }).waitFor();
        assert.equal(await dialog.locator('[data-field]').inputValue(), 'lots', 'what was typed is still in the field');
        assert.doesNotMatch(await page.locator('#toast').innerText(), /limit|whole number/i, 'not said in a toast behind the dialog');
        await settled();
        assert.equal(calls(n, `PATCH /people/${BO}`).length, 1, 'a limit that isn’t one sends nothing');
        await dialog.locator('.choice', { hasText: 'No limit' }).click();
        await until(async () => /no limit\./.test(await person(BO).locator('[data-pstore]').innerText()), 'the storage line after No limit');
        assert.deepEqual(calls(n, `PATCH /people/${BO}`).at(-1).body, { storage_quota_mb: 'unlimited' });
        await person(BO).locator('[data-pquota]').click();
        await dialog.locator('[data-field]').fill('750');
        await dialog.locator('.choice').first().click();
        await until(async () => /a limit of 750 MB\./.test(await person(BO).locator('[data-pstore]').innerText()), 'the storage line after a limit');
        assert.deepEqual(calls(n, `PATCH /people/${BO}`).at(-1).body, { storage_quota_mb: 750 });
        // The homeserver account off asks first
        await person(BO).locator('[data-phs]').click();
        assert.match(await dialog.innerText(), /Switch the homeserver account off for Bo\?[\s\S]*can’t save anything new/);
        await dialog.locator('.choice').click();
        await until(async () => /account is switched off/.test(await person(BO).locator('[data-pstore]').innerText()), 'the storage line after the account is off');
        assert.deepEqual(calls(n, `PATCH /people/${BO}`).at(-1).body, { homeserver_enabled: false });

        // Remove asks first, says what goes, and is sent once; the row goes and the focus stays on the screen
        await person(DI).locator('[data-premove]').click();
        const rmText = await dialog.innerText();
        assert.match(rmText, /Remove Di\?/);
        assert.match(rmText, /They are signed out everywhere, and what this node keeps for them goes: their music service sign-ins here, preferences, devices and problem reports\. Their Pubky account and what is on their own homeserver are not touched\./);
        assert.doesNotMatch(rmText, /stay in this list/);
        await settled();
        assert.equal(calls(n, `DELETE /people/${DI}`).length, 0, 'nothing is sent before the answer');
        await dialog.locator('[data-ci="-1"]').click();
        await settled();
        assert.equal(calls(n, `DELETE /people/${DI}`).length, 0, 'Cancel sends nothing');
        assert.equal(await person(DI).count(), 1);
        // A removal that fails after the node took them off its list: they stay on the screen with what
        // the node said, and Remove is still there, since removing again finishes it
        n.half[DI] = 'their devices could not be cleared';
        await person(DI).locator('[data-premove]').click();
        await dialog.locator('.choice').click();
        await toast('Removing Di did not finish: their devices could not be cleared');
        await person(DI).locator('[data-premove-err]').waitFor();
        await settled();
        assert.equal(calls(n, `DELETE /people/${DI}`).length, 1, 'sent once');
        assert.equal(n.people.some((p) => p.key === DI), false, 'the node no longer lists them');
        assert.match(await person(DI).innerText(), /^Di\s+Removing them did not finish: their devices could not be cleared Remove again to finish it\.\s+Remove…$/);
        assert.equal(await page.evaluate(() => document.activeElement && document.activeElement.dataset.premove), DI, 'the focus stays on their Remove');
        await person(DI).locator('[data-premove]').click();
        await dialog.locator('.choice').click();
        await toast('Removed Di');
        await person(DI).waitFor({ state: 'detached' });
        await settled();
        assert.equal(calls(n, `DELETE /people/${DI}`).length, 2, 'the second Remove is sent, once');
        assert.equal(await page.evaluate(() => { const a = document.activeElement; return !!a && a !== document.body && !!a.closest('#peoplebox'); }), true, 'the focus is on the screen after the row is gone');
        // Someone with an account on this node's homeserver stays, without music: said before, shown after.
        // Their removal failing too: Remove stays on them (it would otherwise go with their music access)
        n.half[BO] = 'their sign-ins at services could not be forgotten';
        await person(BO).locator('[data-premove]').click();
        assert.match(await dialog.innerText(), /Their account on this node’s homeserver stays, so they stay in this list without music\./);
        await dialog.locator('.choice').click();
        await until(async () => (await says(BO)) === 'Has storage on this node’s homeserver. No music.', 'the sentence after their music access is removed');
        assert.match(await person(BO).locator('[data-premove-err]').innerText(), /Removing them did not finish: their sign-ins at services could not be forgotten Remove again to finish it\./);
        assert.deepEqual(await person(BO).locator('.pa-acts .btn').allInnerTexts(), ['Rename', 'Switch music on', 'Switch homeserver account on', 'Storage limit…', 'Remove…']);
        await person(BO).locator('[data-premove]').click();
        await dialog.locator('.choice').click();
        await person(BO).locator('[data-premove-err]').waitFor({ state: 'detached' });
        assert.equal(calls(n, `DELETE /people/${BO}`).length, 2);
        assert.deepEqual(await person(BO).locator('.pa-acts .btn').allInnerTexts(), ['Rename', 'Switch music on', 'Switch homeserver account on', 'Storage limit…'], 'nothing left to remove: no Remove');

        // 2. Add someone: three plain choices; an invite not used yet can be shown again or taken off the list
        await openSec('invite');
        assert.deepEqual(await box.locator('.ways .way-name').allInnerTexts(), ['Homeserver account and music', 'Homeserver account only', 'Music only']);
        const eve = box.locator('[data-pinv="inv1"]');
        assert.match(await eve.innerText(), /Eve[\s\S]*Homeserver account and music · made/);
        assert.match(await box.locator('[data-pinv-head]').innerText(), /an invite already sent stays usable on the homeserver/);
        assert.equal(await eve.locator('.qrbox').isVisible(), false);
        await eve.locator('summary', { hasText: 'Show the invite again' }).click();
        assert.equal(await eve.locator('.qrbox').isVisible(), true);
        await strayP('with an invite shown again');
        await cleanCopy(page); // the long link and the signup code wrap inside the box
        await box.locator('.way', { hasText: 'Music only' }).click();
        assert.equal((await box.locator('#pinvgo').innerText()).trim(), 'Add them');
        await box.locator('#pinvname').fill('Fay'); await box.locator('#pinvkey').fill(FAY);
        await box.locator('#pinvgo').dblclick();
        await toast('Added Fay: send them this node’s address');
        await person(FAY).waitFor();
        await settled();
        assert.deepEqual(calls(n, 'POST /people/invites').map((c) => c.body), [{ access: 'music', name: 'Fay', key: FAY }], 'sent once');
        // What to send them: this node's address and how they log in, with Copy
        const sent = box.locator('#pinvout');
        assert.match(await sent.innerText(), new RegExp(`Fay can log in now\\.[\\s\\S]*Open ${ORIGIN.replace(/\./g, '\\.')} and log in with Pubky Ring\\.`));
        assert.equal(await sent.locator('[data-copylink]').getAttribute('data-copylink'), `Open ${ORIGIN} and log in with Pubky Ring.`);
        assert.equal(await box.locator('#pinvname').inputValue(), '', 'the form starts again');
        await eve.locator('[data-pinv-del]').click();
        await toast('Invite taken off the list');
        await eve.waitFor({ state: 'detached' });
        assert.equal(calls(n, 'DELETE /people/invites/inv1').length, 1);
        // What Add someone said goes when it no longer holds: with an invite taken off the list, and
        // with a person removed
        assert.equal((await sent.innerText()).trim(), '', 'nothing left to send after an invite is taken off');
        await box.locator('.way', { hasText: 'Music only' }).click();
        await box.locator('#pinvname').fill('Gus'); await box.locator('#pinvkey').fill(GUS);
        await box.locator('#pinvgo').click();
        await sent.locator('[data-copylink]').waitFor();
        assert.match(await sent.innerText(), /Gus can log in now\./);
        await person(GUS).locator('[data-premove]').click();
        await dialog.locator('.choice').click();
        await toast('Removed Gus');
        await person(GUS).waitFor({ state: 'detached' });
        assert.equal((await sent.innerText()).trim(), '', 'nothing left to send about someone who was removed');

        // 3. What people you add can do: plain words, Bookmarks and Collections
        await openSec('cando');
        const cando = await box.locator('#pcando').innerText();
        assert.match(cando, /They can\s+Play and search this node’s library\s+Use the zones you open to them\s+Keep their own bookmarks and collections, on their own Pubky account\s+Sign in to their own TIDAL, Qobuz or Spotify\s+Report a problem/);
        assert.match(cando, /They can’t\s+Change settings\s+Add, change or remove music\s+See your bookmarks, collections, tags or history\s+Add or remove people\s+Back up or restore\s+Change zones and speakers/);

        // 4. Zones: "Only these" with none ticked is refused and sends nothing
        await openSec('zones');
        assert.equal(await box.locator('input[name="pzmode"][value="all"]').isChecked(), true);
        assert.equal(await box.locator('#pzones').isVisible(), false, 'the zones show under “Only these”');
        await box.locator('label.check', { hasText: 'Only these' }).click();
        assert.deepEqual((await box.locator('#pzones label').allInnerTexts()).map((t) => t.trim()), ['Kitchen', 'Den', 'Hall']);
        await box.locator('#pzsave').click();
        await box.locator('#pzerr', { hasText: 'Tick at least one zone' }).waitFor();
        await settled();
        assert.equal(calls(n, 'PUT /people/zones').length, 0, 'nothing sent with none ticked');
        await box.locator('#pzones label', { hasText: 'Kitchen' }).click(); await box.locator('#pzones label', { hasText: 'Den' }).click();
        assert.equal(await box.locator('#pzerr').isVisible(), false);
        // Busy at once while it saves; pressed twice, sent once
        let release; n.gate['PUT /people/zones'] = new Promise((res) => { release = res; });
        await box.locator('#pzsave').dblclick();
        await box.locator('#pzsave[aria-busy="true"]').waitFor();
        release(); delete n.gate['PUT /people/zones'];
        await until(async () => (await says(ANA)) === 'Music is switched off for them.' && (await says(FAY)) === 'Plays music here, on Kitchen and Den, with their own TIDAL, Qobuz or Spotify.', 'the sentence names the zones');
        await settled();
        assert.deepEqual(calls(n, 'PUT /people/zones').map((c) => c.body), [{ zones: ['kitchen', 'den'] }], 'sent once');
        assert.equal(await box.locator('input[name="pzmode"][value="some"]').isChecked(), true);
        await box.locator('label.check', { hasText: 'Every zone' }).click();
        await box.locator('#pzsave').click();
        await until(async () => /on every zone/.test(await says(FAY)), 'every zone again');
        assert.deepEqual(calls(n, 'PUT /people/zones').at(-1).body, { zones: [] }, 'every zone is the empty list');
        // No zone at all is the third choice: they then play in their own browser or app only
        assert.deepEqual((await box.locator('.pa-zmode label').allInnerTexts()).map((t) => t.trim()), ['Every zone', 'Only these', 'No zone']);
        assert.equal(await box.locator('#pznonehelp').isVisible(), false);
        await box.locator('label.check', { hasText: 'No zone' }).click();
        assert.equal(await box.locator('#pznonehelp').innerText(), 'They play in their own browser or app only.');
        assert.equal(await box.locator('#pzones').isVisible(), false);
        const putsBefore = calls(n, 'PUT /people/zones').length;
        await box.locator('#pzsave').dblclick();
        await toast('The people you add play in their own browser or app only');
        await until(async () => (await says(FAY)) === 'Plays music here in their own browser or app only, with their own TIDAL, Qobuz or Spotify.', 'the sentence under “No zone”');
        await settled();
        // Sent once, as `none` next to the list of one zone without a name: a node before `none`
        // keeps that list and opens no zone either (an empty list would be every zone there)
        assert.deepEqual(calls(n, 'PUT /people/zones').slice(putsBefore).map((c) => c.body), [{ zones: [''], none: true }], 'sent once, as no zone');
        if (phone) assert.deepEqual(n.zones, [''], 'an older node keeps the list that opens no zone');
        else assert.deepEqual([n.zones, n.none], [[], true], 'the node has “no zone”');
        assert.equal(await box.locator('input[name="pzmode"][value="none"]').isChecked(), true, 'the choice is shown as saved');
        assert.equal(await box.locator('#pznonehelp').isVisible(), true);
        assert.equal(await box.locator('#pzones').isVisible(), false);
        assert.equal(await box.locator('#pzones input:checked').count(), 0, 'no zone is ticked, and none without a name is listed');
        assert.equal(await box.locator('#pzones input').count(), ZONES.length);
        // What they can do follows: no zone is something they can't do
        if (!phone) {
          const [canNow, cantNow] = (await box.locator('#pcando').innerText()).split('They can’t');
          assert.doesNotMatch(canNow, /zone/i, 'nothing about zones under “They can”');
          assert.match(cantNow, /^\s+Play on a zone: none is open to them\s+Change settings/);
        }
        // Every zone again is the empty list without `none`
        await box.locator('label.check', { hasText: 'Every zone' }).click();
        await box.locator('#pzsave').click();
        await until(async () => /^Plays music here, on every zone,/.test(await says(FAY)), 'every zone after no zone');
        assert.deepEqual(calls(n, 'PUT /people/zones').at(-1).body, { zones: [] }, 'every zone: the empty list, without `none`');
        assert.deepEqual([n.zones, !!n.none], [[], false], 'the node no longer has “no zone”');
        if (!phone) assert.match(await box.locator('#pcando').innerText(), /Use the zones you open to them/);

        // 5. Whose music services: the three choices, here; each one's words in the sentence
        await openSec('services');
        const chips = box.locator('[data-srcpolicy]');
        assert.equal(await chips.count(), 3);
        assert.equal(await box.locator('[data-srcpolicy="own"]').getAttribute('aria-checked'), 'true');
        await box.locator('[data-srcpolicy="choice"]').click();
        await toast('Each person chooses');
        await until(async () => (await says(FAY)) === 'Plays music here, on every zone, with this node’s music services or their own.', 'the sentence under “They choose”');
        assert.deepEqual(calls(n, 'PUT /node/settings').map((c) => c.body), [{ sources_policy: 'choice' }]);
        assert.equal(await box.locator('[data-srcpolicy="choice"]').getAttribute('aria-checked'), 'true');
        await box.locator('[data-srcpolicy="node"]').click();
        await until(async () => (await says(FAY)) === 'Plays music here, on every zone, with this node’s music services.', 'the sentence under “This node’s”');
        assert.doesNotMatch(await box.locator('#pcando').innerText(), /Sign in to their own/, 'not listed while the node plays everyone through its own');
        assert.equal(await page.locator('#set-apps [data-srcpolicy]').count(), 0, 'the choice is no longer on Service apps');
        assert.equal(await page.locator('#set-apps [data-settab="people"]').count(), 1, 'Service apps points here');

        // 6. Browsers and apps signed in as you: this one first with Log out, then the others
        await openSec('mine');
        const mine = box.locator('#pmine');
        const here = mine.locator('[data-si-here]');
        assert.match(await here.innerText(), /This browser[\s\S]*Chromium on Linux · since/);
        assert.equal((await here.locator('#logout').innerText()).trim(), 'Log out');
        assert.equal(await page.locator('#set-about #logout, #set-about #tokennew').count(), 0, 'Log out and the password are no longer on About');
        assert.equal(await page.locator('#set-about [data-settab="people"]').count(), 1, 'About points here');
        const m2 = await mine.locator('[data-si="m2"]').innerText();
        assert.match(m2, /Odeion app 0\.2\.4/); assert.ok(m2.includes('since 11/19/2026 · signed in away from home'), m2);
        assert.match(await mine.locator('[data-si="m3"]').innerText(), /A browser/);
        await mine.locator('[data-si-end="m2"]').click();
        await mine.locator('[data-si="m2"]').waitFor({ state: 'detached' });
        await settled();
        assert.equal(calls(n, 'DELETE /me/sign-ins/m2').length, 1);
        assert.equal(await mine.locator('[data-si="m3"]').count(), 1);
        assert.equal(await page.evaluate(() => document.activeElement && document.activeElement.dataset.siEnd), 'm3', 'the focus goes to the next one’s Sign out');
        await mine.locator('[data-si-all]').click();
        assert.match(await dialog.innerText(), /Sign out all other browsers and apps\?[\s\S]*This browser stays logged in\./);
        await dialog.locator('.choice').click();
        await mine.locator('[data-si-none]', { hasText: 'Only this one.' }).waitFor();
        await settled();
        assert.equal(calls(n, 'DELETE /me/sign-ins').length, 1);
        assert.equal(await mine.locator('[data-si-all]').count(), 0, 'nothing to sign out when there are no others');

        // 8. Party: a list that can't be read says so, with Retry (not "no parties")
        await openSec('party');
        await box.locator('#ptgo').click();
        const party = box.locator('[data-party="p1"]');
        await party.waitFor();
        assert.equal(await party.locator('details[open] .qrbox').isVisible(), true, 'the new party’s guest link is open');
        await cleanCopy(page);
        n.fail['GET /party'] = 'the parties could not be read';
        await party.locator('[data-party-end]').click();
        const partyErr = box.locator(phone ? 'details[data-fold="party"] [data-pa-error]' : '[data-pa-error]');
        await partyErr.waitFor();
        assert.match(await partyErr.innerText(), /Couldn’t read the parties: the parties could not be read/);
        assert.equal(await box.locator('#ptgo').count(), 1, 'a party can still be started');
        delete n.fail['GET /party'];
        await partyErr.locator('[data-pa-retry]').click();
        await partyErr.waitFor({ state: 'detached' });
        assert.equal(await box.locator('[data-party]').count(), 0, 'the party that was ended is gone');

        // 9. The two ways on: the node's own identity, here; remote access only where it can be set up
        assert.equal(await box.locator('[data-settab="accounts"]').count(), 1);
        assert.equal(await box.locator('[data-settab="remote"]').count(), 0, 'no row for a tab this node doesn’t have');

        // The words: no jargon in what a person reads, Bookmarks and Collections
        for (const id of ['who', 'invite', 'cando', 'zones', 'services', 'mine', 'password', 'party']) await openSec(id);
        plainWords(await box.innerText(), 'the owner’s screen');
        await cleanCopy(page);
        await shot(page, width, `owner-${width}-all-sections.png`);
        if (phone) {
          // Every control is a 44 px tap
          const small = await box.evaluate((b) => [...b.querySelectorAll('button, a.btn, summary, label.check, label.chip, select, input.input')].filter((e) => e.getClientRects().length).map((e) => [e.getBoundingClientRect().height, (e.innerText || e.getAttribute('aria-label') || e.id || e.tagName).trim().slice(0, 40)]).filter(([h]) => h < 43.5));
          assert.deepEqual(small, [], '44 px touch targets');
        }
        // Every control is reachable with the keyboard
        const unreachable = await box.evaluate((b) => [...b.querySelectorAll('button, a[href], summary, input, select')].filter((e) => e.getClientRects().length && !e.disabled && e.tabIndex < 0).map((e) => e.outerHTML.slice(0, 80)));
        assert.deepEqual(unreachable, [], 'no control out of the tab order');

        // 7. The password for apps is here: made once, shown once
        await openSec('password');
        assert.equal(await box.locator('#tokennew').count(), 1);
        await box.locator('#tokennew').click();
        assert.match(await dialog.innerText(), /Make a new password for apps\?[\s\S]*The old one stops working at once/);
        await dialog.locator('.choice').click();
        await box.locator('#tokenout code').waitFor();
        // The node ended the owner's other browser log ins with it: the screen's lists are read again,
        // and the new password stays shown
        const sinceNew = (call) => n.calls.slice(n.calls.findIndex((c) => c.call === 'POST /node/token') + 1).some((c) => c.call === call);
        await until(() => sinceNew('GET /me/sign-ins') && sinceNew('GET /people'), 'the lists read again after the new password');
        await until(async () => (await page.evaluate(() => document.activeElement && document.activeElement.id)) === 'tokendone', 'the focus on Copy and reload');
        await settled();
        assert.equal(calls(n, 'POST /node/token').length, 1);
        assert.equal(await box.locator('#tokenout code').innerText(), NEW_TOKEN, 'the new password is still shown after the lists were read again');
        assert.equal(await box.locator('#tokennew').isDisabled(), true, 'one new password per visit');
        assert.equal(await page.evaluate(() => localStorage.getItem('pm.token')), NEW_TOKEN, 'this browser goes on with the new one');
        await cleanCopy(page);
        await saidWords(page, 'the owner’s dialogs and toasts', 20);
        assert.deepEqual(state.errors, [], 'no page errors');
        await context.close();
      }
      // ---- the owner, the node's own states ----
      {
        // The people can't be read: said in their place with Retry; the rest of the screen works
        const n = node({ replaceable: false, exposed: T0, fail: { 'GET /people': 'the people list can’t be read' } });
        // (its own sign-ins wait, to see them loading)
        let release; n.gate['GET /me/sign-ins'] = new Promise((res) => { release = res; });
        const { context, page, state } = await owner(browser, width, n);
        const box = page.locator('#peoplebox');
        const err = box.locator('[data-pa-error]');
        await err.waitFor();
        assert.match(await err.innerText(), /Couldn’t read who else can use this node: the people list can’t be read/);
        assert.equal(await box.locator('[data-person], #pinvgo').count(), 0, 'no people and no form on an answer that failed');
        if (phone) for (const id of ['mine', 'password']) await box.locator(`details[data-fold="${id}"]`).evaluate((d) => { d.open = true; });
        assert.equal(await box.locator('#pmine [data-si-loading]').count(), 1, 'the sign-ins say they are loading');
        assert.equal(await box.locator('#pmine #logout').isVisible(), true, 'Log out is there meanwhile');
        // Its own sign-ins fail too: said, with Retry
        n.fail['GET /me/sign-ins'] = 'the sign-ins could not be read';
        release(); delete n.gate['GET /me/sign-ins'];
        const siErr = box.locator('#pmine [data-si-error]');
        await siErr.waitFor();
        assert.match(await siErr.innerText(), /Couldn’t read the other sign-ins: the sign-ins could not be read/);
        delete n.fail['GET /me/sign-ins'];
        await siErr.locator('[data-si-retry]').click();
        await box.locator('#pmine [data-si="m2"]').waitFor();
        // A password set in the config is changed there; one that leaked says so
        assert.equal(await box.locator('#tokennew').count(), 0);
        const pass = await box.locator(phone ? 'details[data-fold="password"]' : '.set-group:has(#tokenout), .set-group:has(a[href*="troubleshooting.md"])').innerText();
        assert.match(pass, /Set in the node’s config/);
        assert.match(pass, /This password was in a saved queue’s links/);
        assert.equal(await page.locator('#main a[href*="troubleshooting.md"]').count(), 1);
        await cleanCopy(page);
        // Retry reads the people again
        delete n.fail['GET /people'];
        await err.locator('[data-pa-retry]').click();
        await box.locator(`[data-person="${ANA}"]`).waitFor();
        assert.equal(await err.count(), 0);
        assert.equal(await box.locator('#pinvgo').count(), 1);
        // Zones that can't be read: nobody plays on "every zone" by mistake
        n.fail['GET /people/zones'] = 'the zone list can’t be read';
        await box.locator(`[data-person="${ANA}"] [data-prename]`).click();
        await page.keyboard.type('!'); await page.keyboard.press('Enter');
        await until(async () => (await box.locator(`[data-person="${ANA}"] [data-psays]`).innerText()) === 'Plays music here, on no zone until the zones are saved again, with their own TIDAL, Qobuz or Spotify.', 'the sentence while the zones can’t be read');
        if (phone) await box.locator('details[data-fold="zones"]').evaluate((d) => { d.open = true; });
        assert.equal(await box.locator('input[name="pzmode"]:checked').count(), 0, 'neither choice is taken for them');
        await box.locator('#pzsave').click();
        await box.locator('#pzerr', { hasText: 'Choose “Every zone”, “Only these” or “No zone” first.' }).waitFor();
        assert.equal(calls(n, 'PUT /people/zones').length, 0);
        await cleanCopy(page);
        assert.deepEqual(state.errors, [], 'no page errors');
        await context.close();
      }
      // The node's settings can't be read: no choice is shown as the one in use and nothing is said
      // about the password; both say so, with Retry
      {
        const n = node({ fail: { 'GET /node/settings': 'the settings could not be read' } });
        const { context, page, state } = await owner(browser, width, n);
        const box = page.locator('#peoplebox');
        await box.locator(`[data-person="${ANA}"]`).waitFor();
        assert.equal(await box.locator(`[data-person="${ANA}"] [data-psays]`).innerText(), 'Plays music here, on every zone.', 'no guess at whose services');
        if (phone) for (const id of ['services', 'password']) await box.locator(`details[data-fold="${id}"]`).evaluate((d) => { d.open = true; });
        assert.equal(await box.locator('[data-srcpolicy], #tokennew').count(), 0);
        assert.doesNotMatch(await box.innerText(), /Set in the node’s config/);
        const errs = box.locator('[data-pa-error]', { hasText: 'Couldn’t read this node’s settings: the settings could not be read' });
        assert.equal(await errs.count(), 2, 'said for the music services and for the password');
        await cleanCopy(page);
        delete n.fail['GET /node/settings'];
        await errs.first().locator('[data-pa-reload]').click();
        await until(async () => (await page.locator('#peoplebox [data-pa-error]').count()) === 0 && (await page.locator('#peoplebox [data-srcpolicy]').count()) === 3, 'the settings read again');
        assert.equal(await page.locator('#peoplebox [data-srcpolicy="own"]').getAttribute('aria-checked'), 'true');
        assert.deepEqual(state.errors, [], 'no page errors');
        await context.close();
      }
      // A removal that did not finish on an earlier visit: the node remembers it, so the person is
      // still on the screen (off the node's list, without a name) with Remove, which finishes it; and
      // when nothing of theirs is left after all, that is said and the row goes
      if (!phone) {
        for (const nothingLeft of [false, true]) {
          const n = node({ leftover: new Set([DI]), nothingLeft });
          n.people = n.people.filter((p) => p.key !== DI);
          const { context, page, state } = await owner(browser, width, n);
          const box = page.locator('#peoplebox');
          const gone = box.locator(`[data-person="${DI}"][data-pgone]`);
          await gone.waitFor();
          assert.match(await gone.innerText(), /^Someone you removed \(key ybndrf…[a-z0-9]{10}\)\s+Removing them did not finish\. Remove again to finish it\.\s+Remove…$/);
          assert.equal(await gone.locator('[data-premove-err]').getAttribute('role'), null, 'not announced again on every draw');
          await gone.locator('[data-premove]').click();
          await page.locator('dialog.dialog .choice').click();
          await page.locator('#toast', { hasText: nothingLeft ? /^Nothing of this person is left on this node$/ : /^Removed this person$/ }).waitFor();
          await gone.waitFor({ state: 'detached' });
          await sleep(150);
          assert.equal(calls(n, `DELETE /people/${DI}`).length, 1, 'sent once');
          assert.equal(await box.locator('[data-premove-err]').count(), 0, 'nothing is left to finish');
          await cleanCopy(page);
          assert.deepEqual(state.errors, [], 'no page errors');
          await context.close();
        }
      }
      // The same for someone who is still listed (a homeserver account, no sign-in left): Remove is
      // offered on their own row, which otherwise has none
      if (!phone) {
        const n = node({ leftover: new Set([CY]) });
        const { context, page, state } = await owner(browser, width, n);
        const box = page.locator('#peoplebox');
        const cy = box.locator(`[data-person="${CY}"]`);
        await cy.locator('[data-premove-err]').waitFor();
        assert.equal(await cy.locator('[data-premove-err]').innerText(), 'Removing them did not finish. Remove again to finish it.');
        assert.equal(await cy.locator('[data-premove]').count(), 1, 'Remove is offered though they have no sign-in left');
        assert.equal(await box.locator('[data-pgone]').count(), 0, 'no second row: they are listed');
        assert.equal(await box.locator(`[data-person="${BO}"] [data-premove-err]`).count(), 0, 'nobody else is marked');
        await cleanCopy(page);
        assert.deepEqual(state.errors, [], 'no page errors');
        await context.close();
      }
      // A node that answers "no zone" and does not keep it: the screen is drawn again from what the
      // node has now, and says so where the choice is (no word of a change that was not made)
      if (!phone) {
        const n = node({ dropsNone: true, zones: ['kitchen'] });
        const { context, page, state } = await owner(browser, width, n);
        const box = page.locator('#peoplebox');
        const says = () => box.locator(`[data-person="${ANA}"] [data-psays]`).innerText();
        await box.locator(`[data-person="${ANA}"]`).waitFor();
        assert.equal(await says(), 'Plays music here, on Kitchen, with their own TIDAL, Qobuz or Spotify.');
        assert.equal(await box.locator('input[name="pzmode"][value="some"]').isChecked(), true);
        await box.locator('label.check', { hasText: 'No zone' }).click();
        await box.locator('#pzsave').click();
        await box.locator('#pzerr', { hasText: 'This node did not keep “No zone”. The choice above is what it has now.' }).waitFor();
        await sleep(150);
        assert.deepEqual(calls(n, 'PUT /people/zones').map((c) => c.body), [{ zones: [''], none: true }], 'asked once');
        assert.equal(await box.locator('input[name="pzmode"][value="all"]').isChecked(), true, 'the choice shown is the node’s');
        assert.equal(await says(), 'Plays music here, on every zone, with their own TIDAL, Qobuz or Spotify.');
        assert.equal(await page.locator('#toast', { hasText: 'own browser or app only' }).count(), 0, 'no word of a change that was not kept');
        await cleanCopy(page);
        assert.deepEqual(state.errors, [], 'no page errors');
        await context.close();
      }
      // About's row opens this screen (on a computer in place, with the focus on its tab)
      if (!phone) {
        const n = node();
        const { context, page, state } = await owner(browser, width, n, { at: '/settings/about' });
        await page.locator('#set-about [data-settab="people"]').click();
        await page.locator(`#peoplebox [data-person="${ANA}"]`).waitFor();
        assert.equal(await page.locator('#set-people').isVisible(), true);
        assert.equal(await page.evaluate(() => document.activeElement && document.activeElement.id), 'settab-people');
        assert.deepEqual(state.errors, [], 'no page errors');
        await context.close();
      }
      // ---- someone the owner added: their own Settings ----
      // (their own services only where the node lets them; a zone, or none open to them right now)
      // (a node before "no zone" was a choice says it as a list of one zone without a name)
      for (const [ownSignIn, zones] of [[true, ['kitchen']], [false, ['kitchen']], [true, []], [true, ['']]]) {
        const n = node({ mine: [
          { id: 'l1', kind: 'person', label: 'Chromium on Linux', remote: false, created_at: T0, current: true },
          { id: 'l2', kind: 'person', label: 'Odeion app 0.2.4', remote: true, created_at: T0 - DAY, current: false },
        ] });
        const can = PERSON_CAN.filter((c) => ownSignIn || c !== 'own_sign_in');
        const { context, page, state } = await create(browser, width, {
          role: 'listener', touch: phone,
          fixtures: {
            '/me': { body: { role: 'listener', key: ANA, zones, can } },
            '/me/sources': { body: { policy: ownSignIn ? 'own' : 'node', use: ownSignIn ? 'own' : 'node', plays_with: ownSignIn ? 'own' : 'node', may_choose: false } },
            '/profiles': { body: [] }, '/zones': { body: ZONES },
          },
        });
        await keepSaid(context);
        await serve(context, n);
        await go(page, '/settings');
        const cando = page.locator('#lcando');
        await cando.waitFor();
        assert.equal(await page.locator('.set-group', { has: cando }).locator('.set-h').innerText().then((t) => t.toLowerCase()), 'what you can do here');
        const text = await cando.innerText();
        const [canText, cantText] = text.split('You can’t');
        if (zones.filter(Boolean).length) {
          assert.match(canText, /^You can\s+Play and search this node’s library\s+Use this zone: Kitchen\s+Keep your own bookmarks and collections, on your own Pubky account/);
          assert.match(cantText, /^\s+Change settings\s+Add, change or remove music\s+See the owner’s bookmarks, collections, tags or history\s+Add or remove people\s+Back up or restore\s+Change zones and speakers/);
        } else {
          // No zone open to them is something they can't do: not a tick under "You can"
          assert.match(canText, /^You can\s+Play and search this node’s library\s+Keep your own bookmarks and collections, on your own Pubky account/);
          assert.doesNotMatch(canText, /zone/i, 'nothing about zones under “You can”');
          assert.match(cantText, /^\s+Play on a zone: none is open to you right now, ask the node’s owner\s+Change settings/);
        }
        assert.equal(/Sign in to your own TIDAL, Qobuz or Spotify/.test(canText), ownSignIn, 'own services only where the node lets them');
        const mine = page.locator('#mysignins');
        assert.equal(await page.locator('.set-group', { has: mine }).locator('.set-h').innerText().then((t) => t.toLowerCase()), 'your sign-ins on this node');
        await mine.locator('[data-si="l2"]').waitFor();
        assert.match(await mine.locator('[data-si-here]').innerText(), /This browser[\s\S]*Chromium on Linux/);
        assert.match(await mine.locator('[data-si="l2"]').innerText(), /Odeion app 0\.2\.4[\s\S]*signed in away from home/);
        assert.equal(await mine.locator('[data-si-all]').count(), 1, 'Sign out all others');
        // None of the owner's controls
        assert.equal(await page.locator('#peoplebox, #tokennew, #logout, #pzsave, #ptgo, #pinvgo, [data-srcpolicy], [data-premove], [data-pmusic], .set-tab').count(), 0, 'no owner control');
        await cleanCopy(page);
        if (ownSignIn && zones.length) await shot(page, width, `listener-${width}.png`);
        if (phone) {
          const small = await mine.evaluate((b) => [...b.querySelectorAll('button')].filter((e) => e.getClientRects().length).map((e) => e.getBoundingClientRect().height).filter((h) => h < 43.5));
          assert.deepEqual(small, [], '44 px touch targets');
        }
        // The words of their two groups, and of the dialog and the toast they lead to
        plainWords(await page.locator('.set-group', { has: cando }).innerText(), 'What you can do here');
        plainWords(await page.locator('.set-group', { has: mine }).innerText(), 'Your sign-ins on this node');
        await mine.locator('[data-si-all]').click();
        assert.match(await page.locator('dialog.dialog').innerText(), /Sign out all other browsers and apps\?/);
        await page.locator('dialog.dialog [data-ci="-1"]').click();
        await mine.locator('[data-si-end="l2"]').click();
        await page.locator('#toast', { hasText: 'Signed out: Odeion app 0.2.4' }).waitFor();
        await mine.locator('[data-si-none]', { hasText: 'Only this one.' }).waitFor();
        await sleep(150);
        assert.equal(calls(n, 'DELETE /me/sign-ins/l2').length, 1);
        assert.equal(calls(n, 'DELETE /me/sign-ins').length, 0, 'Cancel sent nothing');
        await saidWords(page, 'a person’s dialogs and toasts', 2);
        assert.deepEqual(state.errors, [], 'no page errors');
        await context.close();
      }
      // ---- who the page takes the caller for ----
      if (!phone) {
        const health = [{ id: 'api-token', name: 'API token', state: 'change', detail: 'change it', ms: null }, { id: 'pinakes', name: 'Pinakes', state: 'down', detail: 'no answer', ms: null }];
        const GROUP = { ...zone('group:g1', 'Kitchen + Den'), output: 'upnp:kitchen' };
        const zonesWithGroup = async (context) => context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [...ZONES, GROUP] })); });
        const outputs = ZONES.map((z) => ({ id: z.output, name: z.name, kind: 'upnp', caps: { hw_volume: 'percent' } }));
        // The node could not say who this is (not a 404): the node is out of reach, and none of the
        // owner's controls show. (Before, any failure was read as the owner.)
        {
          const { context, page } = await create(browser, width, { fixtures: { '/me': { status: 500, body: { error: 'the node could not say who this is' } } } });
          await serve(context, node());
          await go(page, '/settings/people');
          const notice = page.locator('#main .notice', { hasText: 'Cannot reach the node' });
          await notice.waitFor();
          assert.match(await notice.innerText(), /Cannot reach the node: the node could not say who this is/);
          await sleep(300);
          assert.equal(await page.locator('#peoplebox, .set-tab, #logout, #tokennew, [data-person], [data-zone-del]').count(), 0, 'no owner control on a /me that failed');
          await context.close();
        }
        // A node with no such route (404: one without roles) is the owner's, as before
        {
          const { context, page, state } = await create(browser, width, { fixtures: { '/me': { status: 404, body: { error: 'not found' } }, '/zones': { body: ZONES }, '/node/apps': { body: [] } } });
          await serve(context, node());
          await go(page, '/settings/people');
          await page.locator(`#peoplebox [data-person="${ANA}"]`).waitFor();
          assert.equal(await page.locator('#peoplebox #logout').count(), 1);
          assert.deepEqual(state.errors, [], 'no page errors');
          await context.close();
        }
        // What the node says the caller can do decides the controls: Ungroup needs `zone_manage`, the
        // owner's chores in the status chip need `settings`
        for (const [who, me, ungroup, chip] of [
          ['the owner', { role: 'owner', key: null, zones: null, can: OWNER_CAN }, 1, /^Change the API token · 1 more$/],
          ['someone the owner added', { role: 'listener', key: ANA, zones: null, can: PERSON_CAN }, 0, /^Pinakes not answering$/],
        ]) {
          const { context, page, state } = await create(browser, width, { role: me.role, fixtures: { '/me': { body: me }, '/zones': { body: [...ZONES, GROUP] }, '/outputs': { body: outputs }, '/health': { body: health }, '/profiles': { body: [] } } });
          await zonesWithGroup(context);
          await go(page, '/zones');
          await page.locator('#main .zone-card[data-zcard="group:g1"]').waitFor();
          assert.equal(await page.locator('#main [data-zone-del]').count(), ungroup, `${who}: Ungroup`);
          const up = page.locator('#upstreams:not([hidden]) .up-text');
          await up.waitFor();
          assert.match((await up.innerText()).trim(), chip, `${who}: the status chip`);
          assert.deepEqual(state.errors, [], 'no page errors');
          await context.close();
        }
      }
      // ---- opened at an address of the node's own machine ----
      if (phone) {
        // After adding someone by their key there is nothing to copy: the page's own address only works
        // on the node's machine, and the page says to send the node's address on the network instead
        const n = node();
        const { context, page, state } = await owner(browser, width, n, { origin: LOOPBACK, before: atLoopback });
        const box = page.locator('#peoplebox');
        await box.locator(`[data-person="${ANA}"]`).waitFor();
        await box.locator('details[data-fold="invite"]').evaluate((d) => { d.open = true; });
        await box.locator('.way', { hasText: 'Music only' }).click();
        await box.locator('#pinvname').fill('Fay'); await box.locator('#pinvkey').fill(FAY);
        await box.locator('#pinvgo').click();
        await page.locator('#toast', { hasText: 'Added Fay: they log in at this node’s address on your network' }).waitFor();
        const sent = box.locator('#pinvout .pa-send');
        await sent.waitFor();
        const told = await sent.innerText();
        assert.match(told, /Fay can log in now\. This page is open at 127\.0\.0\.1:7788, an address that only works on the node’s own machine\.\s+Send them the node’s address on your network instead, and tell them to log in there with Pubky Ring\./);
        assert.doesNotMatch(told, /this computer|Open http/i);
        assert.equal(await box.locator('#pinvout [data-copylink]').count(), 0, 'no Copy for an address they can’t use');
        plainWords(told, 'what to send them');
        await cleanCopy(page);
        assert.deepEqual(state.errors, [], 'no page errors');
        await context.close();
      }
      console.log(`ok ${width}`);
    }
  } finally {
    await browser.close();
  }
})().catch((e) => { console.error(e); process.exit(1); });

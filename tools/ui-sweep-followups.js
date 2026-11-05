// The UX sweep's follow-ups (FIX-ux-sweep-2026-11-01.md, "Still open", rechecked on 2ddc70b), at
// 390, 430 (phones), 768, 834 (tablets, touch) and 1180 (a tablet in landscape); tablets have the
// desktop layout since 2026-11-04, with touch sizes:
//   * A7: a slow TIDAL collection says so under the page's title, draws its playlists as they come,
//     and shows the last good copy at once next time;
//   * A10: touch layouts give a playlist's Move and Remove, a song row's actions, Select and the
//     play-from order's grips 40 px;
//   * A16: an album's and a playlist's hero buttons stay on one row on touch screens, and a
//     playlist's Delete is in its ⋯ menu up to 1024 px;
//   * A17: phones have the back link only, the desktop layout (tablets too) the trail; a song opened
//     from its album of the same name says Back on a phone;
//   * A19: a tablet's Now playing tabs all show, and its sound pill keeps its words;
//   * A21: a song's File is its name, the path a tooltip;
//   * A25: long notes in Service apps fold behind More, and a sign-in way not chosen shows one sentence.
// The node's answers for TIDAL and the item pages are mocked; the rest is the CI node's own library.
//   node tools/ui-sweep-followups.js <api token> <node URL>
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const TOKEN = process.argv[2] || 'ci-token';
const BASE = process.argv[3] || 'http://127.0.0.1:7788';
const problems = [];
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) problems.push(what); };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, ms = 8000) => { const end = Date.now() + ms; for (;;) { if (await fn().catch(() => false)) return true; if (Date.now() > end) return false; await sleep(150); } };
const api = async (method, path, body) => { const r = await fetch(BASE + '/v1' + path, { method, headers: { Authorization: 'Bearer ' + TOKEN, 'content-type': 'application/json' }, body: body ? JSON.stringify(body) : undefined }); return r.status === 204 ? null : r.json().catch(() => null); };
const WIDTHS = [[390, 844], [430, 932], [768, 1024], [834, 1194], [1180, 820]];
const touch = (w) => w < 1025;
// Every button's box in a selector, as [w, h]
const boxes = (p, sel) => p.$$eval(sel, (els) => els.filter((e) => e.offsetParent).map((e) => { const r = e.getBoundingClientRect(); return [Math.round(r.width), Math.round(r.height)]; }));

(async () => {
  // A playlist with songs (web links play from a service: their rows have Move and Remove)
  const made = await api('POST', '/playlists', { name: 'Sweep follow-ups', uris: ['https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC', 'https://open.spotify.com/track/7GhIk7Il098yCjg4BQjzvb'] });
  const plid = made && (made.summary ? made.summary.id : made.id);
  const browser = await chromium.launch();
  try {
    for (const [w, h] of WIDTHS) {
      const ctx = await browser.newContext({ viewport: { width: w, height: h }, hasTouch: touch(w), isMobile: w < 600, colorScheme: 'dark' });
      await ctx.addInitScript((t) => { localStorage.setItem('pm.token', t); localStorage.setItem('pm.zone', 'browser'); HTMLMediaElement.prototype.play = async function () { this.dispatchEvent(new Event('playing')); }; }, TOKEN);
      const p = await ctx.newPage(); const errors = []; p.on('pageerror', (e) => errors.push(e.message));

      // ---- A10 and A16: a playlist ----
      if (plid) {
        await p.goto(`${BASE}/playlist/${encodeURIComponent(plid)}`); await p.waitForSelector('#plsongs .srow', { timeout: 20000 });
        const hero = await p.$$eval('.hero .actions > .btn', (bs) => bs.filter((b) => b.offsetParent).map((b) => Math.round(b.getBoundingClientRect().top)));
        if (touch(w)) {
          check(hero.length >= 3 && new Set(hero).size === 1, `${w}: the playlist's hero buttons are one row (${JSON.stringify(hero)})`);
          check(!(await p.$('#pldel')), `${w}: no Delete button beside Play`);
          await p.click('#plmore'); await p.waitForSelector('.popover.menu:not([hidden]) .pop-item');
          check(await p.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Delete playlist' }).count() === 1, `${w}: Delete playlist… is in ⋯`);
          await p.keyboard.press('Escape'); await sleep(200);
          const q = await boxes(p, '#plsongs .q-actions .icon-btn');
          if (w >= 600) check(q.length && q.every(([bw, bh]) => bw >= 40 && bh >= 40), `${w}: Move and Remove are 40 px (${JSON.stringify(q.slice(0, 3))})`);
          const sel = await boxes(p, '[data-selstart]');
          check(sel.length && sel.every(([, bh]) => bh >= 40), `${w}: Select is 40 px tall (${JSON.stringify(sel)})`);
        } else check(!!(await p.$('#pldel')), `${w}: the desktop keeps Delete beside Play`);
      }

      check(errors.length === 0, `${w}: no page errors so far${errors.length ? ': ' + errors.join(' | ') : ''}`);

      // ---- A10: song rows' actions, on a tablet ----
      if (w >= 600 && touch(w)) {
        await p.goto(BASE + '/tracks'); await p.waitForSelector('.track', { timeout: 20000 });
        const ra = await boxes(p, '.track .row-actions .icon-btn');
        check(ra.length && ra.every(([bw, bh]) => bw >= 40 && bh >= 40), `${w}: a song row's actions are 40 px (${JSON.stringify(ra.slice(0, 3))})`);
        // A touch screen has no hover: ⋯ is shown without one (the pointer is moved away first)
        await p.mouse.move(0, 0); await sleep(200);
        const op = await p.$eval('.track [data-act="more"]', (b) => { let o = 1; for (let e = b; e && e !== document.body; e = e.parentElement) o *= Number(getComputedStyle(e).opacity); return o; }).catch(() => 0);
        check(op === 1, `${w}: a song row's ⋯ shows without hover (opacity ${op})`);
      }
      // ---- A10: the play-from order's grips, on touch ----
      if (touch(w)) {
        await p.goto(BASE + '/settings/playfrom'); await p.waitForSelector('[data-ogrip]', { timeout: 20000 }).catch(() => {});
        const g = await boxes(p, '[data-ogrip]');
        check(g.length && g.every(([bw, bh]) => bw >= 40 && bh >= 40), `${w}: the play-from order's grips are 40 px (${JSON.stringify(g.slice(0, 2))})`);
      }

      // ---- A17 and A21: a song opened from its album of the same name ----
      const page = (kind, id, title, extra) => ({ kind, id, target: { track_id: 1 }, title, source: 'Your library', source_id: 'library', section: 'albums', songs: [], albums: [], versions: [], about: [], open_on: [], also_on: [], connect: [], ...extra });
      const A = { id: 'lib-7', name: 'Artist A' }; const B = { id: 'lib-8', name: 'Same Name' };
      await p.route(/\/v1\/item\/album\/lib-8$/, (r) => r.fulfill({ json: page('album', 'lib-8', 'Same Name', { artist: A, songs: [{ no: 1, title: 'Same Name', artist: 'Artist A', duration_ms: 1000, track_id: 1, id: 'lib-1' }] }) }));
      await p.route(/\/v1\/item\/song\/lib-1$/, (r) => r.fulfill({ json: page('song', 'lib-1', 'Same Name', { artist: A, album: B, about: [{ label: 'File', value: '/home/someone/music/samples/14_wasted_bits.flac' }, { label: 'Format', value: 'FLAC' }] }) }));
      await p.route(/\/v1\/item\/[a-z]+\/[^/]+\/extras$/, (r) => r.fulfill({ json: { related: [], concerts: [], similar: [] } }));
      await p.goto(BASE + '/album/lib-8'); await p.waitForSelector('.pagebar .back', { state: 'attached', timeout: 20000 });
      // A16: the album's hero buttons
      const ahero = await p.$$eval('.hero .actions > .btn', (bs) => bs.filter((b) => b.offsetParent).map((b) => Math.round(b.getBoundingClientRect().top)));
      if (touch(w)) check(ahero.length >= 4 && new Set(ahero).size === 1, `${w}: the album's hero buttons are one row (${JSON.stringify(ahero)})`);
      const vis = (sel) => p.$eval(sel, (e) => !!e.offsetParent).catch(() => false);
      if (w < 600) check(await vis('.pagebar .back'), `${w}: the album has its back link`);
      check(await vis('.pagebar .crumbs') === (w >= 600), `${w}: the trail shows ${w < 600 ? 'nowhere on a phone' : 'in the desktop layout'}`);
      await p.click('[data-ssong="0"]'); await until(async () => /\/song\//.test(p.url()), 8000);
      await p.waitForSelector('#iabout', { timeout: 20000 });
      const backText = (await p.textContent('.pagebar .back').catch(() => '')).trim();
      check(/\/song\//.test(p.url()) && backText !== 'Same Name' && (w >= 600 || backText === 'Back'), `${w}: Back from the album of the same name says Back, never the title again ("${backText}")`);
      const file = await p.$eval('#iabout dd .filefact', (e) => ({ text: e.textContent, title: e.title })).catch(() => null);
      check(file && file.text === '14_wasted_bits.flac' && /\/home\/someone\//.test(file.title) && !/\/home\/someone/.test(await p.textContent('#iabout')), `${w}: the File fact is the name, the path a tooltip (${JSON.stringify(file)})`);
      await p.unroute(/\/v1\/item\/album\/lib-8$/); await p.unroute(/\/v1\/item\/song\/lib-1$/);

      // ---- A19: a tablet's Now playing ----
      if (w >= 600 && touch(w)) {
        await p.goto(BASE + '/tracks'); await p.waitForSelector('.track'); const first = p.locator('.track').first(); await first.hover().catch(() => {});
        await first.locator('[data-act="play"]').first().click().catch(async () => { await first.dblclick(); });
        await until(async () => !/Nothing playing/.test(await p.textContent('#np')), 8000);
        await p.click('#np .np-title'); await p.waitForSelector('#nowplaying:not([hidden]) .np-sheet-tabs', { timeout: 8000 }).catch(() => {});
        const tabs = await p.$$eval('.np-sheet-tabs > *', (ts) => { const row = ts[0] && ts[0].parentElement.getBoundingClientRect(); return ts.map((t) => { const r = t.getBoundingClientRect(); return r.right <= row.right + 1 && r.left >= row.left - 1; }); }).catch(() => []);
        check(tabs.length >= 4 && tabs.every(Boolean), `${w}: every Now playing tab is whole (${tabs.length} tabs)`);
        const pill = await p.$eval('#npquality', (e) => { const r = e.getBoundingClientRect(); const d = e.querySelector('.qdot').getBoundingClientRect(); return { w: Math.round(r.width), dotIn: d.left >= r.left && d.right <= r.right, words: e.textContent.trim().length > 0 }; }).catch(() => null);
        check(pill && pill.w > 60 && pill.dotIn && pill.words, `${w}: Now playing's sound pill keeps its words, its dot inside (${JSON.stringify(pill)})`);
        await p.keyboard.press('Escape');
      }

      // ---- A25: Service apps ----
      await p.goto(BASE + '/settings/apps'); await p.waitForSelector('#main .set-row:visible', { timeout: 20000 });
      const folds = await p.$$eval('#main .morenote', (ds) => ds.map((d) => ({ open: d.open, more: d.querySelector('summary').textContent })));
      const longHelp = await p.$$eval('#main .set-row .set-help', (hs) => hs.filter((x) => x.offsetParent && !x.closest('details:not([open])')).map((x) => x.textContent.length).reduce((a, b) => Math.max(a, b), 0));
      check(folds.every((f) => !f.open) && longHelp < 160, `${w}: Service apps notes are short, the rest behind More (${folds.length} folds, longest line ${longHelp})`);

      check(errors.length === 0, `${w}: no page errors${errors.length ? ': ' + errors.join(' | ') : ''}`);
      await ctx.close();
    }

    // ---- A25: a sign-in way not chosen shows one sentence (mocked ways) ----
    {
      const ctx = await browser.newContext({ viewport: { width: 834, height: 1194 }, hasTouch: true });
      await ctx.addInitScript((t) => { localStorage.setItem('pm.token', t); localStorage.setItem('pm.zone', 'browser'); }, TOKEN);
      const p = await ctx.newPage();
      const LONG = 'Nothing in the known places on this machine (rescrobbled). From another machine, paste its key, secret and session key below. Uses the imported API key and secret for this node.';
      await p.route(/\/v1\/sources\/lastfm\/signin$/, (r) => r.request().method() !== 'GET' ? r.fulfill({ json: {} }) : r.fulfill({ json: { ways: [{ id: 'a', label: 'First way', note: 'An app id and secret, then your sign-in.', available: true, recommended: true }, { id: 'b', label: 'Second way', note: LONG, available: true }], imports: [], places: [], in_use: null, chosen: 'a', line: 'Not signed in', details: {} } }));
      await p.goto(BASE + '/settings/accounts'); await p.waitForSelector('.ways .way', { timeout: 20000 }).catch(() => {});
      const second = p.locator('.ways .way', { hasText: 'Second way' });
      const shown = async () => (await second.locator('.way-note').innerText()).trim();
      const before = await shown().catch(() => '');
      check(before === 'Nothing in the known places on this machine (rescrobbled).', `a way not chosen shows its first sentence ("${before}")`);
      await second.click(); await sleep(300);
      const after = await shown().catch(() => '');
      check(after.length > before.length && /session key/.test(after), 'chosen, it shows all of it');
      await ctx.close();
    }

    // ---- A7: a cold TIDAL collection ----
    {
      const ctx = await browser.newContext({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true });
      await ctx.addInitScript((t) => { localStorage.setItem('pm.token', t); localStorage.setItem('pm.zone', 'browser'); }, TOKEN);
      const p = await ctx.newPage(); const errors = []; p.on('pageerror', (e) => errors.push(e.message));
      let colDelay = 4000;
      await ctx.route(/\/v1\/sources$/, async (r) => { const res = await r.fetch(); const j = await res.json(); return r.fulfill({ json: j.map((s) => (s.id === 'tidal' ? { ...s, enabled: true, status: 'signed in' } : s)) }); });
      await ctx.route(/\/v1\/sources\/tidal\/signin$/, (r) => r.fulfill({ json: { ways: [], imports: [], places: [], in_use: { way: 'app', label: 'Your own TIDAL app' }, chosen: 'app', line: 'Signed in', details: { status: { configured: true, signed_in: true }, unofficial: {} } } }));
      await ctx.route(/\/v1\/sources\/tidal\/playlists$/, (r) => r.fulfill({ json: [{ id: 'tp1', name: 'Mock TIDAL list', items: 3 }] }));
      let imports = 0; let importDelay = 4500;
      await ctx.route(/\/v1\/sources\/tidal\/playlists\/tp1\/import/, async (r) => { imports++; await sleep(importDelay); return r.fulfill({ json: { playlist: { summary: { id: 'x', name: 'Mock TIDAL list' } }, imported: 3, kept: 0, missing: [] } }); });
      let colFail = false;
      await ctx.route(/\/v1\/sources\/tidal\/collection$/, async (r) => { if (colFail) return r.fulfill({ status: 502, json: { error: 'TIDAL answered 502' } }); await sleep(colDelay); return r.fulfill({ json: { tracks: [{ id: 'tidal-1', title: 'Cold Song', creator: 'Someone', kind: 'track' }], albums: [{ id: 'tidal-2', title: 'Cold Album', creator: 'Someone', kind: 'album' }] } }); });
      await p.goto(BASE + '/sources/tidal');
      const status = '#src-tidal .srcmine-status';
      check(await until(async () => /Loading your TIDAL collection/.test(await p.textContent(status)), 5000), 'A7: the top of the page says the collection is loading');
      const top = await p.$eval(status, (e) => Math.round(e.getBoundingClientRect().top)).catch(() => 9999);
      check(top < 844 - 160, `A7: where the eye is, above the fold (${top} px)`);
      check(await until(async () => /Mock TIDAL list/.test(await p.textContent('#tidalmine')), 3000), 'A7: the playlists draw before the slow collection');
      check(await until(async () => /take a while/.test(await p.textContent(status)), 4000), 'A7: after 2 s it says the first time takes a while');
      check(await until(async () => /Cold Album/.test(await p.textContent('#tidalmine')) && !(await p.isVisible(status)), 8000), 'A7: the collection draws, and the status goes');
      // Next time: the last good copy at once, while the node answers slowly again
      colDelay = 3000; await p.reload();
      check(await until(async () => /Cold Album/.test(await p.textContent('#tidalmine')), 1500) && /Updating/.test(await p.textContent(status)), 'A7: next time the last copy shows at once, "Updating…" above');
      await sleep(3500);
      check(!(await p.isVisible(status)), 'A7: and the status goes once the fresh answer is in');
      // An import running while the collection answers: its button stays off, so it can't run twice
      colDelay = 2500; await p.reload();
      await p.waitForSelector('#tidalpls [data-tpl="tp1"]'); await p.click('#tidalpls [data-tpl="tp1"]');
      await p.locator('dialog .choice', { hasText: 'Keep' }).first().click();
      await until(async () => imports === 1, 3000); await sleep(3000); // the collection and the playlists have answered meanwhile
      check(await p.isDisabled('#tidalpls [data-tpl="tp1"]'), 'A7: an import in flight keeps its button off across the redraws');
      await p.click('#tidalpls [data-tpl="tp1"]', { force: true }).catch(() => {}); await sleep(300);
      check(imports === 1 && await p.locator('dialog .choice').count() === 0, `A7: so it isn't imported twice (${imports} imports)`);
      check(await until(async () => !(await p.isDisabled('#tidalpls [data-tpl="tp1"]')), 6000), 'A7: and the button is back once it is done');
      // A failed refresh over the kept copy: the copy shows, and the top says the refresh failed, with Try again
      colFail = true;
      await p.reload();
      check(await until(async () => /Cold Album/.test(await p.textContent('#tidalmine')) && await p.isVisible(status) && /Couldn’t update/.test(await p.textContent(status)) && await p.locator(`${status} [data-tretry]`).count() === 1, 8000), 'A7: a failed refresh over the last copy says so at the top, with Try again');
      colFail = false; colDelay = 0;
      await p.click(`${status} [data-tretry]`);
      check(await until(async () => !(await p.isVisible(status)), 8000), 'A7: and Try again clears it');
      // A failure with no copy says so, with Try again
      colFail = true;
      await p.evaluate(() => { for (const k of Object.keys(localStorage)) if (k.startsWith('pm.srcmine.')) localStorage.removeItem(k); });
      await p.reload();
      check(await until(async () => /Couldn’t load it/.test(await p.textContent('#tidalmine')) && await p.locator('#tidalmine [data-tretry]').count() === 1, 8000) && !/Nothing in your TIDAL collection/.test(await p.textContent('#tidalmine')), 'A7: a failed collection says so with Try again, not "Nothing in your collection"');
      check(errors.length === 0, `A7: no page errors${errors.length ? ': ' + errors.join(' | ') : ''}`);
      await ctx.close();
    }
  } finally {
    await browser.close();
    if (plid) await api('DELETE', `/playlists/${encodeURIComponent(plid)}`).catch(() => {});
  }
  if (problems.length) { console.log(`${problems.length} failed`); process.exit(1); }
  console.log('sweep follow-ups ok');
})().catch((e) => { console.error(e); process.exit(1); });

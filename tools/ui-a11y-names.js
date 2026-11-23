// Names, roles and preferences a screen reader, a keyboard or a system setting depends on.
// Real candidate assets on the fixture origin (tools/ui-fixture.js), a node zone playing:
//   * a ⋯ menu is a menu: role and name on it, a role on every item, and its opener says it has a
//     menu and whether it is open;
//   * every dialog opened is named by its heading; a confirmation's words are read with it;
//   * the mini player's name is the song, and Enter on its title opens the song, not Now playing;
//   * the seek bars are read as a time ("1:01 of 4:00");
//   * the first Tab stop is "Skip to the page", which puts the focus on the page;
//   * less motion: the page does not glide when it scrolls;
//   * forced colours: a focused field keeps a mark, the chosen menu item an outline, sliders and
//     switches their own look;
//   * on a phone two fingers can zoom Now playing and the mini player; one finger still swipes.
//   node tools/ui-a11y-names.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'a11y-names-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const fmt = { kind: 'pcm', rate_hz: 44100, bits: 16, channels: 2 };
const tracks = Array.from({ length: 8 }, (_, i) => ({ ...fx.track, id: i + 1, title: `Song number ${i + 1}` }));
const item = (n) => ({ id: n, uri: `track:${n}`, title: `Song number ${n}`, artist: 'Sefa, Dr. Peacock', album: 'Journey', duration_ms: 240000 });
const playing = (n, pos) => ({ item: item(n), index: n - 1, position_ms: pos, duration_ms: 240000, source_format: fmt, output_format: fmt, signal_path: { stages: [] }, format: { codec: 'FLAC', lossless: true, service: 'library', preview: false } });
const zone = (transport, np) => ({ id: 'kitchen', name: 'Kitchen', output: 'upnp:kitchen', transport, queue: [item(1), item(2)], queue_index: np ? np.index : null, queue_version: 1, repeat: 'off', shuffle: false, radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: { percent: 20, muted: false }, now_playing: np });
const outputs = [{ id: 'upnp:kitchen', name: 'Kitchen', kind: 'upnp', caps: { hw_volume: 'percent', max_pcm_bits: 24, pcm_rates: [44100] } }];

const open = async (browser, width) => {
  const zones = [zone('playing', playing(1, 61000))];
  const { context, page, state } = await fx.create(browser, width, { mobile: width < 600, zone: 'kitchen', tracks, fixtures: { '/zones': { body: zones }, '/outputs': { body: outputs }, '/zones/kitchen/queue': { body: zones[0].queue } } });
  await context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones })); });
  return { context, page, state };
};
// The name a dialog is given: its aria-label, else the text of what aria-labelledby points at
const dialogName = (page) => page.evaluate(() => { const d = document.querySelector('dialog[open]'); if (!d) return null; const by = d.getAttribute('aria-labelledby'); const el = by && document.getElementById(by); const says = d.getAttribute('aria-describedby'); const p = says && document.getElementById(says); return { name: (d.getAttribute('aria-label') || (el ? el.textContent : '') || '').trim(), words: p ? p.textContent.trim() : '' }; });

(async () => {
  const browser = await chromium.launch();
  {
    const at = '1280px';
    const { context, page, state } = await open(browser, 1280);
    await fx.go(page, '/tracks'); await page.waitForSelector('#main .track');
    await page.waitForFunction(() => /Song number 1/.test(document.querySelector('#np').innerText), null, { timeout: 5000 }).catch(() => {});

    // the first Tab stop
    await page.keyboard.press('Tab');
    check(await page.evaluate(() => document.activeElement && document.activeElement.id === 'skip' && document.activeElement.getBoundingClientRect().top >= 0), `${at}: the first Tab stop is "Skip to the page", shown while it has the focus`);
    await page.screenshot({ path: path.join(SHOTS, 'skip-1280.png') });
    await page.keyboard.press('Enter');
    check(await page.evaluate(() => document.activeElement && document.activeElement.id === 'main'), `${at}: it puts the focus on the page`);
    check(await page.evaluate(() => { const s = document.getElementById('skip'); return !!s && s.getBoundingClientRect().bottom <= 0; }), `${at}: it is out of sight again without the focus`);

    // a ⋯ menu
    const more = page.locator('#main .track [data-act="more"]').first();
    check(await more.getAttribute('aria-haspopup') === 'menu' && await more.getAttribute('aria-expanded') === 'false', `${at}: a ⋯ says it has a menu before it was ever opened, closed`);
    await more.focus(); await page.keyboard.press('Enter'); await sleep(250);
    const menu = await page.evaluate(() => { const m = document.querySelector('.popover.menu:not([hidden])'); if (!m) return null; const items = [...m.querySelectorAll('.pop-item')]; return { role: m.getAttribute('role'), name: m.getAttribute('aria-label'), roles: [...new Set(items.map((b) => b.getAttribute('role')))], n: items.length, titles: [...m.querySelectorAll('.pop-title')].map((t) => t.getAttribute('role')) }; });
    check(!!menu && menu.role === 'menu' && !!menu.name && menu.n > 3 && menu.roles.every((r) => r === 'menuitem' || r === 'menuitemradio'), `${at}: a ⋯ menu is a menu with a name, each item a menu item (${JSON.stringify(menu)})`);
    check(await more.getAttribute('aria-haspopup') === 'menu' && await more.getAttribute('aria-expanded') === 'true', `${at}: its ⋯ says it has a menu, open`);
    await page.keyboard.press('Escape'); await sleep(150);
    check(await more.getAttribute('aria-expanded') === 'false', `${at}: closed again once it is`);
    // (each titled part is a group named by its title; the title itself is not read a second time)
    await page.focus('#me'); await page.keyboard.press('Enter'); await sleep(250);
    const parts = await page.evaluate(() => {
      const m = document.querySelector('.popover.menu:not([hidden])'); if (!m) return null;
      const titles = [...m.querySelectorAll('.pop-title')]; const bad = [];
      for (const b of m.querySelectorAll('.pop-item')) {
        const before = titles.filter((t) => t.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING).pop();
        if (!before) continue;
        const g = b.closest('[role="group"]'); const lab = g && document.getElementById(g.getAttribute('aria-labelledby'));
        if (!g || lab !== before) bad.push(b.textContent.trim().slice(0, 30));
      }
      return { titles: titles.length, groups: m.querySelectorAll('[role="group"]').length, hidden: titles.every((t) => t.getAttribute('aria-hidden') === 'true'), bad };
    });
    check(!!parts && parts.titles > 0 && parts.groups > 0 && parts.hidden && !parts.bad.length, `${at}: each titled part of a menu (the account menu) is a group named by its title (${JSON.stringify(parts)})`);
    await page.keyboard.press('Escape'); await sleep(150);
    await more.focus(); await page.keyboard.press('Enter'); await sleep(200);
    await page.locator('.popover.menu .pop-item', { hasText: /Add to (playlist|collection)/ }).first().click(); await sleep(350);
    check(await more.getAttribute('aria-expanded') === 'false', `${at}: and once an item is chosen`);
    const picker = await dialogName(page);
    check(!!picker && /^Add to (playlist|collection)$/.test(picker.name), `${at}: the picker it opens is named by its heading (${picker && picker.name})`);
    await page.keyboard.press('Escape'); await sleep(200);

    // a confirmation: named by its question, its words read with it
    await page.keyboard.press('?'); await sleep(300);
    const help = await dialogName(page);
    check(!!help && help.name === 'Keyboard shortcuts' && /play or pause/.test(help.words), `${at}: the shortcuts dialog is named, and its words are read with it (${help && help.name})`);
    await page.keyboard.press('Escape'); await sleep(200);

    // the mini player
    const name = await page.locator('#np').getAttribute('aria-label');
    check(/^Now playing: Song number 1, Sefa, Dr\. Peacock$/.test(name || ''), `${at}: the mini player's name is the song (${name})`);
    check(await page.locator('#seek').getAttribute('aria-valuetext') === '1:01 of 4:00', `${at}: the seek bar is read as a time (${await page.locator('#seek').getAttribute('aria-valuetext')})`);
    {
      // moved by keyboard it says where it is now (it said where the song was), in the same form
      const moved = await page.evaluate(() => { const s = document.getElementById('seek'); s.focus(); s.value = String(Number(s.value) + 250); s.dispatchEvent(new Event('input', { bubbles: true })); const t = s.getAttribute('aria-valuetext'); delete s.dataset.dragging; return t; });
      check(/^2:0[01] of 4:00$/.test(moved || ''), `${at}: moved a quarter on, the seek bar says the time it is moved to (${moved})`);
      // Space on the title's link still opens Now playing (a link has no Space of its own); Enter is the link's
      const sp = await page.evaluate(() => { const a = document.querySelector('#np a'); if (!a) return null; a.focus(); return true; });
      check(!!sp, `${at}: the bar's title is a link`);
      if (sp) { await page.keyboard.press(' '); check(await page.evaluate(() => !document.getElementById('nowplaying').hidden), `${at}: Space on the bar's title opens Now playing`); check(await page.evaluate(() => getComputedStyle(document.getElementById('skip')).display === 'none'), `${at}: no skip link over Now playing`); await page.keyboard.press('Escape'); await page.evaluate(() => { const o = document.getElementById('nowplaying'); if (!o.hidden) document.getElementById('np').click(); }); }
    }
    const link = page.locator('#np .np-title [data-np-song]').first();
    if (await link.count()) {
      await link.focus(); await page.keyboard.press('Enter'); await sleep(500);
      check(/\/song\//.test(page.url()) && await page.evaluate(() => document.getElementById('nowplaying').hidden), `${at}: Enter on the bar's title opens the song, not Now playing (${new URL(page.url()).pathname})`);
    } else check(false, `${at}: the bar's title is a link`);
    await page.locator('#np').focus(); await page.keyboard.press('Enter'); await sleep(400);
    check(await page.evaluate(() => !document.getElementById('nowplaying').hidden), `${at}: Enter on the bar itself still opens Now playing`);
    check(await page.locator('#npseek').getAttribute('aria-valuetext').catch(() => null) !== null || await page.evaluate(() => new Promise((r) => setTimeout(() => r(!!document.querySelector('#npseek[aria-valuetext]')), 700))), `${at}: Now playing's seek bar is read as a time too`);
    await page.keyboard.press('Escape'); await sleep(200);

    // less motion (the fixture's pages ask for it), and without that wish
    check(await page.evaluate(() => getComputedStyle(document.getElementById('main')).scrollBehavior) === 'auto', `${at}: with less motion asked for, the page does not glide when it scrolls`);
    // (what is drawn by ::before or ::after, a toast's spinner, stops too)
    const pseudo = await page.evaluate(() => { const d = document.createElement('div'); d.className = 'toast wait show'; d.style.cssText = 'position:fixed;left:-999px'; d.textContent = 'x'; document.body.appendChild(d); const n = getComputedStyle(d, '::before').animationName; d.remove(); const k = document.createElement('style'); k.textContent = '.t-pseudo::after { content: ""; animation: spin 1s linear infinite; }'; document.head.appendChild(k); const e = document.createElement('i'); e.className = 't-pseudo'; document.body.appendChild(e); const m = getComputedStyle(e, '::after').animationName; e.remove(); k.remove(); return [n, m]; });
    check(pseudo.every((x) => x === 'none'), `${at}: with less motion asked for, nothing drawn by ::before or ::after turns either (a waiting toast's spinner: ${pseudo.join(', ')})`);
    await page.emulateMedia({ reducedMotion: 'no-preference' });
    check(await page.evaluate(() => getComputedStyle(document.getElementById('main')).scrollBehavior) === 'smooth', `${at}: without that wish it still glides`);
    await page.emulateMedia({ reducedMotion: 'reduce' });

    // forced colours
    await fx.go(page, '/playlists'); await page.waitForSelector('#main input.input');
    await page.emulateMedia({ forcedColors: 'active' });
    const fc = await page.evaluate(() => {
      const f = document.querySelector('#main input.input, #main .field input, #main input'); if (f) f.focus();
      const fo = f ? `${getComputedStyle(f).outlineStyle} ${getComputedStyle(f).outlineWidth}` : '';
      const on = document.querySelector('.nav-item.on'); const off = document.querySelector('.nav-item:not(.on)');
      const range = document.querySelector('#seek');
      const sw = document.createElement('input'); sw.type = 'checkbox'; sw.className = 'switch'; document.body.appendChild(sw); const swLook = `${getComputedStyle(sw).appearance} ${getComputedStyle(sw, '::after').display}`; sw.remove();
      document.getElementById('search').focus(); const ss = getComputedStyle(document.querySelector('.search'));
      return { search: `${ss.outlineStyle} ${ss.outlineWidth}`, field: !!f, fieldOutline: fo, on: on ? getComputedStyle(on).outlineStyle : '', off: off ? getComputedStyle(off).outlineStyle : '', range: range ? getComputedStyle(range).appearance : '', sw: swLook };
    });
    check(fc.field && /^solid 2px$/.test(fc.fieldOutline) && /^solid 2px$/.test(fc.search), `${at} forced colours: a focused field and the search box keep a mark (${fc.fieldOutline}; ${fc.search})`);
    check(fc.on === 'solid' && fc.off === 'none', `${at} forced colours: the chosen place in the side menu has an outline, the others none (${fc.on}, ${fc.off})`);
    check(fc.range === 'auto', `${at} forced colours: the seek bar has the system's own look (${fc.range})`);
    check(fc.sw === 'auto none', `${at} forced colours: a switch has the system's own look, without the drawn knob (${fc.sw})`);
    await page.screenshot({ path: path.join(SHOTS, 'forced-colours-1280.png'), timeout: 4000 }).catch(() => {});
    await page.emulateMedia({ forcedColors: 'none' });
    check(!state.errors.length, `${at}: no page error (${state.errors.join(' | ') || 'none'})`);
    await context.close();
  }
  {
    const at = '390px';
    const { context, page, state } = await open(browser, 390);
    await fx.go(page, '/tracks'); await page.waitForSelector('#main .track');
    await page.waitForFunction(() => /Song number 1/.test(document.querySelector('#np').innerText), null, { timeout: 5000 }).catch(() => {});
    const name = await page.locator('#np').getAttribute('aria-label');
    check(/^Now playing: Song number 1, Sefa, Dr\. Peacock$/.test(name || ''), `${at}: the mini player's name is the song (${name})`);
    check(/pinch-zoom/.test(await page.evaluate(() => getComputedStyle(document.querySelector('.player')).touchAction)), `${at}: two fingers can zoom on the mini player`);
    await page.locator('#np').click(); await sleep(600);
    const ta = await page.evaluate(() => { const n = document.querySelector('.np2.compact'); return n ? getComputedStyle(n).touchAction : ''; });
    check(/pinch-zoom/.test(ta) && !/pan/.test(ta), `${at}: two fingers can zoom Now playing; one finger is still the swipe's (${ta})`);
    check(!state.errors.length, `${at}: no page error (${state.errors.join(' | ') || 'none'})`);
    await context.close();
  }
  {
    // A Play that offers a choice only sometimes (a song none of your services has) is no menu
    // button: it says it has a menu while that menu is open, and nothing once it is closed
    const OTHER = 'https://tidal.com/browse/track/77';
    const { context, page, state } = await fx.create(browser, 1280, { fixtures: {
      '/favourites': { body: { uris: [OTHER], tracks: [], albums: [], artists: [], other: [OTHER] } },
      '/discover/describe': { body: [{ uri: OTHER, page: OTHER, title: 'Far Away Song', artist: 'Someone Else', kind: 'track', site: null, also_on: [], connect: ['tidal'] }] },
      '/discover/play': { body: { uri: OTHER, plays_from: null, items: [], track_ids: [], connect: ['tidal'], reason: 'Not on your connected services' } },
      '/sources': { body: [{ id: 'tidal', name: 'TIDAL', description: '', status: 'ready', enabled: true }] },
    } });
    await fx.go(page, '/favourites');
    const play = page.locator('#favother [data-dplay]').first(); await play.waitFor();
    const said = async () => [await play.getAttribute('aria-haspopup'), await play.getAttribute('aria-expanded')].join(' ');
    const before = await said();
    await play.click(); await page.locator('.popover.menu:not([hidden]) .pop-item').first().waitFor();
    const during = await said();
    await page.keyboard.press('Escape'); await sleep(250);
    const after = await said();
    check(before === ' ' && during === 'menu true' && after === ' ', `a Play that offers a choice says it has a menu only while it is open (before "${before}", open "${during}", closed "${after}")`);
    check(!state.errors.length, `a Play's menu: no page error (${state.errors.join(' | ') || 'none'})`);
    await context.close();
  }
  await browser.close();
  console.log(failed ? `\n${failed} failed` : '\nall passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });

// The apps never say "browser" (TASK-android-no-browser-wording; owner, Android with a USB DAC:
// "this browser should not use this wording when playing on android"). Real candidate assets on
// the fixture origin (tools/ui-fixture.js) at 390 px, a node zone present, the page's own player
// chosen, three ways: as the Android app (window.pmDesktop.platform 'android', set before app.js as
// pm-desktop does), as the desktop app ('linux'), and as the plain web UI (no pmDesktop):
//   * the zone button, the zone picker, the Sound panel and Now playing name the player "This
//     phone" / "This computer" / "This browser", and the picker puts the in-app player last in the
//     apps (the phone's own zone comes first) and first on the web;
//   * the Sound panel says what plays the sound: "Android mixes it to the system rate" in the Android
//     app, "decodes the file itself" on the web; no bit-perfect claim in the apps;
//   * with a song in the in-app player (its saved queue), Now playing › Details says "Played by
//     Android" / "Played by the app" / "Played by this browser";
//   * in the apps, none of those screens (bar, picker, Sound panel, the Queue page, Now playing)
//     shows the word "browser";
// and, from app.js's own text, every string literal that says "this browser" goes through the
// helpers (hereName and friends) or sits in a `appPlatform() ? … : …` branch: a new literal fails here.
//   node tools/ui-here-name.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const REPO = path.resolve(process.argv[2] || path.join(__dirname, '..'));
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'here-name-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// The phone's own zone on the node (the app's player registered as an endpoint), idle
const fmt = { kind: 'pcm', rate_hz: 48000, bits: 16, channels: 2 };
const outputs = [{ id: 'net:10.0.0.5:7790/cpal:default', name: 'Pixel 8 · Speaker or headphones', kind: 'net', caps: { hw_volume: 'percent' } }];
const zone = { id: 'pixel-8-speaker-or-headphones', name: 'Pixel 8 · Speaker or headphones', output: 'net:10.0.0.5:7790/cpal:default', transport: 'stopped', queue: [], queue_index: null, queue_version: 1, repeat: 'off', radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: { percent: 40, muted: false }, now_playing: null };
const fixtures = { '/zones': { body: [zone] }, '/outputs': { body: outputs }, '/library/tracks/101': { body: { ...fx.track, id: 101, title: 'Song 1' } } };
// The in-app player's saved queue: a library song, paused at 12 s (restoreBrowserQueue puts it in the bar)
const SAVED = JSON.stringify([{ id: 1, uri: 'track:101', track_id: 101, title: 'Song 1', artist: 'Sefa', album: 'Journey', duration_ms: 200000 }]);
const CASES = [['android', { platform: 'android', version: '0.0.0' }, 'This phone', 'Played by Android'], ['desktop', { platform: 'linux', version: '0.0.0' }, 'This computer', 'Played by the app'], ['web', null, 'This browser', 'Played by this browser']];
const open = async (browser, desktop, w = 390) => {
  const run = await fx.create(browser, w, { touch: w < 600, fixtures });
  await run.page.setViewportSize({ width: w, height: w < 600 ? 844 : 900 });
  await run.context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [zone] })); });
  await run.context.addInitScript(([d, saved]) => { localStorage.setItem('pm.zone', 'browser'); localStorage.setItem('pm.browserQueue', saved); localStorage.setItem('pm.browserAt', JSON.stringify({ index: 0, pos: 12 })); if (d) window.pmDesktop = d; }, [desktop, SAVED]);
  return run;
};
const text = (page, sel) => page.$eval(sel, (el) => el.innerText || el.textContent || '').catch(() => '');
const hasBrowser = (t) => /\bbrowser\b/i.test(t);

(async () => {
  const browser = await chromium.launch();
  for (const [tag, desktop, name, played] of CASES) {
    const { page, context, state } = await open(browser, desktop);
    await fx.go(page, '/library'); await page.waitForSelector('#zonename'); await sleep(400);
    const inApp = !!desktop; const at = `${tag}`;
    const seen = [];
    check((await text(page, '#zonename')).trim() === name, `${at}: the zone button says "${name}" (${(await text(page, '#zonename')).trim()})`);
    seen.push(await text(page, '.player'));
    // The picker: the in-app player last in the apps (the phone's own zone first), first on the web
    await page.click('#zonebtn'); await page.waitForSelector('#zonepop:not([hidden])'); await sleep(200);
    const rows = (await page.$$eval('#zonepop .name', (ns) => ns.map((n) => n.textContent.trim()))).filter((r) => r === name || r === zone.name);
    seen.push(await text(page, '#zonepop'));
    check(rows.includes(name) && rows.includes(zone.name) && (inApp ? rows[rows.length - 1] === name : rows[0] === name), `${at}: the picker lists "${name}" ${inApp ? 'last, after the phone\'s own zone' : 'first'} (${rows.join(' · ')})`);
    await page.screenshot({ path: path.join(SHOTS, `picker-${tag}.png`) });
    await page.keyboard.press('Escape'); await sleep(200);
    // The Sound panel: what plays the sound
    const pill = await page.$('#quality');
    const pillShown = pill && await pill.evaluate((q) => q.getClientRects().length > 0);
    if (pillShown) {
      await page.click('#quality'); await page.waitForSelector('#qpanel:not([hidden])'); await sleep(200);
      const panel = await text(page, '#qpanel'); seen.push(panel);
      check(inApp ? (tag === 'android' ? /Android mixes it to the system rate/.test(panel) : /the system mixes it/.test(panel)) && !/bit-perfect|decodes the file itself/i.test(panel) : /decodes the file itself/.test(panel), `${at}: the Sound panel says what plays the sound (${panel.replace(/\s+/g, ' ').slice(0, 120)})`);
      await page.screenshot({ path: path.join(SHOTS, `sound-${tag}.png`) });
      await page.keyboard.press('Escape'); await sleep(200);
    } else {
      // A phone hides the pill while nothing plays: the panel is checked in a desktop-wide window
      const wide = await open(browser, desktop, 1280);
      await fx.go(wide.page, '/library'); await wide.page.waitForSelector('#quality'); await sleep(400);
      await wide.page.click('#quality'); await wide.page.waitForSelector('#qpanel:not([hidden])'); await sleep(200);
      const panel = await text(wide.page, '#qpanel'); seen.push(panel);
      check(inApp ? (tag === 'android' ? /Android mixes it to the system rate/.test(panel) : /the system mixes it/.test(panel)) && !/bit-perfect|decodes the file itself/i.test(panel) : /decodes the file itself/.test(panel), `${at} (1280): the Sound panel says what plays the sound (${panel.replace(/\s+/g, ' ').slice(0, 120)})`);
      await wide.page.screenshot({ path: path.join(SHOTS, `sound-${tag}.png`) });
      await wide.context.close();
    }
    // The Queue page is headed by the zone's name; the Zones page has its card; Now playing names it too
    await fx.go(page, '/queue'); await page.waitForSelector('#main'); await sleep(400);
    const queuePage = await text(page, '#main'); seen.push(queuePage);
    check(queuePage.includes(name), `${at}: the Queue page is headed "${name}"`);
    await fx.go(page, '/zones'); await page.waitForSelector('#main'); await sleep(400);
    const zonesPage = await page.$eval('#main', (el) => `${el.innerText} ${[...el.querySelectorAll('[aria-label]')].map((e) => e.getAttribute('aria-label')).join(' ')}`); seen.push(zonesPage);
    check(zonesPage.includes(`Volume of ${name}`), `${at}: the Zones page has the player's card and slider ("Volume of ${name}")`);
    // Now playing with the saved song in the bar: its Details line says who plays it
    await fx.go(page, '/library'); await page.waitForSelector('#npart'); await sleep(500);
    await page.click('#npart'); await page.waitForSelector('#nowplaying:not([hidden])', { timeout: 5000 }); await sleep(400);
    const np = await text(page, '#nowplaying'); seen.push(np);
    const details = page.locator('#nowplaying .np-tabs [data-npsheet="details"]');
    if (await details.count()) { await details.first().click(); await sleep(500); seen.push(await text(page, '#nowplaying')); }
    const npAll = seen[seen.length - 1];
    check(npAll.includes(played), `${at}: Now playing says "${played}"`);
    await page.screenshot({ path: path.join(SHOTS, `nowplaying-${tag}.png`) });
    await page.keyboard.press('Escape'); await sleep(200);
    const where = seen.map((t) => (t.match(/[^.\n]*\bbrowser\b[^.\n]*/i) || [''])[0]).filter(Boolean);
    if (inApp) check(!seen.some(hasBrowser), `${at}: no "browser" on the bar, the picker, the Sound panel, the Queue and Zones pages or Now playing${where.length ? ': ' + where.slice(0, 2).join(' | ') : ''}`);
    else check(seen.some(hasBrowser), `${at}: the web UI still says browser`);
    check(!state.errors.length, `${at}: no page errors${state.errors.length ? ': ' + state.errors.join('; ') : ''}`);
    await context.close();
  }
  // app.js's own text: every string literal with the word "browser", at any depth (a template's
  // ${…} holes are walked too), is one of the helpers' own, the else branch of an `appPlatform() ?`
  // conditional (found structurally: the literal follows that conditional's own `:`), the player's
  // id, a selector or an API path (whole literals), or one of the sentences about real browsers
  // named here whole. A new "this browser" literal fails here.
  const srcText = fs.readFileSync(path.join(REPO, 'crates/pm-api/static/app.js'), 'utf8');
  const lines = srcText.split('\n');
  const lineOf = (pos) => srcText.slice(0, pos).split('\n').length;
  const helperLines = new Set(lines.map((l, i) => (/const (hereName|hereIn|hereOnly|playedBy|decodedBy|decodeNote|wordHere) =/.test(l) ? i + 1 : 0)).filter(Boolean));
  // ('your browser': the app branch of the TIDAL sign-in hint, which really opens the system browser)
  const WHOLE = new Set(["'browser'", '"browser"', "'#browser'", "'your browser'", "'/sources/tidal/unofficial/browser'", "'/sources/tidal/unofficial/browser/finish'", "'internet radio via radio-browser.info'", "'pm.browserQueue'", "'pm.browserAt'", "'browser'"]);
  const SENTENCES = [
    "'A new token for the node: the old one stops working at once, so every app and Subsonic app needs the new one, and every browser of yours is logged out (log in again with Pubky Ring). People you ",
    "'The old token stops working at once. Every app and Subsonic app then needs the new one, and your other browsers are logged out; this one switches to the new token now.'",
    "'To download what you bought: sign in on bandcamp.com in your browser, open its cookies for bandcamp.com and copy the value of the one named identity. The node checks",
    "'Optional: paste your own SoundCloud sign-in token (the one your browser uses while you are signed in). The node checks it once with SoundCloud and keeps it",
  ];
  // Every literal with its position: strings and templates, recursing into a template's holes
  const found = [];
  const scan = (text, base) => {
    let i = 0;
    while (i < text.length) {
      const c = text[i];
      if (c === '/' && text[i + 1] === '/') { i = text.indexOf('\n', i); if (i < 0) break; continue; }
      if (c === '/' && text[i + 1] === '*') { i = text.indexOf('*/', i); if (i < 0) break; i += 2; continue; }
      if (c === "'" || c === '"') { let k = i + 1; while (k < text.length && text[k] !== c) { if (text[k] === '\\') k++; if (text[k] === '\n') break; k++; } found.push([base + i, text.slice(i, k + 1)]); i = k + 1; continue; }
      if (c === '`') {
        let k = i + 1; let depth = 0; let holeStart = -1;
        while (k < text.length) {
          if (text[k] === '\\') { k += 2; continue; }
          if (depth === 0 && text[k] === '`') break;
          if (text[k] === '$' && text[k + 1] === '{') { if (depth === 0) holeStart = k + 2; depth++; k += 2; continue; }
          if (text[k] === '}' && depth > 0) { depth--; if (depth === 0) scan(text.slice(holeStart, k), base + holeStart); }
          k++;
        }
        // The template's own words, holes blanked (their literals were scanned above)
        let own = text.slice(i, k + 1); let d = 0; let out = '';
        for (let q = 0; q < own.length; q++) { if (own[q] === '$' && own[q + 1] === '{') { d++; q++; continue; } if (own[q] === '}' && d > 0) { d--; continue; } if (!d) out += own[q]; }
        found.push([base + i, out]); i = k + 1; continue;
      }
      i++;
    }
  };
  scan(srcText, 0);
  // The literal is the else branch of an `appPlatform() ?` conditional: before it (over spaces) a `:`
  // that belongs, counting nested `?`/`:` pairs backwards, to that conditional
  const elseOfApp = (pos) => {
    let k = pos - 1; while (k >= 0 && /\s/.test(srcText[k])) k--;
    if (srcText[k] !== ':') return false;
    let depth = 0;
    for (let q = k - 1; q >= Math.max(0, k - 2000); q--) {
      const ch = srcText[q];
      if (ch === ':' && srcText[q - 1] !== ':') depth++;
      else if (ch === '?' && srcText[q + 1] !== '.' && srcText[q + 1] !== '?' && srcText[q - 1] !== '?') { if (depth === 0) return srcText.slice(Math.max(0, q - 16), q).includes('appPlatform()'); depth--; }
      else if (ch === '\n' && /^\s*$/.test(srcText.slice(q + 1, pos).split('\n')[0] || '') === false && srcText.slice(q, pos).includes(';')) return false;
    }
    return false;
  };
  const bad = [];
  for (const [pos, str] of found) {
    if (!/\b[Bb]rowsers?\b/.test(str)) continue;
    const line = lineOf(pos);
    if (helperLines.has(line) || WHOLE.has(str) || SENTENCES.some((s) => str.startsWith(s) && str.length - s.length < 400 && !/this browser|the browser/i.test(str.slice(s.length)))) continue;
    if (elseOfApp(pos)) continue;
    bad.push(`${line}: ${str.slice(0, 70)}`);
  }
  check(!bad.length, `app.js: every "browser" the page can show goes through the helpers${bad.length ? ': ' + bad.slice(0, 5).join(' | ') : ''}`);
  await browser.close();
  console.log(failed ? `${failed} failed` : 'all passed'); console.log(`screenshots: ${SHOTS}`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });

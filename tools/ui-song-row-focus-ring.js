// Song rows cut a long title, artist line and album with "…" (overflow: hidden). That clip also cut
// the keyboard focus ring of their links and the Match mark to a sliver, so a keyboard user could
// not see which link they were on. The ring now fits inside the clip, the text stays where it was,
// and a click still lands on the link it points at.
// Real UI in Chromium against fixtures (no node), 1280 to 768 px, plus 390 px as the phone.
//   node tools/ui-song-row-focus-ring.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'song-row-focus-ring-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

// Unmatched songs (no canonical link) so the first row has its Match mark, and two artists per row;
// the second song is matched, so its title is a plain line (no Match mark)
const names = ['Sefa', 'Dr. Peacock', 'Angerfist'];
const tracks = Array.from({ length: 8 }, (_, k) => {
  const i = k + 1;
  return { ...fx.track, ...(i === 2 ? { canonical: [{ kind: 'recording', mbid: fx.track.mbid }] } : {}), id: i, title: `Song number ${i} with a fairly long title`, artists: [{ id: 11 + (i % 3), name: names[i % 3] }, { id: 30, name: 'Guest' }], album: { id: 7 + (i % 4), title: `Album ${i % 4}` }, duration_ms: 200000 + i * 1000 };
});

// The controls of the first row that sit in a box that cuts with "…", by name
const TARGETS = {
  'song title': '.title > a.lnk',
  'Match mark': '.title > .idmark',
  'first artist': '.by a.lnk[data-act="artist"], .by .lnk:not([data-act="album"])',
  'album': '.by [data-act="album"], .album-col [data-act="album"]',
};

// Focus a control as the keyboard does (so :focus-visible applies) and measure its ring against
// the nearest box that clips it
const ringReport = () => {
  const e = document.activeElement; const cs = getComputedStyle(e);
  const grow = (parseFloat(cs.outlineWidth) || 0) + (parseFloat(cs.outlineOffset) || 0);
  const r = e.getBoundingClientRect();
  const ring = { t: r.top - grow, b: r.bottom + grow, l: r.left - grow, rr: r.right + grow };
  let clip = null; let pad = null;
  for (let a = e.parentElement; a && !a.classList.contains('track'); a = a.parentElement) {
    const s = getComputedStyle(a);
    if (s.overflowX !== 'visible' || s.overflowY !== 'visible') { clip = a; pad = ['Top', 'Bottom', 'Left', 'Right'].map((k) => parseFloat(s[`padding${k}`]) || 0); break; }
  }
  const round = Math.min(parseFloat(cs.borderTopLeftRadius) || 0, r.height / 2) + grow;
  const out = { round, visible: e.matches(':focus-visible'), style: cs.outlineStyle, width: parseFloat(cs.outlineWidth) || 0, color: cs.outlineColor, ring, cut: [], past: [] };
  if (clip) {
    const c = clip.getBoundingClientRect(); // padding box: these boxes have no border
    // A side where the link itself runs on past the "…" can't have its ring there: that is noted
    // apart ("past"), and only a ring cut beside a link that fits counts as cut
    const side = (name, over, linkPast) => { if (over > 0.5) (linkPast ? out.past : out.cut).push(`${name} ${Math.round(over)}px`); };
    // (the "…" is at the text's edge, inside the padding)
    side('top', c.top - ring.t, r.top < c.top + pad[0] - 0.5);
    side('bottom', ring.b - c.bottom, r.bottom > c.bottom - pad[1] + 0.5);
    side('left', c.left - ring.l, r.left < c.left + pad[2] - 0.5);
    side('right', ring.rr - c.right, r.right > c.right - pad[3] + 0.5);
  }
  return out;
};

// Count the pixels of the ring's colour on its top and bottom edges, in a real screenshot
async function paintedEdges(page, ring, color, round) {
  const want = color.match(/\d+(\.\d+)?/g).slice(0, 3).map(Number);
  const near = (buf, w) => page.evaluate(async ([b64, want, w]) => {
    const img = new Image(); img.src = `data:image/png;base64,${b64}`; await img.decode();
    const c = document.createElement('canvas'); c.width = img.width; c.height = img.height;
    const g = c.getContext('2d'); g.drawImage(img, 0, 0);
    const d = g.getImageData(0, 0, c.width, c.height).data; let n = 0;
    // columns (in CSS px) with a pixel of the ring's colour
    const dpr = c.width / w; const hit = new Set();
    for (let i = 0; i < d.length; i += 4) if (Math.abs(d[i] - want[0]) + Math.abs(d[i + 1] - want[1]) + Math.abs(d[i + 2] - want[2]) < 60) hit.add(Math.floor(((i / 4) % c.width) / dpr));
    n = hit.size;
    return n;
  }, [buf.toString('base64'), want, w]);
  // (away from the rounded ends of a pill like the Match mark)
  const inset = Math.ceil(Math.max(6, round + 2)); const x = Math.floor(ring.l) + inset; const width = Math.max(4, Math.floor(ring.rr - ring.l) - 2 * inset);
  const top = await near(await page.screenshot({ clip: { x, y: Math.floor(ring.t), width, height: 2 } }), width);
  const bottom = await near(await page.screenshot({ clip: { x, y: Math.ceil(ring.b) - 2, width, height: 2 } }), width);
  return { top, bottom, width };
}

async function focusByKeyboard(page, loc) {
  await loc.focus();
  await page.keyboard.press('Shift+Tab');
  await page.keyboard.press('Tab');
}

// Where the row's text sits: compared with the clip-room rule switched off, nothing may move
const textPlaces = () => [...document.querySelectorAll('#main .track')].slice(0, 3).map((row) => {
  const at = (sel) => { const e = sel ? [...row.querySelectorAll(sel)].find((x) => x.getClientRects().length) : row; if (!e) return null; const r = e.getBoundingClientRect(); return [r.left, r.top, r.width, r.height].map((v) => Math.round(v * 10) / 10).join(','); };
  return { title: at('.title > a.lnk'), mark: at('.title > .idmark'), artist: at('.by .lnk'), album: at('[data-act="album"]'), row: at('') };
});
const OFF = '.track .title, .track .by, .track .album-col { padding: 0 !important; margin: 0 !important; }';

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 1024, 901, 768, 390]) {
      const T = `${width}`;
      const { page, context, state } = await fx.create(browser, width, { tracks });
      try {
        await fx.go(page, '/tracks');
        await page.waitForFunction(() => document.querySelectorAll('#main .track').length >= 8, null, { timeout: 8000 });
        const shell = await page.evaluate(() => document.documentElement.dataset.shell);
        const row = page.locator('#main .track').first();
        await row.hover();
        // (with a mouse, the row shows its Match a moment after the mouse arrives)
        if (shell !== 'phone') await row.locator('.title > .idmark').waitFor({ state: 'visible' });
        let seen = 0;
        for (const [name, sel] of Object.entries(TARGETS)) {
          const loc = row.locator(`${sel} >> visible=true`).first();
          if (!(await loc.count())) continue; // the phone's rows have no album and no Match mark
          seen++;
          await focusByKeyboard(page, loc);
          const on = await loc.evaluate((e) => e === document.activeElement);
          // (on a phone a tap on the row plays it and its names are not Tab stops: tools/ui-phone-song-rows.js)
          if (shell === 'phone') { check(!on && await loc.evaluate((e) => e.tabIndex) === -1, `${T}: the ${name} is not a Tab stop on a phone`); continue; }
          check(on, `${T}: the ${name} takes the keyboard focus`);
          if (!on) continue;
          const r = await page.evaluate(ringReport);
          check(r.visible && r.style !== 'none' && r.width >= 2, `${T}: the ${name} shows a focus ring by keyboard (${r.style} ${r.width}px, ${shell})`);
          check(r.cut.length === 0, `${T}: the ${name}'s ring is not cut by its row (${r.cut.join(', ') || 'whole'})`);
          const p = await paintedEdges(page, r.ring, r.color, r.round);
          check(p.top >= p.width * 0.8 && p.bottom >= p.width * 0.8, `${T}: the ${name}'s ring is painted above and below it (${p.top} and ${p.bottom} of ${p.width} px)`);
          if (name === 'first artist') await page.screenshot({ path: path.join(SHOTS, `artist-ring-${width}.png`), clip: { x: Math.max(0, r.ring.l - 60), y: r.ring.t - 30, width: 360, height: 70 } });
        }
        // A matched song's title is a plain line, not the Match mark's flex row: its ring fits too
        const plain = page.locator('#main .track').nth(1);
        check(await plain.locator('.idmark').count() === 0, `${T}: the second song is matched (no Match mark)`);
        await plain.hover();
        await focusByKeyboard(page, plain.locator('.title > a.lnk'));
        const pr = await page.evaluate(ringReport);
        // (and the link cuts itself with "…", so it doesn't run on past the title's edge)
        if (shell !== 'phone') check(pr.visible && pr.cut.length === 0 && pr.past.length === 0, `${T}: a matched song's title ring is whole (${pr.cut.join(', ') || 'whole'}${pr.past.length ? `; the link runs on past the "…": ${pr.past.join(', ')}` : ''})`);
        await row.hover();
        check(seen >= (shell === 'phone' ? 2 : 4), `${T}: the row's title, artist${shell === 'phone' ? '' : ', album and Match mark'} were each checked (${seen})`);
        await page.locator('#main h1, #main h2').first().hover();
        await page.evaluate(() => document.activeElement && document.activeElement.blur());

        // The text has not moved: the same places with the rule switched off
        const now = await page.evaluate(textPlaces);
        const off = await page.addStyleTag({ content: OFF });
        const without = await page.evaluate(textPlaces);
        await off.evaluate((e) => e.remove());
        check(JSON.stringify(now) === JSON.stringify(without), `${T}: title, artist, album and Match mark sit where they would without the room (${JSON.stringify(now[0])})`);

        // A click on the very edge of a link still lands on that link (the boxes now overlap a little);
        // on a phone the names take no taps (the row plays: tools/ui-phone-song-rows.js), so both land on the row
        const hits = await page.evaluate(() => {
          const r = document.querySelector('#main .track'); const t = r.querySelector('.title > a.lnk'); const a = r.querySelector('.by .lnk');
          const tr = t.getBoundingClientRect(); const ar = a.getBoundingClientRect();
          const tb = document.elementFromPoint(tr.left + 8, tr.bottom - 1); const at = document.elementFromPoint(ar.left + 4, ar.top + 1);
          return { titleBottom: tb === t, artistTop: at === a, inRow: r.contains(tb) && r.contains(at) };
        });
        if (shell === 'phone') check(!hits.titleBottom && !hits.artistTop && hits.inRow, `${T}: the title's and the artist's edges land on the row, not the links (${JSON.stringify(hits)})`);
        else check(hits.titleBottom && hits.artistTop, `${T}: the title's bottom edge and the artist's top edge each hit their own link (${JSON.stringify(hits)})`);

        const over = await page.evaluate(() => document.documentElement.scrollWidth - innerWidth);
        check(over <= 0, `${T}: nothing wider than the window (${over})`);
        await page.screenshot({ path: path.join(SHOTS, `songs-${width}.png`) });
        check(!state.errors.length, `${T}: no page errors (${state.errors.join(' | ')})`);
      } finally { await context.close(); }
    }

    // A long matched title (a plain link, no Match mark) cut by "…": the link itself ends at the
    // "…", so its ring is whole on all four sides, and the text sits where it did as a plain line
    const LONG = 'A matched song whose title goes on and on well past the width of its column';
    const matched = tracks.slice(0, 3).map((t, k) => ({ ...t, title: k ? t.title : LONG, canonical: [{ kind: 'recording', mbid: fx.track.mbid }] }));
    const FLAT = '.track .title { display: block !important; } .track .title > .lnk { overflow: visible !important; }';
    const textAt = () => { const e = document.querySelector('#main .track .title > a.lnk'); const g = document.createRange(); g.selectNodeContents(e); const r = g.getClientRects()[0]; return [r.left, r.top, r.height].map((v) => Math.round(v * 10) / 10).join(','); };
    for (const width of [1280, 1024, 901, 768, 390]) {
      const T = `${width} long matched title`;
      const { page, context, state } = await fx.create(browser, width, { tracks: matched });
      try {
        await fx.go(page, '/tracks');
        await page.waitForFunction(() => document.querySelectorAll('#main .track').length >= 3, null, { timeout: 8000 });
        const row = page.locator('#main .track').first();
        check(await row.locator('.idmark').count() === 0, `${T}: the song is matched (no Match mark)`);
        const link = row.locator('.title > a.lnk');
        const cut = await link.evaluate((e) => ({ dots: getComputedStyle(e).textOverflow === 'ellipsis' && e.scrollWidth > e.clientWidth + 1, inside: e.getBoundingClientRect().right <= e.parentElement.getBoundingClientRect().right - 3.5 }));
        check(cut.dots && cut.inside, `${T}: the title link ends in "…" inside the title (${JSON.stringify(cut)})`);
        await row.hover();
        // (a phone's song names are not Tab stops: no ring to see; the "…" and the text's place are)
        const phone = await page.evaluate(() => document.documentElement.dataset.shell === 'phone');
        if (!phone) {
          await focusByKeyboard(page, link);
          const r = await page.evaluate(ringReport);
          check(r.visible && r.cut.length === 0 && r.past.length === 0, `${T}: its ring is not cut (${[...r.cut, ...r.past].join(', ') || 'whole'})`);
          const p = await paintedEdges(page, r.ring, r.color, r.round);
          check(p.top >= p.width * 0.8 && p.bottom >= p.width * 0.8, `${T}: its ring is painted above and below it (${p.top} and ${p.bottom} of ${p.width} px)`);
          // the right side: a column of the ring's colour just inside the ring's right edge
          // (away from the rounded corners)
          const side = { x: Math.ceil(r.ring.rr) - 2, y: Math.floor(r.ring.t + r.round + 2), h: Math.max(4, Math.floor(r.ring.b - r.ring.t - 2 * r.round - 4)) };
          // (a ring running off the window has no right side to see)
          const col = side.x + 2 <= width ? await page.screenshot({ clip: { x: side.x, y: side.y, width: 2, height: side.h } }) : null;
          const rows = col ? await page.evaluate(async ([b64, color]) => {
            const want = color.match(/\d+(\.\d+)?/g).slice(0, 3).map(Number);
            const img = new Image(); img.src = `data:image/png;base64,${b64}`; await img.decode();
            const c = document.createElement('canvas'); c.width = img.width; c.height = img.height; const g = c.getContext('2d'); g.drawImage(img, 0, 0);
            const d = g.getImageData(0, 0, c.width, c.height).data; const hit = new Set();
            for (let i = 0; i < d.length; i += 4) if (Math.abs(d[i] - want[0]) + Math.abs(d[i + 1] - want[1]) + Math.abs(d[i + 2] - want[2]) < 60) hit.add(Math.floor(i / 4 / c.width));
            return { hit: hit.size, of: c.height };
          }, [col.toString('base64'), r.color]) : { hit: 0, of: side.h, off: true };
          check(rows.hit >= rows.of * 0.8, `${T}: its ring is painted on its right side (${rows.off ? 'off the window' : `${rows.hit} of ${rows.of} rows`})`);
          await page.screenshot({ path: path.join(SHOTS, `long-title-ring-${width}.png`), clip: { x: Math.max(0, r.ring.l - 60), y: r.ring.t - 20, width: Math.min(width, r.ring.rr - r.ring.l + 120), height: 70 } });
        }
        await page.locator('#main h1, #main h2').first().hover();
        await page.evaluate(() => document.activeElement && document.activeElement.blur());
        const now = await page.evaluate(textAt);
        const flat = await page.addStyleTag({ content: FLAT });
        const before = await page.evaluate(textAt);
        await flat.evaluate((e) => e.remove());
        check(now === before, `${T}: the title's text sits where it did as a plain line (${now} vs ${before})`);
        check(!state.errors.length, `${T}: no page errors (${state.errors.join(' | ')})`);
      } finally { await context.close(); }
    }

    // A long artist line: the album link past the "…" still scrolls into the line's view when
    // focused, and its ring is whole there too
    {
      const T = '768 long names';
      const long = [{ ...tracks[0], artists: [{ id: 31, name: 'Somebody With A Rather Long Artist Name Ensemble' }], album: { id: 9, title: 'An Album Title That Goes On And On For Sixty Characters Or So' } }];
      const { page, context, state } = await fx.create(browser, 768, { tracks: long });
      try {
        await fx.go(page, '/tracks');
        await page.waitForSelector('#main .track .by-album [data-act="album"]', { state: 'attached', timeout: 8000 });
        await focusByKeyboard(page, page.locator('#main .track .by [data-act="album"]'));
        const r = await page.evaluate(ringReport);
        const inView = await page.evaluate(() => {
          const by = document.querySelector('#main .track .by'); const a = document.activeElement;
          const br = by.getBoundingClientRect(); const ar = a.getClientRects()[0];
          return a.dataset.act === 'album' && ar.left < br.right - 8 && ar.right > br.left + 8;
        });
        check(inView, `${T}: focused, the album link scrolls into the artist line's view`);
        // The link is wider than the line, so only its visible part has a ring: above and below it is whole
        check(r.visible && r.cut.length === 0 && r.past.length > 0, `${T}: and its ring is whole above and below it (${r.cut.join(', ') || 'whole'}; the link runs on past the line: ${r.past.join(', ')})`);
        await page.screenshot({ path: path.join(SHOTS, 'long-768.png') });
        check(!state.errors.length, `${T}: no page errors (${state.errors.join(' | ')})`);
      } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(failed ? `song row focus ring: ${failed} FAIL` : 'song row focus ring: all ok');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });

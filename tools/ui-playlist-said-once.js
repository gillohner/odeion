// A playlist page says each thing once. Its top card already shows the count ("1 song · 1 link ·
// 4 min"), who sees it and where it is kept (the small line and the note under the buttons). The
// About section at the end repeated all three, and got two of them wrong: "Songs 2" counted a web
// link as a song, and a shared playlist read "Stored: your homeserver (private)". The About section
// is gone; the top card, the songs and Description stay. The one thing only About said, how many
// songs are in this library, is on the top line where nothing else says it: for a listener (who
// gets no source chips) and when finding where the songs play failed ("· 1 in this library", as a
// shared playlist's "N play here"). With the chips there ("1 Library"), the top line doesn't repeat it.
// Who sees it and where it is kept is said once too, by the note under the buttons: the small line
// above the name is the kind alone, "Playlist" (it said "PLAYLIST · PRIVATE · ON THIS NODE" above
// "Private on this node; …"), and the note has 8 px of air under the buttons (it touched them).
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-playlist-said-once.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/playlist-said-once-shots');
fs.mkdirSync(shots, { recursive: true });

let failed = 0; let passed = 0;
const check = (ok, what) => { if (ok) { passed++; console.log(`ok   ${what}`); } else { failed++; console.log(`FAIL ${what}`); } };

const withSummary = (o) => ({ ...fx.detail, summary: { ...fx.summary, ...o }, playlist: { ...fx.detail.playlist, ...o } });
const cases = [
  { name: 'private', d: withSummary({}), note: /Private on this node/, sub: '1 song · 1 link · 4 min', chips: true },
  { name: 'public-synced', d: withSummary({ visibility: 'public', synced: true }), note: /Public on Pubky/, sub: '1 song · 1 link · 4 min', chips: true },
  { name: 'private-synced', d: withSummary({ synced: true }), note: /^Private on your homeserver/, sub: '1 song · 1 link · 4 min', chips: true },
  { name: 'public-pending', d: withSummary({ visibility: 'public' }), note: /^Public copy pending/, sub: '1 song · 1 link · 4 min', chips: true },
  { name: 'listener', role: 'listener', d: withSummary({}), note: /Private/, sub: '1 song · 1 link · 4 min · 1 in this library', chips: false },
  { name: 'sources-failed', d: withSummary({}), pfFail: true, note: /Private on this node/, sub: '1 song · 1 link · 4 min · 1 in this library', chips: false },
];

(async () => {
  const browser = await chromium.launch();
  try {
    for (const w of [1280, 1024, 768, 412]) {
      for (const c of cases) {
        const fixtures = { '/playlists/c1': { body: c.d }, ...(c.pfFail ? { '/playlists/c1/play-from': { body: { error: 'Fixture: no answer' }, status: 500 } } : {}) };
        const { page, context, state } = await fx.create(browser, w, { ...(c.role ? { role: c.role } : {}), collections: [c.d.summary], fixtures });
        try {
          await fx.go(page, '/playlist/c1');
          await page.waitForSelector('#plsongs .srow');
          await page.waitForSelector('#pldesc');
          const got = await page.evaluate(() => {
            const main = document.querySelector('#main');
            return {
              heads: [...main.querySelectorAll('.section-head h3')].map((h) => h.textContent.trim()),
              kv: main.querySelectorAll('dl.kv').length,
              sub: (main.querySelector('.hero .sub') || {}).textContent || '',
              note: (main.querySelector('.hero p[role="status"]') || {}).textContent || '',
              chips: !!main.querySelector('.plsummary [data-pfilter]'),
              eyebrow: (main.querySelector('.hero .eyebrow') || {}).innerText || '',
              eyebrowLines: (() => { const e = main.querySelector('.hero .eyebrow'); if (!e) return 0; const cs = getComputedStyle(e); return Math.round(e.getBoundingClientRect().height / (parseFloat(cs.lineHeight) || parseFloat(cs.fontSize) * 1.2)); })(),
              air: (() => { const a = main.querySelector('.hero .actions'); const n = a && a.nextElementSibling; return n ? n.getBoundingClientRect().top - Math.max(...[...a.children].filter((b) => b.getClientRects().length).map((b) => b.getBoundingClientRect().bottom)) : null; })(),
              text: main.innerText,
            };
          });
          const at = `${w} ${c.name}`;
          check(!got.heads.includes('About'), `${at}: no About section (sections: ${got.heads.join(', ')})`);
          check(got.kv === 0, `${at}: no Songs / Length / Stored list`);
          check(!/\bStored\b/.test(got.text), `${at}: "Stored" is not said again below the songs`);
          check(got.heads[0] === 'Songs' && got.heads.includes('Description'), `${at}: Songs and Description stay`);
          check(got.sub.trim() === c.sub, `${at}: the top line reads ${JSON.stringify(got.sub.trim())} (want ${JSON.stringify(c.sub)})`);
          check(got.chips === c.chips, `${at}: source chips ${got.chips ? 'shown' : 'not shown'} (want ${c.chips ? 'shown' : 'not shown'})`);
          check(!/\b2 · 1 in this library\b/.test(got.text), `${at}: the link is not counted as a second song`);
          check(c.note.test(got.note), `${at}: the note under the buttons still says who sees it and where (${JSON.stringify(got.note)})`);
          check(/^playlist$/i.test(got.eyebrow.trim()) && got.eyebrowLines === 1, `${at}: the line above the name is the kind alone, on one line (${JSON.stringify(got.eyebrow.trim())}, ${got.eyebrowLines} lines)`);
          check(!/\b(private|public|homeserver|this node)\b/i.test(got.eyebrow), `${at}: who sees it and where is not said again above the name`);
          check(got.air >= 6, `${at}: the note has air under the buttons (${got.air == null ? 'no note' : got.air.toFixed(1) + ' px'})`);
          if (c.name === 'public-synced') check(!/\(private\)/.test(got.text), `${at}: a shared playlist is never called private`);
          let copy = null; try { await fx.cleanCopy(page); } catch (e) { copy = e.message; }
          check(!copy, `${at}: clean copy, no horizontal overflow ${copy || ''}`);
          check(!state.errors.length, `${at}: no page errors ${state.errors.join(' ')}`);
          await page.evaluate(() => { const s = document.querySelectorAll('#main .section'); s[s.length - 1].scrollIntoView({ block: 'end' }); });
          await page.screenshot({ path: path.join(shots, `playlist-${c.name}-${w}.png`) });
        } finally { await context.close(); }
      }
    }
  } finally { await browser.close(); }
  console.log(`\n${passed} ok, ${failed} FAIL`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });

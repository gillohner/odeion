// Queue rows are one height at every width: a song's artist · album line is cut with "…" when it
// doesn't fit, as on Songs and on a phone's queue, and the whole words stay in the row for a
// screen reader. Before, on a computer or tablet a long album
// name ("… (Deluxe Anniversary Edition)") wrapped onto a second line, so one row was taller than
// the rest. Short lines are not cut. Nothing wider than the window.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-queue-row-height.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'queue-row-height-shots'));
fs.mkdirSync(shots, { recursive: true });

let ok = 0; let fail = 0;
const check = (cond, msg) => { if (cond) { ok++; console.log(`ok   ${msg}`); } else { fail++; console.log(`FAIL ${msg}`); } };
const LONG = 'The Very Long Album Name That Goes On And On Through Every Season Of The Year (Deluxe Anniversary Edition, Remastered 2026, Bonus Tracks)';
const saved = [
  { id: 1, uri: 'track:1', track_id: 1, title: 'Song 1', artist: 'Sefa', album: 'Journey', duration_ms: 60000 },
  { id: 2, uri: 'track:2', track_id: 2, title: 'Song 2', artist: 'An Artist With A Long Name', album: LONG, duration_ms: 60000 },
  { id: 3, uri: 'track:3', track_id: 3, title: 'Song 3', artist: 'Sefa', album: 'Journey', duration_ms: 60000 },
];

const run = async (browser, width) => {
  const phone = width < 600;
  const { context, page, state } = await fx.create(browser, width, { ...(phone ? { mobile: true } : {}) });
  try {
    await context.addInitScript(([q, a]) => { localStorage.setItem('pm.browserQueue', q); localStorage.setItem('pm.browserAt', a); }, [JSON.stringify(saved), JSON.stringify({ index: 0, id: 1, pos: 0 })]);
    await fx.go(page, '/queue');
    await page.waitForSelector('#main [data-qrow="2"]', { timeout: 8000 });
    await page.waitForTimeout(200);
    const rows = await page.evaluate(() => [...document.querySelectorAll('#main [data-qrow]')].map((r) => {
      const sub = r.querySelector('.sub'); const lh = parseFloat(getComputedStyle(sub).lineHeight) || 16;
      return { h: r.clientHeight, subLines: Math.round(sub.getBoundingClientRect().height / lh), cut: sub.scrollWidth > sub.clientWidth + 1 && getComputedStyle(sub).textOverflow === 'ellipsis', text: sub.textContent };
    }));
    const at = `${width}${phone ? ' (phone)' : ''}`;
    check(rows.length === 3, `${at}: three queue rows (${rows.length})`);
    check(new Set(rows.map((r) => r.h)).size === 1, `${at}: every queue row is one height (${rows.map((r) => r.h).join(', ')} px)`);
    check(rows[1].subLines === 1, `${at}: the long artist · album line stays on one line (${rows[1].subLines} lines)`);
    check(rows[1].cut, `${at}: the long line is cut with "…", not wrapped`);
    check(rows[1].text.includes(LONG), `${at}: the whole words stay in the row`);
    check(!rows[0].cut && rows[0].text === 'Sefa · Journey', `${at}: a short line is not cut`);
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${at}: no sideways scroll`);
    await page.screenshot({ path: path.join(shots, `queue-${width}.png`) });
    check(state.errors.length === 0, `${at}: no page errors (${state.errors.join(' | ')})`);
  } finally { await context.close(); }
};

(async () => {
  const browser = await chromium.launch();
  try { for (const w of [1280, 1024, 768, 412, 360]) await run(browser, w); } finally { await browser.close(); }
  console.log(`${ok} ok, ${fail} failed`);
  if (fail) process.exit(1);
})().catch((e) => { console.error(e); process.exit(1); });

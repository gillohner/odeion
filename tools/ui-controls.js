// Control audit: every visible button does something (STEWARD-PLAN step 4, "nav and control
// audits"). For each main view, each distinct visible button (rows repeat the same buttons; one
// of each kind is enough) is clicked on a fresh page load, and the click must change something:
// the DOM, the URL, a request to the node, a dialog or a new window. A button that changes nothing
// is a dead control and fails the run, and so does one still busy (aria-busy) 10 s after its click.
// Buttons with lasting effects (delete, sign in or out, scan, sync, import, …) are skipped; run it
// against a node whose data you can lose (a copy, or CI's fixture node).
//   npm install playwright@1.49.1 && npx playwright install chromium   (anywhere; run from there)
//   node tools/ui-controls.js <api token> [http://127.0.0.1:7788]
// UI_SHARD=k/n clicks only the controls whose view and kind hash to k (CI splits the audit over
// runners this way; every control falls in exactly one of the n shards). UI_CONTROLS_REPORT=<file>
// writes the controls this run saw and clicked, so CI can check that each one a shard saw was also
// in its own shard's list (a control that appears only after another shard's click would otherwise
// be clicked by none).
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const TOKEN = process.argv[2]; const BASE = process.argv[3] || 'http://127.0.0.1:7788';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const [SHARD, SHARDS] = (process.env.UI_SHARD || '1/1').split('/').map(Number);
if (!(SHARDS >= 1 && SHARD >= 1 && SHARD <= SHARDS)) { console.log(`UI_SHARD must be k/n with 1 <= k <= n, not ${process.env.UI_SHARD}`); process.exit(2); }
// FNV-1a of the view and the control's kind
// A control's shard comes from its view and its `key`: its kind without the state it shows (a
// Repeat button reads "Repeat: off" in one shard and "Repeat: this song" in another, after that
// shard's own clicks; run 42571268508 lost 38 such controls when the state picked the shard).
const owner = (view, key) => { let h = 2166136261; for (const ch of `${view}\n${key}`) { h ^= ch.codePointAt(0); h = Math.imul(h, 16777619) >>> 0; } return (h % SHARDS) + 1; };
const mine = (view, key) => owner(view, key) === SHARD;
const SKIP = /compact|comfortable|share on pubky|add my|bring my|make private|delete|remove|trash|sign (in|out)|sign in again|log ?out|forget|clear|reset|rescan|scan|sync|import|mirror|upload|download|backup|restore|tag my whole|identify|accept|write ids|write into|fixgo|tag every|copy library|bring files|restart|stop|change token|add folder|create|save|apply|disconnect|connect|pubky ring|copy/i;
const api = async (path) => (await fetch(BASE + '/v1' + path, { headers: { Authorization: 'Bearer ' + TOKEN } })).json();

(async () => {
  const views = ['/', '/favourites', '/albums', '/artists', '/genres', '/tracks', '/playlists', '/radio', '/sources', '/queue', '/zones', '/settings', ...['playback', 'accounts', 'discover', 'backup', 'import', 'appearance', 'about'].map((t) => '/settings/' + t), '/search?q=queen'];
  // A few Sources tabs with their own controls (the overlap and a11y audits visit every tab; all of them here
  // would take this audit past CI's 30 minutes)
  views.push(...['tidal', 'spotify', 'subsonic', 'youtube'].map((id) => '/sources/' + id));
  try { const pls = await api('/playlists'); const pl = pls.find((p) => p.items > 0); if (pl) views.push('/playlist/' + pl.id); } catch { /* none */ }
  try { const al = await api('/library/albums'); if (al[0]) views.push('/album/lib-' + al[0].id); } catch { /* none */ }
  views.push('/song/mb-b1a9c0e9-d987-4042-ae91-78d6a3267d69');
  const browser = await chromium.launch();
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 900 }, colorScheme: 'dark' });
  if (process.env.UI_CONTROLS_INPUTS) {
    const frozen = JSON.parse(require('node:fs').readFileSync(process.env.UI_CONTROLS_INPUTS, 'utf8'));
    if (frozen.query !== 'queen' || !frozen.sources.length || !frozen.answers.length) throw new Error('invalid control-audit search inputs');
    await ctx.route(/\/v1\/search\?/, (route) => {
      const url = new URL(route.request().url());
      if (url.searchParams.get('q') !== frozen.query) return route.continue();
      const scope = url.searchParams.get('in') || 'everywhere';
      const sources = scope === 'everywhere' ? frozen.sources : scope === 'pubky' ? frozen.sources.filter((s) => ['pubky', 'people', 'posts'].includes(s)) : scope.split(',');
      const answers = frozen.answers.filter((a) => sources.includes(a.source));
      return route.fulfill({ status: 200, contentType: 'application/x-ndjson', headers: { 'x-pm-sources': sources.join(',') }, body: answers.map((a) => JSON.stringify(a)).join('\n') + '\n' });
    });
  } else if (SHARDS > 1) throw new Error('sharded control audits require shared UI_CONTROLS_INPUTS');
  await ctx.addInitScript((t) => {
    localStorage.setItem('pm.token', t); localStorage.setItem('pm.zone', 'browser');
    HTMLMediaElement.prototype.play = async function () {};
    window.open = () => { window.__opened = (window.__opened || 0) + 1; return null; };
  }, TOKEN);
  const p = await ctx.newPage();
  const pageErrors = [];
  p.on('pageerror', (e) => pageErrors.push(`${p.url()}: ${e.message}`));
  let requests = 0; p.on('request', (r) => { if (r.url().includes('/v1/') || r.url().includes('/mb/')) requests++; });
  // Requests still open, and when the last one ended: a click's answer is handled (and may throw)
  // when it arrives, so the audit moves on only once nothing is in flight
  let inflight = 0; let lastDone = 0;
  p.on('request', () => { inflight++; });
  for (const ev of ['requestfinished', 'requestfailed']) p.on(ev, () => { inflight = Math.max(0, inflight - 1); lastDone = Date.now(); });
  const settle = async () => { await p.waitForLoadState('networkidle', { timeout: 15000 }).catch(() => {}); await sleep(400); };
  // Visible, enabled buttons, one per kind: class, data attributes and wording (digits dropped).
  const kinds = () => p.$$eval('button', (els) => {
    const seen = new Set(); const out = [];
    els.forEach((b, i) => {
      const r = b.getBoundingClientRect(); const st = getComputedStyle(b);
      if (b.disabled || r.width < 2 || r.height < 2 || st.visibility === 'hidden' || st.display === 'none' || Number(st.opacity) === 0 || b.closest('[hidden]')) return;
      if (r.bottom < 0 || r.top > innerHeight * 4) return;
      const data = Object.keys(b.dataset).sort().join(',');
      const words = (b.getAttribute('aria-label') || b.title || b.textContent || '').replace(/\d+/g, '#').replace(/\s+/g, ' ').trim().slice(0, 40);
      const sig = `${b.className}|${data}|${words}`;
      if (seen.has(sig)) return; seen.add(sig);
      b.dataset.auditIdx = String(i);
      // The kind without its state: state classes dropped, and the wording up to a colon
      const cls = b.className.split(/\s+/).filter((c) => c && !/^(on|off|active|selected|current|open|playing|paused|pressed|checked|is-.*)$/.test(c)).join(' ');
      const key = `${cls}|${data}|${words.split(':')[0].trim()}`;
      out.push({ i, sig, key, words, id: b.id });
    });
    return out;
  });
  const dead = []; const stuck = []; let stuckBudget = 60000; let clicked = 0; let skipped = 0;
  const seen = []; const clickedSigs = [];
  for (const view of views) {
    await p.goto(BASE + view); await settle();
    if (process.env.UI_CONTROLS_INPUTS && view === '/search?q=queen') {
      // Count the actual merged results. Raw hits may be duplicate copies or failed-source
      // answers, neither of which supplies the controls that this audit must exercise.
      const rendered = await p.evaluate(() => ({
        moreSongs: !!document.querySelector('[data-sallsongs]'),
        songs: document.querySelectorAll('[data-skind="song"] [data-srow]').length,
        albums: document.querySelectorAll('[data-skind="album"] .tile').length,
        artists: document.querySelectorAll('[data-skind="artist"] .tile').length,
      }));
      if (!rendered.moreSongs || rendered.songs < 6 || rendered.albums < 12 || rendered.artists < 1) {
        throw new Error('shared search inputs lost rendered control coverage: ' + JSON.stringify(rendered));
      }
    }
    const list = await kinds();
    for (const k of list) {
      const lasting = SKIP.test(k.words) || SKIP.test(k.id) || /layoutPick/.test(k.sig);
      if (!lasting) seen.push({ view, sig: k.sig, key: k.key, owner: owner(view, k.key) });
      if (!mine(view, k.key)) continue;
      if (lasting) { skipped++; continue; }
      await p.goto(BASE + view); await settle();
      const now = await kinds(); const target = now.find((x) => x.sig === k.sig);
      if (!target) continue;
      const before = { url: p.url(), req: requests };
      await p.evaluate(() => { window.__scroll = (document.querySelector('#main') || document.scrollingElement).scrollTop + scrollY; window.__focus = document.activeElement; window.__mut = 0; window.__mo && window.__mo.disconnect(); window.__mo = new MutationObserver((ms) => { window.__mut += ms.filter((m) => !(m.type === 'attributes' && m.attributeName === 'data-audit-idx')).length; }); window.__mo.observe(document.body, { subtree: true, childList: true, attributes: true, characterData: true }); window.__opened = 0; });
      const el = await p.$(`button[data-audit-idx="${target.i}"]`);
      if (!el) continue;
      try { await el.click({ timeout: 3000 }); } catch { try { await el.click({ force: true, timeout: 2000 }); } catch { continue; } }
      clicked++; clickedSigs.push({ view, sig: k.sig, key: k.key });
      // Moving the view (scrolling to a section) counts; moving focus alone does not: people do not see it.
      // Up to 900 ms for the click to show, looked at every 100 ms. The audit moves on sooner only once
      // the click showed, no request is open and the last one ended 150 ms ago (its answer handled, any
      // error thrown); a control that changes nothing is still watched for the whole 900 ms.
      const clickedAt = Date.now(); let did = false;
      for (;;) {
        const after = await p.evaluate(() => ({ mut: window.__mut || 0, opened: window.__opened || 0, dialog: !!document.querySelector('dialog[open]'), scrolled: Math.abs((document.querySelector('#main') || document.scrollingElement).scrollTop + scrollY - window.__scroll) > 4 })).catch(() => ({ mut: 1 }));
        did = after.mut > 0 || after.opened > 0 || after.dialog || after.scrolled || p.url() !== before.url || requests > before.req;
        const quiet = inflight === 0 && Date.now() - lastDone >= 150;
        if ((did && quiet) || Date.now() - clickedAt >= 900) break;
        await sleep(100);
      }
      if (!did) dead.push(`${view}  "${k.words}"  (${k.sig})`);
      await p.keyboard.press('Escape').catch(() => {});
      // A control whose action never settles would stay busy (and ignore presses) for good
      // (at most 10 s for one control, 60 s of waiting in all, so the run's time stays bounded)
      // A control is counted only when it was watched for the full 10 s; once the budget is spent the
      // check is skipped
      let busyNow = []; let waited = 0;
      if (stuckBudget > 0) for (;;) { busyNow = await p.evaluate(() => [...document.querySelectorAll('.is-busy')].map((e) => (e.textContent || e.getAttribute('aria-label') || e.title || e.tagName).trim().slice(0, 40))).catch(() => []); if (!busyNow.length || waited >= 10000 || stuckBudget <= 0) break; await sleep(500); waited += 500; stuckBudget -= 500; }
      if (busyNow.length && waited >= 10000) stuck.push(`${view}  "${k.words}"  (${k.sig}): still busy after 10 s: ${busyNow.join(', ')}`);
    }
  }
  await browser.close();
  if (process.env.UI_CONTROLS_REPORT) require('node:fs').writeFileSync(process.env.UI_CONTROLS_REPORT, JSON.stringify({ shard: SHARD, shards: SHARDS, seen, clicked: clickedSigs }));
  console.log(`${views.length} views, ${clicked} controls clicked, ${skipped} skipped (lasting effects)${SHARDS > 1 ? `, shard ${SHARD} of ${SHARDS}` : ''}`);
  for (const d of dead) console.log('DEAD ' + d);
  for (const d of stuck) console.log('STUCK ' + d);
  for (const e of pageErrors) console.log('page error: ' + e);
  if (dead.length || stuck.length || pageErrors.length) { console.log(`${dead.length} dead controls, ${stuck.length} controls left busy, ${pageErrors.length} page errors`); process.exit(1); }
  console.log('every control does something');
})();

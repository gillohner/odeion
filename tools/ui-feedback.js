// Help › Report a problem (feedback.js) on phones, tablets and desktops (TASK-trusted-user-feedback):
// - "?" opens a menu by keyboard or pointer; Escape closes it and focus goes back to "?";
// - the dialog says where a report goes before anything is sent, and no claim that anyone has it;
// - technical details are off until ticked, and what is sent is exactly what the preview shows;
// - an empty report isn't sent; a lost connection, "too many" and a full inbox keep the text, and
//   sending again after a lost connection uses the same report id (no duplicate);
// - "My reports" shows each state, and a report's text stays text (no markup runs);
// - the dialog fits a 390 px phone with nothing cut off.
// The page that mounts feedback.js is a test page. `mock`: the node is mocked too (no build);
// a node URL: reports go to that node for real (synthetic text only).
//   node tools/ui-feedback.js <api token> <node URL | mock> [screenshot dir]
//   node tools/ui-feedback.js <api token> <node URL> --after-intake   (after tools/feedback-intake.py ran: every
//   report shows "Received for review" in My reports)
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const TOKEN = process.argv[2] || 'ci-token';
const NODE = process.argv[3] || 'mock';
const AFTER_INTAKE = process.argv[4] === '--after-intake';
const SHOTS = AFTER_INTAKE ? null : process.argv[4];
const MOCK = NODE === 'mock';
const ORIGIN = MOCK ? 'http://feedback.test' : NODE;
const STATIC = path.join(__dirname, '..', 'crates', 'pm-api', 'static');
const HOST = `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="stylesheet" href="/app.css"><title>feedback test page</title></head>
<body><header class="topbar" id="top" style="display:flex;justify-content:flex-end;padding:8px 16px"></header><main><p>Test page</p></main>
<script src="/feedback.js"></script>
<script src="/__fb-mount.js"></script></body></html>`;
const MOUNT = `PM_FEEDBACK.mount(document.getElementById('top'), { token: () => ${JSON.stringify(TOKEN)}, view: () => 'queue',
  shell: () => document.documentElement.clientWidth < 600 ? 'phone' : document.documentElement.clientWidth < 1024 ? 'tablet' : 'desktop', version: () => '0.2.3' });`;
let passed = 0;
const ok = (cond, what) => { assert.ok(cond, what); passed += 1; console.log(`ok ${what}`); };

// After an intake: the person sees their reports received, as the node now says
const afterIntake = async (browser) => {
  const page = await browser.newPage();
  await page.route(`${ORIGIN}/__fb-host`, (r) => r.fulfill({ contentType: 'text/html', body: HOST }));
  await page.route(`${ORIGIN}/__fb-mount.js`, (r) => r.fulfill({ contentType: 'text/javascript', body: MOUNT }));
  for (const f of ['feedback.js', 'feedback.css', 'app.css']) {
    await page.route(`${ORIGIN}/${f}`, (r) => r.fulfill({ contentType: f.endsWith('.js') ? 'text/javascript' : 'text/css', body: fs.readFileSync(path.join(STATIC, f), 'utf8') }));
  }
  await page.goto(`${ORIGIN}/__fb-host`);
  await page.getByRole('button', { name: 'Help and problem reports' }).click();
  await page.getByRole('menuitem', { name: 'My reports' }).click();
  const list = page.getByRole('dialog', { name: 'My reports' });
  await list.getByRole('listitem').first().waitFor();
  const rows = await list.getByRole('listitem').allTextContents();
  ok(rows.length >= 3 && rows.every((r) => r.includes('Received for review')), `after intake every report shows received (${rows.length})`);
};

(async () => {
  const browser = await chromium.launch();
  try {
    if (AFTER_INTAKE) { await afterIntake(browser); console.log(`${passed} passed`); return; }
    for (const width of [390, 768, 1280]) {
      const phone = width < 600;
      const ctx = await browser.newContext({ viewport: { width, height: phone ? 760 : 900 }, reducedMotion: 'reduce', hasTouch: phone, isMobile: phone });
      const page = await ctx.newPage();
      const errors = []; page.on('pageerror', (e) => errors.push(e.message));
      // The test page and its scripts; with a real node only /v1 goes to it
      await page.route(`${ORIGIN}/__fb-host`, (r) => r.fulfill({ contentType: 'text/html', body: HOST }));
      await page.route(`${ORIGIN}/__fb-mount.js`, (r) => r.fulfill({ contentType: 'text/javascript', body: MOUNT }));
      for (const f of ['feedback.js', 'feedback.css', 'app.css']) {
        await page.route(`${ORIGIN}/${f}`, (r) => r.fulfill({ contentType: f.endsWith('.js') ? 'text/javascript' : 'text/css', body: fs.readFileSync(path.join(STATIC, f), 'utf8') }));
      }
      // Requests the page makes, and (mock) the node's answers
      const posts = [];
      const reports = [];
      let next = null; // the next POST's forced answer: 'abort', 429, 507
      await page.route(`${ORIGIN}/v1/feedback`, async (r) => {
        const body = r.request().postDataJSON();
        posts.push({ body, auth: r.request().headers().authorization });
        if (next === 'abort') { next = null; return r.abort('connectionreset'); }
        if (next === 429) { next = null; return r.fulfill({ status: 429, headers: { 'retry-after': '120' }, json: { error: 'too many' } }); }
        if (next === 507) { next = null; return r.fulfill({ status: 507, json: { error: "The node's report inbox is full. Tell the node's owner." } }); }
        if (!MOCK) return r.continue();
        const same = reports.find((x) => x.client_id === body.client_id);
        if (same) return r.fulfill({ status: 200, json: { id: same.id, state: 'saved' } });
        const id = `fb_${String(reports.length).padStart(16, '0')}`;
        reports.push({ id, client_id: body.client_id, kind: body.kind, sent_at: 1_800_000_000 + reports.length, state: 'saved', preview: body.text.trim().slice(0, 80) });
        return r.fulfill({ status: 201, json: { id, state: 'saved' } });
      });
      if (MOCK) await page.route(`${ORIGIN}/v1/feedback/mine`, (r) => r.fulfill({ json: [...reports].sort((x, y) => y.sent_at - x.sent_at) }));
      await page.goto(`${ORIGIN}/__fb-host`);
      const w = `[${width}]`;

      // "?" by keyboard: the menu, arrows, Escape back to "?"
      const help = page.getByRole('button', { name: 'Help and problem reports' });
      await help.focus();
      await page.keyboard.press('Enter');
      const item1 = page.getByRole('menuitem', { name: 'Report a problem…' });
      ok(await item1.evaluate((el) => el === document.activeElement), `${w} Enter on ? opens Help with its first item focused`);
      ok(await help.getAttribute('aria-expanded') === 'true', `${w} ? says its menu is open`);
      await page.keyboard.press('ArrowDown');
      ok(await page.getByRole('menuitem', { name: 'My reports' }).evaluate((el) => el === document.activeElement), `${w} ArrowDown moves to My reports`);
      await page.keyboard.press('Escape');
      ok(await page.getByRole('menu').count() === 0 && await help.evaluate((el) => el === document.activeElement), `${w} Escape closes Help and focus returns to ?`);
      if (phone) { const b = await help.boundingBox(); ok(b.height >= 36 && b.width >= 36, `${w} ? is at least 36 px to touch (${b.width}×${b.height})`); }

      // The dialog: where it goes, details off, nothing sent empty
      await page.keyboard.press('Enter'); await page.keyboard.press('Enter');
      const dialog = page.getByRole('dialog', { name: 'Report a problem' });
      await dialog.waitFor();
      const text = dialog.getByRole('textbox', { name: 'Description' });
      ok(await text.evaluate((el) => el === document.activeElement), `${w} the description has focus`);
      const share = await dialog.locator('.fb-share').textContent();
      ok(/Saved privately on this node/.test(share) && /Not public/.test(share), `${w} the dialog says where a report goes`);
      ok(!/Hermes|received|reviewed/i.test(share), `${w} no claim that anyone has received it`);
      const tick = dialog.getByRole('checkbox', { name: 'Include technical details' });
      ok(!(await tick.isChecked()) && !(await dialog.locator('#fb-preview').isVisible()), `${w} technical details are off and hidden`);
      await dialog.getByRole('button', { name: 'Send' }).click();
      ok(/Describe the problem first/.test(await dialog.getByRole('alert').textContent()) && posts.length === 0, `${w} an empty report isn't sent`);
      const bb = await dialog.boundingBox();
      ok(bb.x >= 0 && bb.x + bb.width <= width + 0.5, `${w} the dialog fits the screen (${Math.round(bb.x)}+${Math.round(bb.width)})`);
      ok(await page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth), `${w} no sideways scrolling`);

      // Lost connection: the text stays; sending again is the same report
      const words = `SYNTHETIC ${width}: the queue skipped <img src=x onerror="window.__pwned=1"> a song`;
      await text.fill(words);
      await dialog.getByRole('radio', { name: 'Idea' }).check();
      next = 'abort';
      await dialog.getByRole('button', { name: 'Send' }).click();
      await dialog.getByRole('alert').filter({ hasText: "Couldn't reach the node" }).waitFor();
      ok(await text.inputValue() === words, `${w} a lost connection keeps the text`);
      ok(!('details' in posts[0].body) && posts[0].body.kind === 'idea' && posts[0].auth === `Bearer ${TOKEN}`, `${w} unticked: no details sent, the chosen kind and the token are`);
      // Too many; inbox full
      next = 429;
      await dialog.getByRole('button', { name: 'Send' }).click();
      await dialog.getByRole('alert').filter({ hasText: 'try again in 2 minutes' }).waitFor();
      ok(await text.inputValue() === words, `${w} "too many" keeps the text and says when`);
      next = 507;
      await dialog.getByRole('button', { name: 'Send' }).click();
      await dialog.getByRole('alert').filter({ hasText: 'inbox is full' }).waitFor();
      ok(true, `${w} a full inbox says so`);
      // Details: ticked, previewed, sent exactly
      await tick.check();
      const shown = JSON.parse(await dialog.locator('#fb-preview').textContent());
      ok(shown.view === 'queue' && shown.shell === (phone ? 'phone' : width < 1024 ? 'tablet' : 'desktop') && shown.version === '0.2.3' && shown.browser === 'chromium', `${w} the preview shows the details (${JSON.stringify(shown)})`);
      if (SHOTS) await page.screenshot({ path: path.join(SHOTS, `feedback-${width}-dialog.png`) });
      await dialog.getByRole('button', { name: 'Send' }).click();
      const saved = page.getByRole('dialog', { name: 'Report saved' });
      await saved.waitFor();
      const last = posts[posts.length - 1].body;
      ok(JSON.stringify(last.details) === JSON.stringify(shown), `${w} exactly the previewed details are sent`);
      ok(new Set(posts.map((p) => p.body.client_id)).size === 1, `${w} every try is the same report (one client id)`);
      ok(/Saved privately on this node/.test(await saved.textContent()) && !/Hermes/.test(await saved.textContent()), `${w} saved, with no claim beyond the node`);
      if (SHOTS) await page.screenshot({ path: path.join(SHOTS, `feedback-${width}-saved.png`) });

      // My reports: states, and the text stays text
      if (MOCK) { reports.push({ id: 'fb_r', client_id: 'x', kind: 'problem', sent_at: 1_700_000_000, state: 'received', preview: 'earlier' }, { id: 'fb_v', client_id: 'y', kind: 'question', sent_at: 1_600_000_000, state: 'reviewed', preview: 'older' }); }
      await saved.getByRole('button', { name: 'My reports' }).click();
      const list = page.getByRole('dialog', { name: 'My reports' });
      await list.getByRole('listitem').first().waitFor();
      const rows = await list.getByRole('listitem').allTextContents();
      ok(rows.some((r) => r.includes('Saved on the node') && r.includes('<img src=x')), `${w} the new report shows saved, its text as text`);
      if (MOCK) ok(rows.some((r) => r.includes('Received for review')) && rows.some((r) => r.includes('Reviewed')), `${w} received and reviewed show`);
      ok(await list.locator('img').count() === 0 && await page.evaluate(() => window.__pwned === undefined), `${w} no markup from a report runs`);
      if (SHOTS) await page.screenshot({ path: path.join(SHOTS, `feedback-${width}-mine.png`) });
      await page.keyboard.press('Escape');
      ok(await page.getByRole('dialog').count() === 0 && await help.evaluate((el) => el === document.activeElement), `${w} Escape closes My reports, focus back on ?`);
      ok(errors.length === 0, `${w} no page errors ${errors.join('; ')}`);
      await ctx.close();
    }
    console.log(`${passed} passed`);
  } finally {
    await browser.close();
  }
})().catch((e) => { console.error(e); process.exit(1); });

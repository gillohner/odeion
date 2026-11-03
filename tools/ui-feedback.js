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
const MOUNT = `PM_FEEDBACK.mount(document.getElementById('top'), { token: () => window.__fbToken || ${JSON.stringify(TOKEN)}, view: () => 'queue',
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
      let next = null; // the next POST's forced answer: 'abort', 429, 507, 409
      let hold = null; // a promise the next POST (or My reports) waits on before it is answered
      let holdMine = null;
      await page.route(`${ORIGIN}/v1/feedback`, async (r) => {
        const body = r.request().postDataJSON();
        posts.push({ body, auth: r.request().headers().authorization });
        if (hold) { const h = hold; hold = null; await h; }
        if (next === 'abort') { next = null; return r.abort('connectionreset'); }
        if (next === 429) { next = null; return r.fulfill({ status: 429, headers: { 'retry-after': '120' }, json: { error: 'too many' } }); }
        if (next === 507) { next = null; return r.fulfill({ status: 507, json: { error: "The node's report inbox is full. Tell the node's owner." } }); }
        if (next === 409) { next = null; return r.fulfill({ status: 409, json: { error: 'This report was already saved as you first wrote it. Send again to save this version as a new report.' } }); }
        if (!MOCK) return r.continue();
        // Like the node: the same id again is the same report, or 409 when its words changed
        const same = reports.find((x) => x.client_id === body.client_id);
        if (same && same.text !== body.text) return r.fulfill({ status: 409, json: { error: 'This report was already saved as you first wrote it. Send again to save this version as a new report.' } });
        if (same) return r.fulfill({ status: 200, json: { id: same.id, state: 'saved' } });
        const id = `fb_${String(reports.length).padStart(16, '0')}`;
        reports.push({ id, client_id: body.client_id, kind: body.kind, text: body.text, sent_at: 1_800_000_000 + reports.length, state: 'saved', preview: body.text.trim().slice(0, 80) });
        return r.fulfill({ status: 201, json: { id, state: 'saved' } });
      });
      if (MOCK) await page.route(`${ORIGIN}/v1/feedback/mine`, async (r) => { if (holdMine) { const h = holdMine; holdMine = null; await h; } return r.fulfill({ json: [...reports].sort((x, y) => y.sent_at - x.sent_at) }); });
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
      // The phone turns before Send: what goes is still what the preview showed
      if (phone) await page.setViewportSize({ width: 800, height: 760 });
      await dialog.getByRole('button', { name: 'Send' }).click();
      const saved = page.getByRole('dialog', { name: 'Report saved' });
      await saved.waitFor();
      const last = posts[posts.length - 1].body;
      ok(JSON.stringify(last.details) === JSON.stringify(shown), `${w} exactly the previewed details are sent`);
      if (phone) await page.setViewportSize({ width, height: 760 });
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
      if (MOCK) {
        // A draft is its writer's (a shared tablet): someone else signing in never sees or sends it
        await help.click(); await page.getByRole('menuitem', { name: 'Report a problem…' }).click();
        let dlg = page.getByRole('dialog', { name: 'Report a problem' });
        await dlg.getByRole('textbox', { name: 'Description' }).fill('ANN-DRAFT private words');
        await page.keyboard.press('Escape');
        await page.evaluate(() => { window.__fbToken = 'ben-token'; });
        await help.click(); await page.getByRole('menuitem', { name: 'Report a problem…' }).click();
        dlg = page.getByRole('dialog', { name: 'Report a problem' });
        ok(await dlg.getByRole('textbox', { name: 'Description' }).inputValue() === '', `${w} another sign-in starts with an empty report`);
        // Signed in as someone else while the dialog is open: nothing is sent
        await dlg.getByRole('textbox', { name: 'Description' }).fill('BEN words');
        await page.evaluate(() => { window.__fbToken = 'cleo-token'; });
        const before = posts.length;
        await dlg.getByRole('button', { name: 'Send' }).click();
        await dlg.getByRole('alert').filter({ hasText: 'The sign-in changed' }).waitFor();
        ok(posts.length === before && await dlg.getByRole('textbox', { name: 'Description' }).inputValue() === '', `${w} a report is never sent under someone else's sign-in`);
        // An edit after a lost answer: told so, and sent again as a new report
        await dlg.getByRole('textbox', { name: 'Description' }).fill('CLEO first');
        next = 409;
        await dlg.getByRole('button', { name: 'Send' }).click();
        await dlg.getByRole('alert').filter({ hasText: 'already saved as you first wrote it' }).waitFor();
        const firstId = posts[posts.length - 1].body.client_id;
        await dlg.getByRole('button', { name: 'Send' }).click();
        await page.getByRole('dialog', { name: 'Report saved' }).waitFor();
        ok(posts[posts.length - 1].body.client_id !== firstId && !posts.some((p) => p.body.text.includes('ANN-DRAFT')), `${w} after "already saved" the edit goes as a new report, and no one else's draft went`);
        // A late answer to a send never breaks the next dialog (Escape while sending, reopen)
        await page.keyboard.press('Escape');
        await help.click(); await page.getByRole('menuitem', { name: 'Report a problem…' }).click();
        dlg = page.getByRole('dialog', { name: 'Report a problem' });
        await dlg.getByRole('textbox', { name: 'Description' }).fill('SLOW report');
        let release; hold = new Promise((res) => { release = res; });
        await dlg.getByRole('button', { name: 'Send' }).click();
        await page.keyboard.press('Escape');
        await help.click(); await page.getByRole('menuitem', { name: 'Report a problem…' }).click();
        dlg = page.getByRole('dialog', { name: 'Report a problem' });
        release();
        await page.waitForTimeout(300);
        await dlg.getByRole('textbox', { name: 'Description' }).fill('SLOW report, more');
        ok((await dlg.locator('.fb-count').textContent()).startsWith('17 /'), `${w} the reopened dialog still works after the late answer`);
        // The first version was saved meanwhile: the edit is told so, then goes as a new report
        await dlg.getByRole('button', { name: 'Send' }).click();
        await dlg.getByRole('alert').filter({ hasText: 'already saved as you first wrote it' }).waitFor();
        await dlg.getByRole('button', { name: 'Send' }).click();
        await page.getByRole('dialog', { name: 'Report saved' }).waitFor();
        ok(reports.some((x) => x.text === 'SLOW report') && reports.some((x) => x.text === 'SLOW report, more'), `${w} both versions are saved, each once`);
        ok(errors.length === 0, `${w} no page errors after a late answer ${errors.join('; ')}`);
        await page.keyboard.press('Escape');
        // A late "My reports" answer for someone who signed out is never shown to who is here now
        let releaseMine; holdMine = new Promise((res) => { releaseMine = res; });
        await help.click(); await page.getByRole('menuitem', { name: 'My reports' }).click();
        const mineDlg = page.getByRole('dialog', { name: 'My reports' });
        await mineDlg.waitFor();
        await page.evaluate(() => { window.__fbToken = 'dan-token'; });
        releaseMine();
        await page.waitForTimeout(300);
        ok(await mineDlg.getByRole('listitem').count() === 0, `${w} a late My reports answer for an earlier sign-in shows nothing`);
        await page.keyboard.press('Escape');
        // reset() on sign-out: open dialogs and the draft go
        await page.keyboard.press('Escape');
        await help.click(); await page.getByRole('menuitem', { name: 'Report a problem…' }).click();
        await page.getByRole('dialog', { name: 'Report a problem' }).getByRole('textbox', { name: 'Description' }).fill('kept until reset');
        await page.evaluate(() => window.PM_FEEDBACK.reset());
        ok(await page.getByRole('dialog').count() === 0, `${w} reset closes the dialog`);
        await help.click(); await page.getByRole('menuitem', { name: 'Report a problem…' }).click();
        ok(await page.getByRole('dialog', { name: 'Report a problem' }).getByRole('textbox', { name: 'Description' }).inputValue() === '', `${w} reset drops the draft`);
        await page.keyboard.press('Escape');
        await page.evaluate(() => { delete window.__fbToken; });
      }
      ok(errors.length === 0, `${w} no page errors ${errors.join('; ')}`);
      await ctx.close();
    }
    console.log(`${passed} passed`);
  } finally {
    await browser.close();
  }
})().catch((e) => { console.error(e); process.exit(1); });

// A cloud drive's folder picker (Your library › Node library › Add a folder).
// Before, each folder row ended in the down arrow of a row that opens below itself, though it opens
// that folder in place (the next level of the list), and the rows were drawn as the browser's own
// buttons (white, black-edged, only as wide as their name). Opening a folder or going Back redrew the
// list and the keyboard focus fell out of it to the page, so Tab started again at the dialog's top.
// A folder the drive couldn't read left only its error and Cancel: no way back up.
// Now the list is drawn as Add to playlist's (.picker); folder rows end in the right arrow (muted)
// that rows going somewhere use; opening a folder puts the focus on its first folder (or on Add
// this folder when it has none); Back puts it on the folder you came out of; a folder that fails
// keeps Back, which has the focus. Focus moved elsewhere (Cancel) while the drive reads stays there.
// A folder the drive couldn't read (the drive's top level too, which has no Back) said only its
// error, with Add this folder still on for a folder that can't be listed or scanned. Now it says
// it with Try again, which asks the drive again (the focus goes to the first folder
// when it works, stays on Try again when it fails again), and Add this folder is off until a read
// works.
// A slow read of a folder left before it answered, landing after another folder was opened, draws
// nothing; Add this folder adds the folder shown.
// Add this folder was on while a folder just opened (or gone Back to) was still being read, so it
// could add a folder the drive then couldn't list. Now it is off during every read, and on again
// once the folders show.
// Real UI in Chromium against fixtures (no node).
//   node tools/ui-drive-folders.js [repo] [screenshot directory]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/drive-folders-shots');
fs.mkdirSync(shots, { recursive: true });

const google = { id: 'google', name: 'Google Drive', configured: true, connected: true, user: 'me@example.com' };
const fixtures = {
  '/library/folders': { body: [['/srv/music', true]] },
  '/library/webdav': { body: [] },
  '/library/hidden': { body: [] },
  '/library/drives': { body: { providers: [google], roots: [] } },
};
// The drive's folders by parent: root › Albums (› Discs, Singles), Live (no folders), Broken (can't
// be read, until a test lets it: then Recovered), Many (40 folders, more than the list shows)
const tree = {
  root: [{ id: 'f1', name: 'Albums' }, { id: 'f2', name: 'Live' }, { id: 'f3', name: 'Broken' }, { id: 'f4', name: 'Many' }],
  f1: [{ id: 'f11', name: 'Discs' }, { id: 'f12', name: 'Singles' }],
  f2: [], f11: [], f12: [], f3: [{ id: 'f31', name: 'Recovered' }], // (once Broken can be read)
  f4: Array.from({ length: 40 }, (_, k) => ({ id: `f4-${k + 1}`, name: `Disc ${k + 1}` })),
};

(async () => {
  const browser = await chromium.launch();
  let fails = 0; let oks = 0;
  const check = async (name, fn) => { try { await fn(); oks++; console.log('ok  ', name); } catch (e) { fails++; console.log('FAIL', name, '-', e.message.split('\n')[0]); } };
  for (const width of [1280, 1024, 768, 412]) {
    const file = (n) => path.join(shots, `${n}-${width}.png`);
    // fail: the folders the drive can't read just now (a Set; change it to let a read work)
    const open = async (slow, inside, fail = new Set(['f3'])) => {
      const o = await fx.create(browser, width, { mobile: width < 600, fixtures }); o.reads = [];
      await o.page.route('**/v1/library/drives/google/folders*', async (r) => {
        const parent = new URL(r.request().url()).searchParams.get('parent'); o.reads.push(parent);
        if (slow && slow.parent === parent) { await slow.wait; if (inside?.[parent]) return r.fulfill({ contentType: 'application/json', body: JSON.stringify(inside[parent]) }); }
        if (fail.has(parent)) return r.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ error: 'The drive refused to list Broken' }) });
        return r.fulfill({ contentType: 'application/json', body: JSON.stringify(tree[parent] || []) });
      });
      await fx.go(o.page, '/settings/library');
      await o.page.locator('[data-dr-add="google"]').click();
      await o.page.locator(fail.has('root') ? 'dialog.dialog[open] #dfbox [data-dfretry]' : 'dialog.dialog[open] [data-dfopen]').first().waitFor();
      return o;
    };
    const focused = (page) => page.evaluate(() => { const a = document.activeElement; return a ? (a.id ? '#' + a.id : a.textContent.trim()) : ''; });
    const at = async (page, where) => { await page.waitForFunction((w) => document.querySelector('#dfpath')?.textContent === w && !document.querySelector('#dfbox .spin'), where); };

    await check(`folder rows end in the right arrow, muted, as rows that go somewhere (${width})`, async () => {
      const { page, context, state } = await open();
      try {
        await page.screenshot({ path: file('after-picker') });
        const marks = await page.$$eval('[data-dfopen] svg', (l) => l.map((s) => ({ cls: s.getAttribute('class'), t: getComputedStyle(s).transform, c: getComputedStyle(s).color })));
        assert.equal(marks.length, 4);
        // (the same mark as Search's and Artists' rows: .list-row .ic.right)
        const ref = await page.evaluate(() => { const r = document.createElement('div'); r.className = 'list-row'; r.innerHTML = '<svg class="ic right"></svg>'; document.body.appendChild(r); const s = getComputedStyle(r.firstChild); const o = { t: s.transform, c: s.color }; r.remove(); return o; });
        for (const m of marks) { assert.equal(m.cls, 'ic right'); assert.equal(m.t, ref.t, 'turned to point right'); assert.equal(m.c, ref.c, 'muted'); }
        assert.notEqual(ref.t, 'none');
        // Rows the width of the list, drawn as Add to playlist's (.picker), not the browser's buttons
        await page.mouse.move(1, 1); // (not hovering one)
        const look = await page.evaluate(() => {
          const box = document.querySelector('#dfbox'); const pick = (el) => { const s = getComputedStyle(el); return { bg: s.backgroundColor, border: s.borderLeftWidth + ' ' + s.borderBottomWidth, font: s.fontFamily + ' ' + s.fontSize, color: s.color, align: s.textAlign, pad: s.padding }; };
          const p = document.createElement('div'); p.className = 'list compact picker'; p.innerHTML = '<button class="list-row">x</button>'; document.querySelector('dialog[open]').appendChild(p);
          const ref = pick(p.firstChild); p.remove();
          return { ref, rows: [...box.querySelectorAll('[data-dfopen]')].map((b) => ({ ...pick(b), wide: Math.abs(b.getBoundingClientRect().width - box.clientWidth) <= 1 })) };
        });
        for (const r of look.rows) { const { wide, ...s } = r; assert.equal(wide, true, 'the row is as wide as the list'); assert.deepEqual(s, look.ref, "drawn as Add to playlist's rows " + JSON.stringify([s, look.ref])); }
        if (width < 600) {
          const h = await page.$$eval('#dfbox [data-dfopen]', (l) => l.map((b) => b.getBoundingClientRect().height));
          for (const x of h) assert.ok(x >= 44, `a phone's folder row is ${x} px, under 44`);
        }
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });

    await check(`keyboard: Enter on a folder opens it with the focus on its first folder; Back returns to the folder you left (${width})`, async () => {
      const { page, context, state } = await open();
      try {
        await page.locator('[data-dfopen="0"]').focus();
        await page.keyboard.press('Enter');
        await at(page, 'Google Drive › Albums');
        await page.screenshot({ path: file('after-opened') });
        assert.equal(await focused(page), 'Discs', 'the first folder inside has the focus');
        await page.keyboard.press('Tab');
        assert.equal(await focused(page), 'Singles', 'Tab goes on to the next folder, not the dialog top');
        await page.keyboard.press('Enter');
        await at(page, 'Google Drive › Albums › Singles');
        assert.equal(await focused(page), '#dfadd', 'no folders inside: Add this folder has the focus');
        await page.keyboard.press('Shift+Tab'); // (back up to ‹ Back)
        assert.equal(await focused(page), '‹ Back');
        await page.keyboard.press('Enter');
        await at(page, 'Google Drive › Albums');
        assert.equal(await focused(page), 'Singles', 'Back: the folder you came out of has the focus');
        await page.locator('[data-dfup]').focus();
        await page.keyboard.press('Enter');
        await at(page, 'Google Drive');
        assert.equal(await focused(page), 'Albums', 'Back to the top: Albums has the focus');
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });

    await check(`a folder that can't be read keeps Back, with the focus; Back returns to it (${width})`, async () => {
      const { page, context, state } = await open();
      try {
        await page.locator('[data-dfopen="2"]').focus();
        await page.keyboard.press('Enter');
        await at(page, 'Google Drive › Broken');
        await page.screenshot({ path: file('after-failed') });
        assert.match(await page.locator('#dfbox').textContent(), /refused to list Broken/);
        assert.equal(await page.locator('#dfbox [data-dfup]').count(), 1, 'Back is there');
        assert.equal(await focused(page), '‹ Back');
        await page.keyboard.press('Enter');
        await at(page, 'Google Drive');
        assert.equal(await focused(page), 'Broken');
        // (Add this folder is on again for a folder that reads after one that didn't)
        await page.locator('[data-dfopen="0"]').click();
        await at(page, 'Google Drive › Albums');
        assert.equal(await page.locator('#dfadd').isDisabled(), false, 'Add this folder is on for Albums');
        assert.deepEqual(state.errors.filter((e) => !/500|refused/.test(e)), [], 'no other page errors');
      } finally { await context.close(); }
    });

    await check(`a folder that can't be read says so with Try again, and Add this folder is off; Try again asks again (${width})`, async () => {
      const fail = new Set(['f3']);
      const o = await open(undefined, undefined, fail); const { page, context, state } = o;
      const reads = () => o.reads.filter((x) => x === 'f3').length;
      try {
        await page.locator('[data-dfopen="2"]').click();
        await at(page, 'Google Drive › Broken');
        const note = page.locator('#dfbox [role="alert"]');
        assert.match(await note.textContent(), /refused to list Broken/);
        assert.equal(await note.locator('[data-dfretry]').textContent(), 'Try again');
        // (the line sits in the rows' inset, not on the list's edge)
        const inset = await page.evaluate(() => { const t = document.createRange(); const p = document.querySelector('#dfbox [role="alert"]'); t.selectNodeContents(p.firstChild); return t.getBoundingClientRect().left - document.querySelector('#dfbox').getBoundingClientRect().left; });
        assert.ok(inset >= 8, `the error starts ${inset} px from the list's edge`);
        assert.equal(await page.locator('#dfadd').isDisabled(), true, "Add this folder is off for a folder the drive can't list");
        assert.equal(await focused(page), '‹ Back', 'just opened: Back has the focus');
        // Try again by keyboard, failing again: the focus stays on Try again, Back stays
        await page.keyboard.press('Tab');
        assert.equal(await focused(page), 'Try again');
        const before = reads();
        await page.keyboard.press('Enter');
        await page.waitForFunction(() => document.querySelector('#dfbox [data-dfretry]') && !document.querySelector('#dfbox .spin'), null);
        await page.waitForFunction(() => document.activeElement?.hasAttribute('data-dfretry'), null, { timeout: 3000 });
        assert.equal(reads(), before + 1, 'the drive was asked again');
        assert.equal(await page.locator('#dfbox [data-dfup]').count(), 1, 'Back is still there');
        assert.equal(await page.locator('#dfadd').isDisabled(), true);
        // The drive answers now, slowly: Add this folder stays off while it reads; then its
        // folders, the focus on the first, Add this folder on
        fail.delete('f3');
        let go; const held = new Promise((res) => { go = res; });
        await page.route('**/v1/library/drives/google/folders?parent=f3', async (r) => { await held; return r.fallback(); });
        await page.keyboard.press('Enter');
        await page.locator('#dfbox .spin').waitFor();
        assert.equal(await page.locator('#dfadd').isDisabled(), true, 'Add this folder stays off while the folder is asked again');
        go();
        await page.locator('#dfbox [data-dfopen]').first().waitFor();
        await page.screenshot({ path: file('after-retried') });
        assert.equal(await page.locator('#dfpath').textContent(), 'Google Drive › Broken', 'still the same folder');
        assert.equal(await page.locator('#dfbox [data-dfretry]').count(), 0, 'no error left');
        assert.equal(await focused(page), 'Recovered', 'its first folder has the focus');
        assert.equal(await page.locator('#dfadd').isDisabled(), false, 'Add this folder is on again');
        assert.deepEqual(state.errors.filter((e) => !/500|refused/.test(e)), [], 'no other page errors');
      } finally { await context.close(); }
    });

    await check(`the drive's top level that can't be read has Try again (no Back to go to); it then shows the folders (${width})`, async () => {
      const fail = new Set(['root']);
      const { page, context, state } = await open(undefined, undefined, fail);
      try {
        await at(page, 'Google Drive');
        await page.screenshot({ path: file('after-top-failed') });
        assert.equal(await page.locator('#dfbox [data-dfup]').count(), 0, 'no Back at the top');
        const again = page.locator('#dfbox [role="alert"] [data-dfretry]');
        assert.equal(await again.count(), 1, 'Try again is there');
        assert.equal(await page.locator('#dfadd').isDisabled(), true);
        fail.delete('root');
        await again.click();
        await page.locator('#dfbox [data-dfopen]').first().waitFor();
        assert.equal(await page.locator('#dfbox [data-dfopen]').count(), 4, "the drive's folders");
        assert.equal(await focused(page), 'Albums', 'the focus went with the list, to its first folder');
        assert.equal(await page.locator('#dfadd').isDisabled(), true, 'the top level is never added (as before)');
        assert.deepEqual(state.errors.filter((e) => !/500|refused/.test(e)), [], 'no other page errors');
      } finally { await context.close(); }
    });

    // (a row fully within the list's scrolled view)
    const inView = (page, sel) => page.evaluate((s) => { const b = document.querySelector(s).getBoundingClientRect(); const l = document.querySelector('#dfbox').getBoundingClientRect(); return b.top >= l.top - 1 && b.bottom <= l.bottom + 1; }, sel);
    await check(`a long folder: Back by click or tap starts the list at the top, Back in view; by keyboard it shows the folder left (${width})`, async () => {
      const { page, context, state } = await open();
      try {
        await page.locator('[data-dfopen="3"]').click();
        await at(page, 'Google Drive › Many');
        assert.ok(await page.$eval('#dfbox', (b) => b.scrollHeight > b.clientHeight + 100), 'the list scrolls');
        await page.locator('[data-dfopen="39"]').scrollIntoViewIfNeeded();
        await page.locator('[data-dfopen="39"]').click(); // (Disc 40: no folders inside)
        await at(page, 'Google Drive › Many › Disc 40');
        await page.locator('[data-dfup]').click();
        await at(page, 'Google Drive › Many');
        assert.equal(await page.$eval('#dfbox', (b) => b.scrollTop), 0, 'the list starts at the top');
        assert.equal(await inView(page, '[data-dfup]'), true, 'Back is in view');
        assert.equal(await focused(page), 'Disc 40', 'the folder left still has the focus');
        // The keyboard: Enter on Disc 40, Back by Enter: Disc 40 is in view, with the focus
        await page.locator('[data-dfopen="39"]').focus();
        await page.keyboard.press('Enter');
        await at(page, 'Google Drive › Many › Disc 40');
        await page.keyboard.press('Shift+Tab');
        assert.equal(await focused(page), '‹ Back');
        await page.keyboard.press('Enter');
        await at(page, 'Google Drive › Many');
        assert.equal(await focused(page), 'Disc 40');
        assert.equal(await inView(page, '[data-dfopen="39"]'), true, 'the focused folder is in view');
        await page.screenshot({ path: file('after-long-back-keyboard') });
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });

    await check(`the focus moved to Cancel while the drive reads stays there; a click still opens (${width})`, async () => {
      let go; const slow = { parent: 'f1', wait: new Promise((res) => { go = res; }) };
      const { page, context, state } = await open(slow);
      try {
        await page.locator('[data-dfopen="0"]').focus();
        await page.keyboard.press('Enter');
        await page.locator('#dfbox .spin').waitFor();
        await page.locator('[data-dfclose]').focus();
        go();
        await at(page, 'Google Drive › Albums');
        assert.equal(await page.evaluate(() => document.activeElement?.hasAttribute('data-dfclose')), true, 'Cancel keeps the focus');
        await page.locator('[data-dfopen="1"]').click();
        await at(page, 'Google Drive › Albums › Singles');
        assert.equal(await page.locator('#dfadd:not([disabled])').count(), 1);
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });

    await check(`Add this folder is off while a folder opened or gone Back to is read, and on once its folders show (${width})`, async () => {
      let go; const slow = { parent: 'f1', wait: new Promise((res) => { go = res; }) };
      const { page, context, state } = await open(slow);
      const adds = [];
      await page.route('**/v1/library/drives', (r) => { if (r.request().method() !== 'POST') return r.fallback(); adds.push(r.request().postDataJSON()); return r.fulfill({ contentType: 'application/json', body: JSON.stringify({ provider: 'google', folder_id: 'f1', name: 'Albums' }) }); });
      try {
        // The first read of Albums is slow: Add this folder is off until its folders show
        await page.locator('[data-dfopen="0"]').click();
        await page.locator('#dfbox .spin').waitFor();
        assert.equal(await page.locator('#dfpath').textContent(), 'Google Drive › Albums');
        await page.screenshot({ path: file('after-slow-first-read') });
        assert.equal(await page.locator('#dfadd').isDisabled(), true, 'Add this folder is off during the first read of a folder just opened');
        await page.locator('#dfadd').click({ force: true });
        go();
        await at(page, 'Google Drive › Albums');
        assert.equal(await page.locator('#dfbox [data-dfopen]').count(), 2, "Albums' folders show");
        assert.equal(await page.locator('#dfadd').isDisabled(), false, 'Add this folder is on once the folders show');
        assert.deepEqual(adds, [], 'nothing was added during the read');
        // Into Singles, then Back to Albums with a slow read: off again until it answers
        await page.locator('[data-dfopen="1"]').click();
        await at(page, 'Google Drive › Albums › Singles');
        let again; const held = new Promise((res) => { again = res; });
        await page.route('**/v1/library/drives/google/folders?parent=f1', async (r) => { await held; return r.fallback(); });
        await page.locator('[data-dfup]').click();
        await page.locator('#dfbox .spin').waitFor();
        assert.equal(await page.locator('#dfadd').isDisabled(), true, 'Add this folder is off while the folder gone Back to is read');
        again();
        await at(page, 'Google Drive › Albums');
        assert.equal(await page.locator('#dfadd').isDisabled(), false);
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });

    await check(`a slow read of a folder left before it answered draws nothing over the folder opened since; Add adds the folder shown (${width})`, async () => {
      // Discs answers late, and then with a folder of its own, so a late draw would show
      let go; const slow = { parent: 'f11', wait: new Promise((res) => { go = res; }) };
      const { page, context, state } = await open(slow, { f11: [{ id: 'f111', name: 'Late disc' }] });
      // (the node's answer to an added folder: the new drive root)
      const adds = [];
      await page.route('**/v1/library/drives', (r) => { if (r.request().method() !== 'POST') return r.fallback(); adds.push(r.request().postDataJSON()); return r.fulfill({ contentType: 'application/json', body: JSON.stringify({ provider: 'google', folder_id: 'f12', name: 'Singles' }) }); });
      try {
        await page.locator('[data-dfopen="0"]').click();
        await at(page, 'Google Drive › Albums');
        // Open Discs (its read hangs), then go Back from it before it answers. The list is only
        // the spinner while the drive reads, so a person can't start a second read meanwhile:
        // this Back is the one drawn with Albums, kept from before the list was swapped (the
        // check that drops a late answer is defensive)
        await page.evaluate(() => { window.__dfBack = document.querySelector('[data-dfup]'); });
        await page.locator('[data-dfopen="0"]').click();
        await page.locator('#dfbox .spin').waitFor();
        assert.equal(await page.locator('#dfpath').textContent(), 'Google Drive › Albums › Discs');
        await page.evaluate(() => window.__dfBack.click());
        await at(page, 'Google Drive › Albums');
        await page.locator('[data-dfopen="1"]').click();
        await at(page, 'Google Drive › Albums › Singles');
        const shown = await page.locator('#dfbox').innerHTML();
        const late = page.waitForResponse((r) => r.url().includes('parent=f11'));
        go(); // (Discs answers now)
        await (await late).finished();
        await page.evaluate(() => new Promise((res) => requestAnimationFrame(() => requestAnimationFrame(res))));
        assert.equal(await page.locator('#dfpath').textContent(), 'Google Drive › Albums › Singles', 'the path is the folder opened');
        assert.equal(await page.locator('#dfbox').innerHTML(), shown, "the list is still Singles' (no Late disc)");
        await page.screenshot({ path: file('after-late-read') });
        // Add this folder sends the folder shown, and closes the picker
        await page.locator('#dfadd').click();
        await page.locator('dialog.dialog[open] #dfadd').waitFor({ state: 'detached' });
        assert.deepEqual(adds, [{ provider: 'google', folder_id: 'f12', name: 'Singles' }], 'one add, of Singles');
        await page.waitForFunction(() => document.querySelector('#toast')?.textContent === 'Singles added; scanning', null, { timeout: 3000 });
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
  }
  await browser.close();
  console.log(`${oks} ok, ${fails} failed`);
  process.exit(fails ? 1 : 0);
})();

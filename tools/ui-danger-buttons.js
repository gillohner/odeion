// A button that deletes, removes or replaces something looks like one. Real candidate assets on the
// fixture origin (tools/ui-fixture.js), a collection's page, both themes:
//   * the page's Delete is in the warning colour at rest (it was only under the pointer), and
//     reads at 4.5:1 or better;
//   * the confirmation's main choice is filled with the warning colour, not the accent colour that
//     Save and Play have, reads at 4.5:1 or better, and still does what it did: Cancel deletes
//     nothing, Delete asks the node once;
//   * a choice that is not destructive (Save in Rename) keeps the accent colour;
//   * every confirmation in app.js and usage.js whose title begins with Delete, Remove, Forget,
//     Clear, End, Disconnect, Sign out, Log out or Make a new token has a destructive main choice
//     (a new one with such a title cannot be left plain; one with another title is not seen here).
//   node tools/ui-danger-buttons.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const ROOT = path.resolve(process.argv[2] || path.join(__dirname, '..'));
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'danger-buttons-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const frames = (page) => page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));

// The strongest contrast between an element with its words and the same without them (its own pixels)
const lum = (r, g, b) => { const f = (c) => { c /= 255; return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b); };
const pixels = (page, png) => page.evaluate(async (b64) => { const img = new Image(); img.src = 'data:image/png;base64,' + b64; await img.decode(); const c = document.createElement('canvas'); c.width = img.width; c.height = img.height; const g = c.getContext('2d'); g.drawImage(img, 0, 0); return Array.from(g.getImageData(0, 0, c.width, c.height).data); }, png.toString('base64'));
async function contrastOf(page, sel) {
  const clip = await page.evaluate((s) => { const r = document.querySelector(s).getBoundingClientRect(); return { x: Math.floor(r.x), y: Math.floor(r.y), width: Math.ceil(r.width) + 1, height: Math.ceil(r.height) + 1 }; }, sel);
  const withText = await pixels(page, await page.screenshot({ clip }));
  await page.evaluate((s) => { const el = document.querySelector(s); el.dataset.was = el.getAttribute('style') || ''; el.style.setProperty('color', 'transparent', 'important'); el.style.setProperty('transition', 'none', 'important'); }, sel);
  await frames(page);
  const without = await pixels(page, await page.screenshot({ clip }));
  await page.evaluate((s) => { const el = document.querySelector(s); el.setAttribute('style', el.dataset.was); delete el.dataset.was; }, sel);
  let best = 1;
  for (let i = 0; i < withText.length; i += 4) {
    if (withText[i] === without[i] && withText[i + 1] === without[i + 1] && withText[i + 2] === without[i + 2]) continue;
    const a = lum(withText[i], withText[i + 1], withText[i + 2]); const b = lum(without[i], without[i + 1], without[i + 2]);
    best = Math.max(best, (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05));
  }
  return best;
}
// A token's colour as the page resolves it, and an element's colours
const token = (page, name) => page.evaluate((n) => { const p = document.createElement('span'); p.style.color = `var(${n})`; document.body.appendChild(p); const c = getComputedStyle(p).color; p.remove(); return c; }, name);
// (red: more red than green and blue together would give a grey or a purple)
const isRed = (page, sel) => page.evaluate((s) => { const c = document.createElement('canvas'); c.width = c.height = 1; const g = c.getContext('2d'); g.fillStyle = getComputedStyle(document.querySelector(s)).color; g.fillRect(0, 0, 1, 1); const [r, gr, b] = g.getImageData(0, 0, 1, 1).data; return r > gr * 1.5 && r > b * 1.5; }, sel);
const colours = (page, sel) => page.evaluate((s) => { const el = document.querySelector(s); if (!el) return null; const cs = getComputedStyle(el); return { color: cs.color, background: cs.backgroundColor, cls: el.className }; }, sel);

(async () => {
  // Every confirmation that asks about deleting, removing, forgetting, ending or replacing
  const app = fs.readFileSync(path.join(ROOT, 'crates/pm-api/static/app.js'), 'utf8');
  const asks = [...app.matchAll(/choiceDialog\(\{ title: (['`])((?:Delete|Remove|Forget|Clear|End|Disconnect|Sign out|Log out|Make a new (?:token|password))[^'`]*)\1/g)];
  const plain = asks.filter((m) => !app.slice(m.index, app.indexOf('})', m.index)).includes('danger: true')).map((m) => m[2]);
  // (the usage panel asks through the page's dialog: its fourth argument is the destructive mark)
  const usage = fs.readFileSync(path.join(ROOT, 'crates/pm-api/static/usage.js'), 'utf8');
  const uasks = [...usage.matchAll(/await ask\('((?:Delete|Remove|Forget|Clear|End|Disconnect|Sign out|Log out)[^']*)', '(?:[^'\\]|\\.)*', '[^']*'(, true)?\)/g)];
  plain.push(...uasks.filter((m) => !m[2]).map((m) => m[1]));
  check(asks.length >= 6 && uasks.length >= 2 && plain.length === 0 && /danger: !!danger/.test(app), `each of the ${asks.length + uasks.length} confirmations whose title begins with Delete, Remove, Clear… has a destructive main choice (without: ${plain.join(' | ') || 'none'})`);

  const browser = await chromium.launch();
  for (const theme of ['dark', 'light']) {
    const { context, page, state } = await fx.create(browser, 1280, {});
    await page.addInitScript((t) => { localStorage.setItem('pm.theme', t); }, theme);
    await fx.go(page, '/playlist/c1'); await page.waitForSelector('#pldel'); await frames(page);
    await page.mouse.move(2, 2); // (not over a button)
    const bad = await token(page, '--bad'); const ink = await token(page, '--bad-ink'); const accent = await token(page, '--accent');
    check((await page.evaluate(() => document.documentElement.dataset.theme)) === theme, `${theme}: the page is in the ${theme} theme`);

    // 1. The page's own Delete, at rest
    const del = await colours(page, '#pldel');
    const delC = await contrastOf(page, '#pldel');
    check(del.color === ink && await isRed(page, '#pldel') && delC >= 4.5, `${theme}: the page's Delete is in the warning colour at rest and reads at ${delC.toFixed(2)} (${del.color})`);
    // (the same button on the raised surface a bar or a card has)
    await page.evaluate(() => { document.querySelector('#pldel').parentElement.style.background = 'var(--surface-2)'; }); await frames(page);
    const delC2 = await contrastOf(page, '#pldel');
    await page.evaluate(() => { document.querySelector('#pldel').parentElement.style.background = ''; });
    check(delC2 >= 4.5, `${theme}: and at ${delC2.toFixed(2)} on a raised surface (4.5 at least)`);
    await page.locator('.actions').first().screenshot({ path: path.join(SHOTS, `page-delete-${theme}.png`) });

    // 2. Its confirmation (from the ⋯ menu, where a narrower page keeps Delete)
    await page.setViewportSize({ width: 900, height: 900 }); await frames(page);
    const openConfirm = async () => { await page.click('#plmore'); await page.locator('.popover.menu .pop-item', { hasText: 'Delete' }).click(); await page.waitForSelector('dialog.dialog[open] .choice'); await frames(page); };
    await openConfirm();
    const ch = await colours(page, 'dialog.dialog[open] .choice');
    const chC = await contrastOf(page, 'dialog.dialog[open] .choice .choice-label');
    check(/\bprimary\b/.test(ch.cls) && /\bdanger\b/.test(ch.cls) && ch.background === bad && ch.background !== accent, `${theme}: the confirmation's Delete is filled with the warning colour, not the accent (${ch.background}; accent ${accent})`);
    check(chC >= 4.5, `${theme}: its word reads at ${chC.toFixed(2)} on that colour (4.5 at least)`);
    await page.locator('dialog.dialog[open]').screenshot({ path: path.join(SHOTS, `confirm-delete-${theme}.png`) });
    const deletes = () => state.hits.filter((h) => h.m === 'DELETE' && h.v === '/playlists/c1').length;
    await page.locator('dialog.dialog[open] [data-ci="-1"]').click(); await frames(page);
    check(deletes() === 0 && !(await page.$('dialog.dialog[open]')), `${theme}: Cancel closes it and deletes nothing (${deletes()} requests)`);
    await openConfirm(); await page.locator('dialog.dialog[open] .choice').click();
    await page.waitForFunction(() => !document.querySelector('dialog.dialog[open]')); await page.waitForTimeout(300);
    check(deletes() === 1, `${theme}: Delete asks the node once (${deletes()})`);

    // 3. A choice that destroys nothing keeps the accent
    await fx.go(page, '/playlist/c1'); await page.waitForSelector('#plmore'); await frames(page);
    await page.click('#plmore'); await page.locator('.popover.menu .pop-item', { hasText: 'Rename' }).click(); await page.waitForSelector('dialog.dialog[open] .choice');
    const save = await colours(page, 'dialog.dialog[open] .choice');
    check(!/\bdanger\b/.test(save.cls) && save.background === accent, `${theme}: Save in Rename keeps the accent colour (${save.background})`);
    check(state.errors.length === 0, `${theme}: no script errors (${state.errors.join(' | ')})`);
    await context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} check(s) failed` : 'all checks passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });

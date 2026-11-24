// Text that is dimmed (an unavailable song, a switched-off source, a tag's count, a menu line that
// cannot be pressed, the note under a dialog's choice, the TV screen over a bright cover, the second
// line of a ticked row) still reads at WCAG AA: 4.5:1, and 3:1 for large text.
// Measured from the page's own pixels, so an `opacity` on the element or on anything around it, a
// see-through background and a picture behind the text all count: the element is photographed
// with its text and again with the text made transparent, and the strongest difference between
// the two is the text's colour on what is really behind it. Over a picture (the TV screen) the
// text's colour is then held against every pixel behind its box, and the weakest spot counts.
// The page's real style sheet on the fixture origin (tools/ui-fixture.js), both themes, on the
// page's own surfaces; each piece of markup is the one app.js draws (the test fails if app.js no
// longer has its class names).
//   node tools/ui-dimmed-text.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const ROOT = path.resolve(process.argv[2] || path.join(__dirname, '..'));
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'dimmed-text-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
const APP = require('./ui-src').appJs(path.join(ROOT, 'crates/pm-web/static')).toString('utf8');
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

// name, the class names app.js must still draw, the markup, [selector, least contrast, what it is]
const tag = (rgb) => `<button class="ptag" style="--tag-bg:rgba(${rgb}, 0.3);--tag-line:rgba(${rgb}, 0.5)"><span class="ptag-l">evening</span><span class="ptag-n">12</span></button>`;
const dash = (art) => `<a class="dash-home" href="/">‹ Odeion</a><div class="dash"><div class="dash-bg" aria-hidden="true"><div class="art" style="background:${art}"></div></div><div class="dash-inner"><div class="art dash-art" style="background:${art}"></div><div class="dash-text"><div class="eyebrow">Now playing on Kitchen</div><h1>Tell Me Again</h1><p class="dash-artist">The Paper Lanterns</p><p class="muted">Everything We Meant to Say</p><div class="dash-times muted"><span>1:02</span><span>3:04</span></div></div></div><div class="dash-brand muted small">Odeion</div></div>`;
const CASES = [
  { name: 'a song that cannot play, in a collection', needs: ["t.unavailable ? 'unavail'", 'list-row clickable srow', 'class="grow"><div class="name"', 'class="ntext"'],
    html: '<div class="list"><div class="list-row clickable srow unavail"><span class="num muted small">1</span><button class="icon-btn" data-splay="0" aria-label="Play Low Tide">▶</button><div class="grow"><div class="name"><a class="lnk" href="#">Low Tide</a></div><div class="sub">Nils Okonkwo</div></div><span class="muted small">2:30</span></div><div class="list-row clickable srow unavail"><span class="num muted small">2</span><div class="grow"><div class="name"><span class="ntext">Salt and Iron</span></div><div class="sub">Nils Okonkwo</div></div></div></div>',
    read: [['.name .lnk', 4.5, 'its title (a link)'], ['.name .ntext', 4.5, 'its title (plain)'], ['.sub', 4.5, 'its artist line']] },
  { name: 'a song that cannot play, ticked while selecting', needs: ["t.unavailable ? 'unavail'", 'class="selbox"'],
    html: '<div class="selecting"><div class="list"><div class="list-row clickable srow unavail"><label class="selbox"><input type="checkbox" checked></label><div class="grow"><div class="name"><a class="lnk" href="#">Low Tide</a></div><div class="sub">Nils Okonkwo</div></div></div></div></div>',
    read: [['.name .lnk', 4.5, 'its title'], ['.sub', 4.5, 'its artist line']] },
  { name: 'a source that is switched off', needs: ['class="src-tabrow ${x.enabled ? \'\' : \'off\'}"', 'class="src-tname"', 'class="src-tsub"'],
    html: '<div class="set-nav src-nav"><div class="src-tabrow off"><button class="set-tab"><span class="src-mark" style="background:hsl(200 45% 34%)">I</span><span class="src-tname"><span class="src-t1">Internet Archive</span><span class="src-tsub">Not signed in</span></span></button></div></div>',
    read: [['.src-t1', 4.5, 'its name on the tab'], ['.src-tsub', 4.5, 'the line under it']] },
  { name: 'a source that is switched off, its panel open', needs: ['class="src-tabrow ${x.enabled ? \'\' : \'off\'}"'],
    html: '<div class="set-nav src-nav"><div class="src-tabrow off"><button class="set-tab on"><span class="src-tname"><span class="src-t1">Internet Archive</span><span class="src-tsub">Not signed in</span></span></button></div></div>',
    read: [['.src-t1', 4.5, 'its name on the open tab'], ['.src-tsub', 4.5, 'the line under it']] },
  ...[['red', '255, 69, 58'], ['yellow', '255, 214, 10'], ['green', '48, 209, 88'], ['blue', '10, 132, 255'], ['purple', '191, 90, 242'], ['grey', '142, 142, 147']].map(([n, rgb]) => ({ name: `a ${n} tag`, needs: ['ptag-n', 'ptag-l'], html: `<div class="chips">${tag(rgb)}</div>`, read: [['.ptag-n', 4.5, 'its count'], ['.ptag-l', 4.5, 'its label']] })),
  { name: 'a menu line that cannot be pressed', needs: ["'popover menu'", 'class="pop-item ${it.on ? \'on\' : \'\'}"'],
    html: '<div class="popover menu" style="position:static;display:block"><button class="pop-item" disabled><span>No other sources found for this song</span></button><button class="pop-item on" disabled id="tickedoff"><span>TIDAL</span></button></div>',
    read: [['.pop-item:not(.on) span', 4.5, 'its words'], ['#tickedoff span', 4.5, 'the words of a ticked one']] },
  { name: 'a dialog’s main choice', needs: ['choice-note', 'btn choice'],
    html: '<div class="choices"><button class="btn choice primary"><span class="choice-label">Play from TIDAL</span><span class="choice-note">Full songs, with your account</span></button></div>',
    read: [['.choice-note', 4.5, 'the note under it']] },
  { name: 'a ticked row while selecting', needs: ['class="selbox"', "classList.add('selecting')"],
    html: '<div class="selecting"><div class="list"><div class="list-row srow"><label class="selbox"><input type="checkbox" checked></label><div class="grow"><div class="name"><span class="ntext">Low Tide</span></div><div class="sub">Nils Okonkwo</div></div><span class="muted small dur">2:30</span></div></div></div>',
    read: [['.sub', 4.5, 'its artist line'], ['.dur', 4.5, 'its length']] },
  { name: 'Match on a song row, on a touch screen', touch: true, needs: ['idmark'],
    html: '<div class="tracks"><div class="track"><div class="main-col"><div class="title"><a class="lnk">Low Tide</a><button class="idmark">Match</button></div></div></div></div>',
    read: [['.idmark', 4.5, 'the word']] },
  ...[['a white', '#ffffff'], ['a yellow', '#ffd60a'], ['a grey', '#808080'], ['a black', '#000000']].map(([n, art]) => ({ name: `the TV screen over ${n} cover`, plain: true, uneven: true, needs: ['class="dash-bg"', "'dash-art'", 'class="dash-home"', 'dash-times', 'dash-brand'], html: dash(art),
    read: [['.dash-home', 4.5, 'the way back'], ['.eyebrow', 4.5, 'the zone line'], ['h1', 3, 'the title'], ['.dash-artist', 3, 'the artist'], ['p.muted', 4.5, 'the album'], ['.dash-times span', 4.5, 'the times'], ['.dash-brand', 4.5, 'the name in the corner']] })),
];

// The strongest contrast between an element with its text and the same without it
const lum = (r, g, b) => { const f = (c) => { c /= 255; return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b); };
const pixels = (page, png) => page.evaluate(async (b64) => { const img = new Image(); img.src = 'data:image/png;base64,' + b64; await img.decode(); const c = document.createElement('canvas'); c.width = img.width; c.height = img.height; const g = c.getContext('2d'); g.drawImage(img, 0, 0); return Array.from(g.getImageData(0, 0, c.width, c.height).data); }, png.toString('base64'));
async function contrastOf(page, sel, uneven) {
  const clip = await page.evaluate((s) => { const el = document.querySelector('#dimhost ' + s); if (!el) return null; const r = el.getBoundingClientRect(); return { x: Math.floor(r.x), y: Math.floor(r.y), width: Math.ceil(r.width) + 1, height: Math.ceil(r.height) + 1 }; }, sel);
  if (!clip || clip.width < 3 || clip.height < 3) return null;
  const withText = await pixels(page, await page.screenshot({ clip }));
  await page.evaluate((s) => { const el = document.querySelector('#dimhost ' + s); el.dataset.was = el.getAttribute('style') || ''; el.style.setProperty('color', 'transparent', 'important'); el.style.setProperty('-webkit-text-fill-color', 'transparent', 'important'); }, sel);
  const without = await pixels(page, await page.screenshot({ clip }));
  await page.evaluate((s) => { const el = document.querySelector('#dimhost ' + s); el.setAttribute('style', el.dataset.was); delete el.dataset.was; }, sel);
  let best = 1; let ink = null;
  for (let i = 0; i < withText.length; i += 4) {
    if (withText[i] === without[i] && withText[i + 1] === without[i + 1] && withText[i + 2] === without[i + 2]) continue;
    const a = lum(withText[i], withText[i + 1], withText[i + 2]); const b = lum(without[i], without[i + 1], without[i + 2]);
    const c = (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
    if (c > best) { best = c; ink = a; }
  }
  if (!uneven || ink == null) return best;
  // (the text's own colour, a glyph's solid middle, against each pixel behind the box)
  let worst = Infinity;
  for (let i = 0; i < without.length; i += 4) { const b = lum(without[i], without[i + 1], without[i + 2]); worst = Math.min(worst, (Math.max(ink, b) + 0.05) / (Math.min(ink, b) + 0.05)); }
  return worst;
}

(async () => {
  for (const c of CASES) for (const n of c.needs) check(APP.includes(n), `app.js still draws ${n} (${c.name})`);
  const browser = await chromium.launch();
  for (const touch of [false, true]) {
    // (a touch screen wide enough to show Match: a tablet)
    const { context, page } = await fx.create(browser, touch ? 800 : 1280, { mobile: touch });
    await fx.go(page, '/tracks'); await page.waitForSelector('#main .track, #main .empty');
    for (const theme of ['dark', 'light']) {
      // (each surface a list or a card can lie on; the TV screen brings its own)
      for (const surface of ['--bg', '--surface', '--surface-2']) {
        for (const c of CASES) {
          if (!!c.touch !== touch) continue;
          if (c.plain && surface !== '--bg') continue;
          await page.evaluate(([th, sf, html, full]) => {
            document.documentElement.dataset.theme = th;
            let host = document.getElementById('dimhost');
            if (!host) { host = document.createElement('div'); host.id = 'dimhost'; document.body.appendChild(host); }
            // (the TV screen fills the window, as the page draws it; the rest lies on a surface)
            host.setAttribute('style', full ? 'position:fixed;inset:0;z-index:2147483647;background:var(--bg);color:var(--fg)' : `position:fixed;left:0;top:0;width:min(100vw,720px);z-index:2147483647;padding:16px;box-sizing:border-box;background:var(${sf});color:var(--fg)`);
            host.innerHTML = html;
          }, [theme, surface, c.html, !!c.uneven]);
          await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
          for (const [sel, least, what] of c.read) {
            const got = await contrastOf(page, sel, c.uneven);
            check(got != null && got >= least, `${theme}, on ${surface.slice(2)}: ${c.name}: ${what} reads at ${got == null ? 'nothing (not drawn)' : got.toFixed(2)} (${least} at least)`);
          }
          if (surface === '--surface' || c.plain) await page.locator('#dimhost').screenshot({ path: path.join(SHOTS, `${theme}-${c.name.replace(/[^a-z]+/gi, '-').toLowerCase()}.png`) });
        }
      }
    }
    await context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} check(s) failed` : 'all checks passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });

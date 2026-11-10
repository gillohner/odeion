// Installable web app checks (docs/guides/install-web-app.md). No dependencies; CI runs it.
//   1. the manifest has what browsers need to install it (MDN "Making PWAs installable"):
//      a name, a 192 px and a 512 px PNG, a maskable icon, start_url/scope/id on the node's
//      root and a standalone display, and no token or other secret in any of its addresses;
//   2. every icon is a real PNG of the size the manifest says; the maskable and the Apple icon
//      are opaque, and the maskable one keeps its mark inside the 80 % safe zone;
//   3. index.html links the manifest, the Apple icon and the install script;
//   4. the router serves every one of those files (lib.rs);
//   5. the worker and the install script keep their promises;
//   6. the icons are the approved Odeion mark (docs/brand/odeion): byte copies of the supplied
//      files, the sidebar mark draws the same three grooves, and nothing regenerates them.
//   node tools/pwa-check.js [static dir]
const fs = require('fs');
const path = require('path');
const zlib = require('zlib');
const DIR = process.argv[2] || path.join(__dirname, '..', 'crates', 'pm-api', 'static');
const LIB = path.join(DIR, '..', 'src', 'lib.rs');
const problems = [];
const fail = (m) => problems.push(m);

// A PNG's size and pixels (8-bit RGB/RGBA, not interlaced, as the supplied icons are).
function png(file) {
  const b = fs.readFileSync(file);
  if (b.readUInt32BE(0) !== 0x89504e47) throw new Error(`${file}: not a PNG`);
  let at = 8; let w = 0; let h = 0; let depth = 0; let type = 0; let interlace = 0; const idat = [];
  while (at < b.length) {
    const len = b.readUInt32BE(at); const kind = b.toString('ascii', at + 4, at + 8); const data = b.subarray(at + 8, at + 8 + len);
    if (kind === 'IHDR') { w = data.readUInt32BE(0); h = data.readUInt32BE(4); depth = data[8]; type = data[9]; interlace = data[12]; }
    if (kind === 'IDAT') idat.push(data);
    at += 12 + len;
  }
  const ch = { 2: 3, 6: 4 }[type];
  if (depth !== 8 || !ch || interlace) return { w, h, pixel: null };
  const raw = zlib.inflateSync(Buffer.concat(idat)); const stride = w * ch; const out = Buffer.alloc(h * stride);
  for (let y = 0; y < h; y++) {
    const f = raw[y * (stride + 1)]; const row = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    for (let x = 0; x < stride; x++) {
      const a = x >= ch ? out[y * stride + x - ch] : 0; const up = y ? out[(y - 1) * stride + x] : 0; const ul = y && x >= ch ? out[(y - 1) * stride + x - ch] : 0;
      const p = a + up - ul; const pa = Math.abs(p - a); const pb = Math.abs(p - up); const pc = Math.abs(p - ul);
      const pred = [0, a, up, (a + up) >> 1, pa <= pb && pa <= pc ? a : pb <= pc ? up : ul][f];
      out[y * stride + x] = (row[x] + pred) & 255;
    }
  }
  return { w, h, pixel: (x, y) => { const i = y * stride + x * ch; return [out[i], out[i + 1], out[i + 2], ch === 4 ? out[i + 3] : 255]; } };
}
const opaque = (img) => { for (let y = 0; y < img.h; y++) for (let x = 0; x < img.w; x++) if (img.pixel(x, y)[3] !== 255) return false; return true; };
// Outside the safe zone (a circle 80 % across) a maskable icon is plain background: the mask may cut it.
const safeZone = (img) => {
  const bg = img.pixel(0, 0).join(); const c = img.w / 2; const r = 0.4 * img.w;
  for (let y = 0; y < img.h; y++) for (let x = 0; x < img.w; x++) if (Math.hypot(x + 0.5 - c, y + 0.5 - c) > r && img.pixel(x, y).join() !== bg) return false;
  return true;
};
const file = (src) => path.join(DIR, src.replace(/^\//, ''));
const secret = /token|key=|[?&]k=|bearer/i;

// 1. the manifest
let m = {};
try { m = JSON.parse(fs.readFileSync(path.join(DIR, 'manifest.webmanifest'), 'utf8')); } catch (e) { fail(`manifest.webmanifest: ${e.message}`); }
if (m.name !== 'Odeion' || m.short_name !== 'Odeion') fail(`manifest: name/short_name should be "Odeion", got ${JSON.stringify([m.name, m.short_name])}`);
for (const [k, want] of [['id', '/'], ['start_url', '/'], ['scope', '/'], ['display', 'standalone']]) if (m[k] !== want) fail(`manifest: ${k} should be ${JSON.stringify(want)}, got ${JSON.stringify(m[k])}`);
if (m.prefer_related_applications) fail('manifest: prefer_related_applications stops browsers offering the install');
if (secret.test(JSON.stringify(m))) fail('manifest: an address carries a token or key');
const icons = Array.isArray(m.icons) ? m.icons : [];
const has = (size, purpose) => icons.some((i) => i.type === 'image/png' && String(i.sizes).split(' ').includes(size) && String(i.purpose || 'any').split(' ').includes(purpose));
for (const [size, purpose] of [['192x192', 'any'], ['512x512', 'any'], ['512x512', 'maskable']]) if (!has(size, purpose)) fail(`manifest: no ${size} PNG icon for purpose "${purpose}"`);

// 2. the icons themselves
for (const i of icons) {
  if (!fs.existsSync(file(i.src))) { fail(`manifest icon ${i.src}: no such file in static/`); continue; }
  if (i.type !== 'image/png') continue;
  const img = png(file(i.src));
  if (`${img.w}x${img.h}` !== i.sizes) fail(`${i.src}: is ${img.w}x${img.h}, the manifest says ${i.sizes}`);
  if (String(i.purpose).includes('maskable')) {
    if (!img.pixel) fail(`${i.src}: write 8-bit RGB(A), non-interlaced`);
    else { if (!opaque(img)) fail(`${i.src}: a maskable icon needs an opaque background`); if (!safeZone(img)) fail(`${i.src}: the mark reaches outside the maskable safe zone`); }
  }
}

// 3. index.html
const html = fs.readFileSync(path.join(DIR, 'index.html'), 'utf8');
const apple = /<link rel="apple-touch-icon" href="([^"]+)"/.exec(html);
// with credentials: behind a sign-in proxy the manifest needs its cookie (MDN, Web app manifest: deploying)
if (!/<link rel="manifest" href="\/manifest\.webmanifest" crossorigin="use-credentials">/.test(html)) fail('index.html: no manifest link with crossorigin="use-credentials"');
if (!apple || !/\.png$/.test(apple[1])) fail('index.html: the apple-touch-icon should be a PNG (iOS ignores SVG)');
else if (!fs.existsSync(file(apple[1]))) fail(`index.html: ${apple[1]} is missing`);
else { const img = png(file(apple[1])); if (img.w !== 180 || img.h !== 180) fail(`${apple[1]}: is ${img.w}x${img.h}, iOS wants 180x180`); if (img.pixel && !opaque(img)) fail(`${apple[1]}: iOS fills transparency with black; make it opaque`); }
if (!/<meta name="apple-mobile-web-app-title" content="Odeion">/.test(html)) fail('index.html: the home-screen title should be "Odeion"');
if (!/<link rel="stylesheet" href="\/pwa\.css">/.test(html)) fail('index.html: no /pwa.css');
if (!/<script src="\/pwa\.js"><\/script>/.test(html)) fail('index.html: no /pwa.js in the head');

// 4. the router serves them all, without a token
const lib = fs.existsSync(LIB) ? fs.readFileSync(LIB, 'utf8') : '';
const served = [...icons.map((i) => i.src), apple && apple[1], '/pwa.js', '/pwa.css', '/sw.js'].filter(Boolean);
for (const p of new Set(served)) if (!new RegExp(`\\.route\\(\\s*"${p.replace(/[.]/g, '\\.')}"`).test(lib)) fail(`lib.rs: no route for ${p}`);

// 5. the service worker and the install script keep their promises: only public page files,
// no cache deletion, no reload or navigation of an open page (it would stop the music)
const sw = fs.existsSync(path.join(DIR, 'sw.js')) ? fs.readFileSync(path.join(DIR, 'sw.js'), 'utf8') : '';
const pwaJs = fs.existsSync(path.join(DIR, 'pwa.js')) ? fs.readFileSync(path.join(DIR, 'pwa.js'), 'utf8') : '';
const code = (s) => s.split('\n').filter((l) => !/^\s*\/\//.test(l)).join('\n');
const list = /const FILES = \[([^\]]*)\]/.exec(sw);
if (!list) fail('sw.js: no FILES list');
else for (const f of list[1].match(/'[^']*'/g) || []) if (/\/v1|\/art|\/rest|\/party|\/mb\/|token|callback/i.test(f)) fail(`sw.js: ${f} is not a public page file`);
// Every file the page waits for (its stylesheets, and scripts without async or defer) is one the
// worker keeps and one a kept page needs (BLOCKING): else an offline launch waits for the node
const waits = [...html.matchAll(/<link rel="stylesheet" href="(\/[^"]+)"|<script src="(\/[^"]+)"(?![^>]*\b(?:async|defer)\b)[^>]*>/g)].map((x) => x[1] || x[2]);
const blocking = /const BLOCKING = \[([^\]]*)\]/.exec(sw);
if (!blocking) fail('sw.js: no BLOCKING list');
else for (const f of waits) {
  if (!list || !list[1].includes(`'${f}'`)) fail(`sw.js: the page waits for ${f}, FILES doesn't keep it`);
  if (!blocking[1].includes(`'${f}'`)) fail(`sw.js: the page waits for ${f}, BLOCKING doesn't name it`);
}
for (const [src, name] of [[sw, 'sw.js'], [pwaJs, 'pwa.js']]) {
  if (/caches\.delete|caches\.keys|\.unregister\(/.test(code(src))) fail(`${name}: deletes caches or workers`);
  // The one reload allowed: the Reconnect button a person presses once the node answers again
  // (pwa.js, opened from the kept copy), never one that runs by itself
  const PRESSED = "querySelector('#pwareconnect').addEventListener('click', () => location.reload())";
  const own = name === 'pwa.js' ? code(src).split(PRESSED).join('') : code(src);
  if (name === 'pwa.js' && code(src).split(PRESSED).length > 2) fail('pwa.js: more than one Reconnect reload');
  if (/\.reload\(|clients\.navigate|\.navigate\(|location\.(href|assign|replace)\s*[=(]/.test(own)) fail(`${name}: reloads or moves an open page`);
  if (/localStorage|sessionStorage|indexedDB/.test(code(src)) && name === 'sw.js') fail('sw.js: reads storage');
}
if (/pm\.token/.test(code(pwaJs))) fail('pwa.js: touches the sign-in token');

// 6. the approved logo
const ROOT = path.join(DIR, '..', '..', '..');
const BRAND = path.join(ROOT, 'docs', 'brand', 'odeion');
// The worker's app pages (sw.js PAGES) are exactly the node's page routes (lib.rs `web` router and
// WEB_VIEWS): a path the node doesn't serve as the app must not get the kept page while it is away.
{
  const pm = sw.match(/^const PAGES = \/(.+)\/;$/m);
  const block = (lib.match(/let mut web = Router::new\(\)([\s\S]*?)for view in WEB_VIEWS/) || [])[1] || '';
  const views = ((lib.match(/const WEB_VIEWS: \[&str; \d+\] = \[([\s\S]*?)\];/) || [])[1] || '').match(/"[^"]+"/g) || [];
  const routes = [...[...block.matchAll(/\.route\("([^"]+)", get\(web_ui\)\)/g)].map((x) => x[1]), ...views.map((v) => '/' + v.slice(1, -1))];
  if (!pm || routes.length < 20) fail(`sw.js PAGES or the router's page routes not found (${routes.length})`);
  else {
    const PAGES = new RegExp(pm[1]);
    const toRe = (r) => new RegExp('^' + r.replace(/\{\*[^}]+\}/g, '.+').replace(/\{[^}]+\}/g, '[^/]+') + '$');
    const routed = (u) => routes.some((r) => toRe(r).test(u));
    const firsts = new Set(routes.map((r) => r.split('/')[1]).filter(Boolean));
    const tries = new Set(['/', '/app', '/app/', '/x', ...routes.map((r) => r.replace(/\{\*[^}]+\}/g, 'a/b').replace(/\{[^}]+\}/g, 'x'))]);
    for (const f of firsts) for (const t of [`/${f}`, `/${f}/x`, `/${f}/x/y`, `/${f}/`]) tries.add(t);
    for (const u of tries) if (PAGES.test(u) !== routed(u)) fail(`sw.js PAGES ${PAGES.test(u) ? 'takes' : 'misses'} ${u}, which the node ${routed(u) ? 'serves' : 'does not serve'} as the app`);
  }
}
const sha = (f) => require('crypto').createHash('sha256').update(fs.readFileSync(f)).digest('hex');
let brand = null;
try { brand = JSON.parse(fs.readFileSync(path.join(BRAND, 'sources.json'), 'utf8')); } catch (e) { fail(`docs/brand/odeion/sources.json: ${e.message}`); }
if (brand) {
  for (const [f, h] of Object.entries(brand.sources)) if (!fs.existsSync(path.join(BRAND, f)) || sha(path.join(BRAND, f)) !== h) fail(`docs/brand/odeion/${f}: missing or not the approved file`);
  for (const [rel, want] of Object.entries(brand.installed)) {
    const f = path.join(ROOT, rel);
    if (!fs.existsSync(f)) { fail(`${rel}: missing`); continue; }
    if (sha(f) !== want.sha256) fail(`${rel}: not the approved ${want.from} (sha256 ${sha(f).slice(0, 16)}…)`);
    if (/\.png$/.test(rel)) {
      const img = png(f); const [w, h] = want.size.split('x').map(Number);
      if (img.w !== w || img.h !== h) fail(`${rel}: is ${img.w}x${img.h}, want ${want.size}`);
      if (img.pixel && !opaque(img)) fail(`${rel}: not opaque`);
    }
  }
  // The approved app icon is the mark on the source square
  const app = fs.readFileSync(path.join(BRAND, 'odeion-app-icon.svg'), 'utf8');
  const mark = fs.readFileSync(path.join(BRAND, 'odeion-mark.svg'), 'utf8');
  const grooves = (svg) => [...svg.matchAll(/<path d="([^"]+)"/g)].map((m) => m[1]);
  if (grooves(mark).length !== 3 || grooves(app).join() !== grooves(mark).join()) fail('docs/brand/odeion: the app icon and the mark disagree');
  // The sidebar mark draws the same three grooves, in the mark's colour
  const sym = /<symbol id="i-odeion" viewBox="0 0 512 512">([\s\S]*?)<\/symbol>/.exec(html);
  if (!sym) fail('index.html: no i-odeion symbol');
  else if (grooves(sym[1]).join() !== grooves(mark).join() || !/stroke-width="25"/.test(sym[1]) || !/stroke-linecap="round"/.test(sym[1])) fail('index.html: i-odeion is not the approved mark');
  if (!/<span class="brand-mark" aria-hidden="true"><svg class="ic" viewBox="0 0 512 512"><use href="#i-odeion"\/><\/svg><\/span>/.test(html)) fail('index.html: the sidebar brand mark does not use i-odeion');
  // Nothing regenerates the supplied icons
  if (fs.existsSync(path.join(ROOT, 'tools', 'pwa-icons.js'))) fail('tools/pwa-icons.js: would overwrite the approved icons with a generated placeholder');
}

if (problems.length) {
  for (const p of problems) console.log(p);
  console.log(`\n${problems.length} install problem${problems.length === 1 ? '' : 's'}`);
  process.exit(1);
}
console.log(`pwa check: ok (${icons.length} icons, manifest, index links, routes)`);

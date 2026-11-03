// Login and setup guides: every sign-in or setup method the node offers has an inline guide where
// it is offered (docs/ux/login-guides.md). No dependencies, no network, no node; CI runs it.
//   node tools/login-guides-check.js [repo root]            the check
//   node tools/login-guides-check.js --self-test [repo root] proves it fails on a new source, a
//                                                           removed render hook, a removed guide
// It reads the runtime registries from the Rust sources: streaming::services() (every source),
// signin.rs's sources and their ways, apps.rs's Service apps, drives.rs's providers and the
// routes that take an account (lib.rs); the controls the web UI offers (markers in app.js and
// index.html); and the guide tables in app.js (LOGIN_GUIDES, LOGIN_SOURCES, LOGIN_GUIDE_NONE).
// It fails when a source has no classification, a method has no guide, a guide has no method or
// no render call, a guide misses a part, or the inventory document doesn't list it.
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const SRC = 'crates/pm-api/src';

function check(read, exists) {
  const problems = [];
  const fail = (m) => problems.push(m);
  const runtime = new Map(); // method key -> where it comes from
  const sourceIdOf = (mod) => (/pub(?:\(crate\))? const SOURCE_ID: &str = "([^"]+)"/.exec(read(`${SRC}/${mod}.rs`)) || [])[1] || mod;

  // ---- 1. Every source: streaming::services() -------------------------------------------------
  const streaming = read(`${SRC}/streaming.rs`);
  const fnServices = /pub fn services\(\)[\s\S]*?\n\}/.exec(streaming);
  if (!fnServices) fail('streaming.rs: fn services() not found');
  const sources = [...(fnServices ? fnServices[0] : '').matchAll(/Box::new\(crate::(\w+)::\w+\)/g)].map((m) => sourceIdOf(m[1]));
  if (sources.length < 10) fail(`streaming.rs: expected the sources, found ${sources.join(', ') || 'none'}`);

  // ---- 2. Sign-in ways, Service apps, drives, account routes -----------------------------------
  const signin = read(`${SRC}/signin.rs`);
  const fnSource = /pub\(crate\) fn source\(id: &str\)[\s\S]*?\n\}/.exec(signin);
  if (!fnSource) fail('signin.rs: fn source() not found');
  const wayKeys = new Set();
  for (const mod of [...(fnSource ? fnSource[0] : '').matchAll(/Box::new\(crate::(\w+)::\w+\)/g)].map((m) => m[1])) {
    const rs = read(`${SRC}/${mod}.rs`);
    const consts = Object.fromEntries([...rs.matchAll(/const (WAY_\w+): &str = "([^"]+)"/g)].map((m) => [m[1], m[2]]));
    const ids = [...rs.matchAll(/Way \{\s*id: (WAY_\w+)\.into\(\)/g)].map((m) => consts[m[1]]);
    if (!ids.length) fail(`${mod}.rs: no Way { id: WAY_… } found`);
    for (const id of ids) { const k = `${sourceIdOf(mod)}:${id}`; wayKeys.add(k); runtime.set(k, `${mod}.rs Way`); }
  }
  const apps = read(`${SRC}/apps.rs`);
  const listFn = /fn list\([\s\S]*?\n\}/.exec(apps);
  for (const m of (listFn ? listFn[0] : '').matchAll(/(?:\bapp|own_app)\(\s*"([a-z0-9-]+)"/g)) runtime.set(`app:${m[1]}`, 'apps.rs Service app');
  if (![...runtime.keys()].some((k) => k.startsWith('app:'))) fail('apps.rs: no Service apps found');
  const provId = /fn id\(self\) -> &'static str \{[\s\S]*?\n {4}\}/.exec(read(`${SRC}/drives.rs`));
  for (const m of (provId ? provId[0] : '').matchAll(/=> "([a-z]+)"/g)) runtime.set(`drive:${m[1]}`, 'drives.rs Provider');
  if (![...runtime.keys()].some((k) => k.startsWith('drive:'))) fail('drives.rs: no providers found');
  const lib = read(`${SRC}/lib.rs`);
  const ACCOUNT_ROUTES = {
    '/sources/plex/server': ['plex:server'], '/sources/plex/pin': ['plex:pin'], '/sources/plex/pin/{id}': ['plex:pin'],
    '/sources/plex/stream/{id}': [], '/sources/plex/cover/{id}/{ver}': [],
    '/sources/audiobookshelf/server': ['audiobookshelf:server'], '/sources/audiobookshelf/play': [],
    '/sources/audiobookshelf/continue': [], '/sources/audiobookshelf/stream/{item}/{ino}': [], '/sources/audiobookshelf/cover/{item}': [],
    '/podcasts/gpodder': ['podcasts:gpodder'], '/podcasts/gpodder/sync': [],
  };
  for (const m of lib.matchAll(/"(\/sources\/(?:plex|audiobookshelf)\/[^"\n]+|\/podcasts\/gpodder(?:\/[^"\n]+)?)"/g)) {
    if (!Object.hasOwn(ACCOUNT_ROUTES, m[1])) { fail('unclassified account route: ' + m[1]); continue; }
    for (const key of ACCOUNT_ROUTES[m[1]]) runtime.set(key, 'lib.rs account route');
  }
  // Every credential/config field is explicitly classified; a new auth field must not silently
  // inherit the old guide. The schemas themselves remain authoritative runtime Rust code.
  // Named guide identifiers, not credential values.
  const ABS_LOGIN_GUIDE = 'audiobookshelf:password';
  const PODCAST_ACCOUNT_GUIDE = 'podcasts:gpodder';
  const ACCOUNT_FIELDS = [
    ['plex', 'PlexConnect', {url: null, token: 'plex:server'}],
    ['audiobookshelf', 'AbsConnect', {url: null, api_key: 'audiobookshelf:server', user: ABS_LOGIN_GUIDE, password: ABS_LOGIN_GUIDE}],
    ['gpodder', 'SetAccount', {kind: null, server: null, username: PODCAST_ACCOUNT_GUIDE, password: PODCAST_ACCOUNT_GUIDE, device: null}],
  ];
  for (const [mod, type, fields] of ACCOUNT_FIELDS) {
    const rs = read(SRC + '/' + mod + '.rs');
    const body = new RegExp('pub struct ' + type + '\\s*\\{([\\s\\S]*?)\\n\\}').exec(rs);
    if (!body) { fail(mod + '.rs: missing account schema ' + type); continue; }
    const actual = [...body[1].matchAll(/pub\s+(\w+)\s*:/g)].map(m => m[1]);
    for (const field of actual) {
      if (!Object.hasOwn(fields, field)) { fail(mod + '.rs: unclassified account field ' + field); continue; }
      if (fields[field]) runtime.set(fields[field], mod + '.rs account schema');
    }
    for (const field of Object.keys(fields)) if (!actual.includes(field)) fail(mod + '.rs: account schema field removed: ' + field);
  }
  const kind = /pub enum Kind\s*\{([\s\S]*?)\n\}/.exec(read(SRC + '/gpodder.rs'));
  if (!kind) fail('gpodder.rs: no server Kind registry');
  for (const m of (kind ? kind[1] : '').matchAll(/^\s*([A-Z]\w*)\s*,/gm)) runtime.set('podcasts:' + m[1].toLowerCase(), 'gpodder.rs Kind');

  // ---- 3. Controls the web UI offers ----------------------------------------------------------
  const appJs = read('crates/pm-api/static/app.js');
  const indexHtml = read('crates/pm-api/static/index.html');
  const UI_MARKERS = [
    ['node:ring', /const loginStart = /, appJs], ['node:token', /id="logintoken"/, indexHtml],
    ['subsonic:server', /id="subform"/, appJs], ['jellyfin:server', /id="jfform"/, appJs], ['jamendo:signin', /id="jamin"/, appJs],
    ['listenbrainz:token', /id="lbform"/, appJs], ['webdav:share', /const webdavDialog = /, appJs],
    ['soundcloud:client', /id="scclient"/, appJs], ['soundcloud:token', /id="sctoken"/, appJs], ['bandcamp:identity', /id="bcin"/, appJs],
    ['spotify:soloist', /id="slkey"/, appJs], ['youtube:ytdlp', /id="yzget"/, appJs], ['acoustid:key', /id="acidform"/, appJs],
  ];
  for (const [key, re, src] of UI_MARKERS) if (re.test(src)) runtime.set(key, 'web UI control');

  // ---- 4. The tables ---------------------------------------------------------------------------
  const block = /\/\/ LOGIN-GUIDES-BEGIN\n([\s\S]*?)\/\/ LOGIN-GUIDES-END/.exec(appJs);
  let GUIDES = {}; let NONE = {}; let SOURCES = {};
  if (!block) fail('app.js: no LOGIN-GUIDES-BEGIN … LOGIN-GUIDES-END table');
  else {
    const box = {};
    vm.runInNewContext(`${block[1]}\nbox.g = LOGIN_GUIDES; box.n = LOGIN_GUIDE_NONE; box.s = typeof LOGIN_SOURCES === 'undefined' ? {} : LOGIN_SOURCES;`, { box });
    GUIDES = box.g || {}; NONE = box.n || {}; SOURCES = box.s || {};
  }
  const doc = exists('docs/ux/login-guides.md') ? read('docs/ux/login-guides.md') : '';
  if (!doc) fail('docs/ux/login-guides.md: missing (the inventory)');

  // Every source is classified: the guides its page shows (its ways, its own controls, or
  // source:open for "no sign-in needed"), and nothing else is classified
  for (const id of sources) {
    const keys = SOURCES[id];
    if (!Array.isArray(keys) || !keys.length) { fail(`source ${id} (streaming.rs): no LOGIN_SOURCES entry (its page's guides, or source:open)`); continue; }
    for (const k of keys) if (!GUIDES[k]) fail(`source ${id}: lists ${k}, which has no guide`);
    if (!doc.includes(`\`source:${id}\``)) fail(`source ${id}: not in docs/ux/login-guides.md`);
  }
  for (const id of Object.keys(SOURCES)) if (!sources.includes(id)) fail(`LOGIN_SOURCES.${id}: not a source in streaming.rs`);
  // A sign-in way of a source sits on that source's page
  for (const k of runtime.keys()) { const sid = k.split(':')[0]; if (sources.includes(sid) && !(SOURCES[sid] || []).includes(k)) fail(`${k}: a method of ${sid} missing from LOGIN_SOURCES.${sid}`); }

  const official = /^https:\/\/([a-z0-9.-]+\.)?(spotify\.com|github\.com|tidal\.com|last\.fm|google\.com|microsoft\.com|jamendo\.com|listenbrainz\.org|acoustid\.org|pubky\.org|qobuz\.com|jellyfin\.org|navidrome\.org|subsonic\.org|soundcloud\.com|nextcloud\.com|bandcamp\.com|plex\.tv|audiobookshelf\.org|gpodder\.net|archive\.org|audius\.co|deezer\.com|radio-browser\.info)(\/|$)/;
  const VAGUE = [/tab says how/i, /\bsee (the )?docs\b/i, /\bextract/i, /web player'?s? (bundle|code)/i, /scrap(e|ing)/i, /go-librespot|spotifyd/i];
  const text = (v) => (typeof v === 'function' ? v({}) : v);
  for (const [key, from] of runtime) {
    if (NONE[key]) { if (!doc.includes(`\`${key}\``)) fail(`${key}: in LOGIN_GUIDE_NONE but not in docs/ux/login-guides.md`); continue; }
    if (!GUIDES[key]) fail(`${key} (${from}): offered but has no guide`);
  }
  const extra = new Set(['source:open']);
  for (const [key, g] of Object.entries(GUIDES)) {
    if (!runtime.has(key) && !extra.has(key)) fail(`${key}: a guide for a method the node does not offer`);
    if (!doc.includes(`\`${key}\``)) fail(`${key}: not in docs/ux/login-guides.md`);
    if (!g.title || !/^(recommended|advanced|only)$/.test(g.level)) fail(`${key}: needs a title and level (recommended, advanced or only)`);
    if (!['browser', 'server', 'both'].includes(g.where)) fail(`${key}: where must say browser, server or both`);
    if (!g.needs || !g.needs.length) fail(`${key}: no prerequisites`);
    if (!g.steps || g.steps.length < 2) fail(`${key}: fewer than two numbered steps`);
    if (!g.done) fail(`${key}: no success state`);
    if (!g.fix || !g.fix.length) fail(`${key}: no recovery (failure, renewal, sign out and in again)`);
    if (!g.links || !g.links.length) fail(`${key}: no official link`);
    for (const [, url] of g.links || []) if (!official.test(url)) fail(`${key}: link is not an official primary source: ${url}`);
    const all = JSON.stringify(g, (k, v) => (typeof v === 'function' ? text(v) : v));
    for (const re of VAGUE) if (re.test(all)) fail(`${key}: vague, unsupported or forbidden wording ${re}`);
    // Rendered where it is offered: ways through the chooser, apps and drives through their rows,
    // everything else by its own call beside its controls
    const generic = wayKeys.has(key) ? 'loginGuide(`${cfg.source}:${pick}`' : key.startsWith('app:') ? 'loginGuide(`app:${a.id}`' : key.startsWith('drive:') ? 'loginGuide(`drive:${p.id}`' : `loginGuide('${key}'`;
    if (!appJs.includes(generic)) fail(`${key}: not rendered (no ${generic}… in app.js)`);
  }
  for (const [key, from] of runtime) if (from.endsWith('Way') && GUIDES[key] && /:(import|paste|cookies|librespot-file)$/.test(key) && GUIDES[key].level !== 'advanced') fail(`${key}: an import or paste way must be an advanced guide`);
  return { problems, runtime, GUIDES, NONE, SOURCES, sources };
}

const diskReader = (root) => [(p) => fs.readFileSync(path.join(root, p), 'utf8'), (p) => fs.existsSync(path.join(root, p))];

if (process.argv[2] === '--self-test') {
  const root = process.argv[3] || path.join(__dirname, '..');
  const [read, exists] = diskReader(root);
  const with_ = (over) => [(p) => (p in over ? over[p] : read(p)), (p) => p in over || exists(p)];
  const base = check(read, exists);
  const cases = [
    ['a new source in streaming.rs', { [`${SRC}/streaming.rs`]: read(`${SRC}/streaming.rs`).replace('Box::new(crate::youtube::YouTube),', 'Box::new(crate::youtube::YouTube),\n        Box::new(crate::newsvc::NewSvc),'), [`${SRC}/newsvc.rs`]: 'pub const SOURCE_ID: &str = "newsvc";' }, /source newsvc \(streaming\.rs\): no LOGIN_SOURCES entry/],
    ['the Subsonic guide no longer rendered', { 'crates/pm-api/static/app.js': read('crates/pm-api/static/app.js').split("loginGuide('subsonic:server'").join("loginGuideX('subsonic:server'") }, /subsonic:server: not rendered/],
    ['a sign-in way without a guide', { [`${SRC}/youtube.rs`]: read(`${SRC}/youtube.rs`).replace(/const WAY_COOKIES: &str = "cookies"/, 'const WAY_COOKIES: &str = "cookies2"') }, /youtube:cookies2 \(youtube\.rs Way\): offered but has no guide/],
    ['a new Plex authentication route', { [SRC + '/lib.rs']: read(SRC + '/lib.rs').replace('.route("/sources/plex/pin", post(plex::start_pin))', '.route("/sources/plex/new-auth", post(plex::start_pin))\n        .route("/sources/plex/pin", post(plex::start_pin))') }, /unclassified account route: \/sources\/plex\/new-auth/],
    ['a new Audiobookshelf credential field', { [SRC + '/audiobookshelf.rs']: read(SRC + '/audiobookshelf.rs').replace('pub struct AbsConnect {', 'pub struct AbsConnect {\n    pub passkey: String,') }, /audiobookshelf\.rs: unclassified account field passkey/],
    ['a new podcast sync server kind', { [SRC + '/gpodder.rs']: read(SRC + '/gpodder.rs').replace('pub enum Kind {', 'pub enum Kind {\n    Newkind,') }, /podcasts:newkind .*offered but has no guide/],
    ['a new Service app', { [`${SRC}/apps.rs`]: read(`${SRC}/apps.rs`).replace('..app("lastfm", "Last.fm"', '..app("newapp", "New", false, None) }, NodeApp { ..app("lastfm", "Last.fm"') }, /app:newapp \(apps\.rs Service app\): offered but has no guide/],
  ];
  let bad = base.problems.length ? 1 : 0;
  if (bad) console.log('FAILED the unmodified tree has problems:\n  ' + base.problems.join('\n  '));
  for (const [what, over, expect] of cases) {
    const r = check(...with_(over));
    const ok = r.problems.some((p) => expect.test(p));
    console.log(`${ok ? 'ok    ' : 'FAILED'} ${what} → ${ok ? r.problems.find((p) => expect.test(p)) : 'not caught: ' + JSON.stringify(r.problems.slice(0, 3))}`);
    if (!ok) bad += 1;
  }
  process.exit(bad ? 1 : 0);
}

const root = process.argv[2] || path.join(__dirname, '..');
const r = check(...diskReader(root));
if (r.problems.length) {
  console.error(`login-guides-check: ${r.problems.length} problem(s)`);
  for (const p of r.problems) console.error('  ' + p);
  process.exit(1);
}
console.log(`login-guides-check: ${r.sources.length} sources classified, ${r.runtime.size} offered methods, ${Object.keys(r.GUIDES).length} guides, ${Object.keys(r.NONE).length} with nothing to set up; all covered`);

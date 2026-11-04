// Tags in the top card, as pubky.app draws them (TASK-tags-in-hero-pubky-design):
//   * album, song and artist pages show their tags in the hero, above Play; no Tags section below;
//   * the item's own tags and the source link's stay two labelled groups, each adding and removing
//     on its own URI, each with its own public counts and taggers (the same label in both keeps
//     different people);
//   * chips have pubky.app's colour for the label (tagColour, tools/tag-colour-check.js), your own
//     has the border; up to 4 tagger faces then "+N", which list who tagged it;
//   * many tags fold to about two rows and "+N more"; loading, empty and error states; keyboard;
//   * an album with no MusicBrainz match says so in the hero and leads to the matches to confirm;
//   * the tag page (one tag and two: only this one, ×), search's Tags, Genres' "Your Pubky tags" and
//     Home's tags use the same chip and colours (the now-playing tags share tagChip, unchecked here).
// The node's /v1/item, /v1/tags, /v1/discover/uri and /v1/profiles are mocked here.
//   node tools/ui-tags-hero.js <api token> <node URL> [screenshot dir]
//   PM_STATIC_DIR=crates/pm-api/static serves the page from those files instead of the node's
const assert = require('node:assert/strict');
const fs = require('fs');
const path = require('path');
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const TOKEN = process.argv[2] || 'ci-token';
const BASE = process.argv[3] || 'http://127.0.0.1:7788';
const OUT = process.argv[4] || null;
const STATIC = process.env.PM_STATIC_DIR || null;
const CANON = 'https://musicbrainz.org/recording/0c9b1d6e-4e5f-4f3a-9c11-5b7d0a6e2f41';
const RELEASE = 'https://musicbrainz.org/release/7c3a2c8e-2b8f-4c62-9f0e-6f1a4c3d9e21';
const GUESS = 'https://musicbrainz.org/release-group/11111111-2222-4333-8444-555555555555';
const SONG = 'https://musicbrainz.org/recording/b1a9c0e9-d987-4042-ae91-78d6a3267d69';
const ARTIST = 'https://musicbrainz.org/artist/a74b1b7f-71a5-4011-9441-d0b5e4122711';
const LINK = 'https://tidal.com/track/123456';
const ALINK = 'https://tidal.com/album/654321';
// Pubky ids are 52 characters of z-base-32
const P = Array.from({ length: 8 }, (_, i) => `${'p'.repeat(4)}${String.fromCharCode(97 + i)}`.padEnd(52, 'y'));
const NAMES = ['Ann', 'Bo', 'Cy', 'Di', 'Ed', 'Flo', 'Gus', 'Hal'];
const appSrc = fs.readFileSync(path.join(__dirname, '..', 'crates', 'pm-api', 'static', 'app.js'), 'utf8');
const block = /\n  const TAG_COLOURS = [\s\S]*?\n  const tagRgba = [^\n]*\n/.exec(appSrc);
// Without them (before this change) the page checks still run, and fail on the page itself
const { tagColour, tagRgba } = block ? new Function(`${block[0]}\nreturn { tagColour, tagRgba };`)() : { tagColour: () => null, tagRgba: () => 'no tagColour in app.js' };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const PNG = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAQAAAAECAIAAAAmkwkpAAAAEElEQVR4nGO4VmEDRwzEcQCKMhih3mIObwAAAABJRU5ErkJggg==', 'base64'); // 4×4, orange
const MIME = { '.js': 'text/javascript', '.css': 'text/css', '.html': 'text/html', '.json': 'application/json', '.svg': 'image/svg+xml', '.png': 'image/png', '.webmanifest': 'application/manifest+json' };

const item = (kind, id, o) => ({  kind, id, title: o.title || 'Frenchcore Worldwide 05', section: { album: 'albums', song: 'tracks', artist: 'artists' }[kind], source: o.source || 'Your library', source_id: o.source_id || 'library', target: o.target, uri: o.uri || null, song_uri: o.song_uri || null, link: o.link || null, songs: o.songs || [], albums: [], versions: [], also_on: [], about: [], open_on: [], connect: [], artist: kind === 'artist' ? null : { id: 'lib-2', name: 'Various' }, year: 2024, on_pubky: o.on_pubky || null });

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 820, 390]) {
      const ctx = await browser.newContext({ viewport: { width, height: 900 }, reducedMotion: 'reduce', colorScheme: 'dark' });
      await ctx.addInitScript((t) => { localStorage.setItem('pm.token', t); localStorage.setItem('pm.zone', 'browser'); HTMLMediaElement.prototype.play = async function () {}; }, TOKEN);
      const page = await ctx.newPage();
      const errors = []; page.on('pageerror', (e) => errors.push(e.message));
      const shot = async (name) => { if (OUT) { fs.mkdirSync(OUT, { recursive: true }); await page.screenshot({ path: path.join(OUT, `${width}-${name}.png`) }); } };
      if (STATIC) {
        await page.route((u) => u.origin === new URL(BASE).origin && !u.pathname.startsWith('/v1/'), async (r) => {
          const req = r.request(); const p = new URL(req.url()).pathname.slice(1);
          const file = path.join(STATIC, p);
          if (p && !p.includes('..') && fs.existsSync(file) && fs.statSync(file).isFile()) return r.fulfill({ body: fs.readFileSync(file), contentType: MIME[path.extname(file)] || 'application/octet-stream' });
          if (req.resourceType() === 'document') return r.fulfill({ body: fs.readFileSync(path.join(STATIC, 'index.html')), contentType: 'text/html' });
          return r.fallback();
        });
      }
      // ---- the mocked node ----
      let pages = {}; let own = {}; let pub = {}; let uris = {}; let failPut = null; let pubHold = null; let pubFail = false; let me = null; let ownFail = null; let putHold = null;
      const writes = []; const asked = [];
      await page.route(/\/v1\/item\/(album|song|artist)\/[^/?]+(\?.*)?$/, (r) => { const m = /\/v1\/item\/(\w+)\/([^/?]+)/.exec(r.request().url()); const pg = pages[`${m[1]}/${m[2]}`]; return pg ? r.fulfill({ json: pg }) : r.fallback(); });
      await page.route(/\/v1\/item\/[^?]+\/extras(\?.*)?$/, (r) => r.fulfill({ json: { related: [], concerts: [], similar: [] } }));
      await page.route(/\/v1\/comments(\?.*)?$/, (r) => r.fulfill({ json: { comments: [], indexed: true } }));
      await page.route(/\/v1\/reviews(\?.*)?$/, (r) => (r.request().method() === 'GET' ? r.fulfill({ json: { reviews: [] } }) : r.fallback()));
      const keyOf = (q) => q.uri || (q.track_id ? `track:${q.track_id}` : q.album_id ? `album:${q.album_id}` : q.artist_id ? `artist:${q.artist_id}` : '');
      await page.route(/\/v1\/tags(\?.*)?$/, async (r) => {
        const req = r.request(); const m = req.method(); const q = Object.fromEntries(new URL(req.url()).searchParams);
        if (m === 'GET' && !keyOf(q)) return r.fulfill({ json: { tags: Object.entries(own).flatMap(([u, ts]) => (Array.isArray(ts) ? ts.map((t) => ({ uri: u, ...t })) : [])) } });
        const b = m === 'PUT' ? req.postDataJSON() : q; const k = keyOf(b);
        if (m === 'GET' && k === ownFail) return r.fulfill({ status: 500, json: { error: 'database is locked' } });
        if (own[k] === 'needs_match') return m === 'GET' ? r.fulfill({ json: { needs_match: true, tags: [] } }) : r.fulfill({ status: 422, json: { error: 'no canonical identity' } });
        if (m === 'PUT' && putHold) await putHold;
        if (m === 'PUT' && k === failPut) return r.fulfill({ status: 502, json: { error: 'homeserver: 502' } });
        if (m === 'PUT') { writes.push({ m, k, label: b.label }); own[k] = [...(own[k] || []).filter((t) => t.label !== b.label), { label: b.label, synced: false }]; }
        if (m === 'DELETE') { writes.push({ m, k, label: b.label }); own[k] = (own[k] || []).filter((t) => t.label !== b.label); }
        return r.fulfill({ json: { uri: k.includes('://') ? k : uris[k] || null, tags: own[k] || [] } });
      });
      await page.route(/\/v1\/discover\/uri(\?.*)?$/, async (r) => {
        const q = Object.fromEntries(new URL(r.request().url()).searchParams); const k = keyOf(q); asked.push(k);
        if (pubHold) await pubHold;
        if (pubFail) return r.fulfill({ status: 502, json: { error: 'nexus: 502 Bad Gateway' } });
        const v = pub[k]; if (v === 422) return r.fulfill({ status: 422, json: { error: 'no canonical identity' } });
        return r.fulfill({ json: { uri: k, indexed: !!v, tags: (v && v.tags) || [], taggers: (v && v.taggers) || 0, ...(v && v.faces ? { faces: v.faces } : {}) } });
      });
      await page.route(/\/v1\/profiles\?ids=/, (r) => { const ids = new URL(r.request().url()).searchParams.get('ids').split(','); return r.fulfill({ json: ids.map((id) => { const n = P.indexOf(id); return { id, name: n >= 0 ? NAMES[n] : id.slice(0, 4), avatar: n >= 0 && n % 2 === 0 ? `/v1/profiles/${id}/avatar` : null, you: n === 0, following: n === 1, known: n >= 0 }; }) }); });
      await page.route(/\/v1\/profiles\/[^/]+\/avatar/, (r) => r.fulfill({ body: PNG, contentType: 'image/png' }));
      await page.route(/\/v1\/me(\?.*)?$/, (r) => (me ? r.fulfill({ json: me }) : r.fallback()));
      const folder = (key, id) => ({ key, album: 'Frenchcore Worldwide 05', artist: 'Various', songs: [{ track: { id, title: 'Track one', artists: [{ name: 'Various' }], track_no: 1, duration_ms: 200000, file: { path: `/music/${key}/01.flac` } }, options: [{ id: id * 10, recording_mbid: '0c9b1d6e-4e5f-4f3a-9c11-5b7d0a6e2f41', title: 'Track one', artists: [{ name: 'Various' }], source: 'album', score: 0.9 }], status: 'single' }], ready: 1, conflicts: 0 });
      let folders = null; // null: lib-137's folder is on the first page; else these, with more to come
      await page.route(/\/v1\/library\/suggestions\/albums(\?.*)?$/, (r) => r.fulfill({ json: folders ? { albums: folders, songs: folders.length, total_albums: 60, offset: 0, next_after: 'b' } : { albums: [folder('a', 300), folder('b', 400), folder('c', 501)], songs: 3, total_albums: 3, offset: 0 } }));
      const go = async (p) => { await page.goto(BASE + p); await page.locator('#main .hero').first().waitFor({ timeout: 20000 }); await sleep(400); };
      const hero = () => page.locator('#main .hero');
      const group = (scope) => page.locator(`#htags [data-tag-scope="${scope}"]`);
      const chip = (scope, label) => group(scope).locator(`.tagitem[data-label="${label}"]`);
      // Shows every chip of the page's groups (a narrow hero folds them behind "+N more")
      const expandAll = async () => { const more = page.locator('#htags [data-htag-more]:visible'); for (let i = 0; i < 4 && await more.count(); i++) await more.first().click(); };
      const labels = async (scope) => group(scope).locator('.tagitem:visible').evaluateAll((els) => els.map((e) => e.dataset.label));

      // ---- A: a TIDAL song with a MusicBrainz recording: both groups in the hero ----
      pages['song/tidal-a'] = item('song', 'tidal-a', { source: 'TIDAL', source_id: 'tidal', target: { uri: LINK }, uri: CANON, song_uri: CANON, link: LINK, on_pubky: { uri: `${CANON}-older`, indexed: true, tags: [['wrong-scope', 90]], taggers: 90 } });
      own = { [CANON]: [{ label: 'shared', synced: true }], [LINK]: [] };
      pub = {
        [CANON]: { tags: [['frenchcore', 7], ['hardcore', 3], ['shared', 2]], taggers: 8, faces: { frenchcore: P.slice(1, 7), hardcore: P.slice(5, 8), shared: [P[0], P[1]] } },
        [LINK]: { tags: [['link-only', 1], ['shared', 1]], taggers: 1, faces: { 'link-only': [P[4]], shared: [P[4]] } },
      };
      await go('/song/tidal-a');
      await group('song').locator('.tagitem').first().waitFor();
      assert.equal(await page.locator('#itags').count(), 0, 'no Tags section at the bottom');
      assert.equal(await hero().locator('#htags').count(), 1, 'the tags are in the hero');
      assert.ok(await page.evaluate(() => { const t = document.querySelector('#htags'), a = document.querySelector('.hero .actions'); return !!(t && a && (t.compareDocumentPosition(a) & Node.DOCUMENT_POSITION_FOLLOWING)); }), 'tags come before the Play actions');
      assert.equal(await page.locator('#tagscope').count(), 0, 'no scope switch');
      assert.match(await group('song').locator('.tgroup-name').textContent(), /On this song/);
      assert.match(await group('link').locator('.tgroup-name').textContent(), /TIDAL/);
      await shot('album-folded');
      await expandAll();
      assert.deepEqual(await labels('song'), ['frenchcore', 'hardcore', 'shared'], 'most used first; never the page-wide on_pubky');
      assert.deepEqual(await labels('link'), ['link-only', 'shared']);
      // The chip: label without "#", count, the label's colour at 0.3, your own with its border
      const fc = chip('song', 'frenchcore').locator('.ptag');
      assert.equal(await fc.locator('.ptag-l').textContent(), 'frenchcore');
      assert.equal(await fc.locator('.ptag-n').textContent(), '7');
      assert.equal(await fc.evaluate((e) => getComputedStyle(e).backgroundColor), tagRgba(tagColour('frenchcore'), 0.3));
      const mine = chip('song', 'shared').locator('.ptag');
      assert.ok(await mine.evaluate((e) => e.classList.contains('own')), 'your tag is marked');
      assert.equal(await mine.evaluate((e) => getComputedStyle(e).borderTopColor), tagRgba(tagColour('shared'), 0.5));
      assert.equal(await fc.evaluate((e) => getComputedStyle(e).borderTopColor), 'rgba(0, 0, 0, 0)', 'others\' tags have no border');
      assert.equal(await chip('song', 'frenchcore').locator('[data-htag-del]').count(), 0, 'no remove on others\' tags');
      assert.equal(await chip('song', 'shared').locator('[data-htag-del]').count(), 1, 'remove on your own');
      // Taggers: up to 4 faces, then +N (7 people, 4 shown)
      const ff = chip('song', 'frenchcore').locator('.tagfaces');
      await ff.locator('.face').first().waitFor();
      assert.equal(await ff.locator('.face').count(), 4);
      assert.equal((await ff.locator('.face-more').textContent()).trim(), '+3');
      // Per scope and label: "shared" on the album is Ann and Bo, on the TIDAL link only Ed
      await chip('song', 'shared').locator('.tagfaces').click();
      let menu = (await page.locator('.popover.menu').innerText()).replace(/\s+/g, ' ');
      assert.match(menu, /Ann \(you\)/); assert.match(menu, /Bo/); assert.doesNotMatch(menu, /Ed/);
      await page.keyboard.press('Escape');
      await chip('link', 'shared').locator('.tagfaces').click();
      menu = (await page.locator('.popover.menu').innerText()).replace(/\s+/g, ' ');
      assert.match(menu, /Ed/); assert.doesNotMatch(menu, /Ann|Bo\b/);
      await page.keyboard.press('Escape');
      await chip('song', 'frenchcore').locator('.tagfaces').click();
      menu = (await page.locator('.popover.menu').innerText()).replace(/\s+/g, ' ');
      assert.match(menu, /and 1 more/i, 'the people not listed are counted');
      assert.ok(await page.locator('.popover.menu .pop-item .face').count() >= 6, 'each person with their face');
      await page.locator('.popover.menu .pop-item').filter({ hasText: 'Bo' }).click();
      await page.waitForURL(/\/person\//); await page.goBack(); await page.locator('#htags .tagitem').first().waitFor(); await sleep(300);
      await shot('album-both');
      // Add on the album, add on the link: each on its own URI (keyboard: + then type, Enter)
      await group('song').locator('[data-htag-plus]').click();
      const inSong = group('song').locator('input[data-htag-add]');
      assert.ok(await inSong.evaluate((e) => e === document.activeElement), 'the input takes the focus');
      assert.ok(await inSong.evaluate((e) => e.list && [...e.list.options].some((o) => o.value === 'link-only')), 'suggestions from labels in use');
      await inSong.fill('Gabber'); await inSong.press('Enter'); await sleep(500);
      assert.deepEqual(writes.at(-1), { m: 'PUT', k: CANON, label: 'Gabber' });
      await group('link').locator('[data-htag-plus]').focus(); await page.keyboard.press('Enter');
      const inLink = group('link').locator('input[data-htag-add]');
      await inLink.fill('rawstyle'); await inLink.press('Enter'); await sleep(500);
      assert.deepEqual(writes.at(-1), { m: 'PUT', k: LINK, label: 'rawstyle' });
      // Escape closes the input without writing
      await group('song').locator('[data-htag-plus]').click(); await group('song').locator('input[data-htag-add]').fill('nope'); await page.keyboard.press('Escape'); await sleep(200);
      assert.equal(await group('song').locator('input[data-htag-add]').count(), 0); assert.ok(!writes.some((w) => w.label === 'nope'));
      // Remove your "shared" on the album: the link's stays
      own[LINK].push({ label: 'shared', synced: true });
      await go('/song/tidal-a'); await group('song').locator('.tagitem').first().waitFor(); await expandAll();
      await chip('song', 'shared').locator('[data-htag-del]').click(); await sleep(500);
      assert.deepEqual(writes.at(-1), { m: 'DELETE', k: CANON, label: 'shared' });
      assert.ok(own[LINK].some((t) => t.label === 'shared'), 'the link keeps its tag');
      // Move your "rawstyle" from the link to the song: the song's write first; when it fails, nothing is deleted
      await go('/song/tidal-a'); await chip('link', 'rawstyle').waitFor({ state: 'attached' }); await expandAll();
      failPut = CANON; const n = writes.length;
      await chip('link', 'rawstyle').locator('[data-htag-move]').click(); await sleep(500);
      assert.equal(writes.length, n, 'a refused move deletes nothing'); assert.ok(own[LINK].some((t) => t.label === 'rawstyle'));
      failPut = null;
      await chip('link', 'rawstyle').locator('[data-htag-move]').click(); await sleep(600);
      assert.deepEqual(writes.slice(-2), [{ m: 'PUT', k: CANON, label: 'rawstyle' }, { m: 'DELETE', k: LINK, label: 'rawstyle' }]);
      // A chip opens the tag (keyboard)
      await chip('song', 'frenchcore').locator('.ptag').focus(); await page.keyboard.press('Enter');
      await page.waitForURL(/\/tag\/frenchcore/);
      // ---- touch targets, no sideways scroll ----
      await go('/song/tidal-a'); await chip('song', 'frenchcore').waitFor();
      if (width < 1100) {
        for (const sel of ['.tagitem .ptag', '[data-htag-plus]', '.tagitem .tagfaces']) {
          const bb = await page.locator(`#htags ${sel}`).first().evaluate((e) => e.getBoundingClientRect());
          assert.ok(bb.height >= 40 && bb.width >= 40, `${sel} is 40 px to touch at ${width} (${bb.width}×${bb.height})`);
        }
        // ×, and move, beside your own tag: each its own finger-sized target
        await expandAll();
        for (const sel of ['[data-htag-del]', '[data-htag-move]']) {
          const bb = await page.locator(`#htags ${sel}`).first().evaluate((e) => e.getBoundingClientRect());
          assert.ok(bb.height >= 40 && bb.width >= 40, `${sel} is 40 px to touch at ${width} (${bb.width}×${bb.height})`);
        }
      }
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth) <= 0, `no sideways scroll at ${width}`);

      // ---- B: many tags fold to about two rows and "+N more" ----
      own = { [CANON]: [], [LINK]: [] };
      pub[CANON] = { tags: Array.from({ length: 30 }, (_, i) => [`tag-${String(i).padStart(2, '0')}`, 30 - i]), taggers: 30, faces: {} };
      await go('/song/tidal-a'); await chip('song', 'tag-00').waitFor(); await sleep(300);
      const rows = async () => group('song').locator('.tagitem:visible, [data-htag-more]:visible, [data-htag-plus]:visible').evaluateAll((els) => { const t = els.map((e) => e.getBoundingClientRect().top).sort((a, b) => a - b); return t.filter((x, k) => k === 0 || x - t[k - 1] > 8).length; });
      assert.ok(await rows() <= 2, `at most two rows of tags at ${width} (${await rows()})`);
      const more = group('song').locator('[data-htag-more]');
      const hidden = 30 - (await labels('song')).length;
      assert.ok(hidden > 0, 'some tags wait behind +N more');
      assert.equal((await more.textContent()).trim(), `+${hidden} more`);
      await shot('album-many');
      await more.click(); await sleep(200);
      assert.equal((await labels('song')).length, 30, '+N more shows them all, in place');
      assert.equal(await group('song').locator('[data-htag-more]').count(), 0);

      // ---- C: loading, then the public tags fill in; D: Pubky doesn't answer; E: no tags ----
      let release; pubHold = new Promise((r) => { release = r; });
      pub[CANON] = { tags: [['frenchcore', 7]], taggers: 7, faces: { frenchcore: [P[1]] } };
      own[CANON] = [{ label: 'mine-first', synced: true }, { label: 'constructor', synced: true }, { label: 'hasownproperty', synced: false }];
      await go('/song/tidal-a');
      await chip('song', 'mine-first').waitFor({ timeout: 5000 });
      assert.equal(await chip('song', 'constructor').count(), 1, 'a tag named like Object\'s own is a tag');
      assert.equal(await chip('song', 'constructor').locator('.tagfaces').count(), 0, 'and has no people of Object\'s');
      assert.ok(await group('song').locator('.tag-skel').count() > 0, 'a quiet placeholder while Pubky answers');
      await shot('album-loading');
      release(); pubHold = null;
      await chip('song', 'frenchcore').waitFor();
      assert.equal(await group('song').locator('.tag-skel').count(), 0);
      assert.equal(await chip('song', 'constructor').count(), 1);
      // An add on its way when Pubky answers: the input stays as it is (no second send), and after a
      // refusal it is back, enabled, with what was typed
      let releaseDiscover; pubHold = new Promise((r) => { releaseDiscover = r; });
      let releasePut; putHold = new Promise((r) => { releasePut = r; }); failPut = CANON;
      await go('/song/tidal-a'); await chip('song', 'mine-first').waitFor();
      await group('song').locator('[data-htag-plus]').click();
      const held = group('song').locator('input[data-htag-add]');
      await held.fill('slowtag'); await held.press('Enter'); await sleep(200);
      assert.ok(await held.isDisabled(), 'sending');
      releaseDiscover(); pubHold = null; await sleep(400);
      assert.ok(await group('song').locator('input[data-htag-add]').isDisabled(), 'still sending after Pubky answered');
      const sent = writes.length; await group('song').locator('input[data-htag-add]').press('Enter'); await sleep(200);
      releasePut(); putHold = null; await sleep(500);
      assert.equal(writes.length, sent, 'one send only');
      const back = group('song').locator('input[data-htag-add]');
      assert.ok(!(await back.isDisabled()), 'enabled again after the refusal');
      assert.equal(await back.inputValue(), 'slowtag');
      failPut = null;
      pubFail = true;
      await go('/song/tidal-a'); await chip('song', 'mine-first').waitFor(); await sleep(400);
      assert.match(await page.locator('#htags').innerText(), /Pubky tags didn.t load/);
      assert.equal(await group('song').locator('[data-htag-plus]').count(), 1, 'your own tags still work');
      await shot('album-error');
      pubFail = false;
      // Your own tags on the link don't load: it says so, offers no add there, and the album's still work
      ownFail = LINK;
      await go('/song/tidal-a'); await chip('song', 'mine-first').waitFor(); await sleep(300);
      assert.match(await group('link').innerText(), /Your tags didn.t load/);
      assert.equal(await group('link').locator('[data-htag-plus]').count(), 0);
      assert.equal(await group('song').locator('[data-htag-plus]').count(), 1);
      ownFail = null;
      own = { [CANON]: [], [LINK]: [] }; pub = {};
      await go('/song/tidal-a'); await group('song').locator('[data-htag-plus]').waitFor(); await sleep(300);
      assert.match(await group('song').innerText(), /No tags yet/);
      await shot('album-empty');

      // ---- F: a library song and a library artist, matched: one group each ----
      pages['song/lib-1'] = item('song', 'lib-1', { title: 'Hardcore Vibes', target: { track_id: 1 }, uri: SONG, song_uri: SONG, on_pubky: { uri: SONG, indexed: true, tags: [['hardcore', 2]], taggers: 2, faces: { hardcore: [P[0], P[2]] } } });
      pages['artist/lib-2'] = item('artist', 'lib-2', { title: 'Various', target: { artist_id: 2 } });
      own = { 'track:1': [{ label: 'hardcore', synced: true }], 'artist:2': [] }; uris = { 'track:1': SONG, 'artist:2': ARTIST };
      pub = { 'track:1': { tags: [['hardcore', 2]], taggers: 2, faces: { hardcore: [P[0], P[2]] } }, 'artist:2': { tags: [['dutch', 4]], taggers: 4, faces: { dutch: P.slice(2, 6) } } };
      asked.length = 0;
      await go('/song/lib-1'); await chip('song', 'hardcore').waitFor();
      assert.equal(await group('link').count(), 0);
      assert.equal(await group('song').locator('.tag-skel').count(), 0, 'the item\'s own on_pubky, about exactly this URI: no waiting');
      assert.deepEqual(asked, [], 'and no second lookup');
      assert.match(await group('song').locator('.tgroup-name').textContent(), /song/i);
      await group('song').locator('[data-htag-plus]').click(); await group('song').locator('input[data-htag-add]').fill('euphoric'); await page.keyboard.press('Enter'); await sleep(500);
      assert.deepEqual(writes.at(-1), { m: 'PUT', k: 'track:1', label: 'euphoric' });
      await shot('song');
      await go('/artist/lib-2'); await chip('song', 'dutch').waitFor();
      assert.match(await group('song').locator('.tgroup-name').textContent(), /artist/i);
      assert.equal(await chip('song', 'dutch').locator('.tagfaces .face').count(), 4);
      assert.equal(await chip('song', 'dutch').locator('.tagfaces .face-more').count(), 0, '4 people: no +N');
      await shot('artist');

      // ---- G: the screenshot's case: a library album with no MusicBrainz match ----
      pages['album/lib-137'] = item('album', 'lib-137', { target: { album_id: 137 }, songs: [{ id: 'lib-501', track_id: 501, title: 'Track one', artist: 'Various', duration_ms: 200000, no: 1 }], on_pubky: { uri: GUESS, indexed: true, tags: [['guessed', 9]], taggers: 9 } });
      own = { 'album:137': 'needs_match' }; pub = { 'album:137': 422 }; uris = {};
      await go('/album/lib-137'); await page.locator('#htags').waitFor(); await sleep(400);
      assert.doesNotMatch(await page.locator('#htags').innerText(), /guessed/, 'a guessed identity\'s tags never show');
      assert.equal(await page.locator('#htags [data-htag-plus], #htags input[data-htag-add]').count(), 0, 'no tag goes to a guessed identity');
      assert.match(await page.locator('#htags').innerText(), /Match this album to tag it/);
      assert.equal(await page.locator('#itags').count(), 0);
      await shot('album-unmatched');
      await page.locator('#htags [data-htag-match]').click();
      await page.locator('#main h1, #main h2').filter({ hasText: /Confirm matches/ }).first().waitFor();
      await page.waitForFunction(() => document.activeElement && document.activeElement.closest && !!document.activeElement.closest('.idalbum'), null, { timeout: 5000 }).catch(() => {});
      assert.equal(await page.evaluate(() => { const a = document.activeElement; const sec = a && a.closest('.idalbum'); return sec ? sec.querySelector('[data-track]').dataset.track : null; }), '501', 'straight to this album\'s folder');
      // Once only: a redraw of the list (here: back and forward) stays put and says nothing more
      await page.evaluate(() => { document.querySelector('#toast').textContent = ''; window.scrollTo(0, 0); });
      await page.goBack(); await page.locator('#htags [data-htag-match]').waitFor(); await page.goForward();
      await page.locator('#main h1, #main h2').filter({ hasText: /Confirm matches/ }).first().waitFor(); await sleep(500);
      assert.equal(await page.locator('#toast').textContent(), '', 'no second message');
      assert.equal(await page.evaluate(() => !!(document.activeElement && document.activeElement.closest && document.activeElement.closest('.idalbum'))), false, 'no second jump to the folder');
      // Its folder further down the list: the page says how to get there
      folders = [folder('a', 300), folder('b', 400)];
      await go('/album/lib-137'); await page.locator('#htags [data-htag-match]').click();
      await page.locator('#main h1, #main h2').filter({ hasText: /Confirm matches/ }).first().waitFor(); await sleep(400);
      assert.match(await page.locator('#toast').textContent(), /Show more/);
      folders = null;
      // A matched library album: the node's URI for its tags, and on_pubky about exactly that one (no second lookup)
      pages['album/lib-7'] = item('album', 'lib-7', { target: { album_id: 7 }, on_pubky: { uri: RELEASE, indexed: true, tags: [['gabber', 5]], taggers: 5, faces: { gabber: [P[2]] } } });
      own = { 'album:7': [] }; uris = { 'album:7': RELEASE }; pub = { 'album:7': { tags: [['gabber', 5]], taggers: 5, faces: { gabber: [P[2]] } }, 'album:137': 422 };
      asked.length = 0;
      await go('/album/lib-7'); await chip('song', 'gabber').waitFor();
      assert.deepEqual(asked, []); assert.doesNotMatch(await page.locator('#htags').innerText(), /Not matched/);
      // Not matched but on TIDAL: the link's tags (others' too) and a note, no album group
      pages['album/tidal-u'] = item('album', 'tidal-u', { source: 'TIDAL', source_id: 'tidal', target: { uri: ALINK }, uri: ALINK, link: ALINK });
      own = { [ALINK]: [] }; pub = { [ALINK]: { tags: [['raw', 3]], taggers: 3, faces: { raw: [P[3]] } } };
      await go('/album/tidal-u'); await chip('link', 'raw').waitFor();
      assert.equal(await group('song').count(), 0);
      assert.match(await page.locator('#htags').innerText(), /on this TIDAL link only/);
      await shot('album-link-only');

      // ---- H: a listener sees the tags and their people, never an add or a remove ----
      me = { role: 'listener', key: P[3], zones: null };
      pub = { [CANON]: { tags: [['frenchcore', 7]], taggers: 7, faces: { frenchcore: [P[1]] } }, [LINK]: { tags: [], taggers: 0 }, 'album:7': { tags: [['gabber', 5]], taggers: 5, faces: { gabber: [P[2]] } }, 'album:137': 422 };
      await go('/song/tidal-a'); await chip('song', 'frenchcore').waitFor();
      assert.equal(await page.locator('#htags [data-htag-plus], #htags [data-htag-del]').count(), 0);
      // On a library album: Pubky says which URI (matched: its tags, read-only; not matched: the note, no button)
      await go('/album/lib-7'); await chip('song', 'gabber').waitFor();
      assert.doesNotMatch(await page.locator('#htags').innerText(), /Not matched/);
      await go('/album/lib-137'); await page.locator('#htags .hnote').waitFor();
      assert.equal(await group('song').count(), 0);
      assert.match(await page.locator('#htags').innerText(), /Not matched to MusicBrainz yet/);
      assert.equal(await page.locator('#htags [data-htag-match]').count(), 0, 'no match button for a listener');
      assert.doesNotMatch(await page.locator('#htags').innerText(), /guessed/);
      me = null;

      // ---- I: the same chip and colours elsewhere ----
      await page.route(/\/v1\/home\/tag\?/, (r) => r.fulfill({ json: { items: [], tags: [['frenchcore', 9], ['hardcore', 4], ['gabber', 2]] } }));
      await page.goto(BASE + '/tag/frenchcore'); await page.locator('#tagcotags .ptag').first().waitFor();
      const co = page.locator('#tagcotags .ptag').first();
      assert.equal(await co.evaluate((e) => getComputedStyle(e).backgroundColor), tagRgba(tagColour('hardcore'), 0.3));
      assert.doesNotMatch(await co.innerText(), /#/);
      await page.goto(BASE + '/tag/frenchcore,hardcore'); await page.locator('#tagcotags [data-tag-solo]').first().waitFor();
      assert.equal(await page.locator('#tagcotags [data-tag-rm]').count(), 2, 'each chosen tag has ×');
      assert.equal(await page.locator('#tagcotags span.ptag.own').count(), 0, 'chosen tags are a filter, not yours');
      await page.locator('#tagcotags [data-tag-rm="hardcore"]').click(); await page.waitForURL(/\/tag\/frenchcore$/);
      await page.goto(BASE + '/tag/frenchcore,hardcore'); await page.locator('#tagcotags [data-tag-solo="hardcore"]').click(); await page.waitForURL(/\/tag\/hardcore$/);
      await page.route(/\/v1\/search\/tags\?/, (r) => r.fulfill({ json: { mine: [{ label: 'frenchcore', count: 3 }], others: ['frenchtek'] } }));
      await page.goto(BASE + '/search?q=frenc'); await page.locator('#stags .ptag').first().waitFor({ timeout: 20000 });
      assert.equal(await page.locator('#stags .ptag.own').first().evaluate((e) => getComputedStyle(e).backgroundColor), tagRgba(tagColour('frenchcore'), 0.3));
      assert.equal(await page.locator('#stags .ptag:not(.own)').first().evaluate((e) => getComputedStyle(e).backgroundColor), tagRgba(tagColour('frenchtek'), 0.3));
      await page.route(/\/v1\/me\/prefs$/, (r) => (r.request().method() === 'GET' ? r.fulfill({ json: { saved_on: 'node', home_layout: { value: { order: ['tags'], hidden: [] }, applies: 'yours' } } }) : r.fallback()));
      await page.route(/\/v1\/home\/layout$/, (r) => r.fulfill({ json: { order: ['tags'], hidden: [] } }));
      await page.route(/\/v1\/home\/tags(\?.*)?$/, (r) => r.fulfill({ json: { section: 'tags', items: [], tags: [['gabber', 12]] } }));
      await page.goto(BASE + '/'); await page.locator('#main .ptag[data-tag-open="gabber"]').waitFor({ timeout: 20000 });
      assert.equal(await page.locator('#main .ptag[data-tag-open="gabber"]').evaluate((e) => getComputedStyle(e).backgroundColor), tagRgba(tagColour('gabber'), 0.3));
      own = { [CANON]: [{ label: 'frenchcore', synced: true }] };
      await page.goto(BASE + '/genres'); await page.locator('[data-mytag]').first().waitFor();
      assert.equal(await page.locator('[data-mytag]').first().evaluate((e) => getComputedStyle(e).backgroundColor), tagRgba(tagColour('frenchcore'), 0.3));
      if (width < 1100) assert.ok((await page.locator('[data-mytag]').first().boundingBox()).height >= 40);

      assert.deepEqual(errors, []);
      await ctx.close();
      console.log(`ok ${width}: tags in the hero, two scopes, pubky.app chips and faces, overflow, states, unmatched, listener, other pages`);
    }
  } finally { await browser.close(); }
})().catch((e) => { console.error(e); process.exit(1); });

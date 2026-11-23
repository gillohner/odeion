// Someone the owner added sees only what works for them. Following a person, saving a copy of a
// shared playlist and comments are the owner's (crates/pm-api/src/access.rs: `owner_music`), so on a
// person's page, a shared playlist's page and a song's page, at 375 and 1280 px:
// - someone the owner added (by what GET /v1/me says they can do, and on an older node that only
//   names their role): no Follow, no Save a copy, nothing that reads like them, and no request for
//   either; the buttons that stay start where the row starts and keep the row's own gap; Copy link
//   still answers; a song's page has no Comments section and asks for no comments; no page errors,
//   no sideways scroll;
// - the owner (both ways too): Follow, Following and Save a copy are there, and a click sends its
//   request once; a song's page has its Comments, asked for once.
// Fixture only (no node): node tools/ui-listener-buttons.js <repo> <screenshots dir>
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const { create, go, cleanCopy, detail, track, SONG } = require('./ui-fixture');
const OUT = process.argv[3] || '/tmp/ui-listener-buttons';
fs.mkdirSync(OUT, { recursive: true });

const KEY = 'zswn37ir9t8ge4r4posn4r7ow3eh384c8ud4kkxm585igt38x31o';
const ID = '0035WKVKQDHSM';
const LINK = `pubky://${KEY}/pub/social/v1/posts/${ID}`;
// What GET /v1/me lists for each caller (crates/pm-api/src/access.rs)
const PERSON_CAN = ['artwork', 'session', 'library', 'zone_list', 'zone_use', 'own', 'own_play', 'own_sign_in_state', 'own_sign_in'];
const OWNER_CAN = [...PERSON_CAN, 'owner_music', 'zone_manage', 'library_manage', 'settings', 'service_logins', 'people', 'tokens', 'vault', 'backup', 'identity', 'remote_access', 'party', 'diagnostics'];
const REFUSAL = 'that is the owner\'s: a listener\'s own are their bookmarks and collections';
const CALLERS = [
  { who: 'someone the owner added', tag: 'person', owner: false, me: { role: 'listener', key: null, zones: null, can: PERSON_CAN } },
  { who: 'someone the owner added, older node', tag: 'person-old', owner: false, me: { role: 'listener', key: null, zones: null } },
  { who: 'the owner', tag: 'owner', owner: true, me: { role: 'owner', key: null, zones: null, can: OWNER_CAN } },
  { who: 'the owner, older node', tag: 'owner-old', owner: true, me: { role: 'owner', key: null, zones: null } },
];
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, what, ms = 6000) => { const end = Date.now() + ms; for (;;) { const v = await fn(); if (v) return v; if (Date.now() > end) assert.fail(`timed out waiting for: ${what}`); await sleep(40); } };
const shared = () => ({
  summary: { id: ID, name: 'Night drives', items: 1, link: LINK, description: '' },
  entries: [{ kind: 'track', uri: SONG }],
  play_from: { order: { order: ['library'], services: [] }, own_order: false, items: [{ index: 0, uri: SONG, title: track.title, artist: 'Sefa', track_id: 1, plays_from: { service: 'library', label: 'Your library' }, sources: [] }] },
});
const profile = () => ({ id: KEY, name: 'Ada Lovelace', avatar: null, you: false, following: false, followed_by: false, followers: 3, following_count: 2, bio: 'Tags what she plays.', links: [], tags: [['ambient', 2]], tagged: [], profile_url: `https://pubky.app/profile/${KEY}` });

// The hero's row of buttons: what is in it, where each one sits and the gap the row asks for
const actionsRow = (page) => page.locator('#main .hero .actions').evaluate((row) => {
  const box = row.getBoundingClientRect(); const cs = getComputedStyle(row);
  return {
    left: box.left + parseFloat(cs.paddingLeft), height: box.height, gap: parseFloat(cs.columnGap) || 0, kids: row.children.length,
    shown: [...row.children].filter((c) => c.getClientRects().length).map((c) => { const b = c.getBoundingClientRect(); return { id: c.id, text: c.textContent.trim(), left: b.left, right: b.right, top: b.top, width: b.width }; }),
  };
});
// No hole where a button was: the first one starts where the row starts, and two on one line are
// the row's gap apart
const noHoles = (row, where) => {
  assert.equal(row.shown.length, row.kids, `${where}: nothing in the row is hidden`);
  assert.ok(row.height > 0 && row.shown.every((b) => b.width > 0), `${where}: the row and its buttons have a size`);
  assert.ok(Math.abs(row.shown[0].left - row.left) < 1, `${where}: the first button starts where the row starts (${row.shown[0].left} / ${row.left})`);
  for (let k = 1; k < row.shown.length; k++) {
    const a = row.shown[k - 1]; const b = row.shown[k];
    if (Math.abs(a.top - b.top) < 1) assert.ok(Math.abs(b.left - a.right - row.gap) < 1, `${where}: “${a.text}” and “${b.text}” are the row's gap apart (${b.left - a.right} / ${row.gap})`);
    else assert.ok(Math.abs(b.left - row.left) < 1, `${where}: “${b.text}” starts its line`);
  }
};
// Every button, link and menu row in the page, by what it says
const controls = (page) => page.evaluate(() => [...document.querySelectorAll('button, a, [role="button"], [role="menuitem"]')].flatMap((c) => [c.textContent, c.getAttribute('aria-label') || '', c.getAttribute('title') || '']).map((t) => t.trim()).filter(Boolean));
const clean = async (page, state, where) => {
  await cleanCopy(page);
  assert.deepEqual(state.errors, [], `${where}: no page errors`);
  assert.equal(await page.locator('#toast.show.error').count(), 0, `${where}: nothing went wrong`);
};

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [375, 1280]) {
      for (const c of CALLERS) {
        const at = `${width} ${c.who}`;
        const pg = profile();
        const { context, page, state } = await create(browser, width, {
          role: c.me.role, touch: width < 600,
          fixtures: { '/me': { body: c.me }, '/profiles': { body: [] }, '/reviews': { body: { reviews: [] } }, '/tags': { body: { tags: [], uri: null } }, '/comments': c.owner ? { body: { comments: [], indexed: true } } : { status: 403, body: { error: REFUSAL } }, [`/profiles/${KEY}`]: { body: pg }, [`/shared/${KEY}/playlists`]: { body: [{ ...shared().summary, covers: [], tracks_only: true }] }, [`/shared/${KEY}/playlists/${ID}`]: { body: shared() } },
        });
        // The owner's routes, as the node answers each caller: following keeps what it was told, a
        // copy is a playlist of the caller's; anyone else is refused
        const sent = [];
        await context.route(/\/v1\/(profiles\/[^/]+\/follow|shared\/[^/]+\/(playlists|collections)\/[^/]+\/(copy|duplicate))$/, (r) => {
          const req = r.request(); const m = req.method(); const v = new URL(req.url()).pathname.slice(3);
          sent.push(`${m} ${v}`);
          const json = (body, status = 200) => r.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
          if (!c.owner) return json({ error: REFUSAL }, 403);
          if (v.endsWith('/follow')) { pg.following = m === 'PUT'; return json({ following: pg.following }); }
          return json(detail);
        });
        const count = (call) => sent.filter((s) => s === call).length;

        // ---- a person's page ----
        await go(page, `/person/${KEY}`);
        await page.locator('#main .hero.person h1', { hasText: 'Ada Lovelace' }).waitFor();
        await page.locator('#main [data-pshared]').waitFor();
        let row = await actionsRow(page);
        if (c.owner) {
          const fb = page.locator('#pfollow');
          assert.deepEqual(row.shown.map((b) => b.text), ['Follow', 'Profile on pubky.app'], `${at}: the person's buttons`);
          assert.equal(await fb.getAttribute('aria-pressed'), 'false', `${at}: not followed yet`);
          noHoles(row, `${at}: the person's buttons`);
          await page.screenshot({ path: path.join(OUT, `${width}-${c.tag}-person.png`) });
          await fb.click();
          await page.locator('#pfollow[aria-pressed="true"]', { hasText: 'Following' }).waitFor();
          await page.locator('#toast', { hasText: 'Following Ada Lovelace' }).waitFor();
          await sleep(300);
          assert.deepEqual(sent, [`PUT /profiles/${KEY}/follow`], `${at}: Follow is sent once`);
          await page.locator('#pfollow').click();
          await page.locator('#pfollow[aria-pressed="false"]', { hasText: 'Follow' }).waitFor();
          await sleep(300);
          assert.deepEqual(sent, [`PUT /profiles/${KEY}/follow`, `DELETE /profiles/${KEY}/follow`], `${at}: Following is taken back once`);
        } else {
          assert.equal(await page.locator('#pfollow').count(), 0, `${at}: no Follow button`);
          assert.deepEqual(row.shown.map((b) => b.text), ['Profile on pubky.app'], `${at}: the person's buttons`);
          noHoles(row, `${at}: the person's buttons`);
          assert.deepEqual((await controls(page)).filter((t) => /^(un)?follow(ing)?\b/i.test(t)), [], `${at}: nothing offers to follow`);
          await page.screenshot({ path: path.join(OUT, `${width}-${c.tag}-person.png`) });
        }
        await clean(page, state, `${at}: the person's page`);

        // ---- a shared playlist's page, opened from the person's ----
        await page.locator(`#main [data-pshared="${ID}"]`).click();
        await page.locator('#main .hero h1', { hasText: 'Night drives' }).waitFor();
        await page.locator('#shsongs .name', { hasText: track.title }).first().waitFor();
        row = await actionsRow(page);
        if (c.owner) {
          assert.deepEqual(row.shown.map((b) => b.id), ['shplay', 'shcopy', 'shlink'], `${at}: the shared playlist's buttons`);
          assert.equal((await page.locator('#shcopy').innerText()).trim(), 'Save a copy', `${at}: Save a copy`);
          noHoles(row, `${at}: the shared playlist's buttons`);
          await page.screenshot({ path: path.join(OUT, `${width}-${c.tag}-shared.png`) });
          await clean(page, state, `${at}: the shared playlist's page`);
          await page.locator('#shcopy').click();
          await until(() => new URL(page.url()).pathname === `/playlist/${detail.summary.id}`, `${at}: the copy opens`);
          await sleep(300);
          assert.equal(count(`POST /shared/${KEY}/playlists/${ID}/copy`), 1, `${at}: Save a copy is sent once`);
          assert.equal(sent.length, 3, `${at}: nothing else was sent (${sent.join(', ')})`);
          assert.deepEqual(state.errors, [], `${at}: no page errors after the copy`);
        } else {
          assert.equal(await page.locator('#shcopy').count(), 0, `${at}: no Save a copy button`);
          assert.deepEqual(row.shown.map((b) => b.id), ['shplay', 'shlink'], `${at}: the shared playlist's buttons`);
          noHoles(row, `${at}: the shared playlist's buttons`);
          assert.deepEqual((await controls(page)).filter((t) => /save a copy|duplicate/i.test(t)), [], `${at}: nothing offers a copy`);
          await page.screenshot({ path: path.join(OUT, `${width}-${c.tag}-shared.png`) });
          // What stays still answers: Copy link (the link itself where this browser has no clipboard)
          await page.locator('#shlink').click();
          const said = await until(async () => { const t = (await page.locator('#toast').textContent()).trim(); return t || null; }, `${at}: Copy link answers`);
          assert.ok(said === 'Link copied' || said === LINK, `${at}: Copy link answers (“${said}”)`);
          await clean(page, state, `${at}: the shared playlist's page`);
          assert.deepEqual(sent, [], `${at}: nothing the node would refuse was asked for`);
        }

        // ---- a song's page: its Comments ----
        await go(page, `/song/lib-${track.id}`);
        await page.locator('#main .hero h1', { hasText: track.title }).waitFor();
        await page.locator('#main #imore').waitFor();
        const asked = () => state.hits.filter((h) => h.v === '/comments').map((h) => h.m);
        const headings = () => page.locator('#main h3').allInnerTexts().then((hs) => hs.map((h) => h.trim()));
        if (c.owner) {
          await page.locator('#icomments form.comment-form textarea[aria-label="Add a comment"]').waitFor();
          assert.ok((await headings()).includes('Comments'), `${at}: the Comments heading`);
          await sleep(400);
          assert.deepEqual(asked(), ['GET'], `${at}: the comments are asked for once`);
        } else {
          await sleep(400);
          assert.equal(await page.locator('#icomments').count(), 0, `${at}: no Comments section`);
          assert.equal(await page.locator('#main .comment-form, #main .comments-body').count(), 0, `${at}: nothing of the comments`);
          assert.deepEqual((await headings()).filter((h) => /comment/i.test(h)), [], `${at}: no Comments heading`);
          assert.doesNotMatch(await page.locator('#main').innerText(), /that is the owner|comment/i, `${at}: no refusal on the song's page`);
          assert.deepEqual(asked(), [], `${at}: no comments are asked for`);
        }
        await page.screenshot({ path: path.join(OUT, `${width}-${c.tag}-song.png`), fullPage: true });
        await clean(page, state, `${at}: the song's page`);
        await context.close();
        console.log(`ok ${at}`);
      }
    }
  } finally {
    await browser.close();
  }
})().catch((e) => { console.error(e); process.exit(1); });

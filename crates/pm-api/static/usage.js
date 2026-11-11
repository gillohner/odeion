// Usage insights: the browser's part (docs/guides/usage-insights.md; the node's part is usage.rs).
// Nothing is recorded unless the node's owner turned usage insights on (they ship off) and this
// person did not stop them for themselves. Then, and only then, this records fixed categories:
// which page (never which album, search or person), how a page or request went (its kind, never
// its address), which kind of button was used and how, and a script error's built-in class. Never
// anything typed, a value, a name, a link, an id from the page, an error's text or a credential.
// Events wait in memory only (never in storage: nothing comes back after Stop, Log out or a
// reload) and go to this node only, with the page's own sign-in in a header.
//   PM_USAGE.mount({ token, view, shell, offline, open, ask })  once the node said who is signed in
//   PM_USAGE.end()                                            the sign-in ended (Log out, another account)
//   PM_USAGE.nav(view, how) / loaded(view, outcome, ms)       a page opened by the person, and how it went
//   PM_USAGE.apiDone(method, path, status, ms)                a request the person's page or press made
//   PM_USAGE.shortcut(name)                                   a keyboard shortcut
//   PM_USAGE.renderPersonal(el) / renderNode(el) / on()       Settings, and the account menu's line
(() => {
  'use strict';
  if (window.PM_USAGE) return;
  const SCHEMA = 'pm-usage/1';
  // ---- the closed lists (each value is the node's enum; usage.rs tests that it parses) ----
  const KINDS = Object.freeze(['nav', 'load', 'api', 'action', 'exception']);
  const VIEWS = Object.freeze(['add_music', 'albums', 'artists', 'dashboard', 'downloads', 'favourites', 'genre', 'genres', 'home', 'home_section', 'identify', 'item', 'library', 'person', 'playlist', 'playlists', 'queue', 'radio', 'releases', 'search', 'settings', 'shared', 'smart', 'smart_edit', 'source_artist', 'source_item', 'sources', 'tag', 'tracks', 'zones']);
  const AREAS = Object.freeze(['library', 'search', 'item', 'playlists', 'shared', 'sources', 'zones', 'outputs', 'node', 'home', 'me', 'people', 'radio', 'tags', 'favourites', 'listenbrainz', 'other']);
  const SURFACES = Object.freeze(['page', 'player_bar', 'now_playing', 'zone_pop', 'sound_panel', 'menu', 'dialog', 'job_pop', 'notice']);
  const VIAS = Object.freeze(['click', 'key', 'change']);
  const HOWS = Object.freeze(['push', 'pop', 'redirect', 'initial']);
  const VIEWPORTS = Object.freeze(['phone', 'tablet', 'desktop']);
  const ERROR_CLASSES = Object.freeze(['type_error', 'reference_error', 'range_error', 'syntax_error', 'other']);
  const CONTROLS = Object.freeze([
    'player_now_playing', 'player_shuffle', 'player_prev', 'player_toggle', 'player_next', 'player_repeat', 'player_seek', 'player_quality', 'player_volume_button', 'player_volume', 'player_zone', 'player_along_leave', 'job_chip',
    'search_box', 'sidebar_toggle', 'account', 'connection_retry', 'side_nav', 'bottom_nav', 'library_tab', 'view_go',
    'row_play', 'row_next', 'row_add', 'row_more', 'row_favourite', 'row_album', 'row_identify', 'row_song', 'row_artist',
    'select_all', 'select_cancel', 'select_play', 'select_playlist', 'select_queue', 'select_remove', 'select_tag',
    'np_shuffle', 'np_prev', 'np_toggle', 'np_next', 'np_repeat', 'np_sheet_queue',
    'job_cancel', 'job_dismiss', 'job_pause', 'job_resume', 'job_retry',
    'settings_accounts', 'settings_home', 'settings_quality', 'settings_playfrom', 'settings_privacy', 'settings_appearance', 'settings_dac', 'settings_apps', 'settings_people', 'settings_playback', 'settings_node_home', 'settings_usage', 'settings_remote', 'settings_discover', 'settings_backup', 'settings_about', 'settings_homeserver',
    'theme_pick', 'layout_pick',
    'key_search', 'key_toggle', 'key_prev', 'key_next', 'key_now_playing', 'key_help', 'key_escape',
    'usage_notice_dismiss', 'usage_notice_more', 'usage_stop', 'usage_share', 'usage_clear',
    'other',
  ]);
  const has = (list, v) => list.indexOf(v) >= 0;
  // A page of the app as the schema names it, or nothing (a page not on the list is not recorded)
  const viewOf = (v) => { const k = typeof v === 'string' ? v.replace(/-/g, '_') : ''; return has(VIEWS, k) ? k : null; };
  // A request's family, from its path's first part only (the path itself never leaves this file)
  const AREA_OF = { library: 'library', search: 'search', item: 'item', playlists: 'playlists', collections: 'playlists', shared: 'shared', sources: 'sources', zones: 'zones', outputs: 'outputs', node: 'node', home: 'home', me: 'me', people: 'people', radio: 'radio', tags: 'tags', favourites: 'favourites', bookmarks: 'favourites', listenbrainz: 'listenbrainz' };
  const ms = (t) => (t < 100 ? 'lt100' : t < 300 ? 'lt300' : t < 1000 ? 'lt1000' : t < 3000 ? 'lt3000' : t < 10000 ? 'lt10000' : 'ge10000');
  const outcomeOf = (status) => (status === 0 ? 'unreachable' : status >= 200 && status < 400 ? 'ok' : status === 401 || status === 403 ? 'refused' : status < 500 ? 'http_4xx' : 'http_5xx');
  const ERROR_NAMES = { TypeError: 'type_error', ReferenceError: 'reference_error', RangeError: 'range_error', SyntaxError: 'syntax_error' };
  // The registry: which elements are which control. Only an attribute's value on its own finite
  // list is used; anything else of the element (its text, label, other attributes) never is.
  const fixed = (c) => () => c;
  const fromList = (attr, map) => (el) => map[el.getAttribute(attr)] || 'other';
  const REGISTRY = Object.freeze([
    ['#np', fixed('player_now_playing')], ['#shufflebtn', fixed('player_shuffle')], ['#prev', fixed('player_prev')], ['#toggle', fixed('player_toggle')],
    ['#next', fixed('player_next')], ['#repeatbtn', fixed('player_repeat')], ['#quality', fixed('player_quality')], ['#volbtn', fixed('player_volume_button')],
    ['#zonebtn', fixed('player_zone')], ['#alongleave', fixed('player_along_leave')], ['#jobchip', fixed('job_chip')],
    ['#seek', fixed('player_seek'), 'change'], ['#vol', fixed('player_volume'), 'change'],
    ['#search', fixed('search_box')], ['#menu', fixed('sidebar_toggle')], ['#me', fixed('account')], ['#connretry', fixed('connection_retry')],
    ['.nav-item[data-view]', fixed('side_nav'), 'click', 'data-view'], ['.bnav[data-bnav]', fixed('bottom_nav'), 'click', 'data-bnav'],
    ['[data-libnav]', fixed('library_tab'), 'click', 'data-libnav'], ['[data-view-go]', fixed('view_go'), 'click', 'data-view-go'],
    ['[data-act]', fromList('data-act', { play: 'row_play', next: 'row_next', add: 'row_add', more: 'row_more', fav: 'row_favourite', album: 'row_album', identify: 'row_identify', song: 'row_song', tartist: 'row_artist' })],
    ['[data-sel]', fromList('data-sel', { all: 'select_all', cancel: 'select_cancel', play: 'select_play', playlist: 'select_playlist', queue: 'select_queue', remove: 'select_remove', tag: 'select_tag' })],
    ['[data-npt]', fromList('data-npt', { shuffle: 'np_shuffle', prev: 'np_prev', toggle: 'np_toggle', next: 'np_next', repeat: 'np_repeat' })],
    ['[data-npsheet="queue"]', fixed('np_sheet_queue')],
    ['[data-jact]', fromList('data-jact', { cancel: 'job_cancel', dismiss: 'job_dismiss', pause: 'job_pause', resume: 'job_resume', retry: 'job_retry' })],
    ['[data-stab]', (el) => { const c = `settings_${String(el.getAttribute('data-stab')).replace(/-/g, '_')}`.replace('settings_nodehome', 'settings_node_home'); return has(CONTROLS, c) ? c : 'other'; }],
    ['[data-theme-pick]', fixed('theme_pick')], ['[data-layout-pick]', fixed('layout_pick')],
    ['[data-usage]', fromList('data-usage', { dismiss: 'usage_notice_dismiss', more: 'usage_notice_more', stop: 'usage_stop', share: 'usage_share', clear: 'usage_clear' })],
  ]);
  const SURFACE_OF = Object.freeze([['.usage-notice', 'notice'], ['dialog', 'dialog'], ['.popover.menu', 'menu'], ['#zonepop', 'zone_pop'], ['#qpanel', 'sound_panel'], ['#volpop', 'sound_panel'], ['#jobpop', 'job_pop'], ['#nowplaying', 'now_playing'], ['.player', 'player_bar']]);
  // ---- limits (the node's own are tighter checks of the same) ----
  const QUEUE_MAX = 200; const PER_MIN = 110; const FLUSH_MS = 15000; const MAX_BODY = 15 * 1024; const MAX_DROPPED = 500;
  // Pages and requests count only shortly after the person's own press or key (or the page's start,
  // or Back and Forward): a page's own .click(), a socket redraw or a timer while nobody is there is not use
  const OWN_MS = 10000;

  // ---- state: everything of one sign-in, gone with it ----
  let ctx = null; // the mount's callbacks
  let token = ''; // the sign-in this was mounted with
  let gen = 0; // this page's own generation: every end, Stop, Clear or refusal moves it on
  let me = null; // the node's last answer for this sign-in (GET /v1/usage/me)
  let known = false; // that answer has come
  let paused = false; // after a refusal: nothing until the person's next own action
  let alias = null; // { id, max, gen, token }
  let opening = null;
  let queue = []; let dropped = 0; let sending = null; let timer = 0;
  let minute = { start: 0, n: 0 };
  let lastOwn = 0; // when the person last pressed, typed a key or started the page
  let navView = null; // the page whose opening was recorded: its load is, too
  const own = (at = Date.now()) => at - lastOwn < OWN_MS;
  const live = () => !!(ctx && token && known && me && me.node_on && me.sharing);
  const excluded = () => !ctx || !token || (ctx.offline && ctx.offline()) || (ctx.view && ctx.view() === 'dashboard');
  const viewport = () => { const s = ctx && ctx.shell ? ctx.shell() : 'desktop'; if (s === 'phone') return 'phone'; return document.documentElement.clientWidth < 1024 ? 'tablet' : 'desktop'; };
  const drop = () => { queue = []; alias = null; opening = null; clearTimeout(timer); timer = 0; if (sending) { sending.ctl.abort(); sending = null; } };
  // Anything waiting or on its way belongs to the old generation: none of it lands
  const fresh = () => { gen += 1; drop(); dropped = 0; };

  const request = (method, path, body, opts = {}) => fetch(`/v1${path}`, {
    method, keepalive: !!opts.keepalive, signal: opts.signal, cache: 'no-store',
    headers: { authorization: `Bearer ${opts.token || token}`, ...(body === undefined ? {} : { 'content-type': 'application/json' }) },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const answer = async (r) => { let j = null; try { j = await r.json(); } catch { j = null; } return j; };
  // The node's fixed refusals (usage.rs Refusal): which of them means what for this page
  const why = (j) => { const e = (j && j.error) || ''; return /are off on this node/.test(e) ? 'off' : /stopped usage insights for yourself/.test(e) ? 'stopped' : 'stale'; };

  // ---- recording ----
  const record = (ev, own = false) => {
    if (!ctx || excluded()) return;
    if (known && !live()) return;
    if (paused) { if (!own) return; paused = false; }
    const now = Date.now();
    if (now - minute.start >= 60000) minute = { start: now, n: 0 };
    if (queue.length >= QUEUE_MAX || minute.n >= PER_MIN) { dropped = Math.min(MAX_DROPPED, dropped + 1); return; }
    minute.n += 1; queue.push(ev);
    if (!known) return; // sent once the node says it's on (else dropped)
    if (queue.length >= ((alias && alias.max) || 50)) flush(); else if (!timer) timer = setTimeout(() => { timer = 0; flush(); }, FLUSH_MS);
  };
  const openAlias = async (g) => {
    if (alias && alias.gen === g) return alias;
    if (!opening) {
      const p = (async () => {
        const r = await request('POST', '/usage/sessions', { viewport: viewport(), generation: me.generation, choice: me.choice });
        const j = await answer(r);
        if (g !== gen) return null;
        if (!r.ok) { await refused(r.status, j, 0); return null; }
        alias = { id: j.alias, max: Math.max(1, Math.min(50, j.max_batch || 50)), gen: g, token };
        return alias;
      })().catch(() => null);
      opening = p; p.finally(() => { if (opening === p) opening = null; });
    }
    return opening;
  };
  // One batch at a time; the page going away sends what waits, once, with its header (keepalive)
  const flush = async (leaving = false) => {
    if (!live() || !queue.length || (sending && !leaving)) return;
    clearTimeout(timer); timer = 0;
    const g = gen; const t = token;
    // (one send at a time from here: a timer and a hidden page never both send or open an alias)
    const ctl = new AbortController(); const me1 = { ctl };
    if (!leaving) sending = me1;
    const a = leaving ? (alias && alias.gen === g ? alias : null) : await openAlias(g);
    if (!a || g !== gen || t !== token || a.token !== t) { if (sending === me1) sending = null; return; }
    const events = queue.splice(0, a.max);
    const body = { alias: a.id, events, ...(dropped ? { dropped } : {}) };
    while (JSON.stringify(body).length > MAX_BODY && body.events.length > 1) queue.unshift(body.events.pop());
    const sentDropped = dropped; dropped = 0;
    let r; let j = null;
    try { r = await request('POST', '/usage/events', body, { keepalive: leaving, signal: leaving ? undefined : ctl.signal, token: t }); if (!r.ok) j = await answer(r); } catch { r = null; }
    if (sending === me1) sending = null;
    if (g !== gen) return; // Stop, Clear, Log out or a refusal came first: nothing of this counts
    if (!r) { dropped = Math.min(MAX_DROPPED, sentDropped + events.length + dropped); return; }
    if (!r.ok) { await refused(r.status, j, sentDropped + events.length); return; }
    if (queue.length && !leaving) { if (queue.length >= a.max) flush(); else if (!timer) timer = setTimeout(() => { timer = 0; flush(); }, FLUSH_MS); }
  };
  // 401/403: the sign-in is over. 409/410: the node's state moved (Off, Stop, Clear, a sign-in
  // ended, the alias ended): what waits is dropped and the page asks again; a new alias only after
  // the person's next own action. Anything else: dropped and counted.
  const refused = async (status, j, lost) => {
    if (status === 401 || status === 403) { end(); return; }
    if (status === 409 || status === 410) {
      const kind = status === 410 ? 'gone' : why(j);
      fresh(); paused = true;
      if (kind === 'off' && me) me = { ...me, node_on: false };
      if (kind === 'stopped' && me) me = { ...me, sharing: false };
      await readMe(); return;
    }
    dropped = Math.min(MAX_DROPPED, dropped + lost);
  };
  const readMe = async () => {
    const g = gen; const t = token;
    let r; try { r = await request('GET', '/usage/me'); } catch { return null; }
    const j = await answer(r);
    if (g !== gen || t !== token) return null;
    if (r.status === 401 || r.status === 403) { end(); return null; }
    if (!r.ok || !j || j.schema !== SCHEMA) { me = null; known = true; queue = []; return null; }
    apply(j); return j;
  };
  // The node's answer is the state: on and sharing records, anything else drops what waits
  const apply = (j) => {
    const was = live();
    me = j; known = true;
    if (!live()) { if (was || queue.length) fresh(); hideNotice(); return; }
    if (queue.length && !timer) timer = setTimeout(() => { timer = 0; flush(); }, FLUSH_MS);
    showNotice();
  };

  // ---- the person's own actions (trusted events only: a page's own .click() is never one) ----
  const controlOf = (target, type) => {
    for (let el = target; el && el.nodeType === 1; el = el.parentElement) {
      for (const [sel, ctl, on = 'click', toAttr] of REGISTRY) {
        if (!el.matches(sel)) continue;
        if (on !== type) return null;
        const c = ctl(el);
        return { control: has(CONTROLS, c) ? c : 'other', to: toAttr ? viewOf(el.getAttribute(toAttr)) : null };
      }
    }
    return null;
  };
  const surfaceOf = (el) => { for (const [sel, s] of SURFACE_OF) if (el.closest && el.closest(sel)) return s; return 'page'; };
  const onEvent = (e) => {
    if (e.isTrusted) lastOwn = Date.now();
    if (!e.isTrusted || !ctx || !(e.target instanceof Element)) return;
    const hit = controlOf(e.target, e.type);
    if (!hit) return;
    const view = viewOf(ctx.view && ctx.view());
    if (!view) return;
    const via = e.type === 'change' ? 'change' : e.detail === 0 ? 'key' : 'click';
    record({ kind: 'action', control: hit.control, view, surface: surfaceOf(e.target), via, ...(hit.to ? { to: hit.to } : {}) }, true);
  };
  // Before app.js's own listeners (some stop the event on the document)
  window.addEventListener('click', onEvent, true);
  window.addEventListener('change', onEvent, true);
  window.addEventListener('keydown', (e) => { if (e.isTrusted) lastOwn = Date.now(); }, true);
  window.addEventListener('pointerdown', (e) => { if (e.isTrusted) lastOwn = Date.now(); }, true);
  // A script error: its built-in class, nothing of its message or where it came from
  window.addEventListener('error', (e) => { if (!(e instanceof ErrorEvent)) return; record({ kind: 'exception', class: ERROR_NAMES[e.error && e.error.name] || 'other' }); });
  window.addEventListener('unhandledrejection', (e) => { const n = e.reason && e.reason.name; if (n === 'AbortError') return; record({ kind: 'exception', class: ERROR_NAMES[n] || 'other' }); });
  document.addEventListener('visibilitychange', () => { if (document.visibilityState === 'hidden') flush(); });
  window.addEventListener('pagehide', () => { flush(true); });

  // ---- the hooks app.js calls ----
  const SHORTCUTS = { search: 'key_search', toggle: 'key_toggle', prev: 'key_prev', next: 'key_next', now_playing: 'key_now_playing', help: 'key_help', escape: 'key_escape' };
  const nav = (v, how) => {
    const view = viewOf(v); navView = null;
    if (!view || view === 'dashboard') return;
    const h = has(HOWS, how) ? how : how === 'replace' ? 'redirect' : 'push';
    if (h === 'initial' || h === 'pop') lastOwn = Date.now(); else if (!own()) return;
    navView = view; record({ kind: 'nav', view, how: h });
  };
  const loaded = (v, outcome, t) => { const view = viewOf(v); if (!view || view !== navView || !['ok', 'failed'].includes(outcome)) return; navView = null; record({ kind: 'load', view, outcome, ms: ms(t) }); };
  const apiDone = (method, path, status, t) => {
    if (typeof path !== 'string' || status < 0) return;
    const first = path.replace(/^\/+/, '').split(/[/?#]/)[0];
    if (first === 'usage' || /^\/?node\/usage\b/.test(path)) return; // its own requests are not usage
    if (!own(Date.now() - Math.max(0, t || 0))) return; // (started shortly after the person's own press)
    record({ kind: 'api', area: AREA_OF[first] || 'other', class: method === 'GET' ? 'read' : 'write', outcome: outcomeOf(status), ms: ms(t) });
  };
  const shortcut = (name) => { const c = SHORTCUTS[name]; const view = ctx && viewOf(ctx.view && ctx.view()); if (!c || !view) return; record({ kind: 'action', control: c, view, surface: 'page', via: 'key' }, true); };

  // ---- mount and end ----
  const mount = (opts) => {
    const t = (opts && opts.token && opts.token()) || '';
    if (ctx && t === token) return;
    if (ctx) end();
    ctx = opts; token = t; fresh(); me = null; known = false; paused = false; lastOwn = Date.now();
    if (excluded()) { known = true; return; }
    readMe();
  };
  const end = () => { fresh(); ctx = null; token = ''; me = null; known = false; paused = false; hideNotice(); };

  // ---- the notice: once per person, never over the music ----
  const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
  const notice = () => document.getElementById('usagenotice');
  const hideNotice = () => { const n = notice(); if (n) { n.hidden = true; n.innerHTML = ''; } };
  const showNotice = () => {
    const n = notice();
    if (!n || !me || me.notice_seen >= me.notice_version || excluded() || document.body.classList.contains('kiosk')) return;
    if (!n.hidden && n.firstChild) return;
    n.innerHTML = `<div class="usage-notice notice" role="region" aria-label="Usage insights"><p>This node records which buttons and pages are used, never what you type, search, play or name, to improve the app. Only the node’s owner can read them, on this node; nothing goes to anyone else or leaves the node automatically. Party guests are not covered yet.</p><div class="usage-notice-actions"><button class="btn sm ghost" data-usage="more">What’s collected</button><button class="btn sm ghost" data-usage="stop">Turn off for me</button><button class="btn sm" data-usage="dismiss">OK</button></div></div>`;
    n.hidden = false;
    const seen = async () => { const had = n.contains(document.activeElement); hideNotice(); if (had) { const m = document.getElementById('main'); if (m) m.focus({ preventScroll: true }); } const g = gen; try { const r = await request('POST', '/usage/me/notice', { version: me.notice_version }); const j = await answer(r); if (r.ok && g === gen && j && j.schema === SCHEMA) apply(j); } catch { /* shown again next time */ } };
    n.querySelector('[data-usage="dismiss"]').onclick = seen;
    n.querySelector('[data-usage="more"]').onclick = () => { seen(); if (ctx && ctx.open) ctx.open(); };
    n.querySelector('[data-usage="stop"]').onclick = async (e) => {
      const b = e.currentTarget; const had = n.contains(document.activeElement); b.disabled = true;
      // (a Stop the node didn't take still holds on this page; the notice says so and stays)
      try { await choose(false); } catch (err) { const p = n.querySelector('p'); if (p) { p.setAttribute('role', 'alert'); p.textContent = `${err.message || 'That didn’t work.'} Nothing more is recorded on this page; try again, or open Settings › Privacy.`; } b.disabled = false; if (had) b.focus(); return; }
      seen();
      if (had && (document.activeElement === document.body || !document.activeElement)) { const m = document.getElementById('main'); if (m) m.focus({ preventScroll: true }); }
    };
  };

  // ---- Stop for me, share again, Clear mine ----
  // Stop drops what waits here first, so nothing of it lands after the node took the Stop
  const choose = async (share) => {
    // (Stop holds on this page at once, even before or without the node's answer)
    const was = me; if (!share) { fresh(); if (me) me = { ...me, sharing: false }; }
    const t = token;
    const r = await request('PUT', '/usage/me', share ? { share: true, choice: was ? was.choice : 0 } : { share: false }, { token: t });
    const j = await answer(r);
    if (t !== token) return null; // another sign-in meanwhile: nothing of this answer is its
    if (!r.ok) { const e = new Error(r.status === 409 ? 'Usage insights changed since this page showed them. Look again.' : 'The node didn’t take that; try again.'); e.status = r.status; if (r.status === 409) await readMe(); throw e; }
    if (share) fresh();
    apply(j); return j;
  };
  const clearMine = async () => {
    fresh(); // what this page still had waits for nothing: Clear means none of it lands afterwards
    const r = await request('POST', '/usage/me/clear', undefined); const j = await answer(r);
    if (!r.ok) throw new Error((j && j.error) || 'That didn’t work; try again.');
    return (j && j.cleared) || 0;
  };
  const row = (name, help, ctl) => `<div class="set-row"><div class="set-label"><div class="set-name">${name}</div>${help ? `<p class="set-help">${help}</p>` : ''}</div>${ctl ? `<div class="set-ctl">${ctl}</div>` : ''}</div>`;
  const WHAT = 'Which page (never which album, search or person), whether pages and requests worked and roughly how long they took, which kind of button was used and how, a script error’s kind, phone, tablet or desktop, and whether you are the node’s owner or a listener. Never anything you type, names, links, ids, error messages or your key.';
  // Draws a Settings panel and keeps its status line (so it is announced) and the focus on its button
  const draw = (el, html) => {
    const msg = el.querySelector(':scope > .usage-msg') || Object.assign(document.createElement('p'), { className: 'usage-msg set-help' });
    msg.setAttribute('role', 'status');
    const focused = el.contains(document.activeElement) && document.activeElement.matches('[data-usage], [data-usage-node]');
    el.innerHTML = html; el.appendChild(msg);
    if (focused) { const b = el.querySelector('[data-usage]:not([disabled]), [data-usage-node]:not([disabled])'); if (b) b.focus(); }
  };
  // After a redraw the pressed button is gone: focus goes to the panel's new one, unless it moved on
  // (the same kind of button first: after Clear all that is Clear all, never the switch next to it)
  const refocus = (el, sel) => { const a = document.activeElement; if (el.isConnected && (!a || a === document.body || !a.isConnected)) { const b = (sel && el.querySelector(`${sel}:not([disabled])`)) || el.querySelector('[data-usage]:not([disabled]), [data-usage-node]:not([disabled])'); if (b) b.focus(); } };
  const say = (el, text, bad) => { const m = el.querySelector('.usage-msg'); if (m) { m.textContent = text; m.classList.toggle('warn-text', !!bad); } };
  const ask = (title, text, label) => (ctx && ctx.ask ? ctx.ask(title, text, label) : Promise.resolve(window.confirm(`${title}\n\n${text}`)));
  const renderPersonal = async (el) => {
    if (!el) return;
    el.classList.add('usage-slot');
    if (!ctx || !token) { el.innerHTML = row('Usage insights', 'Usage insights need a sign-in; nothing is recorded in this browser.', ''); return; }
    draw(el, row('Usage insights', 'Loading…', ''));
    const j = await readMe();
    if (!el.isConnected) return;
    if (!j) { draw(el, row('Usage insights', 'The node can’t say right now.', '')); return; }
    const L = j.limits || {};
    const keep = `Raw events are kept ${L.raw_retention_days || 14} days, daily counts ${L.aggregate_retention_days || 90} days. Only the node’s owner can read them, on this node; nothing leaves it automatically.`;
    const state = !j.node_on ? 'Off on this node: nothing is recorded.' : j.sharing ? 'On: this browser sends which pages and buttons you use.' : 'You stopped usage insights for yourself: nothing of yours is recorded, on any of your browsers.';
    const unsaved = j.saved ? '' : ' The node couldn’t save your choice yet; it applies now and is saved as soon as it can.';
    draw(el, row('Usage insights', esc(state + unsaved), j.node_on ? (j.sharing ? '<button class="btn" data-usage="stop">Stop for me</button>' : '<button class="btn" data-usage="share">Share again</button>') : '')
      + row('What is recorded', esc(WHAT), '')
      + row('Clear mine', esc(`Deletes the raw events still linked to you. ${keep} Clear does not stop, and Stop does not clear.`), '<button class="btn ghost" data-usage="clear">Clear mine</button>'));
    const btn = (k) => el.querySelector(`[data-usage="${k}"]`);
    const go = (k, fn) => { const b = btn(k); if (b) b.onclick = async () => { b.disabled = true; try { await fn(); } catch (e) { say(el, e.message, true); b.disabled = false; b.focus(); return; } refocus(el, `[data-usage="${{ stop: 'share', share: 'stop' }[k] || k}"]`); }; };
    go('stop', async () => { await choose(false); await renderPersonal(el); say(el, 'Stopped for you. Nothing more of yours is recorded.'); });
    go('share', async () => { await choose(true); await renderPersonal(el); say(el, 'Sharing again from your next page.'); });
    go('clear', async () => {
      if (!await ask('Clear your usage events?', 'The raw events still linked to you are deleted. Older ones are no longer linked to anyone and go when they expire.', 'Clear mine')) { btn('clear').disabled = false; return; }
      const n = await clearMine(); await renderPersonal(el); say(el, n ? `Cleared ${n} event${n === 1 ? '' : 's'}.` : 'Nothing linked to you was stored.');
    });
  };
  // The owner's switch (Settings › This node): off as shipped; on asks first
  const renderNode = async (el) => {
    if (!el) return;
    el.classList.add('usage-slot');
    if (!ctx || !token) { el.innerHTML = row('Usage insights', 'Usage insights need a sign-in.', ''); return; }
    let r; let s = null;
    try { r = await request('GET', '/node/usage'); s = await answer(r); } catch { r = null; }
    if (!el.isConnected) return; // (the node's switch doesn't depend on this page's own generation)
    if (!r || !r.ok || !s) { el.innerHTML = row('Usage insights', 'The node can’t say right now.', ''); return; }
    const cov = s.coverage && s.coverage.guests === 'not instrumented' ? ' Party guests are not covered yet.' : '';
    draw(el, row('Usage insights', esc(s.enabled ? `On: every signed-in browser, yours and listeners’, records which pages and buttons are used. Each person is told, and can stop for themselves.${cov}` : `Off: nothing is recorded on this node.${cov}`) + (s.saved ? '' : ' <span class="warn-text">Not saved yet: a restart before the node can save it would forget it.</span>'),
      `<button class="btn ${s.enabled ? '' : 'primary'}" data-usage-node="${s.enabled ? 'off' : 'on'}">${s.enabled ? 'Turn off' : 'Turn on'}</button>`)
      + row('Stored', esc(`${s.events || 0} events from ${s.aliases || 0} open browsers. Raw events go after ${(s.limits && s.limits.raw_retention_days) || 14} days, daily counts after ${(s.limits && s.limits.aggregate_retention_days) || 90}. The summary: odeiond usage summary on the node.`), '<button class="btn ghost" data-usage-node="clear">Clear all</button>'));
    const go = (k, fn) => { const b = el.querySelector(`[data-usage-node="${k}"]`); if (b) b.onclick = async () => { b.disabled = true; try { await fn(); } catch (e) { say(el, e.message, true); b.disabled = false; b.focus(); return; } refocus(el, `[data-usage-node="${{ on: 'off', off: 'on' }[k] || k}"]`); }; };
    const put = async (body) => { const x = await request('PUT', '/node/usage', body); const j = await answer(x); if (!x.ok) throw new Error(x.status === 409 ? 'Usage insights changed since this page showed them. Look again.' : (j && j.error) || 'That didn’t work; try again.'); };
    go('on', async () => {
      if (!await ask('Turn on usage insights?', 'Every signed-in browser on this node, yours and listeners’, then records which pages and buttons are used: never what anyone types, plays or names. Each person sees a notice and can stop for themselves.', 'Turn on')) { await renderNode(el); return; }
      await put({ enabled: true, generation: s.generation }); await readMe(); await renderNode(el);
    });
    go('off', async () => { fresh(); await put({ enabled: false }); await readMe(); await renderNode(el); say(el, 'Off: nothing more is recorded. What is stored stays until it expires or you clear it.'); });
    go('clear', async () => {
      if (!await ask('Clear all usage events?', 'Every stored event and daily count on this node is deleted. It doesn’t turn usage insights off.', 'Clear all')) { await renderNode(el); return; }
      fresh(); const x = await request('POST', '/node/usage/clear', undefined); if (!x.ok) throw new Error('That didn’t work; try again.');
      await renderNode(el); say(el, 'Cleared.');
    });
  };
  const on = () => live() && !excluded();

  window.PM_USAGE = Object.freeze({ mount, end, nav, loaded, apiDone, shortcut, renderPersonal, renderNode, on });
})();

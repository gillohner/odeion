  // ---- views ---------------------------------------------------------------
  const main = $('#main');
  // Rows drawn after the song started (a page opened, more of a list read in) get the playing
  // song's mark as they come, while it plays or is paused: asked once per change of the page
  new MutationObserver((recs) => { if (recs.some((r) => [...r.addedNodes].some((n) => n.nodeType === 1))) markPlaying(); }).observe(main, { childList: true, subtree: true }); // (elements only: a time rewritten as text, the dashboard's, adds no row)
  // Another page opened: the page on screen stays where it is until the new one (or its skeleton)
  // replaces it, and the new one starts at its top. (Put at the top at once, the old page showed
  // its own top for a moment first.) The reset comes with that first draw, before the page's own
  // code can scroll it (Zones to the zone in use, Identify to an album), and never after it.
  let topLater = null; // the navigation waiting for it, set by setView
  // (a redrawn page's place-keeping hold ends with it: ended before, the old page would jump up)
  const topNow = () => { if (topLater) { topLater = null; if (state.placeHold) state.placeHold(); mainTo(0); } };
  // Puts the page at a place at once: .main scrolls smoothly, so a page opened (or Back) would glide there
  const mainTo = (top) => { topLater = null; try { main.scrollTo({ top, behavior: 'instant' }); } catch { main.scrollTop = top; } };
  { const html = Object.getOwnPropertyDescriptor(Element.prototype, 'innerHTML');
    Object.defineProperty(main, 'innerHTML', { configurable: true, get() { return html.get.call(this); }, set(v) { html.set.call(this, v); topNow(); } }); }
  new MutationObserver(topNow).observe(main, { childList: true }); // (a page drawn another way)
  // (their own touch, wheel or key on the page comes before Zones showing the zone in use)
  ['pointerdown', 'wheel', 'touchstart', 'keydown'].forEach((e) => main.addEventListener(e, () => { state.zoneInView = false; }, { passive: true }));
  // Settings, one topic per tab (STEWARD-UX-SETTINGS.md)
  // Side tabs: a vertical tab list beside one panel per tab (Settings; Sources reuses it). On a
  // phone the list is a page of its own and each tab a page with Back. `tabs`: [[key, label, icon,
  // extra html after the label (a status dot, a switch)?]]; `ids`: panel ids that differ from
  // `<prefix>-<key>`. The view's address is `/<view>/<key>` (add the view to the router).
  //   const t = sideTabs({ view: 'sources', title: 'Sources', tabs, prefix: 'src' });
  //   main.innerHTML = t.page(t.panel('tidal', html) + …); t.wire(main);
  const sideTabs = ({ view, title, tabs: entries, prefix = view, ids = {}, remembered }) => {
    const phone = !!(window.matchMedia && matchMedia('(max-width: 720px)').matches); // the side tabs' CSS is by width
    // Entries are tabs ([id, label, icon, extra]) and group headers ({group, title, saved}) between them
    const tabs = entries.filter(Array.isArray);
    const known = (k) => tabs.some(([x]) => x === k);
    const asked = known(state.viewArg) ? state.viewArg : null;
    const active = asked || (phone ? null : (known(remembered) ? remembered : tabs[0][0]));
    const name = (k) => (tabs.find(([x]) => x === k) || [k, k])[1];
    const pid = (k) => ids[k] || `${prefix}-${k}`;
    const nav = () => `<nav class="set-nav" role="tablist" aria-orientation="vertical" aria-label="${esc(title)}">${(() => { let grp = null; return entries.map((e) => (!Array.isArray(e) ? (grp = `${prefix}grp-${e.group}`, `<div class="set-group-h" role="presentation" id="${grp}"><span class="set-group-t">${esc(e.title)}</span>${e.saved ? `<span class="set-group-s">${esc(e.saved)}</span>` : ''}</div>`) : (([k, l, ic, extra]) => `<button class="set-tab ${active === k ? 'on' : ''}" role="tab" id="${prefix}tab-${k}" aria-selected="${active === k}" aria-controls="${pid(k)}" data-stab="${k}"${grp ? ` aria-describedby="${grp}"` : ''}>${icon(ic)}<span>${esc(l)}</span>${extra || ''}${icon('chevron', 'ic chev')}</button>`)(e))).join(''); })()}</nav>`;
    const panel = (k, html) => `<section class="set-panel" id="${pid(k)}" data-panel="${k}" role="tabpanel" aria-labelledby="${prefix}tab-${k}" ${active === k ? '' : 'hidden'}>${phone ? `<button class="btn ghost set-back" data-tabs-back>${icon('back')} ${esc(title)}</button>` : ''}<h2 class="set-title">${esc(name(k))}</h2>${html}</section>`;
    const page = (panels) => (phone && active ? '' : pageHead(title)) + `<div class="settings ${active ? 'has-panel' : 'no-panel'}">${nav()}<div class="set-body-wrap">${panels}</div></div>`;
    // On a computer a tab switches in place (the address follows); on a phone it is its own page
    const wire = (main, onShow) => {
      const show = (k) => {
        if (phone) { setView(view, k); return; }
        state.viewArg = k;
        $$('.set-panel', main).forEach((pnl) => { pnl.hidden = pnl.dataset.panel !== k; });
        $$('.set-tab', main).forEach((t) => { const on = t.dataset.stab === k; t.classList.toggle('on', on); t.setAttribute('aria-selected', String(on)); });
        try { history.replaceState({ ...(history.state || {}), arg: k }, '', `/${view}/${k}`); } catch { /* sandboxed */ }
        if (onShow) onShow(k);
      };
      $$('[data-stab]', main).forEach((t) => t.onclick = () => show(t.dataset.stab));
      $('.set-nav', main).onkeydown = (e) => { if (!['ArrowDown', 'ArrowUp'].includes(e.key)) return; e.preventDefault(); const all = $$('.set-tab', main); const i = all.indexOf(document.activeElement); const n = all[(i + (e.key === 'ArrowDown' ? 1 : all.length - 1)) % all.length]; n.focus(); if (!phone) show(n.dataset.stab); };
      $$('[data-tabs-back]', main).forEach((b) => b.onclick = () => goBack(view));
      return show;
    };
    return { phone, active, name, nav, panel, page, wire };
  };
  // Settings in four groups, by where each setting is saved (STEWARD-SETTINGS-GROUPS.md, docs/ux/responsive.md):
  // You (your homeserver), This device, This node (admin), This homeserver (admin, with a bundled one)
  const SETTINGS_GROUPS = { you: 'Saved on your homeserver: follows you to every node and device', device: 'Only on this device', node: 'For everyone on this node', homeserver: 'The homeserver this node runs' };
  const settingsTabs = ({ saved = {}, bundled = false, tunnel = false, dac = null }) => [
    { group: 'you', title: 'You', saved: saved.you || SETTINGS_GROUPS.you },
    ['accounts', 'My accounts', 'artist'], ['home', 'Home', 'home'], ['quality', 'Playback quality', 'zone'], ['playfrom', 'Play from', 'play'], ['privacy', 'Privacy', 'eye-off'],
    { group: 'device', title: 'This device', saved: saved.device || SETTINGS_GROUPS.device },
    ['appearance', 'Display', 'sun'],
    // The app's own output with a USB DAC on it (`dac`: "This phone"), read-only for now
    ...(dac ? [['dac', dac, 'zone']] : []),
    { group: 'node', title: 'This node · admin', saved: saved.node || SETTINGS_GROUPS.node },
    ['apps', 'Service apps', 'settings'], ['people', 'People & access', 'artist'], ['playback', 'Zones and outputs', 'zone'],
    ['nodehome', 'Home for everyone', 'home'],
    ...(tunnel && !bundled ? [['remote', 'Remote access', 'external']] : []),
    ['discover', 'Discover', 'compass'], ['usage', 'Usage insights', 'eye'], ['backup', 'Backup', 'download'], ['about', 'About and updates', 'info'],
    ...(bundled ? [{ group: 'homeserver', title: 'This homeserver · admin', saved: saved.homeserver || SETTINGS_GROUPS.homeserver }, ['homeserver', 'Homeserver', 'archive'], ...(tunnel ? [['remote', 'Domain and tunnel', 'external']] : [])] : []),
  ];
  const SETTINGS_TABS = settingsTabs({ bundled: true, tunnel: true, dac: 'This phone' }).filter(Array.isArray); // every tab, for routes
  // ---- paths and back (navigation brief): real URLs, back/forward restore view and scroll ----
  const NAV_VIEWS = ['library', 'home', 'favourites', 'albums', 'artists', 'genres', 'tracks', 'playlists', 'radio', 'sources', 'queue', 'zones', 'settings', 'identify', 'add-music', 'downloads'];
  const VIEW_LABELS = { library: 'Your library', downloads: 'Downloads', 'add-music': 'Add music', home: 'Home', favourites: 'Bookmarks', albums: 'Albums', artists: 'Artists', genres: 'Genres', tracks: 'Songs', playlists: 'Collections', radio: 'Radio', sources: 'Sources', discover: 'Discover', queue: 'Queue', zones: 'Zones', settings: 'Settings', identify: 'Confirm matches', album: 'Album', artist: 'Artist', genre: 'Genre', playlist: 'Collection', shared: 'Shared collection' };
  const b64u = (x) => btoa(unescape(encodeURIComponent(x))).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
  const unb64u = (x) => decodeURIComponent(escape(atob(x.replace(/-/g, '+').replace(/_/g, '/'))));
  const itemSeg = (kind) => (kind === 'artist' ? 'artist' : kind === 'album' ? 'album' : 'song');
  // A page id is the canonical identity in short form: mb-<mbid>, isrc-<ISRC>, else u-<base64url of the link>.
  const idOf = (uri) => { let m = uri.match(/^https:\/\/musicbrainz\.org\/(recording|artist|release-group)\/([0-9a-f-]{36})$/); if (m) return 'mb-' + m[2]; m = uri.match(/^https:\/\/musicbrainz\.org\/isrc\/([A-Za-z0-9]{12})$/); if (m) return 'isrc-' + m[1].toUpperCase(); m = svcPage(uri); if (m) return m; return 'u-' + b64u(uri); };
  // A service's own song/album/artist page has a readable id (/song/tidal-1), as on the node
  const SVC_HOSTS = { 'tidal.com': 'tidal', 'listen.tidal.com': 'tidal', 'www.tidal.com': 'tidal', 'www.deezer.com': 'deezer', 'deezer.com': 'deezer', 'open.spotify.com': 'spotify' };
  const SVC_BASES = { tidal: 'https://tidal.com', deezer: 'https://www.deezer.com', spotify: 'https://open.spotify.com' };
  const svcPage = (uri) => { let u; try { u = new URL(uri); } catch { return null; } const yt = /^https:\/\/www\.youtube\.com\/watch\?v=([A-Za-z0-9_-]{11})$/.exec(uri); if (yt) return `youtube-${yt[1]}`; if (u.search || u.hash) return null; const svc = SVC_HOSTS[u.host]; if (!svc) return null; const p = u.pathname.split('/').filter(Boolean); const [k, id] = p.length === 2 ? p : p.length === 3 ? p.slice(1) : []; if (!['track', 'album', 'artist'].includes(k)) return null; if (svc === 'spotify' ? !/^[A-Za-z0-9]{22}$/.test(id) : !/^[0-9]{1,20}$/.test(id)) return null; return `${svc}-${id}`; };
  // The song page for a queue item: a library track, or a service track by its page.
  const npPageId = (uri) => { if (!uri) return null; let m = uri.match(/^track:(\d+)$/); if (m) return 'lib-' + m[1]; m = uri.match(/^(tidal|deezer|jamendo):track:([A-Za-z0-9]+)$/); if (m) return idOf({ tidal: `https://tidal.com/track/${m[2]}`, deezer: `https://www.deezer.com/track/${m[2]}`, jamendo: `https://www.jamendo.com/track/${m[2]}` }[m[1]]); if (/^https?:\/\//.test(uri)) return idOf(uri); return null; };
  const uriOf = (seg, id) => { if (id.startsWith('mb-')) return `https://musicbrainz.org/${seg === 'artist' ? 'artist' : seg === 'album' ? 'release-group' : 'recording'}/${id.slice(3)}`; if (id.startsWith('isrc-')) return `https://musicbrainz.org/isrc/${id.slice(5)}`; if (id.startsWith('u-')) { try { return unb64u(id.slice(2)); } catch { return null; } } const yv = /^youtube-([A-Za-z0-9_-]{11})$/.exec(id); if (yv) return `https://www.youtube.com/watch?v=${yv[1]}`; const sm = /^(tidal|deezer|spotify)-([A-Za-z0-9]+)$/.exec(id); if (sm) return `${SVC_BASES[sm[1]]}/${seg === 'artist' ? 'artist' : seg === 'album' ? 'album' : 'track'}/${sm[2]}`; return null; };
  const pathFor = (v, arg) => {
    if (v === 'item' && arg && arg.kind && arg.id) return `/${arg.kind}/${arg.id}`;
    if (v === 'releases' && arg && arg.id && arg.section) return `/artist/${arg.id}/releases/${arg.section}${arg.page > 1 ? `?page=${arg.page}` : ''}`;
    if (v === 'library' && (arg === 'node' || arg === 'import')) return `/library/${arg}`;
    if (v === 'home-section' && arg && arg.section) return `/home/${arg.section}${arg.window && arg.window !== '7d' ? `?window=${arg.window}` : ''}`;
    if (v === 'tag' && arg) return `/tag/${String(arg).split(',').map(encodeURIComponent).join(',')}`;
    if (v === 'playlist' && arg != null) return `/playlist/${encodeURIComponent(arg)}`;
    if (v === 'person' && arg) return `/person/${arg}`;
    if (v === 'smart' && arg) return `/smart/${encodeURIComponent(arg)}`;
    if (v === 'smart-edit') return null;
    if (v === 'dashboard') return `/now-playing${arg ? `?zone=${encodeURIComponent(arg)}` : ''}`;
    if (v === 'shared' && arg && arg.user) return `/shared/${arg.user}${arg.id ? '/' + arg.id : ''}`;
    if (v === 'genre' && arg != null) return `/genre/${encodeURIComponent(arg)}`;
    if (v === 'sources' && arg && arg.source) return `/sources/${encodeURIComponent(arg.source)}`;
    if (v === 'source-item' || v === 'source-artist') return null;
    if (v === 'discover-item' && arg && arg.uri) return `/${itemSeg(arg.kind)}/${idOf(arg.uri)}`;
    if (v === 'search') return arg && arg.q ? `/search?q=${encodeURIComponent(arg.q)}${arg.in && arg.in !== 'everywhere' ? `&in=${encodeURIComponent(arg.in)}` : ''}` : '/search';
    if (v === 'home') return '/';
    if (v === 'settings' && typeof arg === 'string' && SETTINGS_TABS.some(([k]) => k === arg)) return `/settings/${arg}`;
    return NAV_VIEWS.includes(v) ? '/' + v : null;
  };
  const routeFromPath = () => {
    // #settings/<tab> (a link from anywhere) and /settings/<tab> open that tab
    const h = /^#settings(?:\/([a-z]+))?$/.exec(location.hash || '');
    if (h) return { v: 'settings', arg: h[1] || null };
    // #sources/<service> likewise opens that source's tab
    const hs = /^#sources(?:\/([a-z0-9-]+))?$/.exec(location.hash || '');
    if (hs) return hs[1] ? { v: 'sources', arg: { source: hs[1] } } : { v: 'sources' };
    const p = location.pathname.replace(/\/+$/, '') || '/';
    if (p === '/library/node' || p === '/settings/library') return { v: 'library', arg: 'node' };
    if (p === '/library/import' || p === '/settings/import') return { v: 'library', arg: 'import' };
    const st = p.match(/^\/settings\/([a-z]+)$/);
    if (st && SETTINGS_TABS.some(([k]) => k === st[1])) return { v: 'settings', arg: st[1] };
    let m = p.match(/^\/artist\/([^/]+)\/releases\/(albums|eps-singles|other)$/);
    if (m && (m[1].startsWith('lib-') || uriOf('artist', m[1]))) { const pg = Number(new URLSearchParams(location.search).get('page')) || 1; return { v: 'releases', arg: pg > 1 ? { id: m[1], section: m[2], page: pg } : { id: m[1], section: m[2] } }; }
    m = p.match(/^\/(song|album|artist)\/([^/]+)$/);
    if (m && (m[2].startsWith('lib-') || uriOf(m[1], m[2]))) return { v: 'item', arg: { kind: m[1], id: m[2] } };
    m = p.match(/^\/home\/([a-z-]+)$/);
    if (m) { const w = new URLSearchParams(location.search).get('window'); return { v: 'home-section', arg: w ? { section: m[1], window: w } : { section: m[1] } }; }
    m = p.match(/^\/(?:tag|discover\/tag)\/([^/]+)$/);
    if (m) return { v: 'tag', arg: m[1].split(',').map(decodeURIComponent).join(',') };
    if (p === '/discover' || p.startsWith('/discover/')) return { v: 'home' };
    m = p.match(/^\/(playlist|genre|sources?)\/([^/]+)$/);
    if (m) { const a = decodeURIComponent(m[2]); return m[1] === 'playlist' ? { v: 'playlist', arg: a } : m[1] === 'genre' ? { v: 'genre', arg: /^\d+$/.test(a) ? Number(a) : a } : { v: 'sources', arg: { source: a } }; }
    if (p === '/now-playing') return { v: 'dashboard', arg: new URLSearchParams(location.search).get('zone') || null };
    m = p.match(/^\/smart\/([A-Za-z0-9_-]+)$/);
    if (m) return { v: 'smart', arg: m[1] };
    m = p.match(/^\/person\/([a-z0-9]{52})$/);
    if (m) return { v: 'person', arg: m[1] };
    m = p.match(/^\/shared\/([a-z0-9]{52})(?:\/([0-9A-Z]{13}))?$/);
    if (m) return { v: 'shared', arg: m[2] ? { user: m[1], id: m[2] } : { user: m[1] } };
    if (p === '/search') { const sp = new URLSearchParams(location.search); const q = sp.get('q') || ''; const sc = sp.get('in'); return { v: 'search', arg: q ? (sc ? { q, in: sc } : { q }) : null }; }
    const v = p.slice(1);
    return NAV_VIEWS.includes(v) ? { v } : null;
  };
  // An address that is no page (/settings/you, a mistyped /library/<tab>, an id the app can't read)
  // opens the nearest page, Settings or Your library, else Home, and says so (not Home without a word).
  const missedRoute = () => {
    const p = location.pathname.replace(/\/+$/, '') || '/';
    if (p === '/' || p === '/app') return null;
    let said = p; try { said = decodeURI(p); } catch { /* kept as typed */ }
    const top = p.split('/')[1].toLowerCase(); const ch = Array.from(said);
    // (the offline Android app routes before this: it opens Downloads, as before)
    return { v: top === 'settings' || top === 'library' ? top : 'home', said: ch.length > 60 ? ch.slice(0, 59).join('') + '…' : said };
  };
  // The same thing gets the same page (docs/ux/08-item-page.md): old detail views open the item page.
  const SERVICE_PAGE = { tidal: (k, id) => `https://tidal.com/${k}/${id}`, deezer: (k, id) => `https://www.deezer.com/${k}/${id}`, spotify: (k, id) => `https://open.spotify.com/${k}/${id}` };
  const toItem = (v, arg) => {
    if (v === 'discover-item' && arg && arg.uri) return { kind: itemSeg(arg.kind), id: idOf(arg.uri) };
    if (v === 'album' && typeof arg === 'number') return { kind: 'album', id: 'lib-' + arg };
    if (v === 'artist' && typeof arg === 'number') return { kind: 'artist', id: 'lib-' + arg };
    if ((v === 'source-item' || v === 'source-artist') && arg && SERVICE_PAGE[arg.source]) {
      const [k, id] = String(arg.id).includes(':') ? String(arg.id).split(':') : [v === 'source-artist' ? 'artist' : 'album', String(arg.id)];
      return { kind: k === 'track' ? 'song' : k, id: idOf(SERVICE_PAGE[arg.source](k, id)) };
    }
    return null;
  };
  // What the back arrow says on the next page.
  const hereLabel = () => state.view === 'item' ? (state.itemTitle || 'Back') : state.view === 'releases' ? (state.relTitle || 'Back') : state.view === 'tag' ? `#${String(state.viewArg).split(',').join(' #')}` : state.view === 'home-section' ? (HOME_TITLES[state.viewArg && state.viewArg.section] || 'Home') : state.view === 'playlist' || state.view === 'shared' ? (state.itemTitle || 'Collection') : state.view === 'search' ? (state.viewArg && state.viewArg.q ? `Results for “${state.viewArg.q}”` : 'Search') : (VIEW_LABELS[state.view] || 'Back');
  // The browser tab (and its Back list, and a screen reader on each page) names the page, not only
  // the app: "Journey · Odeion"; Home is the app's own name. The name is the one Back gives this
  // page, else its heading (a person, a smart playlist); a Back that names no page leaves the app's.
  const BRAND = document.title || 'Odeion';
  // (opening: the page isn't drawn, so its heading is still the last page's)
  const pageTitle = (opening) => {
    const v = state.view; let name = v === 'home' || v === 'dashboard' ? '' : v === 'search' && !(state.viewArg && state.viewArg.q) ? 'Search' : hereLabel();
    // (a page head's h2 by its own words, not its count: the smart playlist editor)
    if (name === 'Back' || v === 'genre') { const h = !opening && ($('#main h1') || $('#main .page-head h2')); const w = h && h.firstChild; name = w ? (w.nodeType === 3 ? w : h).textContent.trim() : ''; }
    const t = name ? `${name} · ${BRAND}` : BRAND; if (document.title !== t) document.title = t;
  };
  // History stores only what restores a view (search keeps its query, not its results).
  const histArg = (v, arg) => (v === 'search' ? (arg && arg.q ? (arg.in && arg.in !== 'everywhere' ? { q: arg.q, in: arg.in } : { q: arg.q }) : null) : (arg && typeof arg === 'object' && !Array.isArray(arg) ? JSON.parse(JSON.stringify(arg)) : arg ?? null));
  let navDepth = 0;
  let tabsBooted = false; // the first page (a reload keeps its history entry): its tab comes from the entry
  // For tests (tools/ui-item-pages.js): every view with a path round-trips through routeFromPath.
  window.pmRoutes = {
    check: () => {
      const cases = [['home'], ['albums'], ['artists'], ['tracks'], ['playlists'], ['queue'], ['zones'], ['settings'], ['favourites'], ['genres'], ['radio'], ['identify'],
        ['item', { kind: 'song', id: 'lib-3' }], ['item', { kind: 'album', id: 'lib-1' }], ['item', { kind: 'artist', id: 'mb-0383dadf-2a4e-4d10-a46a-e9e041da8eb3' }],
        ['item', { kind: 'album', id: idOf('https://tidal.com/album/199574241') }], ['item', { kind: 'song', id: 'isrc-GBUM71029604' }],
        ['releases', { id: 'tidal-4805908', section: 'albums' }], ['releases', { id: 'lib-3', section: 'eps-singles', page: 2 }], ['releases', { id: 'mb-0383dadf-2a4e-4d10-a46a-e9e041da8eb3', section: 'other' }],
        ['home-section', { section: 'trending-songs' }], ['home-section', { section: 'trending-albums', window: '30d' }], ['tag', 'hip-hop,german'],
        ['playlist', 'abc123'], ['shared', { user: 'zswn37ir9t8ge4r4posn4r7ow3eh384c8ud4kkxm585igt38x31o' }], ['shared', { user: 'zswn37ir9t8ge4r4posn4r7ow3eh384c8ud4kkxm585igt38x31o', id: '0035WKVKQDHSM' }], ['genre', 4], ['sources', { source: 'tidal' }], ['search', { q: 'queen' }], ['search', { q: 'queen', in: 'tidal' }]];
      const saved = location.pathname + location.search;
      const out = cases.map(([v, arg]) => {
        const path = pathFor(v, arg); if (!path) return `FAIL ${v}: no path`;
        history.replaceState(history.state, '', path); const r = routeFromPath() || { v: path === '/' ? 'home' : null };
        const same = r.v === v && JSON.stringify(r.arg ?? null) === JSON.stringify(arg ?? null);
        return `${same ? 'ok' : 'FAIL'} ${path}`;
      });
      history.replaceState(history.state, '', saved);
      return out;
    },
  };
  const setView = (v, arg, how = 'push') => {
    if (how !== 'pop') popTransient(true); // a page opened from a sheet takes Now playing's entry, not the sheet's
    if (popPending && how !== 'pop') return popPending.then(() => setView(v, arg, how)); // a menu's entry is still being popped
    cancelArtistReads(); // the page being left reads no more credits
    // (a page opened while a post's Back to the search under it is landing: that page stays, nothing decides there)
    if (searchOver && how !== 'pop') { searchOver = null; overDone(); }
    if (how !== 'pop') overUndo = null; // (an undo's Forward still to land goes nowhere now: no later pop is a step of it)
    { const t = toItem(v, arg); if (t) { v = 'item'; arg = t; } }
    if (v === 'discover') { if (arg && arg.tag) { v = 'tag'; arg = arg.tag; } else { v = 'home'; arg = undefined; } }
    // A section of its own starts with an empty search box (second review: "queen" stayed after leaving Search)
    const asked = v; // (usage insights: a page sent elsewhere below is a redirect)
    if (isListener() && LISTENER_VIEWS_OFF.has(v)) { v = 'albums'; arg = undefined; }
    if (isOffline() && !OFFLINE_VIEWS.has(v)) { v = 'downloads'; arg = undefined; }
    // (a drive's sign-in answer for a page that isn't Node library, or a listener's: the message, as before)
    if (driveBack && (isListener() || !(v === 'library' && arg === 'node'))) { const b = driveBack; driveBack = null; toast(b.text, b.kind); }
    { const sb = document.getElementById('search'); if (sb && NAV_VIEWS.includes(v) && document.activeElement !== sb) sb.value = ''; }
    // (on Back or Forward, history.state is already the entry being shown; on a reload it is the page's own)
    const firstPage = !tabsBooted; const fromEntry = how === 'pop' || !tabsBooted; tabsBooted = true;
    const tab = tabFor(v, fromEntry ? history.state && history.state.tab : state.tab, arg);
    if (how !== 'pop') {
      const path = pathFor(v, arg) || location.pathname + location.search;
      const cur = history.state || {};
      // (a page opened just before and not drawn yet was never scrolled: the old one's place isn't its)
      try { history.replaceState({ ...cur, scroll: topLater ? 0 : main.scrollTop }, '', location.pathname + location.search); } catch { /* sandboxed */ }
      // Changing a query or tab inside the same page replaces; opening another page pushes.
      // A page opened from a menu takes the menu's transient entry (still a new page for back).
      const viaTransient = !!cur.transient && how !== 'replace';
      const replace = how === 'replace' || viaTransient || (v === state.view && (v === 'search' || v === 'discover' || path === location.pathname + location.search)) || !state.view;
      // (fromPath: the page it was opened from, for a back that names its page: Sources' "‹ Sources")
      const entry = { transient: false, v, arg: histArg(v, arg), tab, backLabel: replace && !viaTransient ? cur.backLabel : hereLabel(), fromPath: replace && !viaTransient ? cur.fromPath : location.pathname + location.search, depth: replace && !viaTransient ? (cur.depth || 0) : (cur.depth || 0) + 1, scroll: 0 };
      try { if (replace) history.replaceState(entry, '', path); else history.pushState(entry, '', path); } catch { /* sandboxed */ }
      navDepth = entry.depth;
    }
    // Another page (not a new query on the search page): its skeleton shows if it is slow to draw
    const fresh = v !== 'search' && (v !== state.view || JSON.stringify(arg ?? null) !== JSON.stringify(state.viewArg ?? null));
    const sameReleases = v === 'releases' && state.view === 'releases' && !!arg && !!state.viewArg && arg.id === state.viewArg.id && arg.section === state.viewArg.section;
    const later = fresh || (v === 'search' && state.view !== 'search'); // (a new query on Search: at once)
    // Usage insights: which page, never its argument; the page a load starts on, a page this sent
    // elsewhere, Back or Forward, else opened (a tab or query inside a page too)
    state.view = v; state.viewArg = arg; state.tab = tab;
    // (a page not drawn yet has no name, not the last one's; the next page of the same releases keeps it)
    if (fresh) { state.itemTitle = null; if (!sameReleases) state.relTitle = null; }
    pageTitle(true);
    if (window.PM_USAGE) PM_USAGE.nav(v, firstPage ? 'initial' : v !== asked ? 'redirect' : how === 'pop' ? 'pop' : 'push');
    if (LIB_TOP.some(([k]) => k === v)) state.lastLib = v;
    markTabs();
    $$('.nav-item').forEach((b) => { const on = b.dataset.view === v || (v === 'item' && b.dataset.view === (state.itemSection || (arg && arg.kind === 'artist' ? 'artists' : 'albums'))) || (v === 'album' && b.dataset.view === 'albums') || ((v === 'artist' || v === 'releases') && b.dataset.view === 'artists') || (v === 'genre' && b.dataset.view === 'genres') || ((v === 'playlist' || v === 'shared') && b.dataset.view === 'playlists') || (v === 'source-item' && b.dataset.view === 'sources'); b.classList.toggle('on', on); if (on) b.setAttribute('aria-current', 'page'); else b.removeAttribute('aria-current'); });
    $('#scrim').hidden = true; closePop();
    // (the page takes Now playing's history entry. What had the focus when it opened is on the page this
    // one replaces: the focus goes to the mini player, not to a row about to go, which left it nowhere)
    if (!overlay.hidden) { npReturn = null; toggleNowPlaying(false, true); }
    // (a page opened is drawn from the start: redrawing, keepPlace)
    const top = later ? {} : null; if (top) topLater = top; else { mainTo(0); if (state.placeHold) state.placeHold(); }
    state.drawnPage = null; endSelect(); // (opening a page, the same one too, ends Select)
    state.zoneInView = v === 'zones' && how !== 'pop'; // Zones opened (not Back): show the zone in use
    // (a page that drew nothing, its code done or failed: at its top then, unless a newer page waits)
    // (not for a change inside the page, 'replace': a tag taken off, a tab, the next page of a list;
    // nor before the person has pressed or typed anything: a page the app opens by itself)
    return navigate(fresh, fresh && !firstPage && how !== 'replace' && personActed).finally(() => { if (top && topLater === top) topNow(); });
  };
  // (a post that is no playlist pasted over a search with a query: that search's entry takes its
  // search, as a new query there does, so Back and Escape go where Search was opened from: { q, in, depth
  // (of that search's entry), escaped (Escape pressed meanwhile) }. Ended undecided when the person opens
  // another page before it lands (theirs stays); no Back in 3 s: the box decides in the post's entry.
  let searchOver = null;
  // (the app's own Back asked for it, not landed yet: { at, done }; done: the person went elsewhere or
  // Back themselves meanwhile, or the box decided in the post's entry. Landing then, it is undone (Forward
  // again, as far as it went, nothing redrawn): what shows stays, no entry decides twice. 5 s after it was
  // asked (a done one, where the person's own Back is told from it: a minute), or once the app goes Back for
  // anything else, a Back is never taken for it. leaveSearch's, given up on by its timer: { at, done, left:
  // { depth, v } of the entry it drew that page in }. Where the person's
  // own Back can't be told from it (no Navigation API) it is forgotten instead: never undone)
  let overBack = null; let overUndo = null;
  // (the Back or Forward coming is the person's own, the browser's or the system's, not a history.back()
  // of the app's: the Navigation API says so where there is one)
  let popByPerson = false; const popWho = !!(window.navigation && navigation.addEventListener);
  if (popWho) navigation.addEventListener('navigate', (e) => { if (e.navigationType === 'traverse' && e.destination && e.destination.sameDocument) popByPerson = !!e.userInitiated; });
  const overDone = () => { if (overBack) { if (popWho) overBack.done = true; else overBack = null; } };
  // What is in the box then decides, in the entry shown (the search's, Back landed; else the post's own):
  // the old query, the text typed meanwhile (a link or key opens its page there), or, emptied, the
  // phone's empty Search tab (Escape: Home, as Escape there gives) and on desktop the page Search was
  // opened from (that entry kept as it was: Forward from there is the old query, not an empty Search)
  // (no Back came: the post's entry, which then takes Home in its place, as it would its search)
  const overDecide = (o) => {
    const typed = search.value.trim(); clearTimeout(searchTimer);
    const inPost = !!history.state && history.state.v === 'shared';
    if (typed === o.q) { search.value = o.q; startSearch(o.q, o.in); setView('search', { q: o.q, in: o.in }, 'replace'); }
    else if (typed) runSearch('replace', o.in);
    else if (shellMode() !== 'phone' && !inPost) leaveSearch(true);
    else if (o.escaped || shellMode() !== 'phone') setView('home', undefined, inPost ? 'replace' : 'push');
    else setView('search', undefined, 'replace');
  };
  window.addEventListener('popstate', async (e) => {
    const byPerson = popByPerson; popByPerson = false;
    // (a Forward undoing the app's late Back: nothing to show, whatever entry it lands on; one at a time, as
    // Chromium refuses a go(2) onto an entry pushed while that Back was on its way)
    // (only a pop one entry up from the last step, within 1 s, or the app's own within a minute: any other is the person's)
    // (the app's own Forward, told from the person's, landing later than that, as in a paused WebView: still a step)
    const wasSelf = popWasSelf; popWasSelf = false;
    if (overUndo) { const u = overUndo; overUndo = null; const d = (e.state && e.state.depth) || 0; const late = Date.now() - u.at; if ((late < 1000 || (!byPerson && late < 60000)) && d === u.from + 1) { popHandled = false; navDepth = d; if (u.steps > 1) { overUndo = { at: Date.now(), steps: u.steps - 1, from: d }; history.forward(); } return; } }
    // (a done one is kept until its own Back lands, where the Navigation API tells it from the person's: a minute at most)
    if (overBack && Date.now() - overBack.at >= (popWho && overBack.done ? 60000 : 5000)) overBack = null;
    // (that Back, landing while a menu is open on the page opened meanwhile, only closed the menu: undone all the
    // same, back to that page, its entry and depth; not yet done, it was spent closing a menu on the post's page)
    if (overBack && popHandled && !byPerson && !wasSelf) {
      const d = (e.state && e.state.depth) || 0; const now = history.state;
      if (overBack.done && navDepth > d && !(now && now.transient) && !(overBack.left && overBack.left.depth === navDepth)) {
        overBack = null; popHandled = false; overUndo = { at: Date.now(), steps: navDepth - d, from: d }; history.forward(); return;
      }
      if (overBack.left && overBack.left.depth === navDepth && !(now && now.transient)) navDepth = d; // (leaveSearch's, onto the page its copy shows)
      overBack = null;
    }
    if (popHandled) { popHandled = false; return; } // back only closed a menu or dialog
    if (e.state && e.state.transient) return; // forward onto a closed menu's entry: nothing to show
    const st = e.state || routeFromPath() || { v: 'home' };
    const was = navDepth; navDepth = st.depth || 0;
    // (the app's own Back for a post's search landing: once the person went elsewhere or the box decided,
    // undone, back to the entry it left: a page opened before it landed leaves it 2 entries down)
    const appBack = !byPerson && overBack; if (appBack) overBack = null;
    // (the Back leaveSearch's timer gave up on, landing at last where it was going while that page's copy still
    // shows: the Back it was, nothing to draw again; a page opened meanwhile, it is undone)
    if (appBack && appBack.left && appBack.left.depth === was) { if (state.view === appBack.left.v) return; }
    else if (appBack && appBack.done && was > navDepth) { overUndo = { at: Date.now(), steps: was - navDepth, from: navDepth }; navDepth = was; history.forward(); return; }
    // (Back from a post that is no playlist onto the search it was pasted over: that entry takes the post's search)
    // (the box edited before that Back landed: what is in it now takes that entry, never the old query put back over it;
    // a link or key typed there opens its page in that entry too. The person's own Back there does as the app's
    // would: that one, landing later, is undone)
    if (searchOver) {
      const o = searchOver; searchOver = null; overDone();
      if (st.v === 'search' && (st.depth || 0) === o.depth) { overDecide(o); return; }
    }
    let going;
    if (st.v === 'search' && st.arg && st.arg.q) { search.value = st.arg.q; startSearch(st.arg.q, st.arg.in || 'everywhere'); going = setView('search', st.arg, 'pop'); }
    else going = setView(st.v, st.arg, 'pop');
    // Back to where the page was scrolled, once all of it is drawn, and only if nothing newer was opened
    // meanwhile (this navigation's generation and code, taken before anything else can start)
    const gen = navGen; const drawnAll = navRendering;
    await going; await drawnAll;
    if (st.scroll && gen === navGen) mainTo(st.scroll);
  });
  // The back arrow: one step back when there is in-app history, else the parent page.
  const goBack = (fallback) => { if (navDepth > 0) { overBack = null; overUndo = null; history.back(); } else if (fallback && fallback.startsWith('item:')) { const [, kind, ...id] = fallback.split(':'); setView('item', { kind, id: id.join(':') }); } else setView(fallback || 'home'); };
  $$('.nav-item').forEach((b) => b.onclick = () => setView(b.dataset.view));
  // Back from TIDAL's or Spotify's sign-in page (its callback sends the browser to /?tidal=… or
  // /?spotify=…): once the first page is drawn (connect), the place the sign-in was started opens,
  // signed in, or with the toast and its Sign in button to try again (the sign-in says it "comes back
  // here"; Spotify's, and a failed one, used to stay on Home with only the toast). The owner gets the
  // service's Sources panel, a listener their Settings (My streaming accounts). Home stays under it, so
  // Back returns there, not to the service's sign-in page.
  let signinBack = null;
  // A failed sign-in's reason also stays on the service's panel (the toast is gone in 6 s), until the
  // next try there, a sign-in that worked, or the service shows signed in: { svc, why }
  state.signinFailed = null;
  // (Jamendo's callback already lands on its own panel, /source/jamendo?jamendo=…)
  { const p = new URLSearchParams(location.search); const j = p.get('jamendo'); if (j) { history.replaceState(null, '', location.pathname); state.signinFailed = j === 'signed_in' ? null : { svc: 'jamendo', why: p.get('message') || (j === 'error' ? 'Jamendo gave no reason' : j) }; setTimeout(() => { if (j === 'signed_in') toast('Jamendo signed in'); else toast('Jamendo sign-in failed: ' + (p.get('message') || j), 'error'); }, 300); } }
  { const p = new URLSearchParams(location.search); const t = p.get('tidal'); if (t) { history.replaceState(null, '', location.pathname); signinBack = t === 'signed_in' ? { svc: 'tidal', text: 'TIDAL signed in' } : { svc: 'tidal', text: 'TIDAL sign-in failed: ' + (p.get('detail') || t), kind: 'error', why: p.get('detail') || (t === 'error' ? 'TIDAL gave no reason' : t) }; } }
  { const p = new URLSearchParams(location.search); const t = p.get('spotify'); if (t === 'ok' || t === 'error') { history.replaceState(null, '', location.pathname); signinBack = t === 'ok' ? { svc: 'spotify', text: 'Spotify signed in' } : { svc: 'spotify', text: 'Spotify: ' + (p.get('message') || 'sign-in failed'), kind: 'error', why: p.get('message') || 'Spotify gave no reason' }; } }
  { const p = new URLSearchParams(location.search); const t = p.get('lastfm'); if (t) { history.replaceState(null, '', location.pathname); setTimeout(() => { if (t === 'signed_in') { toast(`Last.fm connected${p.get('user') ? ' · ' + p.get('user') : ''}`); setView('settings', 'accounts'); } else toast('Last.fm sign-in failed: ' + (p.get('detail') || t), 'error'); }, 300); } }


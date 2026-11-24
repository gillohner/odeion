  // The tabs' Library: the library page last open. The tabs' Search focuses the field, except in
  // the Android app: its WebView doesn't open the keyboard for a focus set by script, so the field
  // showed focused with no keyboard (owner's recording); there a tap on the field opens it.
  const libHome = () => state.lastLib || 'library';
  $$('[data-bnav]').forEach((b) => b.onclick = () => { const k = b.dataset.bnav; if (k === 'search') { if (!overlay.hidden) toggleNowPlaying(false); if (appPlatform() !== 'android') search.focus(); const q = search.value.trim(); setView('search', q ? { q } : undefined); return; } setView(k === 'library' ? libHome() : k); });
  // The account menu: who you are here, Settings and Log out (on every shell). A node that needs
  // no token has nothing to log out of.
  $('#me').onclick = async () => {
    const me = state.me || {}; const [who] = me.key ? await loadProfiles([me.key]) : [];
    const name = (who && who.name) || (isListener() ? 'Listener' : 'Owner');
    // What you are here, in words (UX sweep 11-01, A8): how this browser is logged in, and whether
    // the node has a Pubky identity at all; unlinked, the way to link one
    const how = !state.token ? 'no token needed on this node' : /^pmo_|^pml_/.test(state.token) ? 'logged in with Pubky Ring' : 'logged in with the node’s API token';
    const unlinked = !isListener() && !me.key;
    showMenu($('#me'), [{ title: `${name} · ${isListener() ? 'listening here' : 'this node’s owner'}` }, { label: how + (unlinked ? ' · not linked to Pubky' : ''), disabled: true }, ...(unlinked ? [{ label: 'Link a Pubky identity', icon: 'zone', run: () => setView('settings', 'accounts') }] : []), ...(window.PM_USAGE && PM_USAGE.on() ? [{ label: 'Usage insights on', icon: 'eye', run: () => setView('settings', isListener() ? undefined : 'privacy') }] : []), { label: 'Settings', icon: 'settings', run: () => setView('settings') }, ...(state.token ? [{ label: 'Log out', icon: 'logout', run: logOut }] : [])], { all: true });
  };
  // The account button shows your picture once the node knows who you are
  const drawMe = async () => {
    const b = $('#me'); const me = state.me || {}; if (!me.key) return;
    const [who] = await loadProfiles([me.key]); if (!who) return;
    b.innerHTML = face(who, 'md'); b.classList.add('has-face');
    b.setAttribute('aria-label', `${who.name}: settings and Log out`); b.title = `${who.name}: settings and Log out`;
  };
  $('#main').addEventListener('click', (e) => { const b = e.target.closest('[data-libnav]'); if (b) setView(b.dataset.libnav); });
  $('#jobchip').onclick = () => {
    const jp = jobPop(); if (!jp.hidden) { closeJobPop(); return; }
    closePop(); drawJobPop(); jp.hidden = false; $('#scrim').hidden = false; pushTransient();
  };
  $('#scrim').onclick = () => { closePop(); closeJobPop(); closeQPanel(); closeVolPop(); };
  // The menu button (desktop): collapses the sidebar to icons and back, remembered per browser.
  const shell = $('.shell');
  // The shell's arrangement; off the desktop the services warning moves from the sidebar to the top bar
  // The phone's tabs mark the current place; again when the shell changes
  const LIB_VIEWS = ['library', 'albums', 'artists', 'tracks', 'genres', 'genre', 'playlists', 'playlist', 'favourites', 'item', 'releases', 'album', 'artist', 'shared', 'sources', 'source-item', 'radio', 'person', 'identify', 'add-music'];
  // A page opened from a tab (an album from Home, an artist from Search) keeps that tab marked: Back
  // goes there. The tab rides in the page's history entry, so Back and Forward mark it again. A page
  // opened on its own (a link, a reload without the entry) is marked as before: Library's pages are Library's
  const DETAIL_VIEWS = new Set(['item', 'album', 'artist', 'releases', 'person', 'playlist', 'genre', 'tag', 'source-item', 'source-artist', 'shared', 'home-section', 'smart', 'smart-edit', 'lossless', 'lossy', 'dsd']);
  const TAB_ROOTS = { home: 'home', search: 'search', zones: 'zones' };
  // (Identify for some songs, opened from their album's tags, is a page of that album's tab)
  const tabFor = (v, from, arg) => TAB_ROOTS[v] || ((DETAIL_VIEWS.has(v) || (v === 'identify' && arg)) && from ? from : LIB_VIEWS.includes(v) ? 'library' : null);
  const markTabs = () => {
    // Sources is a Library section on phones
    $$('.bnav').forEach((b) => { const on = b.dataset.bnav === state.tab; b.classList.toggle('on', on); if (on) b.setAttribute('aria-current', 'page'); else b.removeAttribute('aria-current'); });
  };
  const applyShell = () => {
    const now = shellFor(window.innerWidth, layoutPref());
    const changed = document.documentElement.dataset.shell !== now;
    document.documentElement.dataset.shell = now;
    if (changed && state.view) markTabs();
    if (changed) $$('.srow [data-ssong], .srow [data-salbum], .srow:not(.card-row) .sub [data-artist-credit], .track .main-col .lnk').forEach((a) => { if (now === 'phone') a.tabIndex = -1; else a.removeAttribute('tabindex'); });
    if (changed) { const pb = $('#peoplebox'); if (pb && pb.reshell) pb.reshell(); }
    if (changed && state.view === 'zones') render(); // (its cards are laid out for the shell)
    if (changed && state.view === 'search') { const run = displayedSearch(); if (run) renderSearchBody(run); } // (Filters folds only on a phone)
    if (densePref()) document.documentElement.dataset.dense = '1'; else delete document.documentElement.dataset.dense;
    const up = $('#upstreams');
    if (up) { if (now === 'desktop') { if (up.parentElement !== $('#sidebar')) $('#sidebar').appendChild(up); } else if (up.parentElement !== $('.topbar')) $('.topbar').insertBefore(up, $('#me')); }
  };
  window.addEventListener('resize', () => { if (shellFor(window.innerWidth, layoutPref()) !== shellMode()) applyShell(); });
  // A scrolling chip row fades at its end only while more chips are off to the right (touch layouts)
  const CHIP_ROWS = '.libnav, .scopechips, .np-sheet-tabs, .qualchips, .azbar';
  const fadeRow = (el) => el.classList.toggle('more-right', el.scrollLeft + el.clientWidth < el.scrollWidth - 1);
  // A newly drawn Library row (or Albums' Quality row on a phone) starts around the current
  // section or the chosen quality. Remember its size so unrelated updates leave a person's swipe
  // alone; resizing keeps keyboard focus in view.
  const libraryNavSizes = new WeakMap();
  const revealChosenChip = (el) => {
    const width = el.clientWidth;
    if (!width) { libraryNavSizes.delete(el); return; }
    const size = `${width}:${el.scrollWidth}`;
    if (libraryNavSizes.get(el) === size) return;
    const focused = document.activeElement;
    const current = el.contains(focused) && focused.matches('[data-libnav], [data-focus]') ? focused : $('[aria-current="page"], [aria-pressed="true"]', el);
    if (!current) return;
    libraryNavSizes.set(el, size);
    const row = el.getBoundingClientRect(), chip = current.getBoundingClientRect();
    // Scroll only this row: scrollIntoView can also move the page vertically.
    el.scrollLeft += chip.left - row.left - (width - chip.width) / 2;
  };
  // The phone's pinned Library chips grow with larger text: the page keeps that much room above a
  // row it scrolls to (app.css's --libnav-h, 60 px until measured)
  // (measured when the page changes and when the chips change size alone, as with the phone's
  // text size changed while the page is open)
  const libnavSized = window.ResizeObserver ? new ResizeObserver((es) => es.forEach((e) => { if (e.target.isConnected) noteLibnavHeight(e.target); })) : null;
  let libnavWatched = null; // (the one row watched: a page's new chips take its place)
  const noteLibnavHeight = (el) => {
    if (libnavSized && libnavWatched !== el) { if (libnavWatched) libnavSized.unobserve(libnavWatched); libnavWatched = el; libnavSized.observe(el); }
    const h = el.offsetHeight; if (h && el.parentElement && el.parentElement.style.getPropertyValue('--libnav-h') !== `${h}px`) el.parentElement.style.setProperty('--libnav-h', `${h}px`);
  };
  let fadeQueued = false;
  const fadeRows = () => { if (fadeQueued) return; fadeQueued = true; requestAnimationFrame(() => { fadeQueued = false; $$(CHIP_ROWS).forEach((el) => { if (el.matches('.libnav, .qualchips')) revealChosenChip(el); if (el.matches('.main > .libnav')) noteLibnavHeight(el); fadeRow(el); }); }); };
  document.addEventListener('scroll', (e) => { if (e.target instanceof Element && e.target.matches(CHIP_ROWS)) fadeRow(e.target); }, true);
  window.addEventListener('resize', fadeRows);
  new MutationObserver(fadeRows).observe(document.body, { childList: true, subtree: true });
  applyShell();
  // Marked before the node answers: the page's own entry says its tab (a reload); Home is Home
  state.tab = (history.state && history.state.tab) || (location.pathname === '/' ? 'home' : undefined); markTabs();
  const mq = (q) => !!(window.matchMedia && window.matchMedia(q).matches);
  const narrow = () => mq('(max-width: 1100px)'); // the sidebar is icons there unless expanded
  const applySide = () => { let m = 'auto'; try { m = localStorage.getItem('pm.sidebar') || 'auto'; } catch { /* no storage */ } shell.classList.toggle('collapsed', m === 'icons'); shell.classList.toggle('expanded', m === 'names'); };
  applySide();
  // Icon-only nav items keep their names as tooltips and accessible labels.
  $$('.nav-item').forEach((b) => { const t = (b.querySelector('span') || {}).textContent; if (t) { b.title = t; b.setAttribute('aria-label', t); } });
  $('#menu').setAttribute('aria-expanded', 'false');
  $('#menu').onclick = () => {
    const collapsedNow = shell.classList.contains('collapsed') || (narrow() && !shell.classList.contains('expanded'));
    const next = collapsedNow ? (narrow() ? 'names' : 'auto') : 'icons';
    try { localStorage.setItem('pm.sidebar', next); } catch { /* no storage */ }
    applySide();
    $('#menu').setAttribute('aria-expanded', String(next !== 'icons' && !(narrow() && next === 'auto')));
  };
  // Menus, popovers and dialogs close on back and the page stays (STEWARD-UX-BRIEF-2 navigation).
  // Opening one adds a transient history entry; back pops it and closes them. Closing one any
  // other way pops it too, and opening a page from inside one replaces it (setView).
  const transients = []; // { open: () => bool, close: (fromBack) => void }
  let popHandled = false; let selfPop = false; let popPending = null; let popResolve = null;
  let popWasSelf = false; // (the pop just handled was the app's own, closing a menu: popTransient)
  // A layer over another (a phone's Now playing sheet) gets an entry of its own when it opens, on top of
  // the one under it. An entry pushed by back's own handler (no tap since) would make the WebView's
  // back skip the page's entry and leave the page: on Android to the app's connect page, which
  // connects again and registers the phone's player once more (TASK-android-back-reregister-1111).
  const pushTransient = (layer) => {
    // (a layer opened again while its last entry is still being popped: its own entry once that is done)
    if (layer && selfPop && popPending) { popPending.then(() => { if (transients.some((t) => t.layer && t.open())) pushTransient(true); }); return; }
    const s = history.state || {}; if (s.transient && (!layer || s.layer)) return; try { history.pushState({ ...s, transient: true, layer: !!(layer && s.transient) }, '', location.pathname + location.search); } catch { /* sandboxed */ }
  };
  // Popping is asynchronous: a page opened right after waits for it (setView), or the pop would undo it.
  // `layer`: only a layer's own entry (its layer closed, what is under it stays); otherwise once all are
  // closed, past a layer's entry too.
  const popTransient = (layer) => {
    const s = history.state || {};
    if (selfPop || !s.transient || (layer ? !s.layer : transients.some((t) => t.open()))) return;
    selfPop = true; popPending = new Promise((r) => { popResolve = r; setTimeout(() => { popPending = null; popResolve = null; r(); }, 600); });
    history.go(!layer && s.layer ? -2 : -1);
  };
  window.addEventListener('popstate', () => {
    if (selfPop) { selfPop = false; popHandled = true; popWasSelf = true; if (popResolve) { popResolve(); popResolve = null; } popPending = null; return; }
    const open = transients.filter((t) => t.open()); if (!open.length) return;
    // A layer over another (a phone's Now playing sheet) closes alone: the next back closes what is under it
    const top = open[open.length - 1];
    // (back from the layer's own entry lands on the entry of what is under it: nothing to push again)
    if (top.layer && open.length > 1 && open.slice(0, -1).every((t) => t === npTr)) { top.close(true); if (!(history.state && history.state.transient)) pushTransient(); popHandled = true; return; }
    open.forEach((t) => t.close(true)); popHandled = true;
    popTransient(); // (closed from a layer's entry: the entry of what was under it goes too)
  });


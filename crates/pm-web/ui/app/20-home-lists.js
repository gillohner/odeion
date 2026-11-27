  // ---- Home (STEWARD-UX-HOME.md): everything Pinakes knows about music, one section per call ----
  const HOME_TITLES = { 'trending-songs': 'Trending songs', 'trending-albums': 'Trending albums', 'trending-artists': 'Trending artists', following: 'From people you follow', tags: 'Tags people use for music', concerts: 'Concerts near you', podcasts: 'Podcasts', yours: 'Yours on Pubky', new: 'New on Pubky' };
  const WINDOWS = [['24h', 'Today'], ['7d', 'This week'], ['30d', 'This month']];
  const homeKind = (it) => (it.kind === 'track' ? 'song' : it.kind);
  // A Discover item as a shared song row or tile.
  const asRow = (it) => ({ title: itemName(it), artist: it.artist, album: it.versions ? `${it.versions + 1} versions` : null, id: idOf(it.page || it.uri), page: it.page || it.uri, uri: it.in_library ? null : it.playable, track_id: it.in_library ? it.in_library.id : null, image: it.image });
  const asTile = (it) => ({ id: idOf(it.page || it.uri), title: itemName(it), image: it.image, year: it.artist || '' });
  // A Home row that didn't answer has Try again: it asks for that row again in place, under a
  // placeholder of the row's own height (nothing below moves). A keyboard that pressed it stays
  // with the row: on its new Try again if it fails again, else on the row itself.
  const retryOn = (el, again) => { const b = $('[data-hretry]', el); if (b) b.onclick = () => again(document.activeElement === b); };
  const holdRow = (el) => { el.innerHTML = `<div class="skeleton" style="height:${Math.round(el.offsetHeight)}px"></div>`; };
  const focusRetry = (el, kept) => {
    if (!kept || !el || !el.isConnected) return;
    const b = $('[data-hretry]', el); if (b) { b.focus(); return; }
    const row = el.closest('.section') || el; row.tabIndex = -1; row.focus({ preventScroll: true });
  };
  const homeBody = (sec, name, full) => {
    // (the reason is the Pinakes client's own words, a request URL or an HTTP body, the same on
    // every row when Pubky is down: it is the line's tooltip, so each row is one line with Try again;
    // a rate limit says to wait, so Try again isn't pressed over and over)
    if (sec.error) return `<p class="muted small hfail"><span title="${esc(sec.error)}">${/rate.?limit|too many requests|answered 429/i.test(sec.error) ? 'Pubky is busy. Wait a minute.' : 'Pubky did not answer.'}</span> <button class="btn sm" type="button" data-hretry>Try again</button></p>`;
    if (sec.needs === 'pubky') return `<div class="card small">${esc(sec.note)} <button class="btn sm primary" data-home-signin>Sign in with Pubky Ring</button></div>`;
    if (sec.needs === 'location') return `<div class="card small">${esc(sec.note)} <span class="row" style="gap:6px;display:inline-flex"><input class="input" data-city placeholder="Your city" style="width:180px"><button class="btn sm primary" data-city-save>Save</button></span></div>`;
    if (sec.needs === 'contact') return `<p class="muted small">${esc(sec.note)}</p>`;
    if (name === 'tags') return sec.tags.length ? `<div class="chips">${sec.tags.map(([l, c]) => tagChip(l, { count: c, attrs: `data-tag-open="${esc(l)}"`, title: `${l}: ${c} ${c === 1 ? 'person' : 'people'}` })).join('')}</div>` : `<p class="muted small">${esc(sec.note || '')}</p>`;
    if (!sec.items.length) return `<p class="muted small">${esc(sec.note || 'Nothing yet.')}</p>`;
    const songs = sec.items.filter((it) => it.kind === 'track');
    const tiles = sec.items.filter((it) => it.kind === 'album' || it.kind === 'artist' || it.kind === 'podcast' || it.kind === 'playlist');
    const events = sec.items.filter((it) => it.kind === 'concert' || it.kind === 'other');
    return (songs.length ? songRows(songs.slice(0, full ? 60 : 6).map(asRow), { numbers: 'index' }) : '')
      + (tiles.length ? `<div data-hgrid>${itemTileGrid(tiles.slice(0, full ? 60 : 12).map(asTile), 'album')}</div>` : '')
      + (events.length ? `<div class="list">${events.slice(0, full ? 60 : 6).map((it, k) => `<div class="list-row clickable" data-hopen="${k}"><div class="grow"><div class="name">${esc(itemName(it))}</div><div class="sub">${esc([it.description, it.site].filter(Boolean).join(' · '))}</div></div>${icon('external')}</div>`).join('')}</div>` : '');
  };
  // Tiles of discovered items (an item page's "Tagged alike on Pubky"), bound to the list they were
  // drawn from: a tile opens the item's page (a song's for a track), a podcast or a playlist its site
  // (the node has item pages for songs, albums and artists only), ▶ plays it
  const SIMILAR_KINDS = ['track', 'album', 'artist', 'podcast', 'playlist'];
  const openDiscovered = (it) => { if (it.kind === 'podcast' || it.kind === 'playlist') return openWeb(it.page || it.uri); setView('item', { kind: homeKind(it), id: idOf(it.page || it.uri) }); };
  const bindDiscoverTiles = (box, items) => {
    if (!box) return;
    $$('[data-tile]', box).forEach((b) => b.onclick = (e) => { if (e.target.closest('[data-tileplay]')) return; const it = items[Number(b.dataset.tile)]; if (it) openDiscovered(it); });
    $$('[data-tileplay]', box).forEach((b) => b.onclick = (e) => { e.stopPropagation(); const it = items[Number(b.dataset.tileplay)]; if (it) return discoverPlay(it, null, b); });
  };
  const bindHome = (el, sec) => {
    const songs = sec.items.filter((it) => it.kind === 'track'); const tiles = sec.items.filter((it) => ['album', 'artist', 'podcast', 'playlist'].includes(it.kind)); const events = sec.items.filter((it) => it.kind === 'concert' || it.kind === 'other');
    if (songs.length) bindSongRows(el, songs.map(asRow), {});
    // Only this section's own tiles (an item page has other tile grids with their own handlers)
    $$('[data-hgrid] [data-tile]', el).forEach((b) => b.onclick = (e) => { if (e.target.closest('[data-tileplay]')) return; const it = tiles[Number(b.dataset.tile)]; if (it) openDiscovered(it); });
    $$('[data-hgrid] [data-tileplay]', el).forEach((b) => b.onclick = (e) => { e.stopPropagation(); const it = tiles[Number(b.dataset.tileplay)]; if (it) discoverPlay(it, null, b); });
    $$('[data-hopen]', el).forEach((r) => r.onclick = () => { const it = events[Number(r.dataset.hopen)]; openWeb(it.page || it.uri); });
    $$('[data-tag-open]', el).forEach((b) => b.onclick = () => setView('tag', b.dataset.tagOpen));
    $$('[data-home-signin]', el).forEach((b) => b.onclick = () => settingsAt('accounts', 'resign'));
    const cs = $('[data-city-save]', el); if (cs) cs.onclick = async () => { const city = $('[data-city]', el).value.trim(); if (!city) return; try { const l = await api('PUT', '/home/location', { city }); toast(`Concerts near ${l.label}`); render(); } catch (e) { toast(e.message, 'error'); } };
  };
  const homeCache = new Map(); // section|window → { at, sec }
  const loadHomeSection = async (name, window, limit) => {
    const key = `${name}|${window || ''}|${limit || ''}`; const c = homeCache.get(key);
    if (c && Date.now() - c.at < 60000) return c.sec;
    const sec = await api('GET', `/home/${name}?${new URLSearchParams({ ...(window ? { window } : {}), ...(limit ? { limit } : {}) })}`).catch((e) => ({ section: name, items: [], tags: [], error: e.message }));
    sec.items = sec.items || []; sec.tags = sec.tags || [];
    // "Needs a contact / a sign-in" answers are not kept: they change as soon as the owner acts
    if (!sec.error && !sec.needs) homeCache.set(key, { at: Date.now(), sec });
    return sec;
  };
  const mins = (ms) => { if (ms && ms < 60000) return `${Math.round(ms / 1000)} s`; const m = Math.round((ms || 0) / 60000); return m >= 60 ? `${Math.floor(m / 60)} h ${m % 60} min` : `${m} min`; };
  // `o.shuffle` (a page's Play or Shuffle): what is found loads with that mode, as the page's songs would;
  // `o.how` ('next', 'end'): where it goes, else in place of the queue. On a node zone the finding takes
  // the zone's turn first, so what it loads keeps its place among the zone's other changes (and its zone)
  const discoverPlay = async (it, v, anchor, o = {}) => {
    if (!onBrowser(o) && !o.turn) return withTurn(state.zone, o, (o2) => discoverPlay(it, v, anchor, o2));
    const replace = !o.how || o.how === 'replace'; const at = replace ? 'end' : o.how;
    if (v) return playSourceItems([{ uri: v.uri, title: it.title || '', artist: it.artist || '', album: '', duration_ms: null, image: it.image || null }], replace, at, o);
    let plan;
    try { plan = await api('GET', `/discover/play?uri=${encodeURIComponent(it.page || it.uri)}`, undefined, { signal: o.turn && o.turn.signal }); } catch (e) { return toast(e.message, 'error'); }
    const pf = plan.plays_from;
    if (pf && pf.service === 'library') {
      try { const tracks = await Promise.all(plan.track_ids.map((id) => api('GET', `/library/tracks/${id}`, undefined, { signal: o.turn && o.turn.signal }))); return playTracks(tracks, replace, at, o); } catch (e) { return toast(e.message, 'error'); }
    }
    if (pf && pf.mode === 'connect') { spotifyPlay(o.shuffle ? shuffleFrom([...plan.spotify_uris]) : plan.spotify_uris, anchor); return; }
    if (pf && plan.items.length) { toast(`Playing from ${pf.label}`); return playSourceItems(plan.items, replace, at, o); }
    const options = [{ title: 'Not on your services' }, ...(plan.connect || []).map((c) => ({ label: `Connect ${SERVICE_NAMES[c] || c}`, icon: 'zone', run: () => setView('sources', { source: c }) }))];
    if (plan.preview) options.push({ label: `Play a 30 s preview (${SERVICE_NAMES[plan.preview.service] || plan.preview.service})`, icon: 'play', run: pressFor(() => playSourceItems([{ uri: plan.preview.id.startsWith('track:') ? `${plan.preview.service}:${plan.preview.id}` : plan.preview.id, title: it.title || '', artist: it.artist || '', album: `30 s preview · ${SERVICE_NAMES[plan.preview.service] || plan.preview.service}`, duration_ms: 30000, image: it.image || null }], false, 'now'), anchor) });
    if (anchor) showMenu(anchor, options); else toast('Not on your services: connect one in Sources', 'error');
  };
  const bindDiscoverRows = (items) => {
    $$('[data-ditem]', main).forEach((row) => row.onclick = (e) => { if (e.target.closest('[data-dplay],[data-dmore]')) return; { const it = items[Number(row.dataset.ditem)]; setView('discover-item', { uri: it.uri, kind: it.kind }); } });
    $$('[data-dplay]', main).forEach((b) => b.onclick = (e) => { e.stopPropagation(); discoverPlay(items[Number(b.dataset.dplay)], null, b); });
    // (a link: Tab reaches it, Ctrl+click opens Sources in a new tab; either way not the row's page too)
    $$('[data-view="sources"]', main).forEach((a) => a.onclick = (e) => { e.stopPropagation(); inAppClick(() => openConnect(a))(e); });
    $$('[data-dmore]', main).forEach((b) => b.onclick = (e) => { e.stopPropagation(); const it = items[Number(b.dataset.dmore)]; showMenu(b, [{ title: itemName(it) }, { label: 'Details, tags and notes', icon: 'compass', run: () => setView('discover-item', { uri: it.uri, kind: it.kind }) }, ...(it.in_library ? [{ label: 'Play from library', icon: 'play', run: () => playTracks([it.in_library], true) }, { label: 'Open album', icon: 'disc', run: () => it.in_library.album && it.in_library.album.id ? setView('album', { id: it.in_library.album.id }) : toast('No album for this song') }] : []), ...(it.playable ? [{ label: `Play on ${it.site || 'service'}`, icon: 'zone', run: () => discoverPlay(it) }] : []), ...it.also_on.filter((v) => v.uri).map((v) => ({ label: `Play on ${v.source.toUpperCase()}`, icon: 'zone', run: () => discoverPlay(it, v) })), { label: `Open ${it.site ? 'in ' + it.site : 'link'}`, icon: 'external', run: () => openWeb(it.uri) }]); });
  };
  // (action: { view, label } opens a page; { id, label } is a button the page binds itself)
  // (songs: the button depends on whether the library has songs; the box notes the count it was drawn
  // with, songsChanged)
  // A read that failed, said the same way wherever it happens: what went wrong, announced, with
  // Try again in its place. Never the page's "nothing here" words: nothing was read, so nothing is
  // known to be missing. (bound by the page's draw, renderPage, to drawing it again; `id`: a
  // caller's own handle on the button)
  const failedHtml = (e, id) => `<div class="notice warn" role="alert" data-failed>${esc((e && e.message) || String(e))} <button class="btn sm" type="button" data-retry${id ? ` id="${id}"` : ''}>Try again</button></div>`;
  const emptyState = (iconName, title, text, action, songs) => `<div class="empty"${songs ? ` data-songs="${hasSongs()}"` : ''}>${icon(iconName)}<h3>${esc(title)}</h3><p>${esc(text)}</p>${action ? `<button class="btn primary" type="button" ${action.id ? `id="${action.id}"` : `data-empty-action="${action.view}"`}>${esc(action.label)}</button>` : ''}</div>`;
  // An empty library's Songs, Albums and Artists: the owner's way in is Add music (folders, shares
  // and drives are on Your library › Node library now; the old "in Settings (Library folders)" led
  // nowhere), a listener can't add to the node's library, so it just says where the music comes from
  // (Artists is empty only with no songs at all: songs without an artist tag are its "Unknown
  // artist" row, GET /library/artists)
  // Nothing hearted yet: Songs, where the hearts are (an empty library: the owner's Add music; a
  // listener has nowhere to go yet)
  // What the empty Bookmarks page says: how to heart something; a listener on a node with no
  // songs yet is told first that there is nothing to heart until the owner adds music. Where they
  // are kept only when they are on a homeserver: the listener's own, or the owner's signed in with
  // Pubky (before, everyone not on one was told "Signed in with Pubky, your bookmarks are also kept
  // in your own Pubky storage", also when not signed in; Settings says how to sign in)
  const favHowTo = (hs) => {
    const noSongs = !!(state.info && state.info.library) && !(state.info.library.tracks > 0);
    // (not while the homeserver no longer lets the node in, nor while the owner must log in again)
    const kept = hs ? !hs.lost : !isListener() && !!state.pubkyUser && !(state.pubkyStatus && state.pubkyStatus.needs_resign);
    if (noSongs && !isOwner()) return `Once the node’s owner adds music, press the heart on a song, album or artist to keep it here${kept ? ', on your own homeserver' : ''}.`;
    return `Press the heart on a song, album or artist to keep it here${kept ? ', on your own homeserver' : ''}.`;
  };
  const favWayOn = () => (hasSongs() ? { view: 'tracks', label: 'Open Songs' } : isOwner() ? { view: 'add-music', label: 'Add music' } : undefined);
  // Songs' and Albums' heads carry the owner's Add music; on a page whose empty state has the same
  // button (no music at all) the head leaves it out (one Add music on the page). Albums with songs but
  // no album tags keeps it: its empty state offers Open Songs
  const headAddMusic = (n) => (phoneMode() ? '' : isOwner() && n ? `<button class="btn iconic" data-addmusic aria-label="Add music">${icon('plus')}<span class="btn-label">Add music</span></button>` : '');
  const noMusicYet = (what) => (phoneMode() ? [`${what} show up here once you add songs to this phone${offers('add-songs') ? ' with Add songs' : ''}.`] : isOwner() ? ['Add the folder with your music, or upload files from this device.', { view: 'add-music', label: 'Add music' }] : [`${what} show up here once the node’s owner adds music.`]);
  const LAYOUTS = [['auto', 'Automatic', 'Phone under 600 px wide, desktop from there (tablets too)'], ['phone', 'Phone', 'Tabs and a mini player, at any width'], ['desktop', 'Desktop', 'The sidebar and the full player bar, also in windows from 480 px']];
  const layoutGroup = (group, row) => group('Layout', row('Arrangement', 'Kept on this device.', `<div class="chips layoutswitch" role="radiogroup" aria-label="Layout">${LAYOUTS.map(([v, l, d]) => `<button class="chip scope ${layoutPref() === v ? 'on' : ''}" role="radio" aria-checked="${layoutPref() === v}" data-layout-pick="${v}" title="${d}">${l}</button>`).join('')}</div>`)
    + row('Dense lists', 'More songs and covers on a tablet or computer screen.', `<input type="checkbox" class="switch" role="switch" id="densepick" ${densePref() ? 'checked' : ''} aria-label="Dense lists">`));
  const colourGroup = (group, row) => group('Colour', row('Colour from where it plays', 'The player takes a tint from the service: TIDAL, Qobuz, Spotify…; your own files keep ours.', `<input type="checkbox" class="switch" role="switch" data-colour-pick="pm.srcColour" ${colourOn('pm.srcColour') ? 'checked' : ''} aria-label="Colour from where it plays">`)
    + row('Colour from the cover', 'Now playing takes a soft gradient from the album art.', `<input type="checkbox" class="switch" role="switch" data-colour-pick="pm.artColour" ${colourOn('pm.artColour') ? 'checked' : ''} aria-label="Colour from the cover">`));
  const bindLayoutPick = () => {
    $$('[data-colour-pick]', main).forEach((c) => c.onchange = () => {
      try { if (c.checked) localStorage.removeItem(c.dataset.colourPick); else localStorage.setItem(c.dataset.colourPick, 'off'); } catch { /* no storage */ }
      const z = currentZone(); lastSrc = undefined; setSourceColour(z && z.now_playing); setArtTint();
    });
    $$('[data-layout-pick]', main).forEach((b) => b.onclick = () => {
      try { if (b.dataset.layoutPick === 'auto') localStorage.removeItem('pm.layout'); else localStorage.setItem('pm.layout', b.dataset.layoutPick); } catch { /* no storage */ }
      $$('[data-layout-pick]', main).forEach((x) => { const on = x === b; x.classList.toggle('on', on); x.setAttribute('aria-checked', String(on)); });
      applyShell();
    });
    const dp = $('#densepick', main);
    if (dp) dp.onchange = () => { try { if (dp.checked) localStorage.setItem('pm.dense', '1'); else localStorage.removeItem('pm.dense'); } catch { /* no storage */ } applyShell(); };
  };
  // The library's sections as a row of chips above the page title; shown on phones (the desktop has the sidebar)
  const LIB_TOP = [['library', 'Your library'], ['albums', 'Albums'], ['artists', 'Artists'], ['tracks', 'Songs'], ['playlists', 'Collections'], ['favourites', 'Bookmarks'], ['genres', 'Genres'], ['radio', 'Radio'], ['sources', 'Sources']];
  const libNav = () => {
    if (!LIB_TOP.some(([k]) => k === state.view)) return '';
    const off = new Set((state.sources || []).filter((x) => x.enabled === false).map((x) => x.id));
    const items = LIB_TOP.filter(([k]) => !(isListener() && LISTENER_VIEWS_OFF.has(k)) && !off.has(k));
    return `<nav class="libnav chips" aria-label="Library">${items.map(([k, l]) => `<button class="chip ${k === state.view ? 'on' : ''}" data-libnav="${k}" ${k === state.view ? 'aria-current="page"' : ''}>${l}</button>`).join('')}</nav>`;
  };
  // A count of 0 is not shown: the page's empty state says there is nothing ("Genres 0" above
  // "No genres yet" said it twice). A text count ("0 of 12", a zone's name) is shown as it is.
  const pageHead = (title, count, actions = '') => libNav() + `<div class="page-head"><h2>${esc(title)}${count != null && count !== 0 ? `<span class="count">${esc(fmtCount(count))}</span>` : ''}</h2><div class="actions">${actions}</div></div>`;
  // One way back (08-item-page.md): history, labelled with the previous page; on a deep link the page's section.
  const backBtn = (view, label) => `<button class="back" data-back="${view}">${icon('back')} <span class="back-l">${esc((navDepth > 0 && history.state && history.state.backLabel) || label)}</span></button>`;
  // A name cut with … shows whole on hover, measured then (the window may have changed); a name that fits has no tooltip.
  // Back's name, and the item page's own name at the end of its breadcrumb (cut at 32ch).
  document.addEventListener('pointerover', (e) => { const b = e.target.closest && e.target.closest('.back, .crumbs li[aria-current]'); const l = b && (b.matches('.back') ? b.querySelector('.back-l') : b); if (!l) return; if (l.scrollWidth > l.clientWidth) b.title = l.textContent; else b.removeAttribute('title'); });
  const loading = () => { if (state.placeHold) state.placeHold(); main.innerHTML = `<div class="skeleton" style="height:28px;width:220px"></div><div class="sk-grid">${'<div class="skeleton"></div>'.repeat(8)}</div>`; };
  // A page drawn again where it stands (its songs still coming, a heart, other sources found) stays
  // on screen until it is drawn anew: no skeleton, so it doesn't jump to the top, and the control
  // that had focus has it again. Opening a page (a link, Back) or Retry draws it from the start.
  // state.drawnPage: the page on screen, set when it is drawn; setView clears it.
  const redrawing = (key) => state.drawnPage === key;
  // Before the page's HTML is replaced: where it is scrolled and what had focus in it (its id, else
  // its data attributes and address and its place among its likes). The returned call puts both
  // back on the new HTML; focus only when nothing else took it meanwhile. The place is read
  // before an earlier hold ends: its spacer going would cut a place at the foot short.
  // Wholly within the page's own scrolling part, not under its sticky head (its scroll padding)
  const inSight = (el) => {
    const r = el.getBoundingClientRect(); const m = main.getBoundingClientRect(); const cs = getComputedStyle(main);
    return r.top >= m.top + (parseFloat(cs.scrollPaddingTop) || 0) - 1 && r.bottom <= m.bottom - (parseFloat(cs.scrollPaddingBottom) || 0) + 1;
  };
  // `own`: the page's own draw puts the focus back (a collection's song control: on that song, not
  // its place in the list, plFocusIn); here only the scroll and the height are kept
  const keepPlace = (own) => {
    const a = document.activeElement; const top = main.scrollTop; const tall = main.scrollHeight;
    // (focus an earlier hold still waits to put back, its part not drawn yet, is wanted here too)
    const was = state.placeHold && state.placeHold.wants ? state.placeHold.wants() : null;
    // (a page drawn again while its hold is on holds on from when that hold began: 6 s after each
    // redraw, so a part slower than the next ask still comes back to the place, but 20 s at most
    // in all, so a page that keeps asking (every 3-4 s while a service reads) doesn't hold on)
    const began = state.placeHold && state.placeHold.began;
    if (state.placeHold) state.placeHold();
    const at = (s) => { try { return $$(s, main); } catch { return []; } };
    let inside = !own && !!a && a !== main && main.contains(a); let sel = ''; let i = -1; let alt = null;
    // (a heading that has the focus only because the page was opened (focusPage: data-navfocus, a
    // tabindex until it is left) is the same place on the new page: found without that mark, and
    // made focusable the same way there)
    let nav = inside && a.hasAttribute('data-navfocus');
    if (inside) {
      // Looked for within its part (the nearest box with an id), so a tile in Similar isn't taken
      // for the same tile in a release section
      const box = a.id ? null : a.parentElement && a.parentElement.closest('[id]');
      const within = box && box !== main && main.contains(box) ? `#${CSS.escape(box.id)} ` : '';
      // (an empty state's button is the same place when the new one names another way in: Add
      // music became Open Songs once the first songs came)
      sel = a.matches('.empty > .btn') ? '.empty > .btn' : a.id ? `#${CSS.escape(a.id)}` : within + a.tagName.toLowerCase() + [...a.attributes].filter((x) => (x.name.startsWith('data-') && x.name !== 'data-uri' && x.name !== 'data-navfocus') || x.name === 'href').map((x) => `[${x.name}="${CSS.escape(x.value)}"]`).join('');
      i = at(sel).indexOf(a);
      // A row that names its song (data-uri; Favourites' other services) is found by that song, so
      // a favourite gone or added above it doesn't move the focus onto another one. When that song
      // itself went, the one now in its place takes it, or the last one when it was the last
      const row = a.id ? null : a.closest('[data-uri]');
      if (row && main.contains(row)) {
        const rbox = row.parentElement && row.parentElement.closest('[id]');
        const rin = rbox && rbox !== main && main.contains(rbox) ? `#${CSS.escape(rbox.id)} ` : '';
        // (within the row: its kind of control, not its place in the list: data-dplay, not data-dplay="2")
        const part = row === a ? '' : ' ' + a.tagName.toLowerCase() + [...a.attributes].filter((x) => x.name.startsWith('data-') || x.name === 'href').map((x) => (x.name === 'href' ? `[href="${CSS.escape(x.value)}"]` : `[${x.name}]`)).join('');
        alt = { sel, i, last: `${rin}[data-uri]${part}` }; sel = `${rin}[data-uri="${CSS.escape(row.dataset.uri)}"]${part}`; i = at(sel).indexOf(a);
        // (wholly in sight before: still in sight after, when the song moved far down or up the list;
        // one half off the edge stays where it is)
        alt.seen = inSight(a);
      }
    } else if (was && (!a || a === document.body || a === main)) { ({ sel, i, alt, nav } = was); inside = true; }
    return () => {
      // The parts drawn after the page (Similar, concerts, comments) come back a moment later: until
      // they do (6 s at most, 20 s across redraws; opening a page ends it) the page keeps its
      // height below, so a place down there isn't cut short, and focus that was in one of them goes
      // back when it is drawn
      let extra = 0;
      const fit = () => { extra = Math.max(0, tall - (main.scrollHeight - extra)); if (extra) { main.style.setProperty('--hold', `${extra}px`); main.dataset.hold = ''; } else { main.style.removeProperty('--hold'); delete main.dataset.hold; } return extra; };
      // Done once focus is back, or something else took it meanwhile. (A clickable row drawn just
      // now isn't keyboard-reachable until the next frame: it is made so here, or focus() wouldn't
      // take and the focus would fall to the page: Artists, Favourites' other services.)
      const refocus = () => {
        const now = document.activeElement; if (now && now !== document.body && now !== main) return true;
        const b = at(sel)[i] || (alt && (at(alt.sel)[alt.i] || at(alt.last).pop())); if (!b || b.disabled) return false;
        if (b.matches(UNTAGGED)) keyable(b);
        const mark = nav && !b.hasAttribute('tabindex');
        if (mark) { b.setAttribute('tabindex', '-1'); b.dataset.navfocus = ''; }
        b.focus({ preventScroll: true });
        if (document.activeElement !== b) { if (mark) { b.removeAttribute('tabindex'); delete b.dataset.navfocus; } return false; }
        if (mark) b.addEventListener('blur', () => { if (b.dataset.navfocus != null) { b.removeAttribute('tabindex'); delete b.dataset.navfocus; } }, { once: true });
        if (alt && alt.seen && !inSight(b)) b.scrollIntoView({ block: 'nearest', behavior: 'instant' });
        return true;
      };
      let held = fit();
      if (main.scrollTop !== top) { try { main.scrollTo({ top, behavior: 'instant' }); } catch { main.scrollTop = top; } }
      let want = inside && !refocus();
      if (held || want) {
        // (a click or key of theirs meanwhile is where they are now: focus isn't taken back)
        const mine = () => { want = false; };
        const acts = ['pointerdown', 'keydown', 'wheel', 'touchstart'];
        acts.forEach((e) => document.addEventListener(e, mine, { capture: true, passive: true }));
        const mo = new MutationObserver(() => {
          if (held) {
            // (a part that shrank first can have cut the place short: back to it, unless they scrolled up)
            const cut = main.scrollTop < top && main.scrollTop >= main.scrollHeight - main.clientHeight - 1;
            held = fit(); if (cut) { try { main.scrollTo({ top, behavior: 'instant' }); } catch { main.scrollTop = top; } }
          }
          if (want) want = !refocus();
          if (!held && !want) state.placeHold();
        });
        mo.observe(main, { childList: true, subtree: true });
        const since = began || Date.now();
        const t = setTimeout(() => state.placeHold(), Math.max(0, Math.min(Date.now() + 6000, since + 20000) - Date.now()));
        state.placeHold = () => { mo.disconnect(); clearTimeout(t); acts.forEach((e) => document.removeEventListener(e, mine, true)); main.style.removeProperty('--hold'); delete main.dataset.hold; state.placeHold = null; };
        state.placeHold.wants = () => (want ? { sel, i, alt, nav } : null); state.placeHold.began = since;
      }
    };
  };
  // Albums and Artists drawn again where they stand (the library changed: a scan, an identify run,
  // an edit) keep the page on screen until the new one is drawn: what is typed in the filter (read
  // as it is when replaced, so typing meanwhile counts) and its caret, the scroll and the focus.
  // A chosen A–Z letter is chosen again. `n`: how many rows were drawn, drawn at once again (the
  // first part only) so the place is there; the list's own height holds it, so no spacer stays
  // below (keepPlace's hold) when it got shorter. `none`: the no-match line as it was, so it isn't
  // said again when nothing changed. Opening the page, a sort, a chip or Favourites draws it from
  // the start (renderSortedList keeps the filter).
  const keepList = (key, again, rows) => {
    if (!again || !redrawing(key)) return null;
    const old = $('#filter', main); const letter = $('[data-az].on', main)?.dataset.az;
    const kept = { n: $$(rows, main).length, none: $('.filter-none', main)?.textContent || '' }; const place = keepPlace();
    kept.done = () => {
      const f = $('#filter', main);
      // (a filter that is off, nothing to filter just now, keeps its text for when it is on again)
      if (old && old.value && f) { f.value = old.value; if (!f.disabled) f.oninput?.(); }
      const az = letter && $$('[data-az]', main).find((b) => b.dataset.az === letter); if (az && !az.disabled) az.click();
      place(); if (state.placeHold) state.placeHold(); kept.n = 0;
      if (old && f && f.value && document.activeElement === f) { try { f.setSelectionRange(old.selectionStart, old.selectionEnd, old.selectionDirection || 'none'); } catch { /* no caret */ } }
    };
    return kept;
  };

  // Each draw of Albums and Artists is numbered: one that answers after a newer one was drawn (two
  // library changes, or a change and a sort, answered out of order) is dropped before it changes
  // anything, an error of its own too, so the newer list stays. (Only once drawn: changes coming
  // faster than a list loads still draw each answer, the page is never left on its placeholder.)
  const pageGen = { albums: 0, artists: 0 }; const pageShown = { albums: 0, artists: 0 };
  const listStale = (v, mine) => state.view !== v || mine < pageShown[v];
  // Each draw of a playlist is numbered as well: only the latest draw of the playlist at the address
  // draws anything, its error too. An older draw that answers late (a library change redrew it, or
  // another playlist was opened while it waited) would show a list the server no longer has, and
  // every Remove on that page goes by a song's place in the list.
  let plGen = 0;
  // The latest favourites load, so a page that lists favourites can wait for it on a cold start
  let favLoad = null; let favRedraw = null; let favRetryFocus = false;
  // Songs: each draw's number and the newest one shown (an older draw's answer is not put in over
  // a newer one's), and the read under way while the page is drawn again where it stands
  let songsDraw = 0; let songsShown = 0; let songsReading = null;
  // Only the latest load's answer counts: on a cold start a slow first answer that lands after a
  // heart was pressed would otherwise put back the list without that bookmark. An older load
  // waits for the latest and answers with it.
  const loadFavourites = () => {
    const p = favLoad = api('GET', '/favourites').then((f) => {
      if (p !== favLoad) return favLoad;
      state.favUris = new Set(f.uris || []); paintHearts(); return f;
    }, (e) => { if (p !== favLoad) return favLoad; throw e; });
    // (`error`: the latest answer was a failure, so a page can say so instead of "none")
    p.then(() => { p.done = true; }, (e) => { p.done = true; p.error = e; });
    return p;
  };
  // The song hearts on screen (song rows, Library › Songs rows, Now Playing) painted again from the
  // favourites just in, and only the hearts, so nothing moves under you: a song saved from one row
  // turns over in its other rows and in Now Playing, and bookmarks that arrive after the page (a
  // cold start) fill them in. (A page's own hearts, album and artist ones, are drawn with the page.)
  const heartSong = new WeakMap();
  const paintHearts = () => {
    $$('[data-sfav]').forEach((b) => { const t = heartSong.get(b); if (t) setHeart(b, songFav(t), t.title); });
    $$('.track [data-act="fav"]').forEach((b) => { const list = b.closest('[data-list]'); const t = (rowLists[list ? list.dataset.list : 'tr'] || [])[Number(b.closest('.track').dataset.i)]; if (t) setHeart(b, isFav(t), t.title); });
    npHeartPaint();
  };
  // No favourites here any more (the homeserver disconnected): a load still on its way is no longer
  // the latest, so it can't put them back
  const clearFavourites = () => {
    const p = favLoad = Promise.resolve({ uris: [], albums: [], tracks: [], artists: [], other: [] }); p.done = true;
    state.favUris = new Set(); paintHearts();
  };
  // A song not identified yet is a favourite on this node only until "Which song is this?".
  // A listener's go to their own homeserver: the first one asks them to connect it.
  // True once changed; false when they chose not to connect (nothing was changed): callers
  // toggle or redraw only on true.
  const setFavourite = async (ref, on, again = true) => {
    let note = '';
    try {
      if (on) {
        const r = await api('PUT', '/favourites', ref); const k = ((r && r.uri) || '').split(':')[0];
        if (isOffline()) note = ' (sent to your node when it is back)';
        else if (r && r.pending) note = ' (on this node until your homeserver answers)';
        else if (k === 'track') note = ' (kept on this node until you pick which song it is: the ? on its row)';
        else if (k === 'album' || k === 'artist') note = ` (kept on this node until MusicBrainz knows this ${k})`;
      } else {
        const r = await api('DELETE', '/favourites?' + new URLSearchParams(Object.entries(ref).map(([k, v]) => [k, String(v)])).toString());
        if (r && r.pending) note = ' (on this node until your homeserver answers)';
      }
    } catch (e) {
      if (!(again && e.status === 409 && e.body && e.body.needs_connect)) throw e;
      if (await homeserverDialog(!!e.body.lost)) return setFavourite(ref, on, false);
      return false;
    }
    await loadFavourites();
    toast(on ? 'Added to Bookmarks' + note : 'Removed from Bookmarks');
    return true;
  };
  // Whether the node holds a Pubky session (menus offer homeserver uploads then). Asked once at a
  // time: a page drawn again while the node hasn't answered (Try again on a held playlist) waits on
  // the same question, for 15 s at most: then everyone waiting on it goes on without an answer (a
  // ⋯ menu opens, not cached) and the next call asks again. Only the latest question's answer
  // counts (pubkyGen), or an older one while no answer is known yet (a node that always answers
  // after 15 s), and none asked before the user was forgotten (forgetPubkyUser).
  // (pubkyKnown: waits for the node's answer itself, past the 15 s too: a playlist drawn without
  // the user draws again once it is known; a wait whose draw is out of date (gone) is let go)
  let pubkyAsking = null; let pubkyForgot = 0; let pubkyGen = 0; let pubkyWaiting = [];
  const pubkyHeard = () => { if (state.pubkyUser === undefined) return; const w = pubkyWaiting; pubkyWaiting = []; w.forEach((x) => x.res()); };
  const forgetPubkyUser = () => { state.pubkyUser = undefined; pubkyAsking = null; pubkyForgot++; };
  const pubkyUser = async () => {
    if (isListener() || !offers('pubky')) return null;
    if (state.pubkyUser === undefined) {
      if (!pubkyAsking) {
        const asked = pubkyForgot; const gen = ++pubkyGen;
        // (a failed older question says nothing while a newer one may still answer)
        const latest = (anyAnswer) => asked === pubkyForgot && (gen === pubkyGen || (anyAnswer && state.pubkyUser === undefined));
        let waitedOn;
        const answer = api('GET', '/pubky')
          .then((pk) => { if (latest(true)) { state.pubkyUser = (pk && pk.user) || null; state.pubkyStatus = pk; } }, () => { if (latest() && state.pubkyUser === undefined) state.pubkyUser = null; })
          .then(pubkyHeard);
        const mine = pubkyAsking = Promise.race([answer, new Promise((res) => { waitedOn = setTimeout(res, PL_READ_MS); })])
          .finally(() => { clearTimeout(waitedOn); if (pubkyAsking === mine) pubkyAsking = null; });
      }
      await pubkyAsking;
    }
    return state.pubkyUser === undefined ? null : state.pubkyUser;
  };
  const pubkyKnown = (gone) => new Promise((res) => { if (state.pubkyUser !== undefined) res(); else if (!isListener()) { pubkyWaiting = pubkyWaiting.filter((x) => !(x.gone && x.gone())); pubkyWaiting.push({ res, gone }); pubkyUser(); } });
  // A session approved for fewer folders than the node uses: one plain line with the way out.
  // The node's own Pubky session lacks folders: the owner's next log in with Pubky Ring asks for
  // them (one approval, no separate sign-in), so the way there is Log out
  const RESIGN_TEXT = 'This node needs your approval for new folders on your homeserver. Log out and log in again with Pubky Ring to give it.';
  const resignNotice = (pk = state.pubkyStatus) => pk && pk.needs_resign ? `<div class="notice warn resign" role="status"><span>${esc(pk.notice || RESIGN_TEXT)}</span>${state.token ? `<button class="btn sm" data-resign>${icon('logout')} Log out</button>` : '<button class="btn sm primary" data-resign>Approve in Pubky Ring</button>'}</div>` : '';
  // With a token: Log out, as the next log in asks for the folders. A node without one has no log
  // in, so its notice approves them here (Settings › My accounts, the node's own Pubky sign-in).
  const bindResign = (root) => $$('[data-resign]', root).forEach((b) => b.onclick = state.token ? logOut : () => settingsAt('accounts', 'resign'));
  // A sync's answer as one plain line: the re-sign notice instead of a raw 403.
  const syncToast = (r) => { const resign = (r.errors || []).find((e) => e === RESIGN_TEXT); if (resign) { forgetPubkyUser(); return toast(RESIGN_TEXT, 'error'); } toast(`Saved: ${fmtCount(r.pushed)} sent to your homeserver, ${fmtCount(r.pulled)} received${r.errors.length ? `, ${fmtCount(r.errors.length)} errors` : ''}${r.pending ? `; ${fmtCount(r.pending)} more going out in the background` : ''}${(r.notes || []).length ? `. ${r.notes.join(' ')}` : ''}`); };
  // The API serves at most 1000 rows per call; follow the `after` cursor to the end (capped at 50k rows).
  // opts.onPage(n): the rows so far, after each part; opts.wait: ms a part may take before it is given
  // up with STOPPED_ANSWERING (a node that stops answering mid-library doesn't leave it waiting for good)
  const STOPPED_ANSWERING = 'The node stopped answering. Try again.';
  const pageAll = async (path, idOf, unwrap = (r) => r, opts = {}) => {
    const out = []; let after = null;
    for (let i = 0; i < 50; i++) {
      const ac = opts.wait ? new AbortController() : null; let cut = null;
      if (ac) { ac.signal.why = new Error(STOPPED_ANSWERING); cut = setTimeout(() => ac.abort(ac.signal.why), opts.wait); }
      let rows; try { rows = unwrap(await api('GET', `${path}?limit=1000${after != null ? `&after=${after}` : ''}`, undefined, ac ? { signal: ac.signal } : {})); } finally { clearTimeout(cut); }
      out.push(...rows);
      if (opts.onPage && rows.length === 1000) opts.onPage(out.length);
      if (rows.length < 1000) break;
      after = idOf(rows[rows.length - 1]); if (after == null) break;
    }
    return out;
  };
  // Long lists draw a screenful at a time and add more as their end scrolls into view, so a
  // library of 50,000 tracks opens as fast as a small one. `more()` appends the next part and
  // says whether anything is left (it may fetch). Returns a function that stops the growing.
  const growList = (box, more) => {
    const sentinel = document.createElement('div'); sentinel.className = 'grow-more'; sentinel.setAttribute('aria-hidden', 'true');
    box.after(sentinel);
    let busy = false; let done = false;
    const io = new IntersectionObserver(async (es) => {
      if (busy || done || !es.some((e) => e.isIntersecting)) return;
      busy = true;
      try { done = !(await more()); } catch (e) { done = true; toast(e.message, 'error'); }
      busy = false;
      if (done) { io.disconnect(); sentinel.remove(); } else { io.unobserve(sentinel); io.observe(sentinel); }
      // (measured from the page's own scrolling box: a margin around the window is cut off by that
      // box, so the next part was only asked for once the list's end was on screen)
    }, { root: /auto|scroll/.test(getComputedStyle(main).overflowY) ? main : null, rootMargin: '1200px 0px' });
    io.observe(sentinel);
    return () => { done = true; io.disconnect(); sentinel.remove(); };
  };
  // The same over items already in memory: `html(slice, start)` draws a part; `bind()` wires it.
  // `first`: how many the first part draws (a list drawn again where it stands: as many as before).
  const growItems = (box, items, html, bind, step = 120, first = step) => {
    let n = 0;
    const more = () => { const part = items.slice(n, n + (n ? step : first)); if (part.length) { box.insertAdjacentHTML('beforeend', html(part, n)); n += part.length; if (bind) bind(); } return n < items.length; };
    more();
    return n < items.length ? growList(box, more) : () => {};
  };
  // A list's filter that matches nothing says so under the list (the count alone read "0 shown").
  // The line is there, empty, from the start so a screen reader hears it when it fills; it fills
  // once typing pauses (not at each key), and empties at once. No filter box: does nothing.
  // `said`: the line as it was before the list was drawn again; it is there at once and not said
  // again while the same filter still matches nothing.
  const filterNone = (box, what, said = '') => {
    if (!box || !$('#filter')) return () => {};
    const p = document.createElement('p'); p.className = 'muted small filter-none'; p.setAttribute('role', 'status'); p.textContent = said; box.after(p);
    let t = null;
    return (q, n) => {
      clearTimeout(t);
      if (!q || n) { p.textContent = ''; return; }
      const line = `No ${what} matches “${q}”.`;
      if (p.textContent !== line) t = setTimeout(() => { p.textContent = line; }, 250);
    };
  };
  // A–Z on a long list jumps within what the filter shows: the matches from that letter on. While a
  // filter is typed, letters with no match are off; typing (or Escape) unmarks the chosen letter.
  // `matches()` is the filtered list, `key` an item's letter. Returns what the filter calls with
  // its new matches.
  const azJump = (key, matches, draw, none) => {
    const bs = $$('[data-az]', main);
    const mark = (on) => bs.forEach((x) => { x.classList.toggle('on', x === on); x.setAttribute('aria-pressed', String(x === on)); });
    bs.forEach((b) => b.onclick = () => { const list = matches(); const i = list.findIndex((a) => key(a) === b.dataset.az); if (i >= 0) { draw(list.slice(i)); none('', 1); mark(b); } });
    return (list) => { const have = new Set(list.map(key)); bs.forEach((x) => { x.disabled = !have.has(x.dataset.az); }); mark(null); };
  };
  // A client-side filter box for long lists: hides rows/tiles whose text does not contain the query.
  // Library list controls (UX review #7): sort, a favourites toggle and an A–Z jump in one row,
  // remembered per list in this browser.
  const LIST_SORTS = {
    albums: [['name', 'Name'], ['artist', 'Artist'], ['year', 'Year'], ['added', 'Recently added']],
    artists: [['name', 'Name'], ['albums', 'Most albums'], ['added', 'Recently added']],
  };
  const listPref = (kind) => { try { return JSON.parse(localStorage.getItem(`pm.list.${kind}`) || '{}'); } catch { return {}; } };
  const setListPref = (kind, p) => { try { localStorage.setItem(`pm.list.${kind}`, JSON.stringify(p)); } catch { /* no storage */ } };
  // Sorting, a quality chip or Favourites changes which of the current matches show and their
  // order, keeping the filter and keyboard position (`target` finds the control again).
  const renderSortedList = async (control, target = '[data-lsort]') => {
    let query = $('#filter')?.value; let focused = document.activeElement === control;
    const gen = navGen;
    // Removing Sort during loading leaves focus on body. Any later focus transfer cancels
    // restoration, even if the person then dismisses that control (Search followed by Escape).
    const moved = () => { focused = false; };
    // What the person types (or clears with Escape) in the filter meanwhile is what is put back
    const typed = (e) => { if (!e.target || e.target.id !== 'filter') return; if (e.type === 'input') query = e.target.value; else if (e.key === 'Escape') query = ''; };
    document.addEventListener('focusin', moved, { capture: true, once: true, signal: navCtl.signal });
    document.addEventListener('input', typed, { capture: true, signal: navCtl.signal });
    document.addEventListener('keydown', typed, { capture: true, signal: navCtl.signal });
    try {
      // A redraw of the same page while it sorts (the library changed) replaces the list again: the
      // filter goes back after each render, the focus once the newest is done. (A redraw that
      // keeps the page on screen meanwhile, keepList, puts back the filter and focus itself.)
      state.drawnPage = null; if (state.placeHold) state.placeHold();
      const rendered = render(); let drawing = lastDrawing;
      await rendered;
      for (;;) {
        if (gen !== navGen) return;
        const filter = $('#filter');
        if (filter && query != null) { filter.value = query; filter.oninput?.(); }
        if (drawing === lastDrawing) break;
        drawing = lastDrawing; await drawing.done;
      }
      if (focused && document.activeElement === document.body) $(target, main)?.focus({ preventScroll: true });
    } finally { document.removeEventListener('focusin', moved, true); document.removeEventListener('input', typed, true); document.removeEventListener('keydown', typed, true); }
  };
  // Albums with nothing to show: says which choice left it empty (Favourites, a quality, or both)
  const albumsNone = (fav, focus, anyFav) => (fav && focus && anyFav ? ['No bookmarked albums of this quality', 'None of your bookmarked albums has a file of this quality.']
    : fav ? ['No bookmarked albums', 'Open an album and press its heart to see it here.']
      : ['No albums of this quality', 'No album has a file of this quality.']);
  // A hearted album MusicBrainz doesn't know yet is kept on this node as album:<id>
  // An album's heart: kept on the node (album:<id>), or on its release or its release group (the
  // album page and Discover heart the group)
  const albumFav = (a) => !!(state.favUris && (state.favUris.has(`album:${a.id}`) || (a.release_mbid && state.favUris.has(`https://musicbrainz.org/release/${a.release_mbid}`)) || (a.release_group_mbid && state.favUris.has(`https://musicbrainz.org/release-group/${a.release_group_mbid}`))));
  const NAME_ORDER = new Intl.Collator(undefined, { sensitivity: 'base', numeric: true });
  const sortList = (kind, list, how) => {
    // (one collator for every comparison: localeCompare with options makes one per call, which was
    // most of the time Albums and Artists took to open on a big library; each name's key once)
    const by = (f) => list.map((x) => [String(f(x) || ''), x]).sort((a, b) => NAME_ORDER.compare(a[0], b[0])).map((p) => p[1]);
    if (kind === 'albums') {
      if (how === 'artist') return by((a) => `${a.album_artist || ''}\u0000${a.title}`);
      if (how === 'year') return [...list].sort((a, b) => (b.year || 0) - (a.year || 0));
      if (how === 'added') return [...list].sort((a, b) => (b.id || 0) - (a.id || 0));
      return by((a) => a.title);
    }
    if (how === 'albums') return [...list].sort((a, b) => (b.albums || 0) - (a.albums || 0));
    if (how === 'added') return [...list].sort((a, b) => (b.id || 0) - (a.id || 0));
    return by((a) => a.name);
  };
  // (letters that don't split into a base letter go under the one they sort with, so "Œuvre" is
  // under O and "#" stays first or last in the A–Z row)
  const AZ_BASE = { Æ: 'A', Ɓ: 'B', Đ: 'D', Ð: 'D', Ɗ: 'D', Ə: 'E', Ħ: 'H', Ɨ: 'I', Ł: 'L', Ŋ: 'N', Œ: 'O', Ø: 'O', Ŧ: 'T', Ƶ: 'Z' };
  const firstLetter = (x) => { let c = String(x || '').trim().charAt(0).toUpperCase().normalize('NFKD').charAt(0); c = AZ_BASE[c] || c; return /[A-Z]/.test(c) ? c : '#'; };
  const listControls = (kind, pref, letters) => `<div class="listbar"><label class="small muted">Sort <select class="input sm" data-lsort aria-label="Sort">${LIST_SORTS[kind].map(([k, l]) => `<option value="${k}" ${pref.sort === k ? 'selected' : ''}>${l}</option>`).join('')}</select></label>${kind === 'albums' ? `<button class="chip ${pref.fav ? 'on' : ''}" data-lfav aria-pressed="${!!pref.fav}">${icon(pref.fav ? 'heart-fill' : 'heart')} Bookmarks</button>` : ''}${letters.length > 1 ? `<span class="azbar" role="group" aria-label="Jump to a letter">${letters.map((l) => `<button class="az" data-az="${l}" aria-pressed="false">${l}</button>`).join('')}</span>` : ''}</div>`;
  // Off while there is nothing to filter: what was typed stays and applies again once there is.
  // On a phone it shows only "Filter" (the heading above names the list; its name stays whole):
  // "Filter songs" didn't fit beside Play all and + with a larger font size
  const filterBox = (name, off = false) => `<input class="input filter" id="filter" type="search" placeholder="${esc(shellMode() === 'phone' ? 'Filter' : name)}" aria-label="${esc(name)}"${off ? ' disabled' : ''}>`;
  // (a load started before a newer one, or before the albums were let go (the library changed),
  // doesn't keep its answer: forgetAlbums)
  let albumsLoad = null;
  const forgetAlbums = () => { state.albums = null; albumsLoad = null; };
  const albumsAll = async () => {
    if (!state.albums) {
      const load = albumsLoad = Promise.all([pageAll('/library/albums', (a) => a.id), api('GET', '/library/albums/formats').catch(() => [])]);
      const [albums, formats] = await load;
      if (load !== albumsLoad) return albums;
      state.albums = albums; state.formats = new Map(formats.map((f) => [f.album_id, f]));
    }
    return state.albums;
  };
  // Roon-style Focus: which albums have a file of this quality.
  const FOCUS = [['', 'All'], ['hi_res', 'Hi-res'], ['dsd', 'DSD'], ['cd', 'CD quality'], ['lossless', 'Lossless'], ['lossy', 'Lossy']];
  const inFocus = (a, focus) => {
    if (!focus) return true;
    const f = fmtOf(a.id); if (!f) return false;
    switch (focus) { case 'hi_res': return f.hi_res; case 'dsd': return f.dsd; case 'cd': return f.lossless && !f.hi_res && !f.dsd; case 'lossless': return f.lossless; case 'lossy': return !f.lossless; default: return true; }
  };
  // One choice, like Favourites beside it: a named group whose chosen button is pressed. Shown
  // only when a quality narrows the albums (or one is chosen): no lone All without format data.
  const focusChips = (albums) => {
    const counts = FOCUS.map(([k]) => (k ? albums.filter((a) => inFocus(a, k)).length : albums.length));
    if (!state.focus && !counts.some((n, j) => j && n && n < albums.length)) return '';
    return `<div class="chips qualchips" role="group" aria-label="Quality">${FOCUS.map(([k, label], j) => { const n = counts[j]; return (k && !n) ? '' : `<button class="chip ${state.focus === k ? 'on' : ''}" data-focus="${k}" aria-pressed="${state.focus === k}">${label}${k ? ` <span class="count">${n}</span>` : ''}</button>`; }).join('')}</div>`;
  };


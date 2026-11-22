  // A small anchored menu (track ⋯, album actions). One at a time; Esc or the scrim closes it.
  const menuEl = document.createElement('div'); menuEl.className = 'popover menu'; menuEl.hidden = true; document.body.appendChild(menuEl);
  // Long-press on a phone, in a list that has Select: selection, with that row ticked (owner,
  // 2026-11-08); elsewhere (and right-click) the row's ⋯ menu
  const longPressSelect = (row) => {
    if (shellMode() !== 'phone' || !matchMedia('(pointer: coarse)').matches || !row.matches('.srow[data-srow]')) return false;
    const sec = row.closest('.section'); const b = sec && $('.section-head [data-selstart]', sec); if (!b) return false;
    if (!sec.classList.contains('selecting')) b.click();
    // (the box is ticked directly: a click now would be taken for the long press's own release)
    const c = $('.selbox input', row); if (c && !c.checked) { c.checked = true; c.dispatchEvent(new Event('change', { bubbles: true })); }
    return !!c;
  };
  // (a song or album name in a row is a link for the keyboard and Ctrl+click; right-click or a long press there is still the row's menu)
  document.addEventListener('contextmenu', (e) => { const row = e.target.closest && e.target.closest('.srow, .track, [data-srow]'); const more = row && row.querySelector('[data-smore], [data-act="more"]'); if (!more || e.target.closest('input, textarea, a[href]:not([data-act="song"], [data-ssong], [data-salbum])')) return; e.preventDefault(); if (longPressSelect(row)) return; more.click(); });
  // Song rows on touch screens (docs/ux/responsive.md, build step 3): swipe right to play next, left to
  // add to a playlist (the owner's choice); a long press opens the row's menu, also where the browser
  // sends no contextmenu (iOS). Each runs the row's own ⋯ menu, so every kind of song row behaves alike.
  {
    let sw = null; let lastCtx = 0; let lastLong = 0; let longEnd = 0;
    document.addEventListener('contextmenu', (e) => { lastCtx = Date.now(); if (Date.now() - lastLong < 2000 && e.target.closest && e.target.closest('.srow, .track, [data-srow]')) { e.preventDefault(); e.stopImmediatePropagation(); } }, true);
    // the click a long press leaves on release (iOS) must not play the row
    document.addEventListener('click', (e) => {
      // the click iOS sends on releasing a long press: whatever is under the finger then (a row, the menu sheet)
      if (e.isTrusted && (Date.now() - longEnd < 400 || (Date.now() - lastLong < 1500 && e.target.closest && e.target.closest('.srow, .track, [data-srow]') && !e.target.closest('[data-smore], [data-act="more"]')))) { e.preventDefault(); e.stopImmediatePropagation(); lastLong = 0; longEnd = 0; }
    }, true);
    const ROWS = '.srow, .track, [data-srow]';
    const moreOf = (row) => row.querySelector('[data-smore], [data-act="more"]');
    const reset = (row) => { row.style.transform = ''; row.classList.remove('swiping', 'swipe-ready'); delete row.dataset.swipeLabel; };
    document.addEventListener('touchstart', (e) => {
      if (sw) { clearTimeout(sw.timer); if (sw.on) reset(sw.row); }
      const row = e.target.closest && e.target.closest(ROWS);
      if (!row || e.touches.length !== 1 || e.target.closest('input, textarea, [data-qgrip], [draggable="true"]') || row.closest('.selecting') || !moreOf(row)) { sw = null; return; }
      const t = e.touches[0];
      sw = { row, x: t.clientX, y: t.clientY, dx: 0, on: false, moved: false };
      sw.timer = setTimeout(() => { const s2 = sw; if (s2 && s2.row === row && !s2.moved && Date.now() - lastCtx > 800) { s2.long = true; lastLong = Date.now(); if (!longPressSelect(row)) moreOf(row).click(); } }, 600);
    }, { passive: true });
    document.addEventListener('touchmove', (e) => {
      if (!sw) return; const t = e.touches[0]; const dx = t.clientX - sw.x; const dy = t.clientY - sw.y;
      if (!sw.dir && (Math.abs(dx) > 10 || Math.abs(dy) > 10)) { sw.moved = true; sw.dir = Math.abs(dx) > Math.abs(dy) * 1.5 ? 'x' : 'y'; } // locked on the first move
      if (sw.dir !== 'x') return;
      if (!sw.on) { sw.on = true; sw.row.classList.add('swiping'); }
      sw.dx = Math.max(-140, Math.min(140, dx));
      sw.row.style.transform = `translateX(${sw.dx}px)`;
      sw.row.dataset.swipeLabel = sw.dx > 0 ? 'Play next' : 'Add to collection';
      sw.row.classList.toggle('swipe-ready', Math.abs(sw.dx) >= 80);
    }, { passive: true });
    const end = () => {
      if (!sw) return; clearTimeout(sw.timer); const s2 = sw; sw = null;
      if (s2.long) longEnd = Date.now();
      if (!s2.on) return;
      reset(s2.row);
      if (Math.abs(s2.dx) >= 80) { const more = moreOf(s2.row); menuPick = { label: s2.dx > 0 ? 'Play next' : 'Add to collection', at: Date.now(), anchor: more }; more.click(); }
    };
    document.addEventListener('touchend', end, { passive: true });
    document.addEventListener('touchcancel', () => { if (sw) { clearTimeout(sw.timer); if (sw.on) reset(sw.row); sw = null; } }, { passive: true });
  }
  // The keyboard in a ⋯ menu: opened with Enter or Space, the menu takes the focus; ↑ ↓ Home End move
  // through its items (← → skip no song there), Esc and Tab close it, and the focus goes back to its
  // ⋯ (Tab then goes on from there), after an item too, so a page drawn again keeps it on the row.
  // Before, the focus stayed on the ⋯ and no item could be reached without a mouse; after one it
  // fell to the page. A mouse or a finger leaves the focus as it was.
  let menuAnchor = null;
  // (only after a key: a clicked item keeps the focus where the mouse left it)
  const menuBack = () => { const a = menuAnchor; if (keyedLast && a && a.isConnected && menuEl.contains(document.activeElement)) a.focus({ preventScroll: true }); };
  menuEl.addEventListener('keydown', (e) => {
    if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') { e.stopPropagation(); return; }
    if (e.key === 'Tab') { closeMenu(); return; }
    const items = $$('.pop-item:not([disabled])', menuEl); const i = items.indexOf(document.activeElement); const n = items.length;
    const to = !n ? null : e.key === 'ArrowDown' ? (i + 1) % n : e.key === 'ArrowUp' ? (i <= 0 ? n - 1 : i - 1) : e.key === 'Home' ? 0 : e.key === 'End' ? n - 1 : null;
    if (to == null || e.altKey || e.ctrlKey || e.metaKey) return;
    e.preventDefault(); e.stopPropagation(); items[to].focus({ preventScroll: true });
  });
  const closeMenu = (fromBack) => { if (menuEl.hidden) return; menuBack(); menuEl.hidden = true; if (fromBack !== true) popTransient(); };
  transients.push({ open: () => !menuEl.hidden, close: closeMenu }, { open: () => !zonePop.hidden, close: closePop }, { open: () => !qPanel.hidden, close: closeQPanel }, { open: () => !jobPop().hidden, close: closeJobPop });
  // Phones: a sheet closes with a swipe down while its list is at the top (it follows the finger;
  // a short pull springs back). A swipe that starts sideways or scrolls the list is left alone.
  const sheetSwipe = (el, close) => {
    let t = null;
    const end = (cancel) => {
      if (!t) return; const was = t; t = null;
      el.classList.remove('sheet-drag'); el.style.transform = '';
      if (!cancel && was.on && was.dy > Math.min(120, el.offsetHeight / 3)) close();
    };
    el.addEventListener('touchstart', (e) => {
      // (only with the sheet and every list in it between the finger and the sheet at their top)
      let inner = false; for (let a = e.target; a && a !== el; a = a.parentElement) if (a.scrollTop > 0) inner = true;
      t = shellMode() === 'phone' && e.touches.length === 1 && el.scrollTop <= 0 && !inner ? { x: e.touches[0].clientX, y: e.touches[0].clientY, dy: 0, on: false } : null;
    }, { passive: true });
    el.addEventListener('touchmove', (e) => {
      if (!t) return;
      const dy = e.touches[0].clientY - t.y; const dx = e.touches[0].clientX - t.x;
      if (!t.on) {
        if (Math.abs(dy) < 10 && Math.abs(dx) < 10) return;
        if (dy < 0 || Math.abs(dx) > Math.abs(dy) || el.scrollTop > 0) { t = null; return; }
        t.on = true; el.classList.add('sheet-drag');
      }
      // The pull is the sheet's from its first move: the browser doesn't also pan (it would cancel the touch)
      if (e.cancelable) e.preventDefault();
      t.dy = Math.max(0, dy); el.style.transform = `translateY(${t.dy}px)`;
    }, { passive: false });
    el.addEventListener('touchend', () => end(false), { passive: true });
    el.addEventListener('touchcancel', () => end(true), { passive: true });
  };
  [[menuEl, closeMenu], [zonePop, closePop], [qPanel, closeQPanel], [jobPop(), closeJobPop], [volPop(), closeVolPop]].forEach(([el, close]) => { if (el) sheetSwipe(el, close); });
  const OWNER_MENU = /^tag|post about|link another|wrong match|homeserver|upload|copy to|rename|description|identify|which song|fix tags|delete|remove from library/i;
  let menuPick = null; // { label, at }: run that item of the next menu instead of showing it (row swipes)
  // `opts.all`: every item stays for a listener too (the items are names, not owner actions)
  const showMenu = (anchor, items, opts = {}) => {
    if (isListener() && !opts.all) { items = items.filter((it) => it.title || it.own || !OWNER_MENU.test(it.label || '')); if (!items.some((it) => !it.title)) { if (menuPick && menuPick.anchor === anchor) { toast(`${menuPick.label} isn't available here`); menuPick = null; } return; } }
    if (menuPick && (menuPick.anchor !== anchor || Date.now() - menuPick.at > 4000)) { if (Date.now() - menuPick.at > 4000) menuPick = null; } // another menu: open it as usual
    else if (menuPick) {
      const want = menuPick; menuPick = null; closeMenu();
      {
        const it = items.find((x) => !x.title && !x.disabled && String(x.label || '').replace(/…$/, '') === want.label);
        if (it) busy(anchor, () => it.run()); else toast(`${want.label} isn't available for this row`);
        return;
      }
    }
    // (a key used last, with the focus on the ⋯, the row it is in (the context menu key), nowhere, or
    // in the menu before (a sub-menu): not a mouse's click that left a menu item focused)
    const fa = document.activeElement;
    const keyed = keyedLast && (!fa || fa === document.body || fa === anchor || menuEl.contains(fa) || (fa.contains && fa.contains(anchor)));
    menuAnchor = anchor;
    menuEl.innerHTML = items.map((it, i) => it.title ? `<div class="pop-title">${esc(it.title)}</div>` : `<button class="pop-item ${it.on ? 'on' : ''}" data-mi="${i}" ${it.disabled ? 'disabled' : ''} ${it.on != null ? `aria-checked="${!!it.on}" role="menuitemradio"` : ''}>${it.face ? face(it.face) : it.svc ? svcBadge(it.svc) : it.icon ? icon(it.icon) : ''}<span>${esc(it.label)}${it.note ? `<span class="pop-note">${esc(it.note)}</span>` : ''}</span>${it.on ? icon('check', 'ic pop-check') : ''}</button>`).join('');
    // The item runs first: a page it opens replaces the transient entry; otherwise it is popped. The menu
    // closes at once, so the control that opened it (its ⋯) shows the progress until the item is done.
    $$('[data-mi]', menuEl).forEach((b) => b.onclick = () => { menuBack(); menuEl.hidden = true; Promise.resolve(busy(anchor, () => items[Number(b.dataset.mi)].run())).catch(() => {}).finally(popTransient); });
    menuEl.hidden = false; pushTransient();
    const r = anchor.getBoundingClientRect(); const mw = 280;
    menuEl.style.left = Math.max(8, Math.min(window.innerWidth - mw - 8, r.right - mw)) + 'px';
    menuEl.style.top = Math.min(window.innerHeight - menuEl.offsetHeight - 8, r.bottom + 6) + 'px';
    menuEl.style.bottom = 'auto'; menuEl.style.right = 'auto'; menuEl.style.width = mw + 'px';
    // Phones: a sheet from the bottom, not a dropdown that runs off the screen (UX review #3)
    menuEl.classList.toggle('sheet', shellMode() === 'phone');
    if (menuEl.classList.contains('sheet')) { menuEl.style.left = '0px'; menuEl.style.right = '0px'; menuEl.style.top = 'auto'; menuEl.style.bottom = '0px'; menuEl.style.width = 'auto'; }
    if (keyed) { const first = $('.pop-item:not([disabled])', menuEl); if (first) first.focus({ preventScroll: true }); }
    // A click outside only closes the menu (it does not also press what is under it).
    setTimeout(() => { const outside = (e) => { if (menuEl.hidden) { document.removeEventListener('click', outside, true); return; } if (!menuEl.contains(e.target)) { e.stopPropagation(); e.preventDefault(); document.removeEventListener('click', outside, true); closeMenu(); } }; document.addEventListener('click', outside, true); }, 0);
  };
  // Songs not identified yet go in too, kept on this node until they are: the toast says how many
  // of the n songs, after an add to a playlist picked or a new one. A song from a page (itemMenu)
  // comes as { id, local }: its page id says whether it is matched.
  const stayNote = (tracks) => {
    const n = tracks.length; const localN = tracks.filter((t) => t && ('local' in t ? t.local : !trackHasId(t))).length;
    return localN ? `. ${localN === n ? (n === 1 ? 'It stays' : 'They stay') : `${localN} ${localN === 1 ? 'stays' : 'stay'}`} on this node until you pick which song ${localN === 1 ? 'it is' : 'each is'} (the ?).` : '';
  };
  // The toast names the playlist picked (as a new one's "Created “…”" does), so with many of them
  // you can tell which row the tap hit
  const addToPlaylist = async (pl, tracks, uris) => {
    const to = pl.name ? `“${pl.name}”` : 'collection';
    try { await api('POST', `/playlists/${encodeURIComponent(pl.id)}/items`, uris ? { uris } : { track_ids: tracks.map((t) => t.id) }); const n = uris ? uris.length : tracks.length; toast((n === 1 ? `Added to ${to}` : `${countOf(n, 'songs')} added to ${to}`) + (uris ? '' : stayNote(tracks))); }
    catch (e) { toast(e.message, 'error'); }
  };
  // "Add to playlist" for canonical URLs (Internet Archive files): the same menu, by uris.
  const uriMenu = async (anchor, uris, name) => {
    if (!state.playlists.length) { try { state.playlists = await api('GET', '/playlists'); } catch { /* keep */ } }
    return playlistPicker((pl) => addToPlaylist(pl, [], uris), name || 'New collection', async (n) => { try { await api('POST', '/playlists', { name: n, uris }); state.playlists = await api('GET', '/playlists'); toast(`Created “${n}”`); } catch (e) { toast(e.message, 'error'); } });
  };
  // (head: items before the song's own, e.g. a queue row's name and its Move up / Move down / Remove;
  // live: whether the menu is still wanted once its reads are in, e.g. that queue row still drawn
  // (or the control to open it at instead: the same song's ⋯ on the queue drawn again);
  // guard: wraps each item's action, e.g. a queue row's: nothing for another zone than the page's)
  const trackMenu = async (anchor, tracks, { head = [], live = () => true, guard = (f) => f } = {}) => {
    if (!state.playlists.length) { try { state.playlists = await api('GET', '/playlists'); } catch { /* keep */ } }
    await pubkyUser();
    const single = tracks.length === 1 ? tracks[0] : null;
    const favItem = single ? [{ label: isFav(single) ? 'Remove from Bookmarks' : 'Add to Bookmarks', icon: isFav(single) ? 'heart-fill' : 'heart', run: async () => { try { if (await setFavourite({ track_id: single.id }, !isFav(single))) render(); } catch (e) { toast(e.message, 'error'); } } }] : [];
    // Start radio: from the single track, or from a random track of a set (album page menu).
    // (the radio goes on for the zone whose queue the seed replaced, also when you switch zone while
    // the seed is sent: before, that queue was replaced and its radio silently left off; the toast
    // then names that zone)
    const seeds = tracks.filter((t) => t && trackHasId(t));
    let versionItems = [];
    if (single && single.id != null && single.isrc) {
      try { const vs = await api('GET', `/library/tracks/${single.id}/versions`); versionItems = vs.filter((v) => v.uri).map((v) => ({ label: `Stream from ${v.source.toUpperCase()} (${v.label.replace(/^TIDAL · /, '')})`, icon: 'zone', run: () => playSourceItems([{ uri: v.uri, title: single.title, artist: artists(single), album: single.album ? single.album.title : '', duration_ms: single.duration_ms }], true) })); const tv = vs.find((v) => v.source === 'tidal' && /^track:/.test(v.id)); if (tv) versionItems.push({ label: 'Full length in TIDAL\'s player (here)', icon: 'external', run: () => tidalEmbed('tracks', tv.id.replace(/^track:/, ''), single.title) }); } catch { /* no versions */ }
    }
    const radioItem = seeds.length && !isBrowser() ? [{ label: 'Start radio', icon: 'radio', run: async () => { const seed = single || seeds[Math.floor(Math.random() * seeds.length)]; const zid = state.zone; try { if (await playTracks([seed], true) === false) return; await api('PUT', `/zones/${encodeURIComponent(zid)}/radio`, { on: true }); const zs = await api('GET', `/zones/${encodeURIComponent(zid)}`); state.zones[zs.id] = zs; const away = state.zone !== zid; toast(`Radio from “${seed.title}”${away ? ` on ${zs.name || 'the other device'}` : ''}: similar music follows`); if (state.view === 'queue' && !away) render(); } catch (e) { toast(e.message, 'error'); } } }] : [];
    const nav = single ? [
      // (the song's own page first, as in itemMenu)
      { label: 'Go to song page', icon: 'compass', run: () => setView('item', { kind: 'song', id: trackPageId(single) }) },
      ...(goArtists(artistCredits({ ...single, track_id: single.id })) || (single.artists && single.artists[0] && single.artists[0].id != null ? [{ label: 'Go to artist', icon: 'artist', run: () => setView('item', { kind: 'artist', id: 'lib-' + single.artists[0].id }) }] : [])),
      ...(single.album && single.album.id != null ? [{ label: 'Go to album', icon: 'disc', run: () => setView('item', { kind: 'album', id: 'lib-' + single.album.id }) }] : []),
    ] : [];
    const items = [
      // The same order and words as a song's ⋯ on its page, an album or search (itemMenu)
      { label: 'Play next', icon: 'playnext', run: () => playTracks(tracks, false, 'next') },
      { label: 'Add to queue', icon: 'plus', run: () => playTracks(tracks, false, 'end') },
      { label: 'Add to collection…', icon: 'playlist', run: () => playlistPicker((pl) => addToPlaylist(pl, tracks), (tracks.length === 1 ? tracks[0].title : (tracks[0].album ? tracks[0].album.title : 'New collection')) || 'New collection', async (name) => { try { const d = await api('POST', '/playlists', { name, track_ids: tracks.map((t) => t.id) }); state.playlists = await api('GET', '/playlists'); toast(`Created “${d.summary ? d.summary.name : name}”` + stayNote(tracks)); } catch (e) { toast(e.message, 'error'); } }) },
      ...(single && single.id != null ? dlItem('track', single.id, single.title, artists(single)) : []),
      ...favItem,
      ...radioItem,
      ...nav,
      ...versionItems,
      // The owner's library: edit tags on the node, take the song out (Your library › Node library)
      ...(single && single.id != null && isOwner() ? [{ label: 'Edit tags…', icon: 'notes', run: () => editTrackTags(single.id) }, { label: 'Remove from library', icon: 'trash', run: () => removeFromLibrary(single.id, single.title) }] : []),
      // Phones hide the "Match" chip in song rows (no room), so matching is offered here too
      ...(single && single.id != null && !trackHasId(single) && isOwner() ? [{ label: 'Which song is this?…', icon: 'search', run: async () => { const done = await whichSong(single); if (done) { toast(`Identified: ${done.title}`); render(); } } }] : []),
      ...(single && canFixMatch(single) ? [{ label: 'Wrong match?…', icon: 'search', run: () => setView('identify', { fix: [single.id] }) }] : []),
      ...(single ? [{ label: 'Copy link', icon: 'copy', run: () => copyLink(`/song/${trackPageId(single)}`) }] : []),
      ...(tracks.length && tracks.every((t) => t && trackHasId(t)) && state.pubkyUser ? [{ label: tracks.length === 1 ? 'Copy to my homeserver' : `Copy ${countOf(tracks.length, 'songs')} to my homeserver`, icon: 'upload', run: () => startUpload({ track_ids: tracks.map((t) => t.id) }) }] : []),
    ];
    const at = live(); if (!at) return;
    showMenu(at instanceof Element ? at : anchor, [...head, ...items.map((it) => (it.run ? { ...it, run: guard(it.run) } : it))]);
  };
  const BAND_KINDS = ['peaking', 'low_shelf', 'high_shelf', 'low_pass', 'high_pass'];
  const dspEditor = (zone, cfg) => `<div class="card dsp" style="margin-top:12px"><div class="section-head"><h3>DSP · ${esc(zone.name)}</h3><span class="muted small">Every stage shows in the signal path; defaults keep the path bit-perfect.</span></div>
    <div class="dsp-grid">
      <label><span>Headroom (dB)</span><input class="input" id="d-headroom" type="number" step="0.5" min="-30" max="0" value="${cfg.headroom_db ?? ''}" placeholder="off"></label>
      <label><span>Software volume (dB)</span><input class="input" id="d-volume" type="number" step="0.5" min="-60" max="0" value="${cfg.volume_db ?? ''}" placeholder="off"></label>
      <label><span>Volume limit (dB)</span><input class="input" id="d-maxvol" type="number" step="0.5" min="-90" max="0" value="${cfg.max_volume_db ?? ''}" placeholder="none" title="Every volume request for this zone, hardware or software, is capped here"></label>
      <label><span>Loudness (ReplayGain)</span><select class="input" id="d-loudness">${['off', 'track', 'album'].map((v) => `<option value="${v}" ${cfg.loudness === v ? 'selected' : ''}>${v}</option>`).join('')}</select></label>
      <label><span>DSD</span><select class="input" id="d-dsd">${['auto', 'native', 'do_p', 'to_pcm'].map((v) => `<option value="${v}" ${cfg.dsd_mode === v ? 'selected' : ''}>${v === 'do_p' ? 'DoP' : v}</option>`).join('')}</select></label>
      <label class="check"><input type="checkbox" id="d-dither" ${cfg.dither ? 'checked' : ''}> TPDF dither when reducing bit depth</label>
      <label class="check"><input type="checkbox" id="d-resample" ${cfg.resample ? 'checked' : ''}> Resample when the output cannot take the rate (shown as Lossy)</label>
      <label class="check"><input type="checkbox" id="d-cf" ${cfg.crossfeed && cfg.crossfeed.enabled !== false ? 'checked' : ''}> Headphone crossfeed</label>
      <div class="cf-fields"><label><span>Level (dB)</span><input class="input" id="d-cf-level" type="number" step="0.5" value="${cfg.crossfeed ? cfg.crossfeed.level_db : -6}"></label><label><span>Cutoff (Hz)</span><input class="input" id="d-cf-cutoff" type="number" step="50" value="${cfg.crossfeed ? cfg.crossfeed.cutoff_hz : 700}"></label><label><span>Delay (µs)</span><input class="input" id="d-cf-delay" type="number" step="10" value="${cfg.crossfeed ? cfg.crossfeed.delay_us : 300}"></label></div>
    </div>
    <h3 style="margin-top:14px">Parametric EQ</h3>
    <div class="eq" id="eq">${(cfg.eq || []).map((b, i) => eqRow(b, i)).join('')}</div>
    <div class="actions" style="margin-top:8px"><button class="btn sm" id="eq-add">${icon('plus')} Add band</button></div>
    <div class="actions" style="margin-top:14px"><button class="btn primary" id="d-save">Save</button><button class="btn" id="d-reset">Reset to bit-perfect</button><button class="btn ghost" id="d-close">Close</button></div></div>`;
  const eqRow = (b, i) => `<div class="eq-row" data-i="${i}"><select class="input eq-kind">${BAND_KINDS.map((k) => `<option value="${k}" ${b.kind === k ? 'selected' : ''}>${k.replace('_', ' ')}</option>`).join('')}</select><input class="input eq-freq" type="number" min="10" max="22000" step="1" value="${b.freq_hz}" title="Hz"><input class="input eq-gain" type="number" min="-24" max="24" step="0.5" value="${b.gain_db ?? 0}" title="dB"><input class="input eq-q" type="number" min="0.1" max="20" step="0.1" value="${b.q ?? 0.7071}" title="Q"><label class="check small"><input type="checkbox" class="eq-on" ${b.enabled !== false ? 'checked' : ''}> on</label><button class="icon-btn eq-del" title="Remove band">${icon('close')}</button></div>`;
  const bindDspEditor = (zoneId, cfg) => {
    const num = (id) => { const v = $(id).value.trim(); return v === '' ? null : Number(v); };
    const collect = () => ({
      ...cfg,
      headroom_db: num('#d-headroom'), volume_db: num('#d-volume'), max_volume_db: num('#d-maxvol'), loudness: $('#d-loudness').value, dsd_mode: $('#d-dsd').value,
      dither: $('#d-dither').checked, resample: $('#d-resample').checked,
      crossfeed: $('#d-cf').checked ? { level_db: Number($('#d-cf-level').value), cutoff_hz: Number($('#d-cf-cutoff').value), delay_us: Number($('#d-cf-delay').value), enabled: true } : null,
      eq: $$('#eq .eq-row').map((r) => ({ kind: $('.eq-kind', r).value, freq_hz: Number($('.eq-freq', r).value), gain_db: Number($('.eq-gain', r).value), q: Number($('.eq-q', r).value), enabled: $('.eq-on', r).checked })),
    });
    const bindRows = () => $$('#eq .eq-del').forEach((b) => b.onclick = () => { b.closest('.eq-row').remove(); });
    bindRows();
    $('#eq-add').onclick = () => { $('#eq').insertAdjacentHTML('beforeend', eqRow({ kind: 'peaking', freq_hz: 1000, gain_db: 0, q: 1.0, enabled: true }, $$('#eq .eq-row').length)); bindRows(); };
    $('#d-save').onclick = async () => { try { await api('PUT', `/zones/${encodeURIComponent(zoneId)}/dsp`, collect()); toast('DSP saved; applies from the next song'); } catch (e) { toast(e.message, 'error'); } };
    $('#d-reset').onclick = async () => { try { await api('DELETE', `/zones/${encodeURIComponent(zoneId)}/dsp`); toast('DSP reset'); render(); } catch (e) { toast(e.message, 'error'); } };
    $('#d-close').onclick = () => { state.dspOpen = null; render(); };
  };
  // "New releases from your artists": release groups MusicBrainz lists for tagged artists.
  const newReleasesSection = (nr) => {
    if (!nr || !nr.artists_total) return '';
    const sub = nr.refreshing ? `checking ${nr.artists_checked} of ${countOf(nr.artists_total, 'artists')}…` : `${nr.artists_checked} of ${nr.artists_total} tagged artists checked`;
    const rows = nr.items.slice(0, 12).map((r) => `<div class="list-row"><div class="grow"><div class="name">${esc(r.title)} <span class="muted small">${esc(r.kind)}</span></div><div class="sub">${esc(r.artist)} · ${esc(r.first_release_date)}</div></div>${r.in_library ? `<button class="btn ghost small" data-album="${r.in_library.id}">In library</button>` : `<a class="btn ghost small" href="${esc(webUrl(r.url))}" target="_blank" rel="noopener">MusicBrainz</a>`}</div>`).join('');
    return `<section class="section"><div class="section-head"><h3>New releases from your artists</h3><span class="muted small">${sub}</span>${nr.refreshing ? '' : `<button class="link" id="nrrefresh">Refresh</button>`}</div>${rows ? `<div class="list">${rows}</div>` : `<p class="muted">Nothing from the last year yet${nr.artists_checked < nr.artists_total ? ' (still checking)' : ''}.</p>`}</section>`;
  };
  const albumGrid = (albums) => `<div class="grid">${albums.map(tile).join('')}</div>`;
  const strip = (title, albums, more) => (!albums || !albums.length) ? '' : `<section class="section"><div class="section-head"><h3>${esc(title)}</h3>${more ? `<button class="link" data-more="${more}">See all</button>` : ''}</div><div class="strip">${albums.map(tile).join('')}</div></section>`;
  const bindTiles = () => {
    // Only library album tiles (tile()): other grids' tiles carry a list index in data-tileplay and
    // bind their own Play before this runs after every page.
    $$('.tile[data-album] [data-tileplay]', main).forEach((p) => p.onclick = async (e) => {
      e.stopPropagation();
      try { const r = await api('GET', `/library/albums/${p.dataset.tileplay}/tracks`); playTracks(r.tracks, true); } catch (er) { toast(er.message, 'error'); }
    });
    $$('.tile[data-album]', main).forEach((el) => el.onclick = () => setView('album', Number(el.dataset.album)));
    $$('[data-more]', main).forEach((b) => b.onclick = () => setView(b.dataset.more));
  };
  // Discover rows: a Pinakes card joined to the library. Play what you own, else the service page the node can stream, else "Play on…".
  // Songs and more from other services (Favourites): the kind ends the row, as on Sources' lists
  // ("album", "song", never the describer's "other"), and an album's Play says it is the album. On a
  // phone it stays in the line under the title: next to a plays chip a row-end word left the
  // title no room.
  const discoverRows = (items) => `<div class="list">${items.map((it, i) => { const kind = it.kind && it.kind !== 'other' ? homeKind(it) : ''; return `<div class="list-row clickable" data-ditem="${i}"${it.uri ? ` data-uri="${esc(it.uri)}"` : ''}>${it.image ? `<img class="thumb" src="${esc(it.image)}" alt="" loading="lazy">` : `<div class="avatar" style="background:hsl(${hue(it.title || it.uri)} 35% 28%)">${esc(initials(it.title || 'M'))}</div>`}<div class="grow"><div class="name">${esc(itemName(it))} ${it.site ? `<span class="src-badge">${esc(it.site.toUpperCase())}</span>` : ''}${it.in_library ? '<span class="pill ok">in library</span>' : ''}</div><div class="sub">${esc([it.artist, it.group_size ? `on ${it.group_size} services` : null, it.taggers ? `${it.taggers} tagger${it.taggers === 1 ? '' : 's'}` : null].filter(Boolean).join(' · '))}${kind ? `<span class="subkind">${it.artist || it.group_size || it.taggers ? ' · ' : ''}${esc(kind)}</span>` : ''}</div></div>${kind ? `<span class="muted small rowkind">${esc(kind)}</span>` : ''}${playsChip(it)}<span class="row-actions"><button class="icon-btn" data-dplay="${i}" title="Play" aria-label="Play ${it.kind === 'album' ? 'the album ' : ''}${esc(itemName(it))}">${icon('play')}</button><button class="icon-btn" data-dmore="${i}" title="More" aria-label="More for ${esc(itemName(it))}">${icon('more')}</button></span></div>`; }).join('')}</div>`;
  // Charts and search as artwork tiles (Roon-style), with the same play/menu actions as the rows.
  // Service marks for badges: simple monochrome glyphs in the spirit of each
  // service's logo (no trademarked artwork copied), a note for the rest.
  const SERVICE_GLYPHS = {
    tidal: '<path d="M4 8l4-4 4 4-4 4zm8 0l4-4 4 4-4 4zm-4 8l4-4 4 4-4 4z"/>',
    spotify: '<path d="M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20zm4.4 14.4a.7.7 0 0 1-1 .3c-2.6-1.6-5.9-2-9.8-1.1a.7.7 0 0 1-.3-1.4c4.2-1 7.9-.5 10.8 1.3.3.2.4.6.3.9zm1.2-2.8a.9.9 0 0 1-1.2.3c-3-1.8-7.5-2.4-11-1.3a.9.9 0 0 1-.5-1.7c4-1.2 9-.6 12.4 1.5.4.3.5.8.3 1.2zm.1-2.9C14.1 8.6 8.3 8.4 4.9 9.4a1 1 0 0 1-.6-2c3.9-1.2 10.4-1 14.5 1.5a1 1 0 0 1-1.1 1.8z"/>',
    deezer: '<path d="M2 15h4v3H2zm5 0h4v3H7zm5 0h4v3h-4zm5 0h4v3h-4zM7 11h4v3H7zm5 0h4v3h-4zm5 0h4v3h-4zm-5-4h4v3h-4zm5 0h4v3h-4zm0-4h4v3h-4z"/>',
    youtube: '<path d="M21.6 7.2a2.5 2.5 0 0 0-1.8-1.8C18.3 5 12 5 12 5s-6.3 0-7.8.4A2.5 2.5 0 0 0 2.4 7.2 26 26 0 0 0 2 12a26 26 0 0 0 .4 4.8 2.5 2.5 0 0 0 1.8 1.8C5.7 19 12 19 12 19s6.3 0 7.8-.4a2.5 2.5 0 0 0 1.8-1.8A26 26 0 0 0 22 12a26 26 0 0 0-.4-4.8zM10 15V9l5.2 3z"/>',
    soundcloud: '<path d="M2 13h1.5v4H2zm2.5-2H6v6H4.5zm2.5-2h1.5v8H7zm2.5-1H11v9H9.5zm2.5-2h1.5v11H12zm3-1h3a4 4 0 0 1 0 8h-3z"/>',
    bandcamp: '<path d="M2 18l6-12h14l-6 12z"/>',
    amazonmusic: '<path d="M3 13c5 3.5 13 3.5 18 0l-1 2.5c-5 3-11 3-16 0zM19 15l2-1 0 3zM9 4h2v8H9zm4 0h2v8h-2z"/>',
    applemusic: '<path d="M16 3v11.5a3.5 3.5 0 1 1-2-3.2V7.2l-5 1.4v7.9A3.5 3.5 0 1 1 7 13.3V5.3z"/>',
    audius: '<path d="M12 3l9 17h-4.5L12 11l-4.5 9H3z"/>',
    jellyfin: '<path d="M12 3c-2.5 0-9.5 13-8.5 15S20.5 20 20.5 18 14.5 3 12 3zm0 6c1.2 0 4.5 6.3 4 7.3S8.4 17.3 8 16.3 10.8 9 12 9z"/>',
    jamendo: '<path d="M13 3h4v11.5a5.5 5.5 0 1 1-11 0h4a1.5 1.5 0 0 0 3 0z"/>',
    archive: '<path d="M3 20h18v2H3zM4 9h2v9H4zm4 0h2v9H8zm4 0h2v9h-2zm4 0h2v9h-2zm4 0h1v9h-1zM2 7l10-5 10 5v1H2z"/>',
    qobuz: '<path d="M12 3a8 8 0 1 0 4.9 14.3l2.4 2.4 1.4-1.4-2.4-2.4A8 8 0 0 0 12 3zm0 2.5a5.5 5.5 0 1 1 0 11 5.5 5.5 0 0 1 0-11z"/>',
    musicbrainz: '<path d="M12 2l9 5v10l-9 5-9-5V7zm-1 4.5L6 9v6l5 2.5zm2 0v11l5-2.5V9z"/>',
    library: '<path d="M4 4h16v3H4zm0 5h16v3H4zm0 5h10v3H4zm12 0h4v3h-4z"/>',
  };
  const serviceKey = (site) => String(site || '').toLowerCase().replace(/[^a-z]/g, '').replace(/^youtubemusic$/, 'youtube').replace(/^internetarchive$/, 'archive');
  const svcIcon = (site) => { const g = SERVICE_GLYPHS[serviceKey(site)]; return `<svg viewBox="0 0 24 24" aria-hidden="true">${g || '<path d="M9 3v12.3A3.5 3.5 0 1 0 11 18.5V7h6V3z"/>'}</svg>`; };
  const svcBadge = (site, extra = '') => site ? `<span class="src-badge ico${extra ? ' ' + extra : ''}" title="${esc(site)}">${svcIcon(site)}</span>` : '';
  // A service we can name on a cover: the library, or one with an accent or a glyph. Not a
  // MusicBrainz or Pubky page (nothing plays from those) and not a plain link.
  const knownSvc = (k) => (k && k !== 'musicbrainz' && (k === 'library' || Object.prototype.hasOwnProperty.call(SRC_ACCENT, k) || Object.prototype.hasOwnProperty.call(SERVICE_GLYPHS, k)) ? k : null);
  // The provider badge on a cover (owner, 2026-10-29): where this plays from, in the cover's bottom
  // left corner. The service's glyph in its own colour on a dark disc with a light ring, so it reads
  // on light and dark covers; our own files get a white glyph. Absolute over the cover (no layout
  // shift) and named for screen readers: "From TIDAL".
  // Inside a button (a tile, the player bar's art) the badge is decoration (`quiet`): the button's
  // name would otherwise start with "From …"; coverFrom() adds the words at the end of the button.
  const coverBadge = (svc, quiet = false) => {
    const k = knownSvc(svc === 'library' ? 'library' : srcSvc(svc)); if (!k) return '';
    const name = k === 'library' ? 'your library' : srcName(k); const a = SRC_ACCENT[k];
    return `<span class="src-badge ico cover"${quiet ? ' aria-hidden="true"' : ` role="img" aria-label="From ${esc(name)}"`} title="From ${esc(name)}"${a ? ` style="--svc:${a[1]}"` : ''}>${svcIcon(k)}</span>`;
  };
  const coverFrom = (svc) => { const k = knownSvc(svc === 'library' ? 'library' : srcSvc(svc)); return k ? `<span class="sr-only">, from ${esc(k === 'library' ? 'your library' : srcName(k))}</span>` : ''; };
  // Where a row, tile or queue item plays from: what it says itself (a service, a source, a site, a
  // "plays from" answer), else its library id, else the service in its id or uri.
  const coverSvc = (x) => {
    if (!x) return null;
    const said = x.service || x.site || (typeof x.source === 'string' ? x.source : null) || (x.plays && x.plays.service);
    const k = said && knownSvc(srcSvc(said)); if (k) return k;
    if (x.track_id != null) return 'library';
    const id = String(x.id || ''); if (/^lib-/.test(id)) return 'library';
    let m = /^([a-z]+)-/.exec(id); if (m && knownSvc(m[1])) return m[1];
    const u = String(x.uri || x.page || ''); if (/^track:\d+$/.test(u)) return 'library';
    m = /^([a-z]+):/.exec(u); if (m && !/^https?$/.test(m[1])) return knownSvc(m[1]);
    if (/^https?:/.test(u)) { try { return knownSvc(srcSvc(new URL(u).hostname)); } catch { return null; } }
    return null;
  };
  // Now playing: the node's own word on the service once it carries it (#94), else the item.
  const npSvc = (np) => (np && np.format && np.format.service ? srcSvc(np.format.service) : coverSvc(np && np.item));
  const discoverTiles = (items) => `<div class="grid">${items.map((it, i) => { const h = hue(it.title || it.uri); const playable = true; return `<button class="tile" data-ditem="${i}" title="${esc(itemName(it))}"><div class="art" style="background:linear-gradient(135deg, hsl(${h} 38% 30%), hsl(${(h + 40) % 360} 42% 18%))"><span>${esc(initials(it.title || 'M'))}</span>${it.image ? `<img loading="lazy" alt="" src="${esc(it.image)}" data-err="remove">` : ''}${coverBadge(it.site, true) || svcBadge(it.site)}${playable ? `<span class="tile-play" data-dplay="${i}" role="button" title="Play">${icon('play')}</span>` : ''}</div><div class="t">${esc(itemName(it))}${it.in_library ? ' <span class="pill ok">owned</span>' : ''}</div><div class="a">${esc([it.artist, it.group_size ? `${it.group_size} services` : null].filter(Boolean).join(' · '))}</div></button>`; }).join('')}</div>`;
  // A row's name: the title, never a URL ("Untitled on TIDAL" when unknown).
  const itemName = (it) => it.title || `Untitled on ${it.site || 'another service'}`;
  // Describe music URLs that are not in the library (favourites, playlists, history).
  const describe = async (uris) => { if (!uris.length) return []; try { return await api('POST', '/discover/describe', { uris }); } catch { return uris.map((u) => ({ uri: u, page: u, title: null, site: null, also_on: [], kind: 'other' })); } };
  // A source's state for its tab's dot: connected, not set up, an error, or switched off.
  const srcState = (x) => {
    if (!x.enabled) return { cls: 'off', label: 'Turned off', long: 'Turned off.' };
    const st = String(x.status || '');
    if (st === 'ready' || /^signed in\b/.test(st)) return { cls: 'ok', label: 'Connected', long: st === 'signed in' ? 'Signed in.' : st === 'ready' ? 'Ready.' : st.charAt(0).toUpperCase() + st.slice(1) + '.' };
    if (st === 'previews') return { cls: 'ok', label: 'Connected', long: '30-second previews, no account needed.' };
    if (/^browser\b/.test(st)) return { cls: 'ok', label: 'Ready', long: `Plays ${hereOnly()}, in YouTube\u2019s own player${/zones/.test(st) ? ', and on the speakers through yt-dlp' : ''}.${/search/.test(st) ? ' Search uses your YouTube Data API key.' : ''}` };
    if (st === 'needs_config' || /^not configured/.test(st) || st === 'needs sign-in') return { cls: 'none', label: 'Not set up', long: st === 'needs sign-in' ? 'Not signed in yet.' : 'Not set up on this node yet.' };
    if (/^ready\b/.test(st)) return { cls: 'ok', label: 'Ready', long: st.charAt(0).toUpperCase() + st.slice(1) + '.' };
    // Only a real failure is red; any other note is shown as it is
    if (/error|fail|expired|refused|revoked|unreachable|broken/i.test(st)) return { cls: 'bad', label: 'Problem', long: st };
    return { cls: 'none', label: 'Not set up', long: st };
  };
  // Sources switched off disappear from the sidebar and search scopes too.
  const applySourcesOff = () => {
    const off = new Set((state.sources || []).filter((x) => x.enabled === false).map((x) => x.id));
    $$('.nav-item[data-view]').forEach((b) => { if (b.dataset.view !== 'sources') b.hidden = off.has(b.dataset.view); });
    if (state.searchScope && off.has(state.searchScope)) state.searchScope = 'everywhere';
  };
  const FEATURE_WORDS = { search: 'Search', play: 'Plays on your zones', connect: 'Plays on its own devices', artists: 'Artist pages', library: 'Your library', playlists: 'Your playlists', downloads: 'Downloads what you bought', lyrics: 'Lyrics', scrobble: 'Reports your plays' };
  const SERVICE_NAMES = { tidal: 'TIDAL', qobuz: 'Qobuz', soundcloud: 'SoundCloud', bandcamp: 'Bandcamp', deezer: 'Deezer', spotify: 'Spotify', jamendo: 'Jamendo', subsonic: 'your server', jellyfin: 'Jellyfin', plex: 'Plex', podcasts: 'Podcasts', radio: 'Internet radio', audiobookshelf: 'Audiobookshelf', youtube: 'YouTube', library: 'your library', audius: 'Audius', archive: 'Internet Archive', musicbrainz: 'MusicBrainz' };
  // "Connect TIDAL" for music none of your services has: with one service named it opens that
  // service's tab in Sources, with more ("TIDAL or Spotify") the Sources page
  const connectArg = (list) => (list && list.length === 1 ? { source: list[0] } : undefined);
  const connectLink = (list) => { const arg = connectArg(list); return `<a data-view="sources" href="${esc(pathFor('sources', arg))}"${arg ? ` data-source="${esc(arg.source)}"` : ''}>Connect ${esc(list.map((c) => SERVICE_NAMES[c] || c).join(' or '))}</a>`; };
  const openConnect = (a) => setView('sources', a.dataset.source ? { source: a.dataset.source } : undefined);
  // ---- where playlist items play from (STEWARD-UX-BRIEF §3) ----
  // A row's source chip: where it plays from, a pin mark when the owner chose it.
  const sourceChip = (t, k) => t.pending ? '<span class="chip srcchip wait" aria-label="Finding where this plays"><span class="spin"></span></span>' : t.unavailable
    ? `<button class="chip srcchip none" data-psrc="${k}" title="${esc(t.unavailable)}">${icon('close')}<span>Unavailable</span></button>`
    : t.plays ? `<button class="chip srcchip${t.pinned ? ' pinned' : ''}" data-psrc="${k}" title="${esc(t.plays.label)}${t.pinned ? ' · pinned' : t.pinLost ? ' · pinned source not available here' : ''}">${svcBadge(t.plays.service)}<span>${esc(srcName(t.plays.service))}</span>${t.pinned ? icon('pin') : ''}</button>` : '';
  const loadOrder = async () => { const box = $('#gorder'); if (!box) return; try { const o = await api('GET', '/play-order'); box.innerHTML = orderList(o); bindOrderList(box, o.order, async (order) => { try { await api('PUT', '/play-order', { order }); state.plPf = null; loadOrder(); } catch (e) { toast(e.message, 'error'); } }); } catch (e) { box.innerHTML = `<p class="muted small">${esc(e.message)}</p>`; } };
  // The "Play from" order: a short list to drag (or Alt+↑/↓, or the arrow buttons).
  const orderList = (o) => `<ol class="porder" aria-label="Play from, in order">${o.order.map((id, k) => { const sv = o.services.find((x) => x.id === id) || { id, name: id }; return `<li class="porder-row${sv.connected === false ? ' off' : ''}" data-oi="${k}"><span class="porder-grip" data-ogrip="${k}" role="button" tabindex="0" aria-label="Move ${esc(sv.name || id)} (Alt+arrow keys)" title="Drag, or Alt+↑/↓">${icon('menu')}</span><span class="num muted small">${k + 1}</span>${svcBadge(id)}<span class="grow">${esc(sv.name)}${sv.note ? ` <span class="muted small">· ${esc(sv.note)}</span>` : ''}</span><button class="icon-btn" data-oup="${k}" title="Earlier" aria-label="Move ${esc(sv.name || id)} earlier" ${k === 0 ? 'disabled' : ''}>${icon('chevron', 'ic up')}</button><button class="icon-btn" data-odown="${k}" title="Later" aria-label="Move ${esc(sv.name || id)} later" ${k === o.order.length - 1 ? 'disabled' : ''}>${icon('chevron')}</button></li>`; }).join('')}</ol>`;
  const bindOrderList = (root, order, save) => {
    const move = (i, j) => { if (j < 0 || j >= order.length || i === j) return; const o = [...order]; const [x] = o.splice(i, 1); o.splice(j, 0, x); save(o); };
    $$('[data-oup]', root).forEach((b) => b.onclick = () => move(Number(b.dataset.oup), Number(b.dataset.oup) - 1));
    $$('[data-odown]', root).forEach((b) => b.onclick = () => move(Number(b.dataset.odown), Number(b.dataset.odown) + 1));
    $$('[data-ogrip]', root).forEach((g) => {
      const from = Number(g.dataset.ogrip);
      g.onkeydown = (e) => { if (e.altKey && (e.key === 'ArrowUp' || e.key === 'ArrowDown')) { e.preventDefault(); move(from, from + (e.key === 'ArrowUp' ? -1 : 1)); } };
      g.onpointerdown = (e) => {
        if (e.button !== 0) return; e.preventDefault(); g.setPointerCapture(e.pointerId);
        const rows = $$('.porder-row', root); const row = rows[from]; let to = from; const startY = e.clientY;
        row.classList.add('dragging');
        g.onpointermove = (ev) => {
          row.style.transform = `translateY(${ev.clientY - startY}px)`;
          to = from; rows.forEach((r, k) => { if (k === from) return; const b = r.getBoundingClientRect(); const mid = b.top + b.height / 2; if (k < from && ev.clientY < mid) to = Math.min(to, k); if (k > from && ev.clientY > mid) to = Math.max(to, k); });
          rows.forEach((r, k) => { if (k === from) return; const h = row.offsetHeight; r.style.transform = to > from && k > from && k <= to ? `translateY(${-h}px)` : to < from && k >= to && k < from ? `translateY(${h}px)` : ''; });
        };
        const end = (commit) => { g.onpointermove = null; g.onpointerup = null; g.onpointercancel = null; rows.forEach((r) => { r.style.transform = ''; }); row.classList.remove('dragging'); if (commit) move(from, to); };
        g.onpointerup = () => end(true); g.onpointercancel = () => end(false);
      };
    });
  };
  // The "Plays from" chip: the resolver's answer for an item (rows, tiles, the item page).
  const playsChip = (it, long = false) => {
    const pf = it.plays_from;
    if (pf) return `<span class="plays" title="${esc(pf.label)}">${svcIcon(pf.service)}<span>${esc(long ? pf.label : (pf.service === 'library' ? 'Library' : SERVICE_NAMES[pf.service] || pf.service))}</span></span>`;
    if (it.connect && it.connect.length) return `<span class="plays none" title="Not on your services">${icon('zone')}<span>${long ? 'Not on your services · ' : ''}${connectLink(it.connect)}</span></span>`;
    return '';
  };
  // Play through the node's connected services: the library, else the best service link
  // (`GET /v1/discover/play`); nothing connected offers Sources and a preview.

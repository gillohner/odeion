  // ---- Now Playing overlay -------------------------------------------------
  const overlay = $('#nowplaying');
  // Back closes it like a menu; opening a page from it takes its history entry (setView)
  const npTr = { open: () => !overlay.hidden, close: (fromBack) => toggleNowPlaying(false, fromBack) };
  let npReturn = null; // (what had the focus when it opened: it gets it back on close)
  const toggleNowPlaying = (force, fromBack) => {
    const open = force === true ? true : force === false ? false : overlay.hidden;
    const was = !overlay.hidden;
    if (open && !was) { const a = document.activeElement; npReturn = a && a !== document.body && !overlay.contains(a) ? a : null; }
    overlay.hidden = !open;
    document.body.classList.toggle('np-open', open);
    // (a toast on screen moves with the bars: Now playing hides the mini player and the tabs)
    { const t = toastBox; if (t.classList.contains('show')) placeToast(t); }
    // (what it covers is out of reach for Tab and screen readers while it is open: the side bar, the
    // page and the phone's tabs; a desktop's player bar is not under it and stays)
    $$('.shell > .sidebar, .shell > .content, .shell > .bottomnav').forEach((el) => { el.inert = open; });
    if (open) { if (!was) { transients.push(npTr); pushTransient(); } renderNowPlaying(); return; }
    const k = transients.indexOf(npTr); if (k >= 0) transients.splice(k, 1);
    const ks = transients.indexOf(npSheetTr); if (ks >= 0) transients.splice(ks, 1);
    if (npCompact()) state.npSheet = null;
    // Focus goes back to what had it when it opened (the ⋯ or row N was pressed on, the mini player
    // clicked), if it is still there; else to the mini player, not to the page's top
    if (was && (!document.activeElement || document.activeElement === document.body || overlay.contains(document.activeElement))) {
      // (not a text field: on a phone its keyboard would come up)
      const r = npReturn; const b = r && r.isConnected && !r.closest('[inert]') && r.getClientRects().length && !r.matches('input, textarea, [contenteditable]') ? r : $('#np');
      if (b) b.focus({ preventScroll: true });
    }
    if (was) npReturn = null;
    if (was && fromBack !== true) popTransient();
  };
  // Swipes on a phone (owner, 2026-11-08: "use swipe gestures … down swipe, up on song, and next song
  // and previous"), with Pointer Events: a finger (or pen) only, never a mouse. The first 10 px decide
  // the direction; sideways, `move` follows the finger and past 40 px (or a quick flick) `left` or
  // `right` runs as it slides out, else it snaps back; up and down run `up` / `down` past 40 / 60 px.
  // A drag that starts on a slider or a field is the slider's. The tap a swipe leaves is dropped.
  const calmMotion = () => matchMedia('(prefers-reduced-motion: reduce)').matches;
  // (how a scripted scroll moves: at once for people who asked for less motion)
  const glide = () => (calmMotion() ? 'auto' : 'smooth');
  const swipeable = (el, o) => {
    let s = null;
    const reset = () => { if (s && s.move) { s.move.classList.remove('dragging'); s.move.style.transform = ''; s.move.style.opacity = ''; } s = null; };
    // (a second finger is not primary and is ignored; a new primary press replaces a gesture whose end
    // never came here)
    el.addEventListener('pointerdown', (e) => {
      if (e.pointerType === 'mouse' || !e.isPrimary) return;
      reset();
      if ((o.when && !o.when(e)) || e.target.closest('input, textarea, select, [data-noswipe]')) return;
      s = { x: e.clientX, y: e.clientY, t: e.timeStamp, id: e.pointerId, dir: null, from: o.origin ? o.origin(e) : null, move: o.move ? o.move(e) : null, xok: o.x ? o.x(e) : true };
    });
    el.addEventListener('pointermove', (e) => {
      if (!s || e.pointerId !== s.id) return;
      const dx = e.clientX - s.x; const dy = e.clientY - s.y;
      if (!s.dir && Math.hypot(dx, dy) > 10) {
        s.dir = Math.abs(dx) > Math.abs(dy) * 1.2 ? 'x' : 'y';
        // The rest of the gesture comes here even when a redraw replaces what the finger pressed
        // (the mini player redraws several times a second while it plays); not on the press, or a
        // tap on a button in it would land here instead
        try { el.setPointerCapture(e.pointerId); } catch { /* not a live pointer (a test's) */ }
      }
      if (s.dir === 'x' && s.xok && s.move && (o.left || o.right) && !calmMotion()) { s.move.classList.add('dragging'); s.move.classList.remove('snap'); s.move.style.transform = `translateX(${dx}px)`; s.move.style.opacity = String(Math.max(0.35, 1 - Math.abs(dx) / 320)); }
    });
    el.addEventListener('pointerup', (e) => {
      if (!s || e.pointerId !== s.id) return;
      const dx = e.clientX - s.x; const dy = e.clientY - s.y; const v = Math.abs(dx) / Math.max(1, e.timeStamp - s.t);
      const g = s; s = null;
      let run = null;
      if (g.dir === 'x' && g.xok && (Math.abs(dx) > 40 || (v > 0.5 && Math.abs(dx) > 20))) run = dx < 0 ? o.left : o.right;
      else if (g.dir === 'y') run = dy < -40 ? o.up : dy > 60 ? o.down : null;
      if (g.dir) el.dataset.swiped = String(Date.now());
      const m = g.move;
      if (m && g.dir === 'x' && g.xok && !calmMotion()) {
        m.classList.remove('dragging'); m.classList.add('snap');
        if (run) {
          // out the side it went, back in from the other (the new song draws meanwhile)
          const w = m.offsetWidth || 300; m.style.transform = `translateX(${dx < 0 ? -w : w}px)`; m.style.opacity = '0';
          setTimeout(() => { m.classList.remove('snap'); m.style.transform = `translateX(${dx < 0 ? w * 0.4 : -w * 0.4}px)`; void m.offsetWidth; m.classList.add('snap'); m.style.transform = ''; m.style.opacity = ''; setTimeout(() => m.classList.remove('snap'), 260); }, 200);
        } else { m.style.transform = ''; m.style.opacity = ''; setTimeout(() => m.classList.remove('snap'), 260); }
      } else if (m) { m.classList.remove('dragging'); m.style.transform = ''; m.style.opacity = ''; }
      if (run) run(g.from);
    });
    el.addEventListener('pointercancel', (e) => { if (s && e.pointerId === s.id) reset(); });
    // (its own capture only: taking it makes the pressed child lose its implicit one, which bubbles here)
    el.addEventListener('lostpointercapture', (e) => { if (e.target === el && s && e.pointerId === s.id) reset(); });
    // (the click a swipe ends with is not a tap)
    // (only a real one: the swipe's own action may press a button in it)
    el.addEventListener('click', (e) => { if (e.isTrusted && Date.now() - Number(el.dataset.swiped || 0) < 400) { e.preventDefault(); e.stopPropagation(); } }, true);
  };
  // Mini player: tap or swipe up opens Now playing, left is the next song, right the previous;
  // down does nothing (no stop by accident)
  swipeable($('.player'), {
    when: () => shellMode() === 'phone' && overlay.hidden,
    move: () => $('#np'),
    left: () => $('#next').click(), right: () => $('#prev').click(),
    up: () => toggleNowPlaying(true),
  });
  // Now playing on one screen (docs/ux/responsive.md, build step 2): the player, and its sheets.
  // Tablets and desktops keep one of five (Queue, Lyrics, Details, Sources, Tags) open beside it.
  // Phones (and windows up to 720 px) show the player alone, on one screen that doesn't scroll, with
  // three buttons under it: Up next, Lyrics and More (Details, Sources, Tags); each opens one
  // full-height sheet with a tab bar, its own scroll, closed by a swipe down or back (owner,
  // 2026-11-08: the one long scroll of 11-05 "feels weird"). The open sheet survives re-renders (state.npSheet).
  const NP_PART = { details: 'Details', sources: 'Sources', tags: 'Tags' };
  const NP_COMPACT = [['queue', 'Up next', 'queue'], ['lyrics', 'Lyrics', 'notes'], ['more', 'More', 'info']];
  const NP_SHEETS_ALL = [['queue', 'Queue', 'queue'], ['lyrics', 'Lyrics', 'notes'], ['details', 'Details', 'info'], ['sources', 'Sources', 'archive'], ['tags', 'Tags', 'compass']];
  const npSheets = () => NP_SHEETS_ALL.filter(([k]) => k !== 'tags' || isOwner()); // listeners cannot tag
  // Compact: the phone arrangement (sheets over the player), also in a desktop window narrower than two columns need
  const npCompact = () => shellMode() === 'phone' || window.innerWidth <= 720;
  // (on a phone Details, Sources and Tags are parts of More: asking for one opens More at it; More
  // asked for on a wider screen is its Details)
  const npSheetNow = () => {
    let k = state.npSheet;
    if (npCompact()) { if (NP_PART[k]) { state.npMoreAt = k; k = state.npSheet = 'more'; } return NP_COMPACT.some(([x]) => x === k) ? k : null; }
    if (k === 'more') k = state.npMoreAt || 'details';
    return npSheets().some(([x]) => x === k) ? k : 'queue';
  };
  const npTabs = () => (npCompact() ? NP_COMPACT : npSheets());
  // What a re-render keeps: the sheet's scroll and the focused control
  const npKeep = () => {
    const a = document.activeElement; const body = $('.np-sheet-body', overlay);
    let focus = null;
    if (a && overlay.contains(a)) {
      const where = a.closest('.np-dock') ? '.np-dock ' : a.closest('.np-sheet-tabs') ? '.np-sheet-tabs ' : a.closest('.np-tabs') ? '.np-tabs ' : a.closest('.np-strip') ? '.np-strip ' : a.closest('.np-transport') ? '.np-transport ' : '';
      if (a.id) focus = '#' + a.id;
      else for (const k of ['npsheet', 'npqi', 'npt']) if (a.dataset && a.dataset[k] != null) { focus = `${where}[data-${k}="${a.dataset[k]}"]`; break; }
    }
    const lines = $('.lyric-lines', overlay);
    return { sheet: overlay.dataset.sheet || '', scroll: body ? body.scrollTop : 0, lyric: lines ? lines.scrollTop : 0, focus };
  };
  // The sheet over a phone's player is a layer of its own for back: back closes it first, then Now playing
  const npSheetTr = { layer: true, open: () => !overlay.hidden && npCompact() && !!state.npSheet && !!$('.np-bsheet:not([hidden])', overlay), close: () => closeNpSheet(true) };
  const closeNpSheet = (fromBack) => {
    const k = transients.indexOf(npSheetTr); if (k >= 0) { transients.splice(k, 1); if (fromBack !== true) popTransient(true); }
    if (!state.npSheet) return;
    const was = state.npSheet; const sh = $('.np-bsheet', overlay);
    const done = () => { state.npSheetClosing = null; if (state.npSheet !== was) return; state.npSheet = null; renderNowPlaying(); const b = $(`.np-dock [data-npsheet="${was}"]`, overlay); if (b && fromBack !== 'quiet') b.focus({ preventScroll: true }); };
    if (sh && !calmMotion()) { sh.classList.add('closing'); state.npSheetClosing = was; setTimeout(done, 190); } else done();
  };
  const renderNowPlaying = () => {
    const keep = npKeep(); npGen += 1; const gen = npGen;
    const prevUri = overlay.dataset.npuri || ''; const prevTransport = overlay.dataset.transport || '';
    const prevImg = $('#nowplaying .np-big img'); const prevSrc = prevImg && prevImg.complete && prevImg.naturalWidth ? (prevImg.currentSrc || prevImg.src) : null;
    const z = currentZone(); const np = z && z.now_playing;
    if (!np) {
      setTimeout(setArtTint, 0); // no cover: no tint
      // (the phone's bar has no zone button or sound pill: both are here, also with nothing playing)
      overlay.innerHTML = `<div class="np-page np2${npCompact() ? ' compact np-idle' : ''}"><div class="np-top"><button class="icon-btn" id="npclose" aria-label="Close Now playing" title="Close">${icon('chevron')}</button><span class="spacer"></span><button class="chip np-zone" id="npzone" aria-label="Playing on ${esc(z ? z.name : 'this zone')}: choose another zone">${icon('zone')}<span class="np-zone-name">${esc(z ? z.name : 'Zone')}</span></button><span class="spacer"></span>${shellMode() === 'phone' ? `<button class="icon-btn" id="npsound" aria-label="Sound" title="Sound">${icon('volume')}</button>` : ''}<button class="btn ghost" id="npq">${icon('queue')} Queue</button></div><div class="empty">${icon('notes')}<h3>Nothing playing on ${esc(z ? z.name : 'this zone')}</h3><p>Pick an album or a song and press play.</p></div></div>`;
      $('#npclose').onclick = () => toggleNowPlaying();
      $('#npq').onclick = () => setView('queue');
      $('#npzone').onclick = () => $('#zonebtn').click();
      { const sb = $('#npsound'); if (sb) sb.onclick = () => openQPanel('#npsound'); }
      { const k = transients.indexOf(npSheetTr); if (k >= 0) { transients.splice(k, 1); popTransient(true); } }
      if (npCompact()) state.npSheet = null;
      overlay.dataset.layout = `${shellMode()}-${npCompact()}`; overlay.dataset.sheet = ''; overlay.dataset.npuri = '';
      // (drawn again, it keeps the focus it had, as with a song: Playing on after a zone picked by keyboard)
      if (keep.focus && (!document.activeElement || document.activeElement === document.body)) { const f = $(keep.focus, overlay); if (f) f.focus({ preventScroll: true }); }
      return;
    }
    const it = np.item;
    const q = (isBrowser() ? state.browserQueue : state.queue) || [];
    const idx = z.queue_index != null && q[z.queue_index] && q[z.queue_index].id === it.id ? z.queue_index : q.findIndex((x) => x.id === it.id);
    const rest = idx >= 0 ? q.slice(idx + 1) : [];
    const h = np.signal_path ? headline(zoneNp(z, np).signal_path) : null;
    const colour = Q_COLOUR;
    const fmtLine = np.signal_path ? fmtFormat(np.source_format) + ' → ' + fmtFormat(np.output_format) : '';
    const sheet = npSheetNow();
    // Phones: the player on one screen, the sheets over it (owner, 2026-11-08)
    const one = npCompact();
    const songPage = npPageId(it.uri);
    const links = np.stream_title ? null : npLinksHtml(it, null);
    const sheetBody = (k) => {
      if (k === 'queue') {
        return `<p class="muted small np-sheet-meta">${rest.length ? `Playing next · ${countOf(rest.length, 'songs')}` : 'Nothing after this song'}</p>`
          + (rest.length ? `<div class="list np-queue">${rest.slice(0, 100).map((n, k2) => `<button class="list-row np-qitem" data-npqi="${idx + 1 + k2}" title="Play this now"><span class="grow"><span class="name">${esc(n.title || n.uri)}</span><span class="sub">${esc(n.artist || '')}</span></span><span class="muted small">${n.duration_ms ? fmt(n.duration_ms) : ''}</span></button>`).join('')}</div>` : '')
          + `<div class="np-sheet-foot"><button class="btn sm ghost" id="npqfull">${icon('queue')} Reorder or clear the queue</button></div>`;
      }
      if (k === 'lyrics') return '<p class="muted" id="nplyricsload">Looking for lyrics…</p><div class="lyrics" id="nplyrics" hidden></div><p class="muted" id="nplyricsnone" hidden>No lyrics found for this song.</p>';
      if (k === 'details') {
        return pathHtml(zoneNp(z, np))
            + `<dl class="kv np-facts">${it.album ? `<dt>Album</dt><dd>${esc(it.album)}</dd>` : ''}${it.artist ? `<dt>Artist</dt><dd>${esc(it.artist)}</dd>` : ''}${np.duration_ms ? `<dt>Length</dt><dd>${fmt(np.duration_ms)}</dd>` : ''}<dt>Playing on</dt><dd>${esc(z.name)}</dd>${(() => { const o = outputFacts(z); return o ? `<dt>Output</dt><dd class="np-output">${esc(o[0])}${o[1] ? `<span class="muted small np-output-why">${esc(o[1])}</span>` : ''}</dd>` : ''; })()}</dl>`;
      }
      // More (phones): Details, Sources and Tags, one heading each, and the song's page once at the end
      if (k === 'more') return Object.keys(NP_PART).filter((x) => x !== 'tags' || isOwner()).map((x) => `<section class="np-part" id="nppart-${x}" aria-labelledby="nppart-h-${x}"><h3 id="nppart-h-${x}">${NP_PART[x]}</h3>${sheetBody(x)}</section>`).join('')
        + (songPage ? `<div class="np-sheet-foot"><button class="btn sm ghost" id="nppage">${icon('compass')} Go to song page</button></div>` : '');
      if (k === 'sources') return `<div id="npsources">${songPage ? '<div class="skeleton" style="height:80px"></div>' : '<p class="muted">This song has no page of its own yet, so its other sources are not known.</p>'}</div>`;
      return '<div class="np-pubky" id="nppubky" aria-live="polite"></div>';
    };
    const tab = ([k, l, ic], cls) => { const on = sheet === k ? ' on' : ''; return `<button class="${cls === 'chip' ? 'chip' : 'np-tab'}${on}" data-npsheet="${k}" ${cls === 'chip' ? `role="tab" id="nptab-${k}" aria-controls="npsheetpanel" aria-selected="${sheet === k}" tabindex="${sheet === k ? 0 : -1}"` : `aria-pressed="${sheet === k}"`}>${cls === 'chip' ? '' : icon(ic)}<span>${l}</span></button>`; };
    const title = np.stream_title ? esc(np.stream_title) : links.title;
    // Phones: three buttons under the player, and the sheet they open (a handle, the song with Play,
    // one tab bar)
    const ctab = ([k, l]) => `<button class="np-stab ${sheet === k ? 'on' : ''}" data-npsheet="${k}" role="tab" id="nptab-${k}" aria-controls="npsheetpanel" aria-selected="${sheet === k}" tabindex="${sheet === k ? 0 : -1}">${l}</button>`;
    const dockHtml = () => `<nav class="np-dock" aria-label="More about this song"${under}>${NP_COMPACT.map(([k, l, ic]) => `<button class="np-dockbtn" data-npsheet="${k}" aria-expanded="${sheet === k}" aria-controls="npbsheet">${icon(ic)}<span>${l}</span></button>`).join('')}</nav>`;
    const bsheetHtml = () => `<section class="np-bsheet${sheet && !keep.sheet ? ' opening' : ''}${sheet && state.npSheetClosing === sheet ? ' closing' : ''}" id="npbsheet" role="dialog" aria-modal="true" aria-label="${sheet ? esc(NP_COMPACT.find(([k]) => k === sheet)[1]) : ''}" data-tab="${sheet || ''}" ${sheet ? '' : 'hidden'}>
            <div class="np-sheet-head"><button class="np-grab" id="npgrab" aria-label="Close the sheet" title="Close (swipe down)"><span aria-hidden="true"></span></button>
              <div class="np-strip"><div class="np-strip-art">${itemArt(it, '', npSvc(np))}</div><div class="np-strip-text"><div class="name">${np.stream_title ? esc(np.stream_title) : esc(it.title || '')}</div><div class="sub">${esc(it.artist || '')}</div></div><button class="icon-btn play" data-npt="toggle" aria-label="${z.transport === 'playing' ? 'Pause' : 'Play'}">${icon(z.transport === 'playing' ? 'pause' : 'play')}</button></div>
              <div class="np-sheet-tabs np-stabs" role="tablist" aria-label="More about this song">${NP_COMPACT.map(ctab).join('')}</div></div>
            <div class="np-sheet-body" role="tabpanel" id="npsheetpanel" ${sheet ? `aria-labelledby="nptab-${sheet}"` : ''} tabindex="-1">${sheet ? sheetBody(sheet) : ''}</div>
          </section>`;
    const artist = np.stream_title ? esc(it.artist || '') : links.artistsHtml;
    const under = one && sheet ? ' inert' : ''; // (what the phone's sheet covers)
    overlay.innerHTML = `
      <div class="np-page np2${sheet && !one ? ' has-sheet' : ''}${one ? ' compact' : ''}">
        <div class="np-top"${under}><button class="icon-btn" id="npclose" aria-label="Close Now playing" title="Close (swipe down)">${icon('chevron')}</button><span class="spacer"></span><button class="chip np-zone" id="npzone" aria-label="Playing on ${esc(z.name)}: choose another zone">${icon('zone')}<span class="np-zone-name">${esc(z.name)}</span></button><span class="spacer"></span><button aria-haspopup="menu" aria-expanded="false" class="icon-btn" id="npmore" aria-label="More: dashboard, sleep timer${isBrowser() ? '' : ', DSP'}" title="More">${icon('more')}</button></div>
        <div class="np-body"${under}>
          <section class="np-player" aria-label="Now playing">
            <div class="np-big">${itemArt(it, '', npSvc(np))}</div>
            <div class="np-titlerow"><div class="np-titles">${np.stream_title ? `<div class="eyebrow">Live · ${esc(it.title || '')}</div>` : ''}<h1>${title}</h1><div class="artist">${artist}</div></div><span id="npheart"></span></div>
            <button class="quality np-quality" id="npquality" title="How the sound gets to you"><span class="qdot" style="background:${h ? colour[h] : 'var(--muted)'}"></span><span>${streamLabel(np) ? esc(streamLabel(np)) : `${h ? LABEL[h] : hereName()}${fmtLine ? ` · ${esc(fmtFormat(np.source_format))}` : ''}`}</span></button>
            <div class="np-progress"><input type="range" id="npseek" min="0" max="1000" value="${np.duration_ms ? Math.round((np.position_ms / np.duration_ms) * 1000) : 0}" aria-label="Seek"><div class="muted small np-times"><span id="nppos">${fmt(np.position_ms)}</span><span>${fmt(np.duration_ms)}</span></div></div>
            <div class="np-transport" role="group" aria-label="Playback"><button class="icon-btn mode-btn" data-npt="shuffle" data-repeat aria-pressed="false" aria-label="Shuffle: off">${icon('shuffle')}</button><button class="icon-btn" data-npt="prev" data-repeat aria-label="Previous">${icon('prev')}</button><button class="icon-btn play" data-npt="toggle" aria-label="${z.transport === 'playing' ? 'Pause' : 'Play'}">${icon(z.transport === 'playing' ? 'pause' : 'play')}</button><button class="icon-btn" data-npt="next" data-repeat aria-label="Next">${icon('next')}</button><button class="icon-btn mode-btn" data-npt="repeat" data-repeat aria-pressed="false" aria-label="Repeat: off">${icon('repeat')}</button></div>${zoneVolume(z) ? `<div class="np-vol">${icon('volume')}<input type="range" id="npvol" aria-label="Volume of ${esc(z.name)}"><span class="vol-level" data-vollevel="${esc(z.id)}"></span></div>` : ''}
            ${one ? '' : `<nav class="np-tabs" aria-label="More about this song">${npSheets().map((t) => tab(t, 'np-tab')).join('')}</nav>`}
          </section>
          ${one ? '' : `          <section class="np-sheet" ${sheet ? '' : 'hidden'} aria-label="${sheet ? esc(NP_SHEETS_ALL.find(([k]) => k === sheet)[1]) : ''}">
            <div class="np-strip"><div class="np-strip-art">${itemArt(it, '', npSvc(np))}</div><div class="np-strip-text"><div class="name">${np.stream_title ? esc(np.stream_title) : esc(it.title || '')}</div><div class="sub">${esc(it.artist || '')}</div></div><button class="icon-btn play" data-npt="toggle" aria-label="${z.transport === 'playing' ? 'Pause' : 'Play'}">${icon(z.transport === 'playing' ? 'pause' : 'play')}</button></div>
            <div class="np-sheet-head"><span class="grab" aria-hidden="true"></span><div class="chips np-sheet-tabs" role="tablist" aria-label="More about this song">${npSheets().map((t) => tab(t, 'chip')).join('')}</div><button class="icon-btn np-sheet-close" id="npsheetclose" aria-label="Close the sheet" title="Close">${icon('close')}</button></div>
            <div class="np-sheet-body" role="tabpanel" id="npsheetpanel" ${sheet ? `aria-labelledby="nptab-${sheet}"` : ''} tabindex="-1">${sheet ? sheetBody(sheet) : ''}</div>
          </section>`}
        </div>
        ${one ? dockHtml() + bsheetHtml() : ''}
      </div>`;
    const sk = $('#npseek'); setFill(sk);
    // A track change: the last cover fades out over the new one, the title slides in; a sheet change
    // fades its panel in; the cover's rest state follows the transport
    // (each marker is kept in state for its animation's length: a second redraw can follow at once,
    // from the queue's refresh, and must not wipe it)
    const now = Date.now();
    if (prevUri && prevUri !== (it.uri || '')) state.npSwap = { at: now, src: prevSrc };
    const swap = state.npSwap && now - state.npSwap.at < 400;
    overlay.classList.toggle('np-swap', !!swap);
    if (swap) { const t = $('.np-titles', overlay); if (t) t.style.animationDelay = `-${now - state.npSwap.at}ms`; }
    overlay.classList.toggle('np-sheet-swap', !one && (sheet || '') !== (keep.sheet || ''));
    if (swap && state.npSwap.src) { const big = $('.np-big', overlay); if (big) { const old = new Image(); old.className = 'np-prev'; old.alt = ''; old.src = state.npSwap.src; old.style.animationDelay = `-${now - state.npSwap.at}ms`; old.onanimationend = () => old.remove(); big.appendChild(old); setTimeout(() => old.remove(), 1200); } }
    overlay.dataset.transport = z.transport || '';
    // A transport change that came with this redraw pops the play icon as the tick would
    if (prevTransport && prevTransport !== (z.transport || '')) state.npPop = now;
    if (state.npPop && now - state.npPop < 300) $$('[data-npt="toggle"] .ic', overlay).forEach((ic) => { ic.style.animationDelay = `-${now - state.npPop}ms`; ic.classList.add('swap'); });
    // (the tick doesn't redraw again for this item)
    renderNowPlaying.lastItem = it; renderNowPlaying.lastTitle = np.stream_title;
    renderNowPlaying.lastOutput = JSON.stringify(outputFacts(z));
    if (!one) {
      const tabs = $('.np-sheet-tabs', overlay); const on = tabs && $('.chip.on', tabs);
      if (tabs && on) {
        // (from where the last render's mark was, so it slides)
        const was = state.npTabMark; if (was) { tabs.style.setProperty('--tab-x', was.x); tabs.style.setProperty('--tab-w', was.w); }
        const next = { x: `${on.offsetLeft}px`, w: String(on.offsetWidth / 100) }; state.npTabMark = next;
        requestAnimationFrame(() => { tabs.style.setProperty('--tab-x', next.x); tabs.style.setProperty('--tab-w', next.w); });
      }
    }
    // Now playing's own transport (phones: the mini player hides behind it)
    // Volume in Now playing (phones and windows under 760 px have none in the bar; UX sweep 11-01, A6):
    // it moves the bar's own slider, which sets the zone's volume
    const nv = $('#npvol', overlay); if (nv) { applyVol(nv, zoneVolume(z)); bindVolSlider(nv, currentZone); syncVolumeSliders(z); }
    else if (z && z.id !== BROWSER && isOwner() && !state.outputs && !state.outputsP) state.outputsP = loadOutputs().then(() => { if (!overlay.hidden) renderNowPlaying(); }).catch(() => {}).finally(() => { state.outputsP = null; });
    // The bar's own handlers, called directly; the modes' state comes from renderModes (as the bar's)
    const npAct = { shuffle: toggleShuffle, repeat: cycleRepeat, prev: () => transport({ action: 'previous' }), toggle: () => transport({ action: 'toggle' }), next: () => transport({ action: 'next' }) };
    $$('[data-npt]', overlay).forEach((b) => b.onclick = () => npAct[b.dataset.npt]());
    renderModes();
    sk.addEventListener('input', () => { sk.dataset.dragging = '1'; setFill(sk); $('#nppos').textContent = fmt((sk.value / 1000) * (np.duration_ms || 0)); sk.setAttribute('aria-valuetext', `${fmt((sk.value / 1000) * (np.duration_ms || 0))} of ${fmt(np.duration_ms)}`); });
    sk.addEventListener('change', () => { delete sk.dataset.dragging; transport({ action: 'seek', position_ms: Math.round((sk.value / 1000) * (np.duration_ms || 0)) }); });
    // (a drag that ends without a change, or is cancelled, lets go too)
    for (const ev of ['pointerup', 'pointercancel', 'touchend', 'touchcancel']) sk.addEventListener(ev, () => { setTimeout(() => { delete sk.dataset.dragging; }, 0); }, { passive: true });
    $('#npclose').onclick = () => toggleNowPlaying();
    $('#npzone').onclick = () => $('#zonebtn').click();
    const openSheet = (k) => {
      state.npSheet = k; renderNowPlaying();
      const f = $(`.np-sheet-tabs [data-npsheet="${npSheetNow()}"]`, overlay); if (f) f.focus({ preventScroll: true });
    };
    $$('[data-npsheet]', overlay).forEach((b) => b.onclick = () => openSheet(b.dataset.npsheet));
    // Tabs: arrow keys move between them (WAI-ARIA tabs)
    const stabs = $('.np-sheet-tabs', overlay); if (stabs) stabs.onkeydown = (e) => {
      const ks = npTabs().map(([k]) => k); const i = ks.indexOf(sheet); if (i < 0) return;
      const j = e.key === 'ArrowRight' ? (i + 1) % ks.length : e.key === 'ArrowLeft' ? (i - 1 + ks.length) % ks.length : e.key === 'Home' ? 0 : e.key === 'End' ? ks.length - 1 : -1;
      if (j >= 0) { e.preventDefault(); e.stopPropagation(); openSheet(ks[j]); }
    };
    const sclose = $('#npsheetclose'); if (sclose) sclose.onclick = () => { const was = sheet; state.npSheet = null; renderNowPlaying(); const t = $(`.np-tabs [data-npsheet="${was}"]`, overlay) || $('.np-tabs [data-npsheet]', overlay); if (t) t.focus(); };
    // The quality line opens Details (the signal path)
    $('#npquality').onclick = () => openSheet('details');
    if (one) {
      // (the swipes are bound once, on the overlay: see npSwipes)
      const sh = $('.np-bsheet', overlay);
      if (sh && sheet) {
        $('#npgrab', sh).onclick = () => closeNpSheet();
        if (sheet === 'more') {
          const pb = $('#nppage', sh); if (pb) pb.onclick = () => setView('item', { kind: 'song', id: songPage });
          // More opened at a part (the quality line: Details; a link: Sources or Tags)
          if (state.npMoreAt && state.npMoreAt !== 'details' && keep.sheet !== 'more') { const part = $(`#nppart-${state.npMoreAt}`, sh); const body = $('.np-sheet-body', sh); if (part && body) body.scrollTop = part.offsetTop - body.offsetTop; }
          state.npMoreAt = null;
        }
      }
    }
    $('#npmore').onclick = (e) => { const b = e.currentTarget; showMenu(b, [
      // (a phone's bar has no sound pill: its panel opens from here)
      ...(shellMode() === 'phone' ? [{ label: 'Sound', note: 'Where it plays from, how it gets to you, DSP', icon: 'volume', run: () => setTimeout(() => openQPanel('#npmore'), 0) }] : []),
      { label: 'Dashboard', note: 'Full screen, for a TV or tablet', icon: 'external', run: () => window.open(`/now-playing?zone=${encodeURIComponent(state.zone)}`, '_blank', 'noopener') },
      { label: sleepLabel() === 'Sleep' ? 'Sleep timer' : sleepLabel(), icon: 'moon', run: () => setTimeout(() => showMenu(b, [{ title: 'Sleep timer' }, ...[15, 30, 45, 60, 90].map((m) => ({ label: `${m} minutes`, icon: 'moon', run: () => setSleep(m * 60) })), { label: 'Off', icon: 'close', run: () => setSleep(null) }]), 0) },
      ...(isBrowser() ? [] : [{ label: 'DSP', note: 'Equaliser, crossfeed, room correction', icon: 'settings', run: () => { state.dspOpen = state.zone; setView('settings', 'playback'); } }]),
      ...(songPage ? [{ label: 'Go to song page', icon: 'compass', run: () => setView('item', { kind: 'song', id: songPage }) }] : []),
    ]); };
    // (the page opened replaces the one the focus came from: it goes to the mini player, as in setView)
    bindNpLinks(overlay, it, () => { npReturn = null; toggleNowPlaying(); });
    // (the row is busy until the zone answers, so a second press sends nothing; drawn again after it)
    $$('[data-npqi]', overlay).forEach((b) => b.onclick = () => transport({ action: 'play', index: Number(b.dataset.npqi) }).then(() => { setTimeout(renderNowPlaying, 500); }));
    const qf = $('#npqfull'); if (qf) qf.onclick = () => setView('queue');
    overlay.dataset.sheet = sheet || ''; overlay.dataset.npuri = it.uri || ''; overlay.dataset.layout = `${shellMode()}-${npCompact()}`;
    state.npWant = { gen, at: Date.now(), scroll: sheet && keep.sheet === sheet ? keep.scroll : null, lyric: sheet === 'lyrics' && keep.sheet === 'lyrics' ? keep.lyric : null, focus: keep.focus };
    npRestore();
    if ((sheet === 'sources' || (one && sheet === 'more')) && songPage) fillNpSources(songPage, gen);
    // Back closes an open sheet first: its layer is registered whenever one shows (a resize into the
    // phone arrangement with a sheet chosen shows one too), and dropped when none does
    { const k = transients.indexOf(npSheetTr); if (one && sheet) { if (k < 0 && state.npSheetClosing !== sheet) { transients.push(npSheetTr); pushTransient(true); } } else if (k >= 0) { transients.splice(k, 1); popTransient(true); } }
    fillNpPubky(it, gen);
    loadLyrics(it);
    setArtTint();
    npRestore();
    refreshSleep();
  };
  // Now playing's swipes on a phone, bound once on the overlay (a render replaces everything in it, and
  // the gesture must survive that): on the first screen down closes Now playing, up opens Up next, and
  // the cover goes to the next song left and the previous one right, following the finger; on the
  // sheet a swipe down on its head closes it, and sideways anywhere on it moves between its tabs
  {
    const npStep = (d) => { const ks = NP_COMPACT.map(([k]) => k); const j = ks.indexOf(state.npSheet) + d; if (j >= 0 && j < ks.length) { state.npSheet = ks[j]; renderNowPlaying(); const f = $(`.np-sheet-tabs [data-npsheet="${ks[j]}"]`, overlay); if (f) f.focus({ preventScroll: true }); } };
    // (where the press was, decided when it happens: the sheet's list can be redrawn under the finger,
    // Sources when its answer comes, and then the pressed element is no longer in the sheet)
    const origin = (e) => (e.target.closest('.np-sheet-head') ? 'head' : e.target.closest('.np-bsheet') ? 'sheet' : e.target.closest('.np-big') ? 'cover' : 'screen');
    const inSheet = (o) => o === 'head' || o === 'sheet';
    swipeable(overlay, {
      when: () => npCompact() && !!$('.np2.compact:not(.np-idle)', overlay),
      origin,
      move: (e) => (!state.npSheet && origin(e) === 'cover' ? $('.np-big', overlay) : null),
      x: (e) => (state.npSheet ? inSheet(origin(e)) : origin(e) === 'cover'),
      left: (o) => { if (inSheet(o)) npStep(1); else if (o === 'cover') { $('#next').click(); setTimeout(renderNowPlaying, 400); } },
      right: (o) => { if (inSheet(o)) npStep(-1); else if (o === 'cover') { $('#prev').click(); setTimeout(renderNowPlaying, 400); } },
      up: (o) => { if (!state.npSheet && !inSheet(o)) { state.npSheet = 'queue'; renderNowPlaying(); const f = $('.np-sheet-tabs [data-npsheet="queue"]', overlay); if (f) f.focus({ preventScroll: true }); } },
      down: (o) => { if (o === 'head') closeNpSheet(); else if (!inSheet(o) && !state.npSheet) toggleNowPlaying(); },
    });
  }
  // The quality sheet (STEWARD-UX-PLAYER-SOURCES §2), from the player's quality label: what is
  // arriving, what the zone makes of it and why, the service's quality choice, and the way to the
  // Play from order and the zone's output settings.
  // Put back what the last render had (scroll, focus) once the content it needs is there; for that
  // render only, and only in its first seconds so a late answer never yanks the view
  const npRestore = () => {
    const w = state.npWant; if (!w || w.gen !== npGen || Date.now() - w.at > 3000) return;
    const body = $('.np-sheet-body', overlay); if (body && w.scroll != null && body.scrollHeight - body.clientHeight >= w.scroll) { body.scrollTop = w.scroll; w.scroll = null; }
    const lines = $('.lyric-lines', overlay); if (lines && w.lyric != null) { lines.scrollTop = w.lyric; w.lyric = null; }
    const free = !document.activeElement || document.activeElement === document.body || !overlay.contains(document.activeElement); // never take focus someone has moved since
    if (w.focus && free) { const f = $(w.focus, overlay); if (f) { f.focus({ preventScroll: true }); if (f.id === 'nptagadd') f.setSelectionRange(f.value.length, f.value.length); w.focus = null; } }
  };
  // A change of arrangement (rotating a phone, resizing a window) redraws an open Now playing
  window.addEventListener('resize', () => { if (!overlay.hidden && overlay.dataset.layout && overlay.dataset.layout !== `${shellMode()}-${npCompact()}`) renderNowPlaying(); });
  // The Sources sheet: where else this song plays (the song page's "also on")
  const npSrcCache = new Map(); const npSrcPending = new Map();
  const fillNpSources = async (songPage, gen = npGen) => {
    let pg = npSrcCache.get(songPage);
    if (!pg) {
      let req = npSrcPending.get(songPage);
      if (!req) { req = api('GET', `/item/song/${encodeURIComponent(songPage)}`).catch(() => null).finally(() => npSrcPending.delete(songPage)); npSrcPending.set(songPage, req); }
      pg = await req;
      if (pg && !pg.reading && !pg.pending) { npSrcCache.set(songPage, pg); if (npSrcCache.size > 50) npSrcCache.delete(npSrcCache.keys().next().value); }
    }
    const box = $('#npsources', overlay); if (!box || gen !== npGen) return;
    // Play from another source (TASK-pinakes-social-ux A4): the same song, where it is, on a node
    // zone; only from the library or a service that is on and working here (a source the node
    // can't play would leave the zone stopped)
    const rk = await loadSvcRank().catch(() => null);
    if (!$('#npsources', overlay) || gen !== npGen) return;
    const also = rankSvc((pg && pg.also_on) || [], (v) => srcSvc(v.source));
    const z0 = currentZone(); const cur = z0 && z0.now_playing ? z0.now_playing.item : null;
    // Full songs only: a service that gives 30 s previews here, or YouTube (only This browser plays it),
    // would swap the song for something less
    const previewOnly = (svc) => /^previews\b/i.test(String(((state.sources || []).find((x) => x.id === svc) || {}).status || ''));
    const canPlay = (v) => { const svc = srcSvc(v.source); if (svc === 'youtube' || /^youtube:/.test(switchUri(v) || '') || previewOnly(svc)) return false; return rk ? rk.healthy.has(svc) : svc === 'library'; };
    const canSwitch = (v) => !isBrowser() && cur && switchUri(v) && switchUri(v) !== cur.uri && canPlay(v);
    box.innerHTML = (also.length ? `<div class="list">${also.map((v, k) => `<div class="list-row">${svcBadge(srcSvc(v.source))}<span class="grow name">${esc(v.label || srcName(srcSvc(v.source)))}</span>${canSwitch(v) ? `<button class="btn sm" data-npswitch="${k}" aria-label="Play from here: ${esc(v.label || srcName(srcSvc(v.source)))}">${icon('play')} Play from here</button>` : ''}${webUrl(v.url) ? `<a class="icon-btn" href="${esc(webUrl(v.url))}" target="_blank" rel="noopener" aria-label="Open in ${esc(v.label || v.source)} (new tab)" title="Open in ${esc(v.label || v.source)} (new tab)">${icon('external')}</a>` : ''}</div>`).join('')}</div>` : '<p class="muted">No other sources found for this song.</p>')
      + ($('.np-bsheet', overlay) ? '' : `<div class="np-sheet-foot"><button class="btn sm ghost" id="npsongpage">${icon('compass')} Go to song page</button></div>`);
    const sp = $('#npsongpage', overlay); if (sp) sp.onclick = () => setView('item', { kind: 'song', id: songPage });
    $$('[data-npswitch]', box).forEach((b) => b.onclick = () => switchSource(also[Number(b.dataset.npswitch)], b, cur.id));
    npRestore();
  };
  // What a source's entry plays as in a queue: a library track, a service's own uri, a YouTube video
  const switchUri = (v) => (v.uri && (v.source !== 'library' || v.uri.startsWith('track:')) ? v.uri : ytOf(v.url) ? `youtube:video:${ytOf(v.url)}` : null);
  // The playing song moves to another source at the same position: the zone's queue is loaded
  // again with that one item swapped (repeat, radio and paused-or-playing stay)
  // `itemId`: the song the sheet was drawn for; if another one plays by now, nothing changes
  const switchSource = async (v, b, itemId) => {
    const zone = state.zone; const uri = switchUri(v);
    const playingIt = () => { const z = state.zones[zone]; return z && z.now_playing && z.now_playing.item.id === itemId ? z : null; };
    if (!uri || !playingIt()) { toast('Another song is playing now: open Sources again', 'error'); return; }
    const name = v.label || srcName(srcSvc(v.source));
    b.disabled = true; b.innerHTML = '<span class="spin"></span> Switching…';
    const z0 = playingIt(); const version = z0.queue_version;
    const wait = z0.transport === 'playing' ? startWait('play', zone, `${z0.now_playing.item.title || ''} · from ${name}`, true, true, false) : null;
    try {
      // (read and loaded in one turn of the zone, so nothing of its queue changes in between)
      const zs = await zoneOp(zone, async (t) => {
        let queue = await api('GET', `/zones/${encodeURIComponent(zone)}/queue`, undefined, { signal: t.signal });
        // The queue changed meanwhile (a song added elsewhere): read it again, so nothing is lost
        if (state.zones[zone] && state.zones[zone].queue_version !== version) queue = await api('GET', `/zones/${encodeURIComponent(zone)}/queue`, undefined, { signal: t.signal });
        const at = queue.findIndex((q) => q.id === itemId);
        const z = playingIt();
        if (at < 0 || !z) throw new Error('another song is playing now: open Sources again');
        // The new source's own cover and length: the old item's cover stays only between two items
        // of the same service (a library file shows its album's); the length comes with the source
        const scheme = (u) => String(u || '').split(':')[0];
        const items = queue.map((q, k) => (k === at ? { ...q, uri, image: scheme(uri) !== 'track' && scheme(uri) === scheme(q.uri) ? q.image || null : null, duration_ms: null } : q));
        // (the start's 20 s run from here, after the reads; the song it replaces is the one playing now)
        if (wait) startSending(wait, zone);
        return api('POST', `/zones/${encodeURIComponent(zone)}/load`, { items, index: at, position_ms: z.now_playing.position_ms || 0, transport: z.transport === 'paused' ? 'paused' : 'playing', repeat: z.repeat || 'off', radio: !!z.radio }, { signal: t.signal });
        // (no shuffle: the node would draw the queue again from its play order, songs heard included;
        // keeping the mode with its order needs a node call that swaps one item's source in place)
      });
      // The zone's answer says whether the new source plays (a paused zone sends no event to wait for)
      if (zs && zs.id) state.zones[zs.id] = zs;
      if (zs && zs.error) throw new Error(zs.error);
      // (the node took the load: a zone playing another song now ends the wait, here or by its event)
      if (wait) startSent(wait);
      toast(`Playing from ${name}`);
      refreshQueue();
    } catch (e) { if (wait) startFail(e.message, true, wait); toast(e.message, 'error'); if (b.isConnected) { b.disabled = false; b.innerHTML = `${icon('play')} Play from here`; } }
  };
  // Sleep timer per zone (node zones through the API, the browser zone locally).
  const sleepLabel = () => { const at = state.sleepAt[state.zone]; if (!at || at <= Date.now()) return 'Sleep'; const m = Math.ceil((at - Date.now()) / 60000); return `Sleep in ${m} min`; };
  const setSleep = async (seconds) => {
    if (isBrowser()) {
      clearTimeout(state.browserSleepTimer);
      if (seconds) { state.sleepAt[BROWSER] = Date.now() + seconds * 1000; state.browserSleepTimer = setTimeout(() => { browserPause(); delete state.sleepAt[BROWSER]; renderNowPlaying(); }, seconds * 1000); } else delete state.sleepAt[BROWSER];
      toast(seconds ? `Sleeping in ${Math.round(seconds / 60)} minutes` : 'Sleep timer off'); renderNowPlaying(); return;
    }
    try { const v = await api('PUT', `/zones/${encodeURIComponent(state.zone)}/sleep`, { seconds }); if (v.ends_at_ms) state.sleepAt[state.zone] = v.ends_at_ms; else delete state.sleepAt[state.zone]; toast(seconds ? `Sleeping in ${Math.round(seconds / 60)} minutes` : 'Sleep timer off'); renderNowPlaying(); } catch (e) { toast(e.message, 'error'); }
  };
  const refreshSleep = async () => {
    if (isBrowser()) return;
    try { const v = await api('GET', `/zones/${encodeURIComponent(state.zone)}/sleep`); if (v.ends_at_ms) state.sleepAt[state.zone] = v.ends_at_ms; else delete state.sleepAt[state.zone]; } catch { /* keep */ }
  };
  // Lyrics for the playing library track (tags or a sidecar .lrc); synced lines follow the position.
  const trackIdOf = (it) => { const m = /^track:(\d+)$/.exec(it && it.uri || ''); return m ? Number(m[1]) : null; };
  const renderLyrics = (l) => {
    const box = $('#nplyrics'); if (!box) return;
    const none = $('#nplyricsnone'); if (none) none.hidden = !!l;
    const ld = $('#nplyricsload'); if (ld) ld.hidden = true;
    setTimeout(npRestore, 0);
    if (!l) { box.hidden = true; box.innerHTML = ''; return; }
    box.hidden = false;
    const off = state.lyricsOffset || 0;
    // (one heading: the sheet's tab says Lyrics; this row says how they are timed and where from,
    // with the timing nudge as two full-size buttons on its right)
    const how = [l.synced ? 'Synced' : '', l.source === 'lrclib' ? 'from <a class="muted" href="https://lrclib.net" target="_blank" rel="noopener">LRCLIB</a>' : ''].filter(Boolean).join(' · ');
    box.innerHTML = `<div class="lyric-head"><span class="muted small lyric-how">${how}</span>${l.synced ? `<span class="lyric-nudge"><span class="muted small lyric-off">${off ? `${off > 0 ? '+' : ''}${(off / 1000).toFixed(1)} s` : ''}</span><button class="icon-btn" data-lnudge="-500" title="Lines earlier (0.5 s)" aria-label="Show lines half a second earlier">−</button><button class="icon-btn" data-lnudge="500" title="Lines later (0.5 s)" aria-label="Show lines half a second later">+</button></span>` : ''}</div><div class="lyric-lines${l.synced ? ' synced' : ''}">${l.lines.map((ln) => `<p class="lyric-line" ${ln.ms != null ? `data-ms="${ln.ms}" role="button" tabindex="0" title="Play from here"${ln.text ? '' : ' aria-label="Instrumental: play from here"'}` : ''}>${esc(ln.text) || icon('notes')}</p>`).join('')}</div>`;
    // Synced: a tap on a line plays from it; a scroll by hand pauses the following for a few seconds
    const lines = $('.lyric-lines', box);
    if (l.synced && lines) {
      // (a line the listener chose is followed at once: the hold their tap set is over)
      // (the hold their tap set is over once the seek has landed: not before, or the panel would
      // first go back to the old line)
      // (only the hold of that tap: a scroll by hand while the seek is on its way is newer, and stays)
      const seekTo = (ln) => { const stamp = state.lyricsUser; return Promise.resolve(transport({ action: 'seek', position_ms: Math.max(0, Number(ln.dataset.ms) + (state.lyricsOffset || 0)) })).finally(() => { if (state.lyricsUser === stamp) { state.lyricsHold = 0; state.lyricsUser = 0; } }); };
      lines.addEventListener('click', (e) => { const ln = e.target.closest('.lyric-line[data-ms]'); if (ln) seekTo(ln); });
      // (Space and Enter on a line are the line's: the page's Space is play/pause)
      lines.addEventListener('keydown', (e) => { if (e.key !== 'Enter' && e.key !== ' ') return; const ln = e.target.closest('.lyric-line[data-ms]'); if (ln) { e.preventDefault(); e.stopPropagation(); seekTo(ln); } });
      // A scroll counts as the listener's only right after their touch, wheel or key: the page's own
      // scrolls (the follower, a redraw putting the place back) don't hold the following off
      // (the listener's own input holds at once, and a scroll right after it holds again)
      const mine = () => { state.lyricsUser = Date.now(); state.lyricsHold = Date.now() + 4000; };
      lines.addEventListener('touchstart', mine, { passive: true });
      lines.addEventListener('touchmove', mine, { passive: true });
      lines.addEventListener('wheel', mine, { passive: true });
      lines.addEventListener('focusin', mine);
      lines.addEventListener('keydown', (e) => { if (/^(Arrow|Page|Home|End)/.test(e.key)) mine(); });
      lines.addEventListener('scroll', () => { if (lines.dataset.auto) return; if (state.lyricsUser && Date.now() - state.lyricsUser < 1500) state.lyricsHold = Date.now() + 4000; }, { passive: true });
    }
  };
  // Library songs: their own lyrics, else LRCLIB (the node decides); streamed songs: LRCLIB by title
  const lyricsKey = (it) => { const id = trackIdOf(it); return id != null ? `t${id}` : it && it.title && it.artist ? `s${it.uri || ''}|${it.title}|${it.artist}` : null; };
  const loadLyrics = async (it) => {
    const key = lyricsKey(it); const id = trackIdOf(it);
    if (!key || (it && it.stream && /^https?:/.test(it.uri || '') && it.album === 'Internet radio')) { renderLyrics(null); return; }
    if (state.lyrics.has(key)) { renderLyrics(state.lyrics.get(key)); return; }
    let l = null;
    try { l = id != null ? await api('GET', `/library/tracks/${id}/lyrics`) : await api('GET', `/lyrics?${new URLSearchParams({ title: it.title, artist: String(it.artist).split(/,| & /)[0], ...(it.album ? { album: it.album } : {}), ...(it.duration_ms ? { duration_ms: String(it.duration_ms) } : {}) })}`); } catch { l = null; }
    state.lyrics.set(key, l);
    if (lyricsKey(currentNowPlayingItem()) === key) { state.lyricsOffset = 0; renderLyrics(l); }
  };
  document.addEventListener('click', (e) => { const b = e.target.closest && e.target.closest('[data-lnudge]'); if (!b) return; state.lyricsOffset = (state.lyricsOffset || 0) + Number(b.dataset.lnudge); const it = currentNowPlayingItem(); const l = state.lyrics.get(lyricsKey(it)); if (l) renderLyrics(l); });
  const currentNowPlayingItem = () => { const z = currentZone(); return z && z.now_playing ? z.now_playing.item : null; };
  const followLyrics = (positionMs) => {
    const lines = $$('#nplyrics .lyric-line[data-ms]'); if (!lines.length) return;
    const at = positionMs - (state.lyricsOffset || 0);
    let k = -1; for (let i = 0; i < lines.length; i++) { if (Number(lines[i].dataset.ms) <= at) k = i; else break; }
    const cur = k >= 0 ? lines[k] : null;
    // The current line, its neighbours by distance (dimmer the further away)
    lines.forEach((ln, i) => { const d = k < 0 ? 9 : Math.abs(i - k); ln.classList.toggle('on', d === 0); ln.classList.toggle('near1', d === 1); ln.classList.toggle('near2', d === 2); });
    if (!cur) return;
    // The box keeps the current line about a third of the way down (only the lyrics' own box
    // scrolls: on a phone's one screen the page holds them too); a scroll by hand holds it off for
    // a few seconds, then it eases back
    const box = cur.closest('.lyric-lines'); if (!box) return;
    const held = state.lyricsHold && Date.now() < state.lyricsHold;
    if (held) { delete cur.dataset.seen; return; }
    if (!cur.dataset.seen) {
      lines.forEach((ln) => delete ln.dataset.seen); cur.dataset.seen = '1';
      const top = cur.getBoundingClientRect().top - box.getBoundingClientRect().top + box.scrollTop - box.clientHeight / 3 + cur.offsetHeight / 2;
      box.dataset.auto = '1'; clearTimeout(box.autoT);
      try { box.scrollTo({ top: Math.max(0, top), behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' }); } catch { /* jsdom */ }
      box.autoT = setTimeout(() => { delete box.dataset.auto; }, 700);
    }
  };
  // Keep the overlay's progress moving without rebuilding it on every tick.
  setInterval(() => {
    if (overlay.hidden) return;
    const z = currentZone(); const np = z && z.now_playing; const s = $('#npseek');
    if (np && s && !s.dataset.dragging) { const words = `${fmt(np.position_ms)} of ${fmt(np.duration_ms)}`; if (s.getAttribute('aria-valuetext') !== words) s.setAttribute('aria-valuetext', words); }
    if (!np || !s || s.dataset.dragging) return;
    if (np.item.id !== (renderNowPlaying.lastItem || {}).id || np.stream_title !== renderNowPlaying.lastTitle || JSON.stringify(outputFacts(z)) !== renderNowPlaying.lastOutput) { renderNowPlaying.lastTitle = np.stream_title; renderNowPlaying(); }
    if ((z.transport || '') !== (overlay.dataset.transport || '')) {
      overlay.dataset.transport = z.transport || '';
      const playing = z.transport === 'playing';
      $$('[data-npt="toggle"]', overlay).forEach((b) => { b.innerHTML = icon(playing ? 'pause' : 'play'); b.setAttribute('aria-label', playing ? 'Pause' : 'Play'); const ic = $('.ic', b); if (ic) ic.classList.add('swap'); });
    }
    s.value = np.duration_ms ? Math.round((np.position_ms / np.duration_ms) * 1000) : 0; setFill(s);
    const p = $('#nppos'); if (p) p.textContent = fmt(np.position_ms);
    followLyrics(np.position_ms);
    renderNowPlaying.lastItem = np.item;
  }, 500);


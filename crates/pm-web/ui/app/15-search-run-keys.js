  const search = $('#search');
  if (window.matchMedia && matchMedia('(max-width: 600px)').matches) search.placeholder = 'Search music';
  let searchTimer = null;
  // ---- one search, scoped, streamed per source (STEWARD-UX-HOME.md "Search") ----
  const searchRuns = new Map(); // `${q}|${scope}` → { q, scope, sources, answers, startedAt, firstAt, doneAt, ctrl }
  const SEARCH_TTL = 5 * 60 * 1000;
  // One run per term and scope, whatever its case; a pasted link keeps its case (ids in paths differ by it)
  const searchKey = (q, scope) => `${/^https?:\/\//i.test(q || '') ? q : (q || '').toLowerCase()}|${scope}`;
  // One search's answers as they stream in (NDJSON, one line per source; x-pm-sources names them)
  const readSearch = async (q, scope, signal, onSources, onAnswer) => {
    const r = await fetch(`/v1/search?${new URLSearchParams({ q, in: scope })}`, { headers: state.token ? { authorization: 'Bearer ' + state.token } : {}, signal });
    if (!r.ok) throw new Error('Search request failed');
    onSources((r.headers.get('x-pm-sources') || '').split(',').filter(Boolean));
    const reader = r.body.getReader(); const dec = new TextDecoder(); let buf = '';
    const accept = (line) => { if (line.trim()) onAnswer(JSON.parse(line)); };
    for (;;) {
      const { value, done } = await reader.read(); if (done) break;
      buf += dec.decode(value, { stream: true });
      let nl;
      while ((nl = buf.indexOf('\n')) >= 0) {
        const line = buf.slice(0, nl); buf = buf.slice(nl + 1);
        accept(line);
      }
    }
    accept(buf + dec.decode());
  };
  // The library changed (a playlist edited, music imported, the node's library_changed): a kept
  // search holding its answers is stale, so the same search started again (typed, Back to it, its
  // scope pressed) asks the node again for its library answer only (a playlist's new count; the
  // services' answers stay as they were). The old results stay drawn, under the progress line, until
  // the new library answer is in, then it takes the old one's place (no empty page, Back keeps the
  // scroll); a failed ask keeps the old one. The one on screen is not asked again by its page being drawn again
  // (keepShown), and one still on its way is neither cut off nor asked again. The library changing
  // again while it is asked keeps it stale (that answer may be older than the change): on screen, it
  // is asked once more as soon as that answer is in (once: a long scan's stream of changes does not
  // keep it asking). An ask that hangs is given up after SEARCH_REFRESH_MS (the old results stay,
  // still stale). One that ended in an error is asked for its library answer only too (no automatic
  // Retry of the services; its Retry button stays, unless that answer was the one missing).
  let shownSearch = null; // (the run whose results #sresults shows, renderSearchBody)
  const forgetLibrarySearches = () => { for (const r of searchRuns.values()) if (['everywhere', 'library'].includes(r.scope) || r.sources.includes('library')) r.stale = true; };
  libWrote = forgetLibrarySearches;
  const SEARCH_REFRESH_MS = 15000;
  // (an answer the same but for the time it took)
  // Songs removed from the library this session, in order (removeFromLibrary): an answer asked before
  // a removal and landing after it drops them (refreshSearch, startSearch), unless Bring back put the
  // song back since (back: its file brought back, Node library; one whose lookup failed stays out
  // until a fresh library answer lists it. One whose lookup is still on its way when its file is
  // brought back is marked once the lookup answers: restoredPaths, the files brought back in order,
  // each removal looking only at those brought back after it, since: never an earlier Bring back)
  const removedTracks = [];
  const restoredPaths = [];
  const goneSince = (since) => removedTracks.slice(since).filter((m) => !m.back).map((m) => m.id);
  const broughtBack = (path) => { restoredPaths.push(path); for (const m of removedTracks) if (m.path && m.path === path) m.back = true; };
  // An answer without library song id: the library's copy goes; a service's copy stays, without the
  // library's id (no Remove or Edit tags on it, its page not the removed song's)
  const withoutTrack = (a, src, id) => {
    const gone = (h) => h.kind === 'song' && h.track_id != null && Number(h.track_id) === Number(id);
    if (!a || !Array.isArray(a.hits) || !a.hits.some(gone)) return a;
    // (a copy with no page: its uri's page when it names one, else no song page)
    const pageId = (h) => { if (h.page) return idOf(h.page); const k = npPageId(h.uri); return k && !k.startsWith('lib-') ? k : null; };
    return { ...a, hits: src === 'library' ? a.hits.filter((h) => !gone(h)) : a.hits.map((h) => (gone(h) ? { ...h, track_id: null, id: h.id === `lib-${id}` ? pageId(h) : h.id } : h)) };
  };
  const sameAnswer = (a, b) => !!a && !!b && JSON.stringify({ ...a, ms: 0 }) === JSON.stringify({ ...b, ms: 0 });
  const refreshSearch = (run, again) => {
    if (run.refreshing) return; // (stays stale: asked once more when that answer is in)
    run.stale = false; if (!run.sources.includes('library')) return; run.refreshing = true;
    (async () => {
      let ok = false; let same = false;
      const ctrl = new AbortController(); const cut = setTimeout(() => ctrl.abort(), SEARCH_REFRESH_MS);
      const since = removedTracks.length; // (songs removed while it is asked: its answer may be older)
      try {
        let fresh = null;
        await readSearch(run.q, 'library', ctrl.signal, () => {}, (a) => { if (a.source === 'library') fresh = a; });
        if (!fresh || fresh.status !== 'ok') throw new Error('Library answer missing');
        ok = true;
        if (searchRuns.get(searchKey(run.q, run.scope)) !== run) return; // (replaced meanwhile: Retry)
        for (const id of goneSince(since)) fresh = withoutTrack(fresh, 'library', id);
        same = sameAnswer(run.answers.library, fresh);
        run.answers = { ...run.answers, library: fresh };
        // (it was the one missing, or the stream broke after every source answered: the banner goes,
        // so the page is drawn again even when the answer is the same)
        if (run.error && run.sources.every((src) => run.answers[src])) { run.error = false; same = false; }
      } catch { run.stale = true; } finally {
        clearTimeout(cut); run.refreshing = false;
        if (ok && run.stale && !again && currentSearch(run)) refreshSearch(run, true);
        searchChanged(run, same);
      }
    })();
  };
  const startSearch = (q, scope, keepShown) => {
    const key = searchKey(q, scope);
    const old = searchRuns.get(key);
    const onScreen = keepShown && old === shownSearch && state.view === 'search' && !!$('#sresults');
    if (old && (old.pending || Date.now() - old.startedAt < SEARCH_TTL)) {
      if (!old.stale || onScreen || old.pending) return old;
      // (one that failed before the node named its sources, the node offline, has no answer to keep:
      // it is searched afresh)
      if (!old.error || old.sources.includes('library')) { refreshSearch(old); return old; }
    }
    for (const r of searchRuns.values()) if (r.pending && r !== old) { try { r.ctrl.abort(); } catch { /* */ } r.pending = false; }
    const run = { q, scope, sources: [], answers: {}, startedAt: Date.now(), firstAt: null, doneAt: null, ctrl: new AbortController(), pending: true, slow: new Set() };
    const since = removedTracks.length; // (an answer landing after a removal may be older than it)
    searchRuns.set(key, run);
    // Tag labels as you type (yours first), beside the results: STEWARD-FIX-search-tags.md
    const word = (/(?:#|\btag:)(\S+)$/.exec(q.trim()) || [])[1] || (/#|\btag:/.test(q) ? '' : q.trim().toLowerCase().replace(/\s+/g, '-'));
    if (word && word.length <= 20) api('GET', `/search/tags?prefix=${encodeURIComponent(word)}`).then((t) => { run.tags = t; searchChanged(run); }).catch(() => {});
    (async () => {
      try {
        await readSearch(q, scope, run.ctrl.signal, (srcs) => {
          run.sources = srcs;
          searchChanged(run);
          setTimeout(() => { if (!run.pending) return; for (const src of run.sources) if (!run.answers[src]) run.slow.add(src); searchChanged(run); }, 5000);
        }, (a) => {
          for (const id of goneSince(since)) a = withoutTrack(a, a.source, id);
          run.answers[a.source] = a; run.slow.delete(a.source);
          if (!run.firstAt) run.firstAt = Date.now();
          searchChanged(run);
        });
        if (run.sources.some((src) => !run.answers[src])) throw new Error('Search response incomplete');
      } catch (e) { if (e.name !== 'AbortError') run.error = true; }
      run.pending = false; run.doneAt = Date.now();
      window.pmLastSearch = { q, scope, first_ms: run.firstAt && run.firstAt - run.startedAt, all_ms: run.doneAt - run.startedAt, sources: Object.fromEntries(Object.values(run.answers).map((a) => [a.source, a.ms])) };
      searchChanged(run);
    })();
    return run;
  };
  // Retried searches have the same query and scope, but a different run. Old tag replies and
  // slow-source timers must never replace the new results or change their progress.
  const displayedSearch = () => state.view === 'search' && state.viewArg && searchRuns.get(searchKey(state.viewArg.q, state.viewArg.in || 'everywhere'));
  const currentSearch = (run) => !!run && displayedSearch() === run;
  // (same: a library answer asked again came back unchanged, so only the progress line moves)
  const searchChanged = (run, same) => {
    if (!currentSearch(run)) return;
    const prog = $('#sprog'); const total = Math.max(run.sources.length, 1); const got = Object.keys(run.answers).length;
    if (prog) { prog.hidden = !run.pending && !run.refreshing; prog.firstElementChild.style.width = `${Math.max(8, (got / total) * 100)}%`; }
    if (!same) renderSearchBody(run);
  };
  // Keyboard on the search box (TASK-pinakes-social-ux A1): ↑/↓ walk the results across the
  // groups, Enter opens the one in focus, Shift+Enter plays it; the box keeps the focus and says
  // which row is active (aria-activedescendant on the combobox, and a polite line naming it).
  // Typing drops the position; a source answering later keeps it on the same search.
  const searchKeys = {
    at: -1,
    q: null,
    items: () => $$('#sresults .srow, #sresults .tile, #sresults [data-sother], #sresults .toprow'),
    reset() {
      this.at = -1; this.key = null; search.removeAttribute('aria-activedescendant');
      $$('#sresults .kfocus').forEach((el) => el.classList.remove('kfocus'));
    },
    // A row by what it is (its group and its first lines), not its place: answers arriving later
    // move rows around
    keyOf: (el) => `${(el.closest('[data-skind]') || { dataset: {} }).dataset.skind || ''}|${(el.innerText || '').split('\n').filter(Boolean).slice(0, 2).join('|')}`,
    // After a re-render: the same search keeps the same row wherever it went; when it is gone the
    // highlight goes too (and says so), so Enter never opens a row nobody announced
    keep(q) {
      if (q !== this.q || this.at < 0) { this.q = q; this.at = -1; this.key = null; search.removeAttribute('aria-activedescendant'); return; }
      const items = this.items(); const k = items.findIndex((el) => this.keyOf(el) === this.key);
      if (k < 0) { this.at = -1; this.key = null; search.removeAttribute('aria-activedescendant'); const said = $('#sresults-said'); if (said) said.textContent = 'The results changed'; return; }
      this.at = k; this.show(false);
    },
    show(scroll) {
      const items = this.items(); const el = items[this.at]; if (!el) return;
      items.forEach((x) => x.classList.remove('kfocus'));
      el.classList.add('kfocus');
      if (!el.id) el.id = `sres-${this.at}`;
      search.setAttribute('aria-activedescendant', el.id);
      if (scroll) el.scrollIntoView({ block: 'nearest' });
    },
    move(d) {
      const items = this.items(); if (!items.length) return;
      this.at = Math.max(0, Math.min(items.length - 1, this.at + d));
      this.key = this.keyOf(items[this.at]);
      this.show(true);
      const said = $('#sresults-said'); if (said) said.textContent = `${(items[this.at].innerText || '').split('\n').filter(Boolean).slice(0, 2).join(', ')} (${this.at + 1} of ${items.length})`;
    },
    // Shift+Enter plays; a row with nothing to play (a person, a post) opens instead
    act(play) {
      const el = this.items()[this.at]; if (!el) return;
      const playBtn = play && $('[data-splay], [data-tileplay], [data-splaytop]', el);
      (playBtn || $('.name .lnk, [data-sopen]', el) || el).click();
    },
  };
  search.addEventListener('keydown', (e) => {
    if (state.view !== 'search') return;
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') { e.preventDefault(); searchKeys.move(e.key === 'ArrowDown' ? 1 : -1); }
    else if (e.key === 'Enter' && searchKeys.at >= 0) { e.preventDefault(); e.stopImmediatePropagation(); clearTimeout(searchTimer); searchKeys.act(e.shiftKey); }
  }, true);
  const isLink = (q) => /^https?:\/\/\S+$/i.test(q);
  const linkKind = (u) => (/\/(artist|artists)\//.test(u) ? 'artist' : /\/(album|albums|release|release-group)\//.test(u) ? 'album' : 'song');
  // Leaving Search (Escape, or the box emptied): back to the page it was opened from, scrolled where it
  // was (Search's entry was pushed from there), else Home, as before: Search opened by a link or a
  // reload, from another search (a menu's Search “…”: Back would refill the box), with a menu open
  // over it (Back would only close the menu), and Escape on the phone's Search tab. (one Back per Search
  // entry: a second Escape before it lands goes nowhere) On a phone, emptying the box stays on the Search
  // tab, on its empty page (as the tab opens), to type again: Home is another tab no one picked.
  let searchLeft = null;
  // (over: from a post's Back onto the search under it, still drawn as the post's page)
  const leaveSearch = (over) => {
    if (state.view !== 'search' && !over) return;
    const cur = history.state;
    if (cur && cur === searchLeft) return;
    const from = cur && typeof cur.fromPath === 'string' ? cur.fromPath.split('?')[0] : '';
    if (from && from !== '/search' && navDepth > 0 && cur.v === 'search' && !cur.transient && shellMode() !== 'phone') {
      searchLeft = cur; overBack = null; overUndo = null; history.back();
      // (over a post's page still drawn as its loading skeleton: that Back not landed in 1.5 s, the page
      // Search was opened from is drawn in this entry, never the skeleton left)
      if (over) { const drawn = state.viewArg; setTimeout(() => { if (state.view !== 'shared' || state.viewArg !== drawn || history.state !== cur) return; searchLeft = null; try { history.replaceState(cur, '', cur.fromPath); } catch { /* sandboxed */ } const r = routeFromPath() || { v: 'home' }; setView(r.v, r.arg, 'replace'); if (popWho && !overBack) overBack = { at: Date.now(), done: true, left: { depth: cur.depth || 0, v: r.v } }; }, 1500); }
    }
    else setView('home');
  };
  // "<user>/<id>" of posts the node read as no playlist (422), when: 10 minutes, then asked again
  // (a node updated meanwhile may read that form)
  const notPlaylists = new Map(); const NOT_PL_MS = 600000;
  const notPlaylist = (key) => Date.now() - (notPlaylists.get(key) ?? -Infinity) < NOT_PL_MS;
  const runSearch = async (how, savedScope) => {
    // (a post's Back to the search under it not landed yet: the box's text then decides, there, overDecide)
    if (searchOver && state.view === 'shared') return;
    const q = search.value.trim();
    if (!q) { if (shellMode() !== 'phone') leaveSearch(); else if (state.view === 'search' && state.viewArg && state.viewArg.q) setView('search'); return; }
    // A pasted link skips searching: its item page (by-uri, then resolve; "Reading this link…").
    if (isLink(q)) { setView('item', { kind: linkKind(q), id: idOf(q) }, how === 'replace' ? 'replace' : 'push'); return; }
    // A pasted Pubky key (with or without pubky:// or the pubky prefix) opens that person
    const pk = /^(?:pubky:\/\/|pubky)?([a-z0-9]{52})\/?$/.exec(q);
    if (pk) { setView('person', pk[1], how === 'replace' ? 'replace' : 'push'); return; }
    const scope = savedScope || (state.viewArg && state.view === 'search' && state.viewArg.in) || state.searchScope || 'everywhere';
    // A pasted playlist link (what a playlist's Copy link gives: pubky://<key>/pub/social/v1/posts/<id>)
    // opens that playlist, as the Playlists page's shared-link box does. (q, in: a post that is no
    // playlist, a comment or an ordinary post, is searched as text instead, on the shared page; one
    // known to be none is searched at once, notPlaylists. A reload of its search takes its entry)
    const post = /^(?:pubky:\/\/)?(?:pubky)?([a-zA-Z0-9]{52})\/pub\/social\/v1\/posts\/([0-9A-HJKMNP-TV-Z]{13})\/?$/.exec(q);
    if (post && !notPlaylist(`${post[1].toLowerCase()}/${post[2]}`)) { setView('shared', { user: post[1].toLowerCase(), id: post[2], q, in: scope, ...(how === 'pop' || how === 'replace' ? { here: 1 } : {}) }, how === 'pop' || how === 'replace' ? 'replace' : 'push'); return; }
    startSearch(q, scope);
    setView('search', { q, in: scope }, how === 'pop' || how === 'replace' ? how : 'push');
  };
  search.addEventListener('input', () => { searchKeys.reset(); clearTimeout(searchTimer); searchTimer = setTimeout(runSearch, 250); });
  search.addEventListener('keydown', (e) => { if (e.key === 'Enter') { clearTimeout(searchTimer); runSearch(); } if (e.key === 'Escape') { clearTimeout(searchTimer); search.value = ''; search.blur(); if (searchOver) searchOver.escaped = true; leaveSearch(); } });

  // Whether the focused element got its focus from a click on it (not from the keyboard or the page)
  let pointerDown = { at: 0, target: null }, focusByPointer = false;
  // (and whether the last thing used was a key, not a mouse or a finger: a ⋯ menu then takes the focus)
  let keyedLast = false;
  document.addEventListener('pointerdown', (e) => { pointerDown = { at: performance.now(), target: e.target }; keyedLast = false; }, true);
  document.addEventListener('keydown', () => { keyedLast = true; }, true);
  document.addEventListener('focusin', (e) => { const t = pointerDown.target; focusByPointer = !!t && performance.now() - pointerDown.at < 1000 && (e.target === t || (e.target.contains && e.target.contains(t))); }, true);
  // A song row's artist line cut with "…" (its artists, and under 900 px its album): the browser leaves a
  // focused link where it is once ~32 px of it shows, so a second artist took focus with most of its
  // name under the "…"; where it did scroll the line, the "…" stayed put and the line drew blank.
  // Reached from the keyboard, the line now moves just enough to show the link (its start, if it is
  // wider than the line), is cut without "…" while moved (.slid), and goes back when focus leaves it.
  const cutLineOf = (el) => (el && el.closest ? el.closest('#main .track .by') : null);
  document.addEventListener('focusin', (e) => {
    const a = e.target; if (focusByPointer || !a.matches || !a.matches('a[href], button')) return;
    const line = cutLineOf(a); if (!line || line.scrollWidth <= line.clientWidth) return;
    // The link goes inside the line's padding (at least 4 px: room for its focus ring)
    const cs = getComputedStyle(line), b = line.getBoundingClientRect(), r = a.getBoundingClientRect();
    const inset = (side) => Math.max(4, parseFloat(cs['padding' + side]) || 0);
    const lo = b.left + line.clientLeft + inset('Left'), hi = b.left + line.clientLeft + line.clientWidth - inset('Right');
    if (r.left < lo || r.width > hi - lo) line.scrollLeft += r.left - lo;
    else if (r.right > hi) line.scrollLeft += r.right - hi;
    line.classList.toggle('slid', line.scrollLeft > 0);
  });
  document.addEventListener('focusout', (e) => {
    const line = cutLineOf(e.target);
    if (line && !(e.relatedTarget && line.contains(e.relatedTarget))) { line.scrollLeft = 0; line.classList.remove('slid'); }
  });
  document.addEventListener('keydown', (e) => {
    if (e.defaultPrevented) return;
    const typing = ['INPUT', 'SELECT', 'TEXTAREA'].includes(e.target.tagName) || e.target.isContentEditable;
    // Escape closes what is on top: a panel or menu, else the phone's sheet, else Now playing
    if (e.key === 'Escape') {
      if (document.querySelector('dialog[open]')) return; // (it closes itself, and only itself)
      if (e.isTrusted && window.PM_USAGE) PM_USAGE.shortcut('escape');
      const pops = !qPanel.hidden || !$('#zonepop').hidden || !menuEl.hidden || !volPop().hidden || !jobPop().hidden;
      if (pops) { closePop(); closeMenu(); closeQPanel(); closeVolPop(); closeJobPop(); return; }
      if (npSheetTr.open()) { closeNpSheet(); return; }
      if (!overlay.hidden) toggleNowPlaying();
      return;
    }
    if (typing) return;
    // A clickable row (a div) is a button for the keyboard: Enter or Space opens it. (Space held with
    // Alt, Ctrl or Cmd is the system's: the window menu, the input language, Spotlight)
    const row = e.target.closest && e.target.closest('.clickable[tabindex]');
    if (row && e.target === row && (e.key === 'Enter' || e.code === 'Space')) { if (e.code === 'Space' && (e.altKey || e.ctrlKey || e.metaKey)) return; e.preventDefault(); row.click(); return; }
    // A control reached with the keyboard (or focused by the page) owns Space: it presses it. The one
    // a click left focused doesn't (Space would press it again): Space plays or pauses there. Arrows
    // belong only to controls that move with them (tabs, menu items, sliders). Search, Help and Now
    // playing stay available everywhere.
    const onControl = e.target.closest('button, a, summary, .clickable[tabindex], [role="button"], [role="tab"], [role="menuitem"], [role="slider"]');
    if (e.code === 'Space' && onControl && !(focusByPointer && document.activeElement === e.target)) return;
    if ((e.key === 'ArrowRight' || e.key === 'ArrowLeft') && e.target.closest('[role="tab"], [role="menuitem"], [role="slider"]')) return;
    // Space, the arrows and N are shortcuts on their own: held with Alt, Ctrl or Cmd they are the
    // browser's or the system's (Alt+← and Alt+→ are Back and Forward, Cmd+← too on a Mac), so they
    // play, pause and skip nothing. (/ and ? go by the character typed: some keyboards need AltGr
    // or Option for them). A control still left focused by a click doesn't take that Space as a
    // press either (the browser would press it again on keyup)
    if ((e.altKey || e.ctrlKey || e.metaKey) && (e.code === 'Space' || e.key === 'ArrowRight' || e.key === 'ArrowLeft' || e.key.toLowerCase() === 'n')) {
      if (e.code === 'Space' && onControl) e.preventDefault();
      return;
    }
    // A dialog on top has the keyboard: Space, the arrows, N, / and ? are the player's and the
    // page's, and behind "Add music" or "Delete this…?" they went to another song, opened Now
    // playing under the dialog, or put a second dialog over the first. (Its own keys are the
    // browser's: Tab, Space and Enter on its buttons, Escape)
    if (document.querySelector('dialog[open]')) return;
    { const sc = e.key === '/' ? 'search' : e.code === 'Space' ? 'toggle' : e.key === 'ArrowRight' ? 'next' : e.key === 'ArrowLeft' ? 'prev' : e.key.toLowerCase() === 'n' ? 'now_playing' : e.key === '?' ? 'help' : null; if (sc && e.isTrusted && window.PM_USAGE) PM_USAGE.shortcut(sc); }
    // An open ⋯ menu, zone picker, Sound panel, volume or copy's popup closes before Search, Now playing
    // or Help: it stayed open (with its grey scrim) over or behind them.
    // (Now playing and Help take over its history entry, so Back then closes them; Search has none. So
    // over Now playing, Help is a layer of its own: Back closes Help, and Now playing stays)
    if (e.key === '/' || e.key === '?' || e.key.toLowerCase() === 'n') { const keep = e.key !== '/'; closeMenu(keep); closePop(keep); closeQPanel(keep); closeVolPop(keep); closeJobPop(keep); }
    // (the search field is under an open Now playing, out of reach there: it closes first)
    if (e.key === '/') { e.preventDefault(); if (!overlay.hidden) toggleNowPlaying(false); search.focus(); search.select(); return; }
    // (Space is a press of its own: it spins no Play clicked just before)
    if (e.code === 'Space') { e.preventDefault(); state.startAnchor = null; transport({ action: 'toggle' }); }
    if (e.key === 'ArrowRight') transport({ action: 'next' });
    if (e.key === 'ArrowLeft') transport({ action: 'previous' });
    if (e.key.toLowerCase() === 'n') toggleNowPlaying();
    // (over Now playing it has its own history entry: Back closes it and Now playing stays)
    if (e.key === '?') { e.preventDefault(); choiceDialog({ title: 'Keyboard shortcuts', text: '/  search  ·  Space  play or pause  ·  ←  →  previous or next  ·  N  now playing  ·  Esc  close  ·  ?  this list', choices: [], layer: !overlay.hidden && transients.every((t) => t === npTr || !t.open()) }); }
  });


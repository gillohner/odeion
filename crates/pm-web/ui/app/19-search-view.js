  const SOURCE_RANK = ['library', 'tidal', 'deezer', 'spotify', 'archive', 'audius', 'pubky'];
  // Groups in the order people scan them: who, then what (UX review #4)
  const SEARCH_KINDS = [['artist', 'Artists'], ['album', 'Albums'], ['song', 'Songs'], ['playlist', 'Collections'], ['person', 'People'], ['post', 'Posts'], ['podcast', 'Podcasts'], ['concert', 'Concerts']];
  // Facets (TASK-pinakes-social-ux A2): chips under the scope chips, counted from the merged
  // results in hand; each facet keeps at most one value on, and the rows and tiles filter by them.
  const SEARCH_FACETS = [['kind', 'Kind'], ['service', 'Source'], ['quality', 'Quality'], ['decade', 'Years'], ['site', 'Site']];
  const QUALITY_WORDS = { hi_res: 'Hi-res', lossless: 'Lossless', lossy: 'Lossy' };
  // People and posts are Pubky's too: one Source chip for the three
  const facetValues = (h) => ({ kind: h.kind ? [h.kind] : [], service: [...new Set((h.sources || []).map((x) => (x === 'people' || x === 'posts' ? 'pubky' : x)))], quality: h.quality ? [h.quality] : [], decade: h.year ? [`${Math.floor(h.year / 10) * 10}s`] : [], site: h.site ? [h.site] : [] });
  const facetWord = (facet, v) => (facet === 'kind' ? (SEARCH_KINDS.find(([k]) => k === v) || [v, v])[1] : facet === 'service' ? (v === 'library' ? 'Your library' : v === 'pubky' ? 'Pubky' : srcName(v)) : facet === 'quality' ? QUALITY_WORDS[v] || v : v);
  const facetsOf = (byKind) => { const counts = Object.fromEntries(SEARCH_FACETS.map(([f]) => [f, new Map()])); for (const list of Object.values(byKind)) for (const h of list) { const fv = facetValues(h); for (const [f] of SEARCH_FACETS) for (const v of fv[f]) counts[f].set(v, (counts[f].get(v) || 0) + 1); } return counts; };
  const passesFacets = (h, on) => SEARCH_FACETS.every(([f]) => !on[f] || facetValues(h)[f].includes(on[f]));
  // A hit "matches" when every word of the query is in its title or subtitle
  const searchWords = (x) => String(x || '').toLowerCase().normalize('NFKD').replace(/[\u0300-\u036f]/g, '').split(/[^a-z0-9]+/).filter(Boolean);
  const closeMatch = (h, q) => { const want = searchWords(q); const have = new Set(searchWords(`${h.title} ${h.subtitle || ''}`)); return want.length > 0 && want.every((w) => have.has(w) || [...have].some((x) => x.startsWith(w))); };
  const srcName = (src) => (src === 'library' ? 'Library' : src === 'pubky' ? 'Pubky' : SERVICE_NAMES[src] || (state.sources || []).find((x) => x.id === src)?.name || src);
  // One order for every list of services (STEWARD-UX-PLAYER-SOURCES §1): services switched on
  // and working first, in the owner's play order; then the rest. Your library always works.
  // Matched by prefix ("signed in · user", "ready · …"); "browser" plays in This browser with no set-up
  const HEALTHY_STATUS = /^(ready|previews|connected|signed in|ok|browser)\b/i;
  const srcHealthy = (x) => !!x && x.enabled !== false && (typeof x.healthy === 'boolean' ? x.healthy : HEALTHY_STATUS.test(String(x.status || '')));
  const loadSvcRank = async () => {
    if (!isOwner()) return null;
    if (state.svcRank && Date.now() - state.svcRank.at < 60000) return state.svcRank.failed ? null : state.svcRank;
    if (state.svcRankLoading) return state.svcRankLoading;
    const gen = state.svcRankGen || 0;
    const load = (async () => {
      try {
        const [srcs, po] = await Promise.all([api('GET', '/sources'), api('GET', '/play-order').catch(() => ({ order: [], services: [] }))]);
        state.sources = srcs;
        // The play order knows who can play right now; fall back to each service's status
        const connected = new Map((po.services || []).map((x) => [x.id, x.connected]));
        const ok = (x) => srcHealthy(x) && connected.get(x.id) !== false;
        const rank = { at: Date.now(), healthy: new Set(['library', ...srcs.filter(ok).map((x) => x.id)]), off: new Set(srcs.filter((x) => x.enabled === false).map((x) => x.id)), order: po.order || [] };
        // A change saved while this was loading wins: keep nothing from before it
        if (gen === (state.svcRankGen || 0)) state.svcRank = rank;
        return rank;
      } catch { if (gen === (state.svcRankGen || 0)) state.svcRank = { at: Date.now(), failed: true }; return null; } finally { if (state.svcRankLoading === load) state.svcRankLoading = null; }
    })();
    state.svcRankLoading = load;
    return load;
  };
  // A source row's service id: Pinakes names sites by slug or host ("youtube-music", "listen.tidal.com");
  // a host counts by its main domain
  const srcSvc = (src) => { const v = String(src || '').toLowerCase(); if (v === 'library') return 'library'; const parts = v.split('.'); return serviceKey(parts.length > 1 ? parts[parts.length - 2] : v); };
  // Sort anything that names a service (`svc(x)` gives its id): healthy first, then by play order
  const rankSvc = (list, svc = (x) => x, r = state.svcRank) => {
    if (!r || r.failed) return list;
    const pos = (id) => { const i = r.order.indexOf(id); return i < 0 ? 999 : i; };
    return list.map((x, k) => [x, k]).sort(([a, i], [b, j]) => (r.healthy.has(svc(a)) ? 0 : 1) - (r.healthy.has(svc(b)) ? 0 : 1) || pos(svc(a)) - pos(svc(b)) || i - j).map(([x]) => x);
  };
  // Merge hits across sources: one row per song (Pinakes group, ISRC, UPC, MBID), best source first.
  const mergeSearch = (run) => {
    const byKind = Object.fromEntries(SEARCH_KINDS.map(([k]) => [k, []]));
    const index = new Map();
    const answers = Object.values(run.answers).filter((x) => x.status === 'ok').sort((x, y) => SOURCE_RANK.indexOf(x.source) - SOURCE_RANK.indexOf(y.source));
    for (const ans of answers) for (const h of ans.hits) {
      if (!byKind[h.kind]) continue;
      // Artists without a shared id: the same name is one row across services, and a service's
      // later namesakes (tribute acts, typos) are dropped: five identical "Queen" rows helped no
      // one (second review #13)
      const byName = h.kind === 'artist' && searchWords(h.title).join(' ');
      let found = (h.keys || []).map((k) => index.get(h.kind + '|' + k)).find(Boolean);
      if (!found && byName) {
        const same = index.get('artist-name|' + byName);
        if (same && same.sources.includes(ans.source)) continue;
        found = same;
        if (found && !found.image && h.image) found.image = h.image;
      }
      if (found) { if (h.tagged && !found.tagged) found.tagged = h.tagged; if (h.exact) { found.exact = true; found.also = h.also; } if (!found.sources.includes(ans.source)) found.sources.push(ans.source); for (const k of h.keys || []) index.set(h.kind + '|' + k, found); continue; }
      const m = { ...h, sources: [ans.source] };
      // Your playlist's count as the Playlists page writes it ("1,200 songs", not "1200 songs")
      if (h.kind === 'playlist' && ans.source === 'library' && h.subtitle) m.subtitle = h.subtitle.replace(/^\d+(?= )/, (d) => fmtCount(Number(d)));
      byKind[h.kind].push(m);
      for (const k of h.keys || []) index.set(h.kind + '|' + k, m);
      if (byName) index.set('artist-name|' + byName, m);
    }
    return byKind;
  };
  // Where the row and its focused button are before a redraw (taken before the phone's Filters button
  // comes or goes beside the row and changes its width)
  const scopeRowBefore = (chips) => {
    const focused = document.activeElement;
    const rowBefore = chips.getBoundingClientRect(), focusBefore = focused.parentElement === chips && focused.getBoundingClientRect();
    return { scrollLeft: chips.scrollLeft, focused, focusWasVisible: focusBefore && focusBefore.left >= Math.max(0, rowBefore.left) && focusBefore.right <= Math.min(innerWidth, rowBefore.right) };
  };
  const renderSearchScopes = (chips, run, before = scopeRowBefore(chips)) => {
    const scope = run.scope;
    // One choice, like the result filters: a named group whose chosen button is pressed
    chips.setAttribute('role', 'group'); chips.setAttribute('aria-label', 'Search in');
    const { scrollLeft, focused, focusWasVisible } = before;
    const services = (state.sources || []).filter((x) => x.enabled !== false && (!x.features || x.features.includes('search')) && !/^not configured/.test(x.status) && x.status !== 'needs_config' && x.status !== 'browser');
    const scopes = new Map([['everywhere', 'Everywhere'], ...(offers('library') ? [['library', 'Library']] : []), ...(offers('pubky') ? [['pubky', 'Pubky']] : []), ...services.map((x) => [x.id, x.name])]);
    // A listener does not read the owner's /sources. A requested service uses its public name,
    // even for tag searches (whose replies come from Library/Pubky). Keep it available after
    // choosing another source, but never reintroduce a service explicitly switched off.
    for (const saved of searchRuns.values()) {
      const id = saved.scope;
      const named = id !== 'musicbrainz' && Object.prototype.hasOwnProperty.call(SERVICE_NAMES, id);
      if (!scopes.has(id) && (named || saved.sources.includes(id)) && !(state.sources || []).some((x) => x.id === id && x.enabled === false)) scopes.set(id, srcName(id));
    }
    const stat = (src) => { const ans = run.answers[src]; if (ans) return ans.status === 'ok' ? `<span class="cnt">${ans.hits.length}</span>` : `<span class="bad" role="img" aria-label="couldn’t search" title="${esc(ans.error || '')}">!</span>`; return run.sources.includes(src) && run.pending ? '<span class="spin"></span>' : ''; };
    const buttons = new Map($$('[data-scope-in]', chips).map((b) => [b.dataset.scopeIn, b]));
    const ordered = [];
    for (const [id, name] of scopes) {
      const b = buttons.get(id) || document.createElement('button');
      b.type = 'button'; b.className = `chip scope ${scope === id ? 'on' : ''}`; b.dataset.scopeIn = id;
      b.setAttribute('aria-pressed', String(scope === id));
      const html = `${['everywhere', 'library', 'pubky'].includes(id) ? '' : svcBadge(id)}${esc(name)} ${scope === 'everywhere' || scope === id ? stat(id === 'everywhere' ? '' : id) : ''}`;
      if (b.innerHTML !== html) b.innerHTML = html;
      ordered.push(b); buttons.delete(id);
      b.onclick = () => {
        const focused = document.activeElement === b;
        state.searchScope = id; startSearch(run.q, id); setView('search', { q: run.q, in: id }, 'replace');
        if (focused && document.activeElement === document.body) $$('#scopechips [data-scope-in]').find((x) => x.dataset.scopeIn === id)?.focus({ preventScroll: true });
      };
    }
    buttons.forEach((b) => b.remove());
    // Keep the focused button attached, arranging its siblings around it when discovery changes
    // their order. Moving that button itself would lose focus and a held Space key's activation.
    let next = null;
    for (const b of ordered.reverse()) {
      if (b !== focused && (b.parentElement !== chips || b.nextElementSibling !== next)) chips.insertBefore(b, next);
      next = b;
    }
    // Inserting earlier choices can make scroll snapping follow the old snap target.
    // A person who already moved the row keeps their position instead.
    if (chips.dataset.manuallyMoved) chips.scrollLeft = scrollLeft;
    if (!chips.dataset.revealBound) {
      chips.dataset.revealBound = '1';
      // Source discovery may insert choices before the selected one. Reveal it again only if
      // the person has not started moving through this row themselves.
      for (const event of ['pointerdown', 'touchstart', 'wheel', 'keydown', 'focusin']) chips.addEventListener(event, () => {
        chips.dataset.manuallyMoved = '1';
        if (event === 'focusin') delete chips.dataset.focusScrolled;
        else if (event !== 'keydown') chips.dataset.focusScrolled = '1';
      }, { passive: true });
    }
    const selected = $$('[data-scope-in]', chips).find((b) => b.dataset.scopeIn === scope);
    // Discovery and counts can move a visible focused button. Keep it visible, but leave a
    // deliberate scroll away intact. Otherwise reveal the selection until the person takes over.
    const reveal = focusWasVisible && !chips.dataset.focusScrolled && focused.parentElement === chips ? focused : !chips.dataset.manuallyMoved ? selected : null;
    if (reveal) {
      const row = chips.getBoundingClientRect(), button = reveal.getBoundingClientRect();
      const padding = getComputedStyle(chips);
      if (button.left < row.left + parseFloat(padding.paddingLeft) || button.right > row.right - parseFloat(padding.paddingRight)) chips.scrollLeft += (button.left + button.right - row.left - row.right) / 2;
    }
  };
  const renderSearchBody = (run) => {
    if (!currentSearch(run)) return;
    const chips = $('#scopechips'); const box = $('#sresults'); if (!chips || !box) return;
    shownSearch = run;
    const retryFocus = $('[data-sretry]:focus, [data-scontact]:focus', box);
    const scope = run.scope;
    if (!state.sources && !state.sourcesLoading) { state.sourcesLoading = true; api('GET', '/sources').then((l) => { state.sources = l; state.sourcesLoading = false; applySourcesOff(); renderSearchBody(displayedSearch()); }).catch(() => { state.sourcesLoading = false; }); }
    const allKinds = mergeSearch(run);
    run.facets = run.facets || {};
    const facetCounts = facetsOf(allKinds);
    const byKind = Object.fromEntries(Object.entries(allKinds).map(([k, list]) => [k, list.filter((h) => passesFacets(h, run.facets))]));
    const facetChips = SEARCH_FACETS.map(([f, label]) => { const vals = [...facetCounts[f].entries()].sort((a, b) => b[1] - a[1]); if (vals.length < 2 && !run.facets[f]) return ''; return `<span class="facet" data-facet-group="${f}"><span class="muted small">${label}</span>${(f === 'kind' ? vals : vals.slice(0, 6)).map(([v, n]) => `<button class="chip sm ${run.facets[f] === v ? 'on' : ''}" data-facet="${f}:${esc(v)}" aria-pressed="${run.facets[f] === v}">${esc(facetWord(f, v))} <span class="muted">${n}</span></button>`).join('')}</span>`; }).filter(Boolean).join('');
    // On phones the facets fold behind Filters (the results come first; UX sweep 11-01, A9)
    const nOn = Object.values(run.facets).filter(Boolean).length;
    const facetRow = facetChips ? `<div class="chips facets" role="group" aria-label="Narrow the results">${facetChips}${nOn ? '<button class="chip sm ghost" data-facet="">Clear</button>' : ''}</div>` : '';
    // Their Filters button ends the source row (outside its "Search in" group), so the results
    // start right under the sources; the same button stays across redraws and keeps the focus
    const fold = facetRow && shellMode() === 'phone';
    const facetBar = fold ? `<div class="sfilters" id="sfacets"${run.filtersOpen ? '' : ' hidden'}>${facetRow}</div>` : facetRow;
    const rowBefore = scopeRowBefore(chips);
    let fbtn = $('#sbar > [data-sfilters]');
    // (a focused Filters leaving a phone hands the focus to the filters it opened, now with the results)
    const filtersFocused = !fold && fbtn && fbtn === document.activeElement;
    if (!fold) fbtn?.remove();
    else {
      if (!fbtn) { fbtn = document.createElement('button'); fbtn.type = 'button'; fbtn.className = 'chip sfilt'; fbtn.dataset.sfilters = ''; fbtn.setAttribute('aria-controls', 'sfacets'); chips.after(fbtn); }
      const label = `Filters${nOn ? ` · ${nOn}` : ''}`; if (fbtn.textContent !== label) fbtn.textContent = label;
      fbtn.setAttribute('aria-expanded', String(!!run.filtersOpen));
      fbtn.onclick = () => { run.filtersOpen = !run.filtersOpen; fbtn.setAttribute('aria-expanded', String(run.filtersOpen)); const p = $('#sfacets'); if (p) p.hidden = !run.filtersOpen; };
    }
    // (after Filters takes its place, so the chosen source is revealed in the row's final width)
    renderSearchScopes(chips, run, rowBefore);
    const problems = run.sources.filter((src) => (run.answers[src] && run.answers[src].status !== 'ok') || (!run.error && !run.answers[src] && run.slow.has(src)));
    const pendingAny = run.pending && (!run.sources.length || run.sources.some((src) => !run.answers[src]));
    const skel = (n, tiles) => (tiles ? `<div class="sk-grid">${'<div class="skeleton"></div>'.repeat(n)}</div>` : `<div class="skeleton" style="height:${n * 54}px"></div>`);
    const sections = [];
    const counts = {}; for (const ans of Object.values(run.answers)) for (const [k, c] of ans.counts || []) counts[k] = Math.max(counts[k] || 0, c);
    // A Top result only when it really matches; a fuzzy hit is not promoted
    // (the Top result opens an item page: songs, albums and artists have one)
    // (and only one with a page to open: a copy left without one when its library song was removed)
    const exactHit = ['song', 'album', 'artist'].map((k) => byKind[k].find((x) => x.exact && x.id != null)).find(Boolean);
    const top = exactHit || [byKind.artist.find((x) => x.sources.includes('library')), byKind.song.find((x) => x.sources.includes('library')), byKind.artist[0], byKind.album[0], byKind.song[0]].find((x) => x && x.id != null && closeMatch(x, run.q));
    const anyHits = SEARCH_KINDS.some(([k]) => byKind[k].length);
    const tg = run.tags; if (tg && ((tg.mine || []).length || (tg.others || []).length)) sections.push(`<section class="section fadein" id="stags"><div class="section-head"><h3>Tags</h3><span class="muted small">on Pubky</span></div><div class="chips">${(tg.mine || []).map((t) => tagChip(t.label, { own: true, count: t.count, attrs: `data-stag="${esc(t.label)}"`, title: `Your tag · ${countOf(t.count, 'items')}` })).join('')}${(tg.others || []).map((l) => tagChip(l, { attrs: `data-stag="${esc(l)}"` })).join('')}</div></section>`);
    // (an exact copy left without a page is listed below, so this would be untrue)
    if (!top && anyHits && !run.q.includes('#') && !['song', 'album', 'artist'].some((k) => byKind[k].some((x) => x.exact))) sections.push('<p class="muted small" id="snomatch">No exact match. The closest results are below.</p>');
    if (top) sections.push(`<section class="section fadein" id="stop"><div class="section-head"><h3>Top result</h3></div><div class="card toprow"><div class="art ${top.kind === 'artist' ? 'round' : ''}">${top.image ? `<img src="${esc(imgSrc(top.image))}" alt="" data-err="remove">` : ''}</div><div class="grow"><div class="name big"><a class="lnk" data-sopen="top"${hrefAttr(pathFor('item', { kind: top.kind, id: top.id }))}>${esc(top.title)}</a></div><div class="sub">${esc([{ song: 'Song', album: 'Album', artist: 'Artist', playlist: 'Collection', podcast: 'Podcast', concert: 'Concert' }[top.kind], top.subtitle].filter(Boolean).join(' · '))} <span class="src-badges">${top.sources.map((x) => svcBadge(x === 'library' ? 'library' : x)).join('')}</span></div>${top.also && top.also.length ? `<div class="sub" id="stopalso">Also on ${esc(top.also.join(' · '))}</div>` : ''}</div><button class="btn primary" data-splaytop>${icon('play')} Play</button></div></section>`);
    const songCap = run.allSongs ? 50 : 6;
    for (const [kind, label] of SEARCH_KINDS) {
      const items = byKind[kind];
      if (!items.length && (!pendingAny || (run.facets.kind && run.facets.kind !== kind))) continue;
      const head = `<div class="section-head"><h3>${label}</h3><span class="muted small">${items.length ? items.length : ''}${counts[kind] && counts[kind] > items.length ? ` of ${counts[kind]} on Pubky` : ''}</span></div>`;
      let body;
      if (!items.length) body = skel(kind === 'song' ? 4 : 1, kind === 'album' || kind === 'artist');
      else if (kind === 'song') body = songRows(items.slice(0, songCap).map((h) => ({ ...h, artist: h.subtitle, id: h.id })), { showAlbum: true, numbers: 'index', extra: (t) => `${t.tagged ? `<span class="muted small tagged-by">tagged ${esc(t.tagged)}</span>` : ''}<span class="src-badges">${t.sources.map((x) => svcBadge(x === 'library' ? 'library' : x)).join('')}</span>` });
      else if (kind === 'album' || kind === 'artist') body = itemTileGrid(items.slice(0, 12).map((h) => ({ ...h, year: h.year || h.subtitle })), kind);
      // People open their page; your playlists open here; posts and the rest open on their site
      else body = `<div class="list">${items.slice(0, 10).map((h, k) => { const own = kind === 'playlist' && h.sources.includes('library'); const lead = kind === 'person' ? (h.image ? `<img class="thumb round" src="${esc(imgSrc(h.image))}" alt="" loading="lazy" data-err="hide">` : `<div class="avatar">${esc(initials(h.title || 'P'))}</div>`) : own ? icon('playlist') : svcBadge(h.sources[0]); return `<div class="list-row clickable" data-sother="${kind}:${k}">${lead}<div class="grow"><div class="name">${esc(h.title)}</div><div class="sub">${esc(h.subtitle || '')}</div></div>${kind === 'person' || own ? icon('chevron', 'ic right') : icon('external')}</div>`; }).join('')}</div>`;
      const more = kind === 'song' && items.length > songCap ? `<button class="btn ghost sm" data-sallsongs>See all ${fmtCount(items.length)} songs</button>` : '';
      sections.push(`<section class="section ${items.length ? 'fadein' : ''}" data-skind="${kind}">${head}${body}${more}</section>`);
    }
    const short = (e, src) => { const m = humanError(String(e || ''), `/sources/${src}`).replace(/^[a-z]+: /, '').split(/(?<=\.)\s/)[0]; return m.length > 110 ? m.slice(0, 107) + '…' : m; };
    // One line per source: its reason when it gave one ("Spotify needs a Premium plan for this ·
    // Sources"), else that it didn't answer; Retry only where trying again can help. Retry is the
    // same small button as the whole search's, so a keyboard reaches it and a finger can hit it
    // A source waiting for the node's contact (Pubky: the node says "the upstream contact (Settings)")
    // says so in plain words, and the node's owner gets one press to the field
    const needsContact = (err) => /upstream contact/i.test(err || '');
    const contactWords = (err) => err.replace(/the (node's )?upstream contact( \(Settings\))?/i, isOwner() ? 'your email or website' : 'an email or website from the node’s owner');
    const notes = problems.length ? `<div class="snotes">${problems.map((src) => { const err = run.answers[src] && run.answers[src].error; const said = (!err ? '' : needsContact(err) ? contactWords(err) : short(err, src)).replace(/[.\s·]+$/, ''); const why = said && !said.toLowerCase().includes(srcName(src).toLowerCase()) ? `${srcName(src)}: ${said}` : said; const again = !err || /answered 5\d\d|timed? ?out|time-?out|took too long|reach|network|busy|later|didn.t answer|isn.t answering|sending request|refused|dns|connect(ion)? (error|reset|closed)/i.test(err); return `<span class="snote" title="${esc(err ? (needsContact(err) ? contactWords(err) : err) : 'still waiting')}">${svcBadge(src)} ${why ? esc(why) : `${esc(srcName(src))} didn’t answer`}${needsContact(err) ? (isOwner() ? ' <button class="btn sm" type="button" data-scontact>Add it</button>' : '') : again ? ` <button class="btn sm" type="button" data-sretry="${src}">Retry</button>` : ''}</span>`; }).join('')}</div>` : '';
    const nothing = !pendingAny && !SEARCH_KINDS.some(([k]) => byKind[k].length);
    const nothingAtAll = nothing && !(tg && ((tg.mine || []).length || (tg.others || []).length));
    // Keep Retry attached while late tags/sources redraw the results. Replacing it and restoring
    // focus loses the browser's native Space activation between keydown and keyup.
    let content = $('#scontent', box);
    if (!content) { box.innerHTML = '<div id="scontent"></div>'; content = $('#scontent', box); }
    const failure = $('#sfailure', box);
    if (run.error && !failure) box.insertAdjacentHTML('afterbegin', '<div class="snotes" id="sfailure"><div class="notice" role="status"><div class="row"><span class="grow">Search couldn’t finish.</span><button class="btn sm" data-sretry>Retry</button></div></div></div>');
    else if (!run.error && failure) failure.remove();
    const rest = facetBar + (nothingAtAll && !run.error ? emptyState('search', 'Nothing matched', scope === 'everywhere' ? (phoneMode() ? 'Not on your music services.' : 'Not in your library, your services or on Pubky.') : `Nothing on ${srcName(scope)}. Try Everywhere.`) : sections.join(''));
    // The same goes for a source's own Retry: its line stays put while it says the same thing
    const kept = $(':scope > .snotes', content);
    if (notes && kept && kept.pmNotes === notes) { [...content.childNodes].forEach((n) => { if (n !== kept) n.remove(); }); kept.insertAdjacentHTML('afterend', rest); }
    else { content.innerHTML = notes + rest; const n = $(':scope > .snotes', content); if (n) n.pmNotes = notes; }
    if (filtersFocused && document.activeElement === document.body) ($('#sresults .facets [data-facet]') || $('#scopechips [aria-pressed="true"]'))?.focus({ preventScroll: true });
    // A facet chip keeps the focus across the re-render (or it moves to the first chip, after Clear)
    $$('[data-facet]', box).forEach((b) => b.onclick = () => {
      const key = b.dataset.facet; const [f, ...rest] = key.split(':'); const v = rest.join(':');
      if (!f) run.facets = {}; else run.facets[f] = run.facets[f] === v ? null : v;
      renderSearchBody(run);
      const chips = $$('#sresults [data-facet]'); const again = chips.find((x) => x.dataset.facet === key && key) || chips[0];
      if (again) again.focus(); else search.focus();
    });
    searchKeys.keep(`${run.q}|${run.scope}`);
    $$('[data-stag]', box).forEach((b) => b.onclick = () => setView('discover', { tag: b.dataset.stag }));
    const sa = $('[data-sallsongs]', box); if (sa) sa.onclick = () => { run.allSongs = true; renderSearchBody(run); };
    // Bind: rows and tiles use the shared components.
    const songsSec = $('[data-skind="song"]', box);
    if (songsSec && byKind.song.length) { const ss = byKind.song.slice(0, run.allSongs ? 50 : 6).map((h) => ({ ...h, artist: h.subtitle })); bindSongRows(songsSec, ss, {}); enableSelect(songsSec, ss); }
    dropStaleSelect(); // (no songs left to select from)
    for (const kind of ['album', 'artist']) { const sec = $(`[data-skind="${kind}"]`, box); if (sec && byKind[kind].length) bindItemTiles(sec, byKind[kind].slice(0, 12), kind); }
    $$('[data-sother]', box).forEach((r) => r.onclick = () => { const [k, n] = r.dataset.sother.split(':'); const h = byKind[k][Number(n)]; if (k === 'person') setView('person', h.id); else if (k === 'playlist' && h.sources.includes('library')) setView('playlist', h.id); else if (h.page) openWeb(h.page); });
    const to = $('[data-sopen="top"]', box); if (to) to.onclick = inAppClick(() => setView('item', { kind: top.kind === 'song' ? 'song' : top.kind, id: top.id }));
    const tp = $('[data-splaytop]', box); if (tp) tp.onclick = async (e) => { if (top.kind === 'song') return playSongs([{ ...top, artist: top.subtitle }], 0, 'replace', null, e.currentTarget); try { const pg = await api('GET', `/item/${top.kind}/${top.id}`); playSongs(pg.songs || [], 0, 'replace', pg.plays_from, e.currentTarget, { artist: pg.artist && pg.artist.name, album: pg.title, image: pg.image }); } catch (er) { toast(er.message, 'error'); } };
    $$('[data-sretry]', box).forEach((b) => b.onclick = () => { searchRuns.delete(searchKey(run.q, run.scope)); startSearch(run.q, run.scope); render(); search.focus(); });
    // (Settings › Discover holds the node's contact: its field takes the keyboard, ready to type)
    $$('[data-scontact]', box).forEach((b) => b.onclick = openContact);
    if (retryFocus && !retryFocus.isConnected) { const again = 'scontact' in retryFocus.dataset ? $('[data-scontact]', box) : $$('[data-sretry]', box).find((b) => b.dataset.sretry === retryFocus.dataset.sretry); (again || search).focus(); }
  };

  // ---- templates -----------------------------------------------------------
  const isFav = (t) => { if (t.id != null && state.favUris.has(`track:${t.id}`)) return true; const c = t.canonical; const uri = Array.isArray(c) ? c[0] : c; const url = uri && typeof uri === 'object' ? canonicalUrl(uri) : null; return url ? state.favUris.has(url) : false; };
  const canonicalUrl = (u) => {
    if (!u) return null;
    // The node's shape: { kind: 'recording', mbid } (fresh-eyes review: every library track
    // looked unidentified because only the older shape below was read)
    if (typeof u.kind === 'string' && u.mbid) return `https://musicbrainz.org/${u.kind.replace('_', '-')}/${u.mbid}`;
    if (u.isrc && typeof u.isrc === 'string') return 'https://musicbrainz.org/isrc/' + u.isrc;
    if (typeof u.url === 'string' && /^https?:/.test(u.url)) return u.url;
    if (u.recording) return 'https://musicbrainz.org/recording/' + (u.recording.mbid || u.recording);
    if (u.release) return 'https://musicbrainz.org/release/' + (u.release.mbid || u.release);
    if (u.release_group) return 'https://musicbrainz.org/release-group/' + (u.release_group.mbid || u.release_group);
    if (u.artist) return 'https://musicbrainz.org/artist/' + (u.artist.mbid || u.artist);
    if (u.isrc) return 'https://musicbrainz.org/isrc/' + (u.isrc.isrc || u.isrc);
    if (u.service) return u.service.url || null;
    return null;
  };
  // A library track's page id: its canonical identity when known, else lib-<id>.
  const trackPageId = (t) => { const c = Array.isArray(t.canonical) ? t.canonical[0] : t.canonical; const u = canonicalUrl(c); return u ? idOf(u) : 'lib-' + t.id; };
  const trackHasId = (t) => !!canonicalUrl(Array.isArray(t.canonical) ? t.canonical[0] : t.canonical);
  // How a song got its match (`auto`, `album_fit`, `tagged_mbid`…), when the node says
  // "Wrong match?": the owner's, for a song with a match
  const canFixMatch = (t) => isOwner() && t && t.id != null && trackHasId(t);
  const MATCH_HOW = { album_fit: 'its folder as an album', fingerprint: 'how it sounds', 'fingerprint+release': 'how it sounds', tag_match: 'its exact title, artist and length', auto: 'picked automatically', confirmed_by_user: 'you', tagged_mbid: 'the IDs in its tags', tagged_isrc: 'the ISRC in its tags' };
  // Playlists and hearts name a song the way other apps understand (its MusicBrainz entry), so a
  // file nobody has matched yet asks "Which song is this?" first (fresh-eyes review: adding your
  // own songs to a playlist failed with "no canonical identity"). Resolves to the tracks, now
  // identified, or null when the owner stops.
  // MusicBrainz asks for the node's email or website: Settings › Discover keeps it (Get started on
  // Home can be hidden or done), and its field takes the keyboard, ready to type
  const openContact = () => setView('settings', 'discover').then(() => { const c = state.view === 'settings' && $('#contact'); if (c) c.focus(); });
  const whichSong = async (t) => {
    const who = artists(t);
    // (said only when it takes a moment: a quick answer opens its dialog with no flash of words first)
    const looking = `Looking up “${t.title}” on MusicBrainz…`; const slow = setTimeout(() => toast(looking), 400);
    let c; try { c = await api('GET', `/library/tracks/${t.id}/candidates`); } catch (e) { toast(e.message, 'error'); return null; } finally { clearTimeout(slow); }
    toastDone(looking);
    const about = `“${t.title}”${who ? ` by ${who}` : ''}`;
    if (c.needs_contact) {
      const go = await choiceDialog({ title: 'Which song is this?', text: `Collections and bookmarks name each song the way other apps understand, by its entry in MusicBrainz (the open music encyclopedia). To look ${about} up there, the node first needs your email or website.`, choices: [{ label: 'Add it', value: 'contact', primary: true }] });
      if (go) openContact();
      return null;
    }
    if (!c.suggestions.length) {
      await choiceDialog({ title: 'Which song is this?', text: `MusicBrainz has nothing that matches ${about}${who ? '' : ' (the file has no artist)'}, so it can't go into collections or bookmarks yet. You can still play it and queue it. Giving the file its proper title and artist tags usually fixes this.`, choices: [{ label: 'OK', value: 'ok', primary: true }] });
      return null;
    }
    const mins = (ms) => (ms ? fmt(ms) : '');
    const pick = await choiceDialog({ title: 'Which song is this?', text: `Collections and bookmarks name each song by its entry in MusicBrainz (the open music encyclopedia). Pick the one that is ${about}${t.duration_ms ? ` (${mins(t.duration_ms)})` : ''}; it is remembered for this file.`, choices: c.suggestions.map((x, k) => ({ label: `${x.title || t.title}${x.artist ? ` · ${x.artist}` : ''}`, note: [x.release_title, `${Math.round(x.score * 100)}% match`].filter(Boolean).join(' · '), value: x.id, primary: k === 0 })) });
    if (pick == null) return null;
    const picked = c.suggestions.find((x) => x.id === pick);
    try { const done = await api('POST', `/library/suggestions/${pick}/accept?track_id=${encodeURIComponent(t.id)}${picked ? `&recording_mbid=${encodeURIComponent(picked.recording_mbid)}` : ''}`); rememberTracks([done]); return done; } catch (e) { toast(e.message, 'error'); return null; }
  };
  // "Wrong match?" (owner, 2026-11-11): the Confirm matches view for songs that have a match, each
  // with its match now, the options it was chosen from, a MusicBrainz search and "Not matched".
  // Nothing changes until Save; a choice names the match the page showed (409: reload).
  const renderFixMatches = async (ids) => {
    const gen = state.idGen = (state.idGen || 0) + 1;
    const live = () => state.view === 'identify' && state.idGen === gen;
    main.innerHTML = pageHead('Wrong match?', null) + '<p class="muted small idloading" role="status">Loading the songs’ matches…</p>';
    let page; let st;
    try { [page, st] = await Promise.all([api('GET', `/library/identify/fix?tracks=${ids.map(Number).join(',')}`), api('GET', '/library/identify').catch(() => null)]); } catch (e) { if (live()) main.innerHTML = pageHead('Wrong match?', null) + emptyState('search', 'Couldn’t load these songs', e.message); return; }
    if (!live()) return;
    const files = !st ? '' : st.write_ids ? '"Write IDs into my files" is on: the file’s IDs are replaced or removed (its tags are backed up first).' : 'Your files stay as they are ("Write IDs into my files" is off).';
    const how = (o) => (o.source === 'fingerprint' ? 'audio fingerprint' : o.source === 'album' ? 'the folder as an album' : 'search');
    const recOf = (o) => o.recording_mbid || o.recording;
    // Each song's options: the kept ones, then what a search found (by recording, once each)
    const extra = new Map(); const chosen = new Map();
    const optRow = (t, o, k) => `<div class="idopt idpick"><input type="radio" id="fx-${t.id}-${k}" name="fx-${t.id}" value="${esc(recOf(o))}" data-rel="${esc(o.source === 'album' ? (o.release_mbid || '') : '')}" ${chosen.get(t.id) === recOf(o) ? 'checked' : ''}><label class="grow" for="fx-${t.id}-${k}"><span class="name">${esc(o.title || 'Recording')} <span class="muted small">${esc(o.artist || '')}</span></span><span class="sub">${o.release_title ? esc(o.release_title) + ' · ' : ''}${how(o)} · ${Math.round((o.score || 0) * 100)}%</span></label><a class="idmb small" href="https://musicbrainz.org/recording/${esc(recOf(o))}" target="_blank" rel="noopener" aria-label="${esc(`${o.title || 'Recording'} on MusicBrainz`)}">MusicBrainz ${icon('external')}</a></div>`;
    const songHtml = (g) => {
      const t = g.track; const cur = g.current;
      const seen = new Set(cur ? [cur.recording_mbid] : []);
      const opts = [...g.options, ...(extra.get(t.id) || [])].filter((o) => { const r = recOf(o); if (seen.has(r)) return false; seen.add(r); return true; });
      const now = cur ? `<div class="idopt idpick"><input type="radio" id="fx-${t.id}-keep" name="fx-${t.id}" value="keep" ${chosen.get(t.id) === 'keep' ? 'checked' : ''}><label class="grow" for="fx-${t.id}-keep"><span class="name">Keep this match${cur.auto ? ' <span class="pill" data-auto>auto</span>' : ''}</span><span class="sub">${cur.auto ? 'Picked automatically: used in Odeion, not written into your file until you keep it here' : `Matched by ${esc(MATCH_HOW[cur.source] || cur.source)}`}</span></label><a class="idmb small" href="https://musicbrainz.org/recording/${esc(cur.recording_mbid)}" target="_blank" rel="noopener" aria-label="The match now on MusicBrainz">MusicBrainz ${icon('external')}</a></div>` : '';
      const body = cur && cur.from_tags && !(st && st.write_ids) ? '<p class="small muted">Its file’s tags name this recording: turn on Write IDs into my files (Your library › Node library › Identify) to change it here, or correct the tags.</p>'
        : `${now}${opts.map((o, k) => optRow(t, o, k)).join('')}<div class="idopt idpick"><input type="radio" id="fx-${t.id}-none" name="fx-${t.id}" value="none" ${chosen.get(t.id) === 'none' ? 'checked' : ''}><label class="grow small" for="fx-${t.id}-none">Not matched <span class="muted">(none of these: the song stays without a match)</span></label></div><div class="idopt idpick idlater"><input type="radio" id="fx-${t.id}-same" name="fx-${t.id}" value="" ${!chosen.get(t.id) ? 'checked' : ''}><label class="grow small muted" for="fx-${t.id}-same">No change</label></div><form class="toolbar" data-find="${t.id}"><input class="input" name="q" placeholder="Title and artist on MusicBrainz" aria-label="${esc(`Search MusicBrainz for ${t.title}`)}" autocomplete="off"><button class="btn sm" type="submit">Search</button></form>`;
      return `<div class="idrow" data-track="${t.id}" data-status="${esc(g.status)}"><div class="idfile"><div class="name">${t.track_no ? `<span class="muted">${t.track_no}.</span> ` : ''}${esc(t.title)}</div><div class="sub">${esc([artists(t), fmt(t.duration_ms), (t.file.path || '').split('/').pop()].filter(Boolean).join(' · '))}</div>${cur ? '' : '<p class="small muted">No match now.</p>'}</div><div class="idopts" role="radiogroup" aria-label="${esc(t.title)}">${body}</div></div>`;
    };
    const draw = () => {
      // None of the songs is in the library now (Back or a reload brings the page back for songs
      // removed since; the node leaves missing songs out): say so, and the way on, not a bare intro
      if (!page.albums.length) {
        main.innerHTML = pageHead('Wrong match?', null) + emptyState('search', 'These songs are no longer in your library', 'They were removed, or moved to another folder. If one was moved, find it in Songs and choose Wrong match? in its menu again.', { id: 'fxsongs', label: 'Open Songs' });
        $('#fxsongs', main).onclick = () => setView('tracks');
        return;
      }
      const say = `Pick the right recording, search MusicBrainz for it, or choose Not matched. The match you replace is never picked again for this song. ${esc(files)} Tags you already published on Pubky for the old match stay on it.`;
      // (on a phone it folds under the title, as Confirm matches' does, so the first choice is on the first screen)
      const opened = !!$('#main details.idhow[open]'); // (a search redraws the page: an opened fold stays open)
      const intro = shellMode() === 'phone' ? `<details class="idhow small"${opened ? ' open' : ''}><summary class="muted">How fixing a match works</summary><p class="muted idintro">${say}</p></details>` : `<p class="muted small idintro">${say}</p>`;
      const albumHtml = (a, ai) => `<section class="section idalbum" data-album="${ai}"><div class="section-head"><h3>${esc(a.album || a.key.split('/').filter(Boolean).slice(-1)[0] || 'Library')}${a.artist ? ` <span class="muted small">${esc(a.artist)}</span>` : ''}</h3><span class="muted small">${countOf(a.songs.length, 'songs')}</span></div><div class="list">${a.songs.map(songHtml).join('')}</div><div class="actions idconfirm"><button class="btn primary" type="button" data-fixsave="${ai}">Save</button><span class="small muted" role="status" data-fixout="${ai}"></span></div></section>`;
      main.innerHTML = pageHead('Wrong match?', page.songs) + intro + page.albums.map(albumHtml).join('');
      $$('.idrow input[type=radio]', main).forEach((r) => r.onchange = () => chosen.set(Number(r.name.slice(3)), r.value));
      $$('[data-find]', main).forEach((f) => f.onsubmit = async (e) => {
        e.preventDefault(); const id = Number(f.dataset.find); const q = f.q.value.trim(); const b = $('button', f); b.disabled = true;
        try {
          const c = await api('GET', `/library/tracks/${id}/candidates${q ? `?q=${encodeURIComponent(q)}` : ''}`);
          if (!live()) return;
          if (c.needs_contact) { b.disabled = false; focusUndo(toast('MusicBrainz needs your email or website first', 'error', { label: 'Add it', run: openContact })); return; }
          extra.set(id, [...(extra.get(id) || []), ...(c.found || [])]);
          if (!(c.found || []).length) toast('MusicBrainz found nothing strong for that');
          draw();
        } catch (err) { toast(err.message, 'error'); b.disabled = false; }
      });
      $$('[data-fixsave]', main).forEach((btn) => btn.onclick = async () => {
        const ai = Number(btn.dataset.fixsave); const a = page.albums[ai]; const out = $(`[data-fixout="${ai}"]`, main);
        const todo = a.songs.filter((g) => chosen.get(g.track.id));
        if (!todo.length) { out.textContent = 'Nothing chosen'; return; }
        btn.disabled = true; out.textContent = 'Saving…'; let done = 0; let failed = null; let notWritten = 0;
        for (const g of todo) {
          const v = chosen.get(g.track.id); const cur = g.current ? g.current.recording_mbid : null;
          const input = $(`input[name="fx-${g.track.id}"]:checked`, main);
          const body = v === 'none' ? { none: true, expect: cur } : v === 'keep' ? { recording_mbid: cur, expect: cur } : { recording_mbid: v, release_mbid: (input && input.dataset.rel) || null, expect: cur };
          try { const r = await api('POST', `/library/tracks/${g.track.id}/match`, body); done++; if (r.files && !r.files.written) notWritten++; } catch (e) { failed = e.message; break; }
        }
        toast(`${countOf(done, 'songs')} corrected${notWritten ? `; IDs not written into ${countOf(notWritten, 'files')}` : ''}${failed ? `; stopped: ${failed}` : ''}`, failed || notWritten ? 'error' : '');
        if (!live()) return;
        // What was saved is the page's now; a stale choice shows as it stands
        if (failed) { await renderFixMatches(ids); return; }
        await renderFixMatches(ids);
      });
    };
    draw();
  };
  // The album has its own column; where the window is too narrow for it (the desktop layout under
  // 900 px) it joins the artist line instead (.by-album), so the title keeps the width. Its dot
  // (.by-dot) goes when the song has no artists (app.css)
  // A list's running number (not an album's track numbers) marks its rows .seq: a phone leaves it
  // out (app.css)
  const trackRows = (tracks, { showAlbum = true, numbers = 'index', key = 'tr', start = 0, bare = false } = {}) => {
    rememberTracks(tracks);
    const z = currentZone(); const np = z && z.now_playing;
    const rows = tracks.map((t, k) => {
      const i = start + k;
      const cur = np && np.item.uri === 'track:' + t.id;
      const fav = isFav(t); const can = trackHasId(t);
      return `<div class="track ${showAlbum ? '' : 'compact'} ${cur ? 'cur' : ''}${numbers === 'track' ? '' : ' seq'}" data-i="${i}" data-tid="${t.id}"${cur ? ' aria-current="true"' : ''}>
        <div class="no"><span class="num">${numbers === 'track' ? (t.track_no ?? '') : i + 1}</span><button class="rowplay" data-act="play" title="Play from here" aria-label="${esc(rowWords.play(t.title))}">${icon('play')}</button></div>
        <div class="main-col"><div class="title"><a class="lnk" data-act="song" href="${esc(pathFor('item', { kind: 'song', id: trackPageId(t) }))}"${rowLinkTab()}>${esc(t.title)}</a>${dlHas(t.id) ? dlMark() : ''}${can ? '' : `<button class="idmark" data-act="identify" title="Not matched to a known song yet. Matching lets other apps and people on Pubky recognise it; it plays fine either way">Match</button>`}</div><div class="by">${artistLinksHtml({ ...t, track_id: t.id }, rowLinkTab())}${showAlbum && t.album ? `<span class="by-album"><span class="by-dot"> · </span><a class="lnk" data-act="album" data-album="${t.album.id}" href="${esc(pathFor('item', { kind: 'album', id: 'lib-' + t.album.id }))}"${rowLinkTab()}>${esc(t.album.title)}</a></span>` : ''}</div></div>
        ${showAlbum ? `<div class="album-col">${t.album ? `<button class="lnk" data-act="album" data-album="${t.album.id}">${esc(t.album.title)}</button>` : ''}</div>` : ''}
        <div class="fmt-col"><span class="fmt ${LOSSY[t.file && t.file.container] ? 'lossy' : ''}">${esc(fileFormat(t.file))}</span></div>
        <div class="dur">${fmt(t.duration_ms)}</div>
        <div class="row-actions">
          <button class="icon-btn heart ${fav ? 'on' : ''}" data-act="fav" aria-pressed="${fav}" title="${fav ? 'Remove from Bookmarks' : 'Add to Bookmarks'}" aria-label="${esc(heartWords(fav, t.title))}">${icon(fav ? 'heart-fill' : 'heart')}</button>
          <button class="icon-btn" data-act="next" title="Play next" aria-label="${esc(rowWords.next(t.title))}">${icon('playnext')}</button>
          <button class="icon-btn" data-act="add" title="Add to queue" aria-label="${esc(rowWords.add(t.title))}">${icon('plus')}</button>
          <button aria-haspopup="menu" aria-expanded="false" class="icon-btn" data-act="more" title="More" aria-label="${esc(rowWords.more(t.title))}">${icon('more')}</button>
        </div>
      </div>`;
    }).join('');
    return bare ? rows : `<div class="tracks" data-list="${key}">${rows}</div>`;
  };
  // Track rows on #main: one listener, bound once, reading the current page's lists (every render
  // used to add another, so ⋯ ran once per visit with the lists of pages left behind)
  let rowLists = {}; let rowsBound = false;
  // Where the click before this one landed, for both kinds of song row: a library row's list, place
  // and song (bindTrackRows), or a playlist's, album's or search's row: its box, place and song (its id,
  // or its address when it has none) (bindSongRows). A redraw keeps them. Every click counts, also one
  // off the rows (''). One on a row's own button or link counts as '' (the heart, then a quick click on
  // the row, plays it), but its play button counts as the row (that click already started it). One that
  // only closes a ⋯ menu (showMenu's outside click, which the menu takes) counts as '' too: the next
  // quick click on the row plays it rather than being held as a double click's second.
  const trackSpot = (el) => { const row = el && el.closest && el.closest('[data-list] .track'); return row ? `t:${row.closest('[data-list]').dataset.list}:${row.dataset.i}:${row.dataset.tid || ''}` : ''; };
  const srowSpot = (r) => `s:${(r.closest('[id]') || {}).id || ''}:${r.dataset.srow}:${r.dataset.tid || r.dataset.suri || ''}`;
  // (a service's own album page: its rows, by place and address)
  const srcSpot = (r) => `a:${r.dataset.i}:${r.dataset.np || ''}`;
  const rowSpot = (el) => { const r = el && el.closest && el.closest('[data-srow], .src-tracks > .track'); return !r ? trackSpot(el) : r.matches('[data-srow]') ? srowSpot(r) : srcSpot(r); };
  let clickWas = ''; let clickLast = '';
  document.addEventListener('click', (e) => {
    const t = e.target; const own = t.closest && t.closest('a, button, input'); const menuTakes = !menuEl.hidden && !menuEl.contains(t);
    clickWas = clickLast; clickLast = menuTakes || (own && !own.matches('[data-act="play"], [data-splay], .src-tracks [data-aplay]')) ? '' : rowSpot(t);
  }, true);
  const bindTrackRows = (container, pageLists) => {
    rowLists = pageLists; if (rowsBound) return; rowsBound = true;
    // A song row's second click of a double click selects no word of it (the first one plays). This
    // stays on every row: a word selected on the next row would hold that row's click
    container.addEventListener('mousedown', (e) => { if (e.detail > 1 && e.target.closest('[data-list] .track') && !e.target.closest('a, button, input, select, textarea, label')) e.preventDefault(); });
    onBusy(container, 'click', async (e) => {
      const lists = rowLists;
      // (a service album's rows have their own handlers: their ⋯ is not a library song's)
      if (e.target.closest('.src-tracks')) return;
      const btn = e.target.closest('[data-act]');
      // A click on the row plays from there, as a playlist's or an album's song row does: with a mouse
      // too, not only on a touch screen (no hover to reveal the play button). Its names and buttons keep
      // their own click (on a phone the names take no taps, app.css, so a tap there plays), and the
      // second click of a double click, or a drag that selects the row's text, doesn't start it. Only a
      // selection with words in it that starts or ends in the row holds it (a mouse drag's click has
      // both ends there): on a touch screen the rows can't be selected, so a selection made elsewhere
      // stays, even one across the rows or an empty one inside a row, and a tap on the row plays.
      // A quick tap on the next row can come as a double tap's second (a phone counts taps up to 100 dp
      // apart, more than a row), so only a second click on the row the first one hit is held.
      if (!btn) {
        const row = e.target.closest('.track'); const list = row && row.closest('[data-list]');
        const again = e.detail > 1 && clickWas === trackSpot(row);
        const sel = getSelection(); const picked = !!row && String(sel) !== '' && (row.contains(sel.anchorNode) || row.contains(sel.focusNode));
        if (row && list && !again && !picked && !e.target.closest('a, button, input')) { const ts = lists[list.dataset.list] || []; state.startAnchor = null; playTracks(ts.slice(Number(row.dataset.i)), true); }
        return;
      }
      const row = btn.closest('.track'); const list = btn.closest('[data-list]');
      const tracks = lists[list ? list.dataset.list : 'tr'] || [];
      const i = Number(row ? row.dataset.i : -1);
      const act = btn.dataset.act;
      // (the artist line's album is a real link, inline so a long name ends in "…": Ctrl or middle click opens a new tab)
      if (act === 'album') { if (btn.tagName === 'A') { if (opensElsewhere(e)) return; e.preventDefault(); } setView('album', Number(btn.dataset.album)); return; }
      if (act === 'identify') { const t = tracks[i]; if (!t) return; const done = await whichSong(t); if (done) { tracks[i] = done; btn.remove(); toast(`Identified: ${done.title}`); } return; }
      if (act === 'song') {
        // A real link (Tab reaches it, Enter opens it); Ctrl or middle click opens the page in a new tab
        if (opensElsewhere(e)) return;
        e.preventDefault();
        const t0 = (lists[list ? list.dataset.list : 'tr'] || [])[Number(btn.closest('.track').dataset.i)]; if (!t0) return;
        setView('item', { kind: 'song', id: trackPageId(t0) });
        return;
      }
      if (i < 0) return;
      if (act === 'play') playTracks(tracks.slice(i), true);
      else if (act === 'next') playTracks([tracks[i]], false, 'next');
      else if (act === 'add') playTracks([tracks[i]], false, 'end');
      else if (act === 'more') trackMenu(btn, [tracks[i]]);
      else if (act === 'fav') {
        const on = btn.classList.contains('on');
        try { if (await setFavourite({ track_id: tracks[i].id }, !on)) setHeart(btn, !on, tracks[i].title); }
        catch (er) { toast(er.message, 'error'); }
      }
    });
  };
  const fmtOf = (id) => (state.formats && state.formats.get(id)) || null;
  const fmtBadge = (id) => { const f = fmtOf(id); return f ? `<span class="fmt ${f.dsd || f.hi_res ? 'hi' : f.lossless ? '' : 'lossy'}" title="Best file on this album">${esc(f.label)}</span>` : ''; };
  // A tile's line under its title in a span of its own, so a long artist name is what is shortened
  // with "…" and the quality badge after it stays whole (app.css .tile .a > .at)
  const tileLine = (text) => (text ? `<span class="at">${esc(text)}</span>` : '');
  const tile = (a) => `<button class="tile" data-album="${a.id}" title="${esc(a.title)}">${art(a.id, a.title, '', 'library', true, `<span class="tile-play" data-tileplay="${a.id}" role="button" title="Play album">${icon('play')}</span>`)}<div class="t">${esc(a.title)}${dlAlbum(a.id) ? dlMark() : ''}</div><div class="a">${tileLine([a.album_artist, a.year].filter(Boolean).join(' · '))}${fmtBadge(a.id)}</div>${coverFrom('library')}</button>`;
  // People on Pubky (STEWARD-UX-PEOPLE.md): profiles from Nexus via the node, cached here too.
  const profileCache = new Map();
  const loadProfiles = async (ids) => {
    const want = [...new Set(ids)].filter((x) => !profileCache.has(x)).slice(0, 50);
    if (want.length) { try { (await api('GET', `/profiles?ids=${want.join(',')}`)).forEach((c) => profileCache.set(c.id, c)); } catch { /* names wait */ } }
    return [...new Set(ids)].map((x) => profileCache.get(x)).filter(Boolean);
  };
  const face = (c, size = 'sm') => (c.avatar ? `<img class="face ${size}" src="${esc(imgSrc(c.avatar))}" alt="" loading="lazy" data-err="hide">` : `<span class="face ${size} mono" style="background:hsl(${hue(c.id)} 35% 34%)">${esc(initials(c.name))}</span>`);
  const peopleMenu = (anchor, title, cards, more = 0) => showMenu(anchor, [{ title }, ...cards.map((c) => ({ label: c.you ? `${c.name} (you)` : c.name, note: c.following ? 'you follow' : (c.known ? null : 'not on Pubky search yet'), face: c, run: () => setView('person', c.id) })), ...(more > 0 ? [{ title: `and ${more} more on Pubky` }] : [])]);
  // Tag colours as pubky.app draws them, so a label looks the same there and here: its
  // generateRandomColor and hexToRgba (pubky/pubky-app @7cb809a, src/libs/utils/utils.ts) ported as
  // they are, the hash that outgrows 32 bits and the mixed-case hex included (tools/tag-colour-check.js)
  const TAG_COLOURS = new Map([['bitcoin', '#FF9900'], ['synonym', '#FF6600'], ['bitkit', '#FF4400'], ['pubky', '#C8FF00'], ['blocktank', '#FFAE00'], ['tether', '#26A17B'], ['ai', '#00C8FF']]);
  const tagColour = (label) => {
    const s = String(label);
    if (TAG_COLOURS.has(s.toLowerCase())) return TAG_COLOURS.get(s.toLowerCase());
    const hash = Math.abs(Array.from(s).reduce((h, ch) => ch.charCodeAt(0) + ((h << 5) - h), 0));
    const byte = hash & 0xff; const hex = byte.toString(16).padStart(2, '0'); const n = hash % 6;
    // The two blue patterns turn teal while their changing channel is low
    if ((n === 3 || n === 4) && byte <= 85) { const cyan = (255 - Math.round(byte * 0.75)).toString(16).padStart(2, '0'); return n === 3 ? `#00${cyan}FF` : `#00FF${cyan}`; }
    return `#${[`FF00${hex}`, `FF${hex}00`, `${hex}FF00`, `${hex}00FF`, `00${hex}FF`, `00FF${hex}`][n]}`;
  };
  // A listener connects their own homeserver (personal.rs): one approval in Pubky Ring for their
  // bookmarks and collections folders, nothing else. Resolves true once the node holds it.
  const homeserverDialog = (lost) => new Promise((resolve) => {
    const d = document.createElement('dialog'); d.className = 'dialog';
    d.innerHTML = `<h2>${lost ? 'Approve this node again' : 'Connect your homeserver'}</h2><p class="muted">${lost ? 'Your homeserver no longer lets this node in (it was revoked in Pubky Ring, say). Approve it again to keep your bookmarks there.' : 'Your bookmarks are kept on your own homeserver, not the node’s. Approve this node once in Pubky Ring, for your bookmarks and collections folders only.'}</p><div id="hsbody"><p class="loginwait"><span class="spin" aria-hidden="true"></span>Starting…</p></div><div class="toolbar"><button class="btn ghost" data-hsclose>Not now</button></div>`;
    document.body.appendChild(d);
    let timer = null; let ended = false;
    const tr = { open: () => d.open, close: (fromBack) => done(fromBack) };
    const done = (fromBack, ok) => { ended = true; clearTimeout(timer); const k = transients.indexOf(tr); if (k >= 0) transients.splice(k, 1); if (d.open) { if (d.close) d.close(); else d.removeAttribute('open'); } d.remove(); if (fromBack !== true) popTransient(); resolve(!!ok); };
    transients.push(tr); pushTransient();
    $('[data-hsclose]', d).onclick = () => done();
    d.addEventListener('cancel', (e) => { e.preventDefault(); done(); });
    const body = $('#hsbody', d);
    const start = async () => {
      try {
        const f = await api('POST', '/me/homeserver/connect');
        if (ended) return;
        body.innerHTML = `<div class="notice qrbox">${f.qr_svg ? `<div class="qr" role="img" aria-label="QR code to scan with Pubky Ring">${f.qr_svg}</div>` : ''}<div><p>Scan with Pubky Ring, or open the link on this device and approve.</p><a class="btn primary" href="${esc(f.url)}">Open in Pubky Ring</a></div></div>`;
        const poll = async () => {
          if (ended) return;
          let p; try { p = await api('GET', `/me/homeserver/connect/poll?id=${encodeURIComponent(f.id)}`); } catch { timer = setTimeout(poll, 3000); return; }
          if (ended) return;
          if (p.state === 'pending') { timer = setTimeout(poll, 2000); return; }
          if (p.state === 'ok') { toast('Your homeserver is connected: your bookmarks are kept there'); done(false, true); return; }
          if (p.state === 'denied') { body.innerHTML = `<p class="notice warn" role="alert">Not kept: ${esc(p.reason || 'Pubky Ring didn’t approve it')}. Approve it with your own identity, for every folder it asks.</p><button class="btn" data-hsagain>Try again</button>`; $('[data-hsagain]', d).onclick = start; return; }
          start(); // the approval expired: a fresh one
        };
        timer = setTimeout(poll, 2000);
      } catch (e) {
        if (ended) return;
        body.innerHTML = `<p class="muted">Can’t start it: ${esc(e.message)}</p><button class="btn" data-hsagain>Try again</button>`;
        $('[data-hsagain]', d).onclick = start;
      }
    };
    nameDialog(d); if (d.showModal) d.showModal(); else d.setAttribute('open', '');
    start();
  });
  const tagRgba = (hex, a) => { const [r, g, b] = hex.match(/\w\w/g).map((x) => parseInt(x, 16)); return `rgba(${r}, ${g}, ${b}, ${a})`; };
  // One tag chip wherever tags show (pubky.app's Tag): the label in its colour, no "#", how many
  // people used it, your own with a border. `attrs` go on the chip's button; `del` (the remove
  // button's attributes) only where there is a remove, like the close on pubky.app's post tags.
  const tagChip = (label, { count, own, attrs = '', title, del, extra = '' } = {}) => {
    const c = tagColour(label);
    const style = `--tag-bg:${tagRgba(c, 0.3)};--tag-line:${tagRgba(c, 0.5)}`;
    const body = `<span class="ptag-l">${esc(label)}</span>${count ? `<span class="ptag-n">${count}</span>` : ''}`;
    if (!del) return `<button class="ptag ${own ? 'own' : ''}" style="${style}" ${attrs} title="${esc(title || label)}">${body}</button>`;
    return `<span class="ptag ${own ? 'own' : ''}" style="${style}"><button class="chiplink" ${attrs} title="${esc(title || label)}">${body}</button>${extra}<button class="chipx" ${del} aria-label="Remove ${esc(label)}" title="Remove">×</button></span>`;
  };
  // Who used a label (pubky.app's TaggedItem): up to four faces, then "+N" for the rest
  const tagFaces = (cards, total) => { const shown = cards.slice(0, 4); return `${shown.map((c) => face(c, 'md')).join('')}${total > shown.length ? `<span class="face-more">+${total - shown.length}</span>` : ''}`; };
  // Your library › Node library › Cloud drives › Add a folder: browse the drive's folders, pick one.
  const driveFolderDialog = (provider, name) => new Promise((resolve) => {
    const d = document.createElement('dialog'); d.className = 'dialog';
    d.innerHTML = `<h2>A folder on ${esc(name)}</h2><p class="muted">Its songs join the library and play from the drive; nothing is copied.</p><div id="dfpath" class="muted small"></div><div id="dfbox" class="list compact picker"></div><p class="muted small" id="dfmsg"></p><div class="toolbar"><button class="btn primary" id="dfadd" disabled>Add this folder</button><button class="btn ghost" data-dfclose>Cancel</button></div>`;
    document.body.appendChild(d);
    const tr = { open: () => d.open, close: (fromBack) => done(fromBack) };
    const done = (fromBack, added) => { const k = transients.indexOf(tr); if (k >= 0) transients.splice(k, 1); if (d.open) { if (d.close) d.close(); else d.removeAttribute('open'); } d.remove(); if (fromBack !== true) popTransient(); resolve(added || null); };
    transients.push(tr); pushTransient();
    $('[data-dfclose]', d).onclick = () => done();
    d.addEventListener('cancel', (e) => { e.preventDefault(); done(); });
    const trail = [{ id: 'root', name }];
    // The list is drawn as Add to playlist's (.picker). A folder row opens that folder in place, so
    // it ends in the right arrow of rows that go somewhere. The focus goes with you (it was lost
    // with the list while the drive reads): a folder just opened gives it to its first folder, or
    // to Add this folder when it has none; Back gives it to the folder you came out of. Only when
    // it was in the list, not when it went to Cancel meanwhile. The list scrolls to it only for
    // the keyboard: after a click or tap it starts at the top, with Back in view.
    let draws = 0; let keyed = false;
    d.addEventListener('keydown', () => { keyed = true; }, true);
    d.addEventListener('pointerdown', () => { keyed = false; }, true);
    const show = async (came, again) => {
      const here = trail[trail.length - 1]; const mine = ++draws; const box = $('#dfbox', d);
      const moved = trail.length > 1 || !!came || !!again;
      $('#dfpath', d).textContent = trail.map((t) => t.name).join(' › ');
      $('#dfadd', d).disabled = true; // (off during every read: on again once it works)
      box.innerHTML = '<p class="muted small"><span class="spin"></span> Reading the drive…</p>';
      const free = () => { const a = document.activeElement; return !a || a === document.body || a === d || box.contains(a); };
      // (Back stays when a folder can't be read, and Try again asks the drive again, so a folder
      // that fails isn't a dead end, the drive's top level included. Add this folder waits for a
      // read that worked: a folder the drive can't list can't be scanned either. The focus goes to
      // Back when you just opened it, to Try again when that failed again.)
      const up = trail.length > 1 ? '<button class="list-row" data-dfup>‹ Back</button>' : '';
      let to = null;
      try {
        const fs = await api('GET', `/library/drives/${provider}/folders?parent=${encodeURIComponent(here.id)}`);
        if (mine !== draws) return;
        $('#dfadd', d).disabled = trail.length < 2;
        box.innerHTML = up + (fs.length ? fs.map((f, k) => `<button class="list-row" data-dfopen="${k}"><span class="grow">${esc(f.name)}</span>${icon('chevron', 'ic right')}</button>`).join('') : '<p class="muted small">No folders inside.</p>');
        $$('[data-dfopen]', d).forEach((b) => b.onclick = () => { trail.push(fs[Number(b.dataset.dfopen)]); show(); });
        const k = came ? fs.findIndex((f) => f.id === came.id) : -1;
        to = $(`[data-dfopen="${Math.max(k, 0)}"]`, d) || $('#dfadd:not([disabled])', d) || $('[data-dfup]', d) || $('[data-dfclose]', d);
      } catch (e) {
        if (mine !== draws) return;
        box.innerHTML = `${up}<p class="muted small hfail" role="alert">${esc(e.message)} <button class="btn sm" type="button" data-dfretry>Try again</button></p>`;
        $('#dfadd', d).disabled = true;
        $('[data-dfretry]', d).onclick = () => show(null, true);
        to = (!again && $('[data-dfup]', d)) || $('[data-dfretry]', d);
      }
      const back = $('[data-dfup]', d); if (back) back.onclick = () => { const left = trail.pop(); show(left); };
      if (moved && to && free()) { to.focus({ preventScroll: true }); if (keyed) to.scrollIntoView({ block: 'nearest' }); }
    };
    $('#dfadd', d).onclick = async () => {
      const here = trail[trail.length - 1];
      try { done(false, await api('POST', '/library/drives', { provider, folder_id: here.id, name: here.name })); } catch (e) { $('#dfmsg', d).textContent = e.message; }
    };
    nameDialog(d); if (d.showModal) d.showModal(); else d.setAttribute('open', '');
    show();
  });
  // Your library › Node library › Add a WebDAV share: the folder's address, user and password; the node
  // checks it, keeps it and scans it (POST /library/webdav).
  const webdavDialog = () => new Promise((resolve) => {
    const d = document.createElement('dialog'); d.className = 'dialog';
    d.innerHTML = `<h2>Add a WebDAV share</h2><p class="muted">A music folder on Nextcloud, ownCloud or a NAS. The node reads it over the network and plays from it; nothing is copied.</p>
      <label class="field"><span>Folder address</span><input class="input" id="wdurl" placeholder="https://cloud.example.org/remote.php/dav/files/me/Music/" autocomplete="off"></label>
      <label class="field"><span>User name</span><input class="input" id="wduser" autocomplete="username"></label>
      <label class="field"><span>Password or app password</span><input class="input" id="wdpass" type="password" autocomplete="current-password"></label>
      <label class="field"><span>Name (optional)</span><input class="input" id="wdname" placeholder="NAS"></label>
      <p class="muted small" id="wdmsg"></p><div class="toolbar"><button class="btn primary" id="wdadd">Add share</button><button class="btn ghost" data-wdclose>Cancel</button></div>`;
    document.body.appendChild(d);
    const tr = { open: () => d.open, close: (fromBack) => done(fromBack) };
    const done = (fromBack, added) => { const k = transients.indexOf(tr); if (k >= 0) transients.splice(k, 1); if (d.open) { if (d.close) d.close(); else d.removeAttribute('open'); } d.remove(); if (fromBack !== true) popTransient(); resolve(added || null); };
    transients.push(tr); pushTransient();
    $('[data-wdclose]', d).onclick = () => done();
    d.addEventListener('cancel', (e) => { e.preventDefault(); done(); });
    $('#wdadd', d).onclick = async () => {
      const b = $('#wdadd', d); b.disabled = true; $('#wdmsg', d).innerHTML = '<span class="spin"></span> Checking the share…';
      try {
        const sh = await api('POST', '/library/webdav', { url: $('#wdurl', d).value.trim(), user: $('#wduser', d).value.trim(), password: $('#wdpass', d).value, name: $('#wdname', d).value.trim() || undefined });
        done(false, sh);
      } catch (e) { b.disabled = false; $('#wdmsg', d).textContent = e.message; }
    };
    showFieldDialog(d, $('#wdurl', d));
  });
  // ---- An item page's tags, in its top card (TASK-tags-in-hero-pubky-design) ----
  // Each identity the page has is a group of its own: the item, whatever source plays it, and this
  // source's link (STEWARD-UX-INSTANCES 3). A group reads, adds, removes and counts on its own URI
  // only, and the people beside a label are that URI's for that label, never a page-wide lookup by
  // label. Others' tags first by how many used them, yours among them with a border; about two rows,
  // then "+N more" shows the rest in place.
  //   ht: { key, kind, groups: [{ scope, name, ref, own: [tag] | null, pub: { state, tags, faces } }],
  //         canEdit, note, match, open: Set of the groups shown whole }
  const qsOf = (r) => new URLSearchParams(Object.entries(r).map(([k, v]) => [k, String(v)])).toString();
  // "On the TIDAL link" inside a sentence: only its first letter goes small
  const lower1 = (t) => t.charAt(0).toLowerCase() + t.slice(1);
  // The people of one label, from a group's Pubky answer: only its own entries (a label named
  // "constructor" is a label, not something every object has)
  const facesOf = (pub, label) => { const f = pub && pub.faces; return f && Object.prototype.hasOwnProperty.call(f, label) && Array.isArray(f[label]) ? f[label] : []; };
  const cardOf = (id) => profileCache.get(id) || { id, name: shortKey(id), known: false };
  const heroTagGroup = (g, ht) => {
    const pub = g.pub; const counts = new Map(pub.tags || []);
    const mine = new Map((g.own || []).map((t) => [t.label, t]));
    const edit = ht.canEdit && !!g.own;
    const other = ht.groups.find((x) => x !== g);
    const moves = edit && other && !!other.own;
    const list = [...new Set([...(pub.tags || []).map(([l]) => l), ...mine.keys()])]
      .map((label, i) => ({ label, count: Math.max(counts.get(label) || 0, mine.has(label) ? 1 : 0), own: mine.get(label), i }))
      .sort((a, b) => b.count - a.count || a.i - b.i);
    const items = list.map((t) => {
      const ids = facesOf(pub, t.label);
      const move = t.own && moves ? `<button class="chipx" data-htag-move="${g.scope}" data-label="${esc(t.label)}" aria-label="Move ${esc(t.label)} ${esc(lower1(other.name))}" title="${g.scope === 'link' ? `Put it on the ${esc(ht.kind)} instead` : `Keep it on the ${esc(ht.linkName)} link only`}">${icon(g.scope === 'link' ? 'compass' : 'disc')}</button>` : '';
      const chip = tagChip(t.label, { count: t.count, own: !!t.own, attrs: `data-htag-open="${esc(t.label)}"`, title: `${t.own ? `Your tag${t.own.synced ? '' : ', not pushed yet'}` : `${t.count} ${t.count === 1 ? 'person' : 'people'}`} · open ${t.label}`, del: t.own && edit ? `data-htag-del="${g.scope}" data-label="${esc(t.label)}"` : null, extra: move });
      const wait = t.own && t.own.pending ? '<span class="muted small tag-wait" title="Published once this file is matched to a song">waits for a match</span>' : '';
      const who = ids.length ? `<button class="faces tagfaces" data-htag-faces="${g.scope}" data-label="${esc(t.label)}" aria-label="Who tagged ${esc(t.label)}" title="Who tagged ${esc(t.label)}">${tagFaces(ids.map(cardOf), t.count)}</button>` : '';
      return `<span class="tagitem" data-label="${esc(t.label)}">${chip}${wait}${who}</span>`;
    }).join('');
    const status = pub.state === 'loading' ? '<span class="ptag tag-skel" aria-hidden="true"></span><span class="ptag tag-skel" aria-hidden="true"></span>'
      : pub.state === 'error' ? '<span class="muted small">Pubky tags didn\'t load</span>' : list.length ? '' : '<span class="muted small">No tags yet</span>';
    const mineFailed = ht.canEdit && !g.own ? '<span class="muted small">Your tags didn\'t load</span>' : '';
    const more = ht.open.has(g.scope) ? '' : `<button class="ptag ptag-more" data-htag-more="${g.scope}" hidden></button>`;
    const add = edit ? `<button class="ptag ptag-add" data-htag-plus="${g.scope}" aria-label="Add a tag ${esc(lower1(g.name))}" title="Add a tag: public on your homeserver">${icon('plus')}</button>` : '';
    return `<div class="tgroup" data-tag-scope="${g.scope}" role="group" aria-label="Tags ${esc(lower1(g.name))}"><span class="tgroup-name">${g.scope === 'link' ? svcBadge(String(ht.linkName).toLowerCase()) : ''}${esc(g.name)}</span>${items}${status}${mineFailed}${more}${add}</div>`;
  };
  // About two rows: the chips that would start a third wait behind "+N more" (measured, so it
  // holds at any width; a row is chips whose tops are within a few pixels)
  const foldHeroTags = (box) => {
    const rows = (els) => { const tops = els.map((el) => el.getBoundingClientRect().top).sort((a, b) => a - b); return tops.filter((t, k) => k === 0 || t - tops[k - 1] > 8).length; };
    $$('.tgroup', box).forEach((gr) => {
      const more = $('[data-htag-more]', gr); if (!more) return;
      const items = $$('.tagitem', gr); items.forEach((it) => { it.hidden = false; }); more.hidden = true;
      const shown = () => [...items.filter((it) => !it.hidden), ...$$('.tgroup-name, [data-htag-more]:not([hidden]), [data-htag-plus]:not([hidden]), input[data-htag-add], .tag-hint, .tgroup > .muted.small', gr)].filter((el) => el.getClientRects().length);
      for (let k = items.length - 1; k > 0 && rows(shown()) > 2; k--) { items[k].hidden = true; more.hidden = false; more.textContent = `+${items.length - k} more`; }
    });
  };
  // Suggestions while adding: the labels on this page, then your own from everywhere (read once)
  const tagLabels = { mine: null };
  // Half-typed tags, per page and group: they outlive the page being drawn again (a tag added in the other group,
  // Pubky's answer). One being sent is kept apart until it lands, so it is never offered twice
  const heroDrafts = new Map(); const heroSending = new Set(); let heroNow = null;
  // Pubky's last tags answers for the item page on screen, per group: the page drawn again (a rating
  // saved, a bookmark) shows them while it asks again, so the stars under them don't jump for a round
  // trip. A tag edit forgets them (an earlier answer would show a removed tag again).
  // (a new map: an answer still on its way from before the edit lands in the old one). Quick redraws
  // each ask; each kept answer has its ask's number, and one that lands is kept only when it is newer
  // than the one kept (an older answer landing late doesn't stand in on the next redraw, a newer one
  // landing before the newest ask's does). A failed ask (network, 5xx) changes nothing kept.
  let heroPub = { page: null, by: new Map() };
  let heroPubAsk = 0;
  function forgetHeroPub() { heroPub.by = new Map(); }
  const openHeroAdd = async (box, ht, b, focus = true) => {
    const g = ht.groups.find((x) => x.scope === b.dataset.htagPlus); if (!g) return;
    b.hidden = true;
    b.insertAdjacentHTML('afterend', `<input class="taginput" data-htag-add="${g.scope}" list="htagsugg" maxlength="20" autocomplete="off" placeholder="Add a tag…" aria-label="Add a tag ${esc(lower1(g.name))}"><span class="muted small tag-hint">Public on your homeserver</span>`);
    const inp = b.nextElementSibling; const hint = inp.nextElementSibling;
    const dk = `${ht.page}|${g.scope}`;
    const close = () => { heroDrafts.delete(dk); inp.remove(); hint.remove(); b.hidden = false; };
    if (focus) inp.focus();
    inp.oninput = () => { if (inp.value.trim()) heroDrafts.set(dk, inp.value); else heroDrafts.delete(dk); };
    inp.onkeydown = async (e) => {
      if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); close(); b.focus(); return; }
      if (e.key !== 'Enter' || e.isComposing || inp.disabled || !inp.value.trim()) return;
      const label = inp.value.trim(); inp.disabled = true; heroDrafts.set(dk, inp.value); heroSending.add(dk);
      try { await api('PUT', '/tags', { ...g.ref, label }); heroSending.delete(dk); heroDrafts.delete(dk); toast(`Tagged ${label.toLowerCase()}`); render(); } catch (er) {
        heroSending.delete(dk); toast(er.message, 'error');
        if (inp.isConnected) { inp.disabled = false; inp.focus(); if (ht.later) { ht.later = false; drawHeroTags(ht); } }
        // The page was drawn again meanwhile (the other group's tag landed): what was typed comes back there
        else if (heroNow && heroNow.page === ht.page) drawHeroTags(heroNow);
      }
    };
    inp.onblur = () => setTimeout(() => { if (inp.isConnected && !inp.value.trim() && !inp.disabled && document.activeElement !== inp) close(); }, 150);
    const here = ht.groups.flatMap((x) => [...(x.pub.tags || []).map(([l]) => l), ...(x.own || []).map((t) => t.label)]);
    const fill = () => { const dl = $('#htagsugg', box); if (dl) dl.innerHTML = [...new Set([...here, ...(tagLabels.mine || [])])].slice(0, 200).map((l) => `<option value="${esc(l)}"></option>`).join(''); };
    fill();
    if (!tagLabels.mine) { try { tagLabels.mine = [...new Set(((await api('GET', '/tags')).tags || []).map((t) => t.label))]; fill(); } catch { /* this page's labels will do */ } }
  };
  const bindHeroTags = (box, ht) => {
    const groupOf = (scope) => ht.groups.find((g) => g.scope === scope);
    const del = (ref, label) => api('DELETE', `/tags?${qsOf({ ...ref, label })}`);
    $$('[data-htag-open]', box).forEach((b) => b.onclick = () => setView('discover', { tag: b.dataset.htagOpen }));
    $$('[data-htag-more]', box).forEach((b) => b.onclick = () => { ht.open.add(b.dataset.htagMore); const gr = b.closest('.tgroup'); const next = $('.tagitem[hidden] button', gr); $$('.tagitem', gr).forEach((it) => { it.hidden = false; }); b.remove(); if (next) next.focus(); });
    $$('[data-htag-faces]', box).forEach((b) => b.onclick = () => { const g = groupOf(b.dataset.htagFaces); const label = b.dataset.label; const ids = facesOf(g.pub, label); const cards = ids.map(cardOf); peopleMenu(b, label, cards, (new Map(g.pub.tags || []).get(label) || ids.length) - cards.length); });
    $$('[data-htag-del]', box).forEach((b) => b.onclick = async () => { const g = groupOf(b.dataset.htagDel); if (!g || b.disabled) return; b.disabled = true; try { await del(g.ref, b.dataset.label); render(); } catch (er) { b.disabled = false; toast(er.message, 'error'); } });
    // A move writes the other URI first: when that fails, the tag stays where it was
    $$('[data-htag-move]', box).forEach((b) => b.onclick = async () => { const g = groupOf(b.dataset.htagMove); const to = ht.groups.find((x) => x !== g); const label = b.dataset.label; if (!g || !to || b.disabled) return; b.disabled = true; try { await api('PUT', '/tags', { ...to.ref, label }); await del(g.ref, label); toast(`${label} is ${lower1(to.name)} now`); render(); } catch (er) { b.disabled = false; toast(er.message, 'error'); } });
    // Opening the input is at once: the suggestions it reads after aren't the button's to wait for
    $$('[data-htag-plus]', box).forEach((b) => b.onclick = () => { openHeroAdd(box, ht, b); });
    const m = $('[data-htag-match]', box); if (m) m.onclick = () => setView('identify', { tracks: ht.match.tracks });
  };
  // The faces come with the people's names and pictures once their profiles are read
  const fillHeroFaces = async (box, ht) => {
    const ids = ht.groups.flatMap((g) => Object.values(g.pub.faces || {}).flat());
    if (!ids.length || ids.every((x) => profileCache.has(x))) return;
    await loadProfiles(ids);
    if (!box.isConnected || box.dataset.for !== ht.key) return;
    $$('[data-htag-faces]', box).forEach((b) => { const g = ht.groups.find((x) => x.scope === b.dataset.htagFaces); const fids = facesOf(g.pub, b.dataset.label); b.innerHTML = tagFaces(fids.map(cardOf), new Map(g.pub.tags || []).get(b.dataset.label) || fids.length); });
  };
  let heroTagsRo = null;
  const drawHeroTags = (ht) => {
    const box = $('#htags'); if (!box || box.dataset.for !== ht.key) return;
    // Half-typed tags stay when the public tags come in meanwhile (each group's own); while one is being sent, the
    // redraw waits for its answer
    heroNow = ht;
    const typing = $$('input[data-htag-add]', box);
    if (typing.some((t) => t.disabled)) { ht.later = true; return; }
    const open = new Map(typing.map((t) => [t.dataset.htagAdd, { value: t.value, focus: document.activeElement === t }]));
    // What was typed on this page before it was drawn again, except one being sent
    ht.groups.forEach((g) => { const dk = `${ht.page}|${g.scope}`; if (!open.has(g.scope) && heroDrafts.has(dk) && !heroSending.has(dk)) open.set(g.scope, { value: heroDrafts.get(dk), focus: false }); });
    // An empty input nobody is in isn't kept (it would never close)
    const drafts = [...open].filter(([, d]) => d.value.trim() || d.focus).map(([scope, d]) => ({ scope, ...d }));
    // Not matched yet: one small row under Play and the others (owner, 2026-11-08: the full-width
    // button above Play was "placed weirdly and very large"); other notes stay with the tags
    const slot = $('#hmatch');
    const matchRow = ht.note && ht.match && slot;
    box.innerHTML = `${ht.groups.map((g) => heroTagGroup(g, ht)).join('')}${ht.note && !matchRow ? `<p class="hnote small"><span class="muted">${esc(ht.note)}</span>${ht.match ? ` <button class="btn sm" data-htag-match>${esc(ht.match.label)}</button>` : ''}</p>` : ''}<datalist id="htagsugg"></datalist>`;
    if (slot) {
      slot.innerHTML = matchRow ? `<p class="hmatch small"><span class="muted" title="${esc(ht.note)}">Not matched to MusicBrainz</span><span class="muted" aria-hidden="true">·</span><button class="lnk hmatch-btn" data-htag-match aria-label="${esc(ht.match.label)}" title="${esc(ht.note)}">Match</button></p>` : '';
      const mb = $('[data-htag-match]', slot); if (mb) mb.onclick = () => setView('identify', { tracks: ht.match.tracks });
    }
    bindHeroTags(box, ht);
    drafts.forEach((d) => { const b = $(`[data-htag-plus="${d.scope}"]`, box); if (b) { openHeroAdd(box, ht, b, d.focus); const inp = $(`input[data-htag-add="${d.scope}"]`, box); if (inp) inp.value = d.value; } });
    foldHeroTags(box);
    // One observer, on the current box (a redrawn page has a new one)
    if (!box.dataset.fold && window.ResizeObserver) { box.dataset.fold = '1'; if (heroTagsRo) heroTagsRo.disconnect(); let w = box.clientWidth; heroTagsRo = new ResizeObserver(() => { if (!box.isConnected) { heroTagsRo.disconnect(); return; } if (box.clientWidth !== w) { w = box.clientWidth; foldHeroTags(box); } }); heroTagsRo.observe(box); }
    fillHeroFaces(box, ht);
  };
  let heroTagsGen = 0;
  // Ratings are 1–10: five stars in halves (7 shows three and a half).
  const starsText = (v) => (v ? `${Math.floor(v / 2) || ''}${v % 2 ? '½' : ''}` : '');
  const starFill = (v, n) => (!v ? 0 : v >= 2 * n ? 2 : v === 2 * n - 1 ? 1 : 0);
  const reviewRow = (r) => { const v = r ? r.rating : null; return `<div class="reviewrow"><span class="eyebrow">My rating</span><span class="stars" id="stars" role="group" aria-label="My rating, in half stars">${[1, 2, 3, 4, 5].map((n) => `<span class="star f${starFill(v, n)}"><span class="glyph" aria-hidden="true">★</span><button class="half l" data-rate="${2 * n - 1}" aria-pressed="${v === 2 * n - 1}" aria-label="${n - 1 ? `${n - 1} and a half stars` : 'Half a star'}" title="${starsText(2 * n - 1)}"></button><button class="half r" data-rate="${2 * n}" aria-pressed="${v === 2 * n}" aria-label="${n} star${n === 1 ? '' : 's'}" title="${n}"></button></span>`).join('')}</span><span class="ratingval" id="ratingval">${v ? `${starsText(v)} of 5` : ''}</span><button class="btn sm ghost" id="notesbtn">${r && r.text ? 'Edit notes' : 'Add notes'}</button>${r && r.text ? `<div class="notes clamp" id="notes">${esc(r.text)}</div><button type="button" class="lnk small notes-more" id="notesmore" aria-controls="notes" aria-expanded="false" hidden>More</button>` : ''}<div class="notes-edit" id="notesedit" hidden><textarea class="input" id="notestext" rows="3" placeholder="Notes, kept privately on your homeserver">${esc(r ? r.text : '')}</textarea><div class="actions" style="margin-top:8px"><button class="btn primary sm" id="notessave">Save</button><button class="btn sm ghost" id="notescancel">Cancel</button></div></div></div>`; }; // lint-allow-glyph: the stars are the rating component's own styled glyphs
  const bindReviewRow = (ref, r) => {
    const stars = $('#stars'); if (!stars) return;
    const save = async (rating, text) => { try { if (rating == null && !text) await api('DELETE', '/reviews?' + new URLSearchParams(Object.entries(ref).map(([k, v]) => [k, String(v)])).toString()); else await api('PUT', '/reviews', { ...ref, rating, text }); toast('Review saved'); render(); } catch (e) { toast(e.message, 'error'); } };
    const cur = r ? r.rating : null;
    const paint = (v) => $$('.star', stars).forEach((s, i) => { s.className = `star f${starFill(v, i + 1)}`; });
    $$('[data-rate]', stars).forEach((b) => {
      b.onmouseenter = () => paint(Number(b.dataset.rate));
      b.onfocus = () => paint(Number(b.dataset.rate));
      // The same value again clears the rating
      // (busy until it is saved: a quick second press on the same star sends nothing)
      b.onclick = () => { const v = Number(b.dataset.rate); return save(cur === v ? null : v, r ? r.text : ''); };
    });
    stars.onmouseleave = () => paint(cur);
    stars.addEventListener('focusout', () => paint(cur));
    // Saved notes show their first lines; More shows them all (measured again when the width or the
    // text size changes, as the About blurb is)
    const notes = $('#notes'); const more = $('#notesmore');
    const syncMore = () => { if (notes && more && !notes.hidden && notes.classList.contains('clamp')) more.hidden = !(notes.scrollHeight > notes.clientHeight + 2); };
    if (notes && more) {
      more.onclick = () => { const open = notes.classList.toggle('clamp') === false; more.textContent = open ? 'Less' : 'More'; more.setAttribute('aria-expanded', String(open)); };
      syncMore(); if (window.ResizeObserver) new ResizeObserver(syncMore).observe(notes);
    }
    $('#notesbtn').onclick = () => { $('#notesedit').hidden = false; $('#notesbtn').hidden = true; if (notes) { notes.hidden = true; more.hidden = true; } $('#notestext').focus(); };
    $('#notescancel').onclick = () => { $('#notesedit').hidden = true; $('#notesbtn').hidden = false; if (notes) { notes.hidden = false; more.hidden = notes.classList.contains('clamp'); syncMore(); } $('#notesbtn').focus(); };
    $('#notessave').onclick = () => save(r ? r.rating : null, $('#notestext').value.trim());
  };

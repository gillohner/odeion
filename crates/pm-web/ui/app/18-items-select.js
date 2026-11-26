  // ---- one row, one tile, one menu (docs/ux/08-item-page.md) ----
  // A node link with this browser's sign-in, for the places that load it (an <img>, MediaSession
  // artwork). Never in data sent to the node: a queue item's image is kept as the page gave it.
  // A node link is a relative `/v1/` link or an absolute one on this page's own origin (the node
  // drops the sign-in from both); any other site's link is left alone. A link saved before (a
  // browser queue in localStorage) may already hold a sign-in: every `token` key, read as the
  // node reads it (decoded, any case), goes first, so there is one, joined to the link's query.
  const isTokenKey = (p) => { try { return decodeURIComponent(p.split('=')[0].replace(/\+/g, ' ')).toLowerCase() === 'token'; } catch { return false; } };
  const imgSrc = (u) => {
    if (!u) return u;
    let origin = '';
    if (!u.startsWith('/v1/')) {
      if (!/^https?:\/\//i.test(u)) return u;
      let x; try { x = new URL(u); } catch { return u; }
      if (x.origin !== location.origin || !x.pathname.startsWith('/v1/')) return u;
      origin = x.origin; u = x.pathname + x.search + x.hash;
    }
    const [head, hash = ''] = u.split('#');
    const [path, query = ''] = head.split('?');
    const keep = query.split('&').filter((p) => p && !isTokenKey(p));
    if (state.token) keep.push('token=' + encodeURIComponent(state.token));
    return origin + path + (keep.length ? '?' + keep.join('&') : '') + (hash ? '#' + hash : '');
  };
  // A link to this node, as sent to a zone: one under this page's own origin made relative, so it
  // names the node as the node names itself whatever name the page was opened under (localhost, a
  // tunnel name), and without any sign-in a saved copy still carries (the node drops those too;
  // a sign-in is never data)
  const nodeRel = (u) => {
    if (typeof u !== 'string') return u;
    if (u.startsWith(location.origin + '/v1/')) u = u.slice(location.origin.length);
    if (!u.startsWith('/v1/')) return u;
    const [head, hash = ''] = u.split('#');
    const [path, query = ''] = head.split('?');
    const keep = query.split('&').filter((p) => p && !isTokenKey(p));
    return path + (keep.length ? '?' + keep.join('&') : '') + (hash ? '#' + hash : '');
  };
  const qOf = (t, fallback = {}) => ({ uri: t.track_id != null ? 'track:' + t.track_id : nodeRel(t.uri), track_id: t.track_id, artists: t.artists, title: t.title, artist: t.artist || fallback.artist || '', album: t.album || fallback.album || '', duration_ms: t.duration_ms || null, image: nodeRel(t.image || fallback.image || null) });
  // Library tracks by id, in order and with duplicates: the ones a page already loaded come
  // from the track cache, the rest are read four at a time (not one request per song at once).
  const readTracks = async (ids, signal) => {
    const out = ids.map((id) => state.trackCache.get('track:' + id) || null);
    const need = [...new Set(ids.filter((id, k) => !out[k]))]; const got = new Map();
    const worker = async () => { while (need.length) { const id = need.shift(); got.set(id, await api('GET', `/library/tracks/${id}`, undefined, { signal })); } };
    await Promise.all(Array.from({ length: Math.min(4, need.length) }, worker));
    rememberTracks([...got.values()]);
    return ids.map((id, k) => out[k] || got.get(id));
  };
  // Play a list of item songs from index k: library tracks, streamed items, or Spotify Connect.
  // (library songs: true once in, false when it failed and said so, as playTracks)
  const playSongs = async (songs, k = 0, how = 'replace', pf, anchorEl, fallback, o = {}) => {
    // (a node zone's turn first: the library read and the finding wait inside it, and it goes to the
    // zone it was asked on)
    if (!onBrowser(o) && !o.turn) return withTurn(state.zone, o, (o2) => playSongs(songs, k, how, pf, anchorEl, fallback, o2));
    const first = songs[k];
    // (a page's Play or Shuffle starts from the songs that play; discovery when none does)
    const playsDirectly = (t) => t.track_id != null || t.uri;
    if (first && first.track_id == null && !first.uri && first.page && (o.shuffle === undefined || !songs.slice(k).some(playsDirectly))) {
      // A song only known by its link: the resolver finds where it plays (with the mode asked for, and
      // where it was asked: next, at the end, or in place of the queue)
      return discoverPlay({ uri: first.page, page: first.page, title: first.title, artist: first.artist, image: first.image }, null, anchorEl, { ...o, how });
    }
    const list = songs.slice(k).filter((t) => t.track_id != null || t.uri);
    if (!list.length) { toast('Nothing here plays on your services', 'error'); return false; }
    // (Spotify plays it on its own device, not in the zone's queue: no turn to hold)
    if (pf && pf.mode === 'connect') { spotifyPlay((o.shuffle ? shuffleFrom([...list]) : list).map((t) => t.uri).filter(Boolean), anchorEl); return; }
    if (list.every((t) => t.track_id != null)) { try { const tr = await readTracks(list.map((t) => t.track_id), o.turn && o.turn.signal); return how === 'replace' ? playTracks(tr, true, 'end', o) : playTracks(tr, false, how, o); } catch (e) { toast(e.message, 'error'); return false; } }
    // In a mixed queue a library row has track_id, not a provider URI.
    // Keep every playable row (and duplicates) in its original position.
    const items = list.map((t) => qOf(t, fallback));
    return how === 'replace' ? playSourceItems(items, true, 'end', o) : playSourceItems(items, false, how, o);
  };
  const copyLink = async (path) => { const u = location.origin + path; try { await navigator.clipboard.writeText(u); toast('Link copied'); } catch { toast(u); } };
  // The ⋯ menu, the same on a page's hero and on every song row.
  // "Add to playlist…" (UX review #3): one dialog instead of every playlist in the menu: a search
  // field, "New playlist" first, then your playlists, most recently changed first.
  // On a phone the heading takes the first focus, not the search field (showFieldDialog): a list
  // that is usually one tap.
  const playlistPicker = (add, newName, makeNew) => new Promise((resolve) => {
    const d = document.createElement('dialog'); d.className = 'dialog';
    d.innerHTML = `<h2>Add to collection</h2><input class="input" id="ppq" placeholder="Find a collection" aria-label="Find a collection" autocomplete="off"><div class="list compact picker" id="pplist"></div><div class="dialog-actions"><button class="btn ghost" data-ppclose>Cancel</button></div>`;
    document.body.appendChild(d);
    const tr = { open: () => d.open, close: (fromBack) => done(fromBack) };
    const done = (fromBack) => { const k = transients.indexOf(tr); if (k >= 0) transients.splice(k, 1); if (d.open) { if (d.close) d.close(); else d.removeAttribute('open'); } d.remove(); if (fromBack !== true) popTransient(); resolve(); };
    transients.push(tr); pushTransient();
    const draw = () => {
      const q = $('#ppq', d).value.trim().toLowerCase();
      const list = [...state.playlists].sort((a, b) => (b.updated_at || 0) - (a.updated_at || 0)).filter((p) => !q || p.name.toLowerCase().includes(q));
      $('#pplist', d).innerHTML = `<button class="list-row" data-ppnew>${icon('plus')}<div class="grow"><div class="name">New collection${q ? `: “${esc($('#ppq', d).value.trim())}”` : ''}</div></div></button>` + list.map((p, k) => `<button class="list-row" data-ppi="${k}">${icon('playlist')}<div class="grow"><div class="name">${esc(p.name)}</div><div class="sub">${plCount(p)}</div></div></button>`).join('');
      $('[data-ppnew]', d).onclick = async () => { done(); await makeNew($('#ppq', d) && $('#ppq', d).value.trim() || newName); };
      $$('[data-ppi]', d).forEach((b) => b.onclick = async () => { const pl = list[Number(b.dataset.ppi)]; done(); await add(pl); });
    };
    $('#ppq', d).oninput = draw; draw();
    $('[data-ppclose]', d).onclick = () => done();
    d.addEventListener('cancel', (e) => { e.preventDefault(); done(); });
    showFieldDialog(d, $('#ppq', d));
  });
  const itemMenu = async (anchorEl, o) => {
    await pubkyUser();
    if (!state.playlists.length) { try { state.playlists = await api('GET', '/playlists'); } catch { /* keep */ } }
    const uris = (o.songs || []).map((t) => t.page || (t.id && (uriOf('song', t.id) || null))).filter(Boolean);
    // A library song's page id is lib-<n> until it is matched (trackPageId)
    const libSongs = (o.songs || []).filter((t) => t.track_id != null).map((t) => ({ id: t.track_id, local: !t.id || /^lib-\d+$/.test(String(t.id)) }));
    const trackIds = libSongs.map((t) => t.id);
    // All of them library songs: they go in by track id (and the toast can tell which are matched)
    const byIds = trackIds.length > 0 && trackIds.length === (o.songs || []).length;
    const single = (o.songs || []).length === 1 ? o.songs[0] : null;
    const singleFav = !!(single && songFav(single));
    // The page you are on: no "Go to" it
    const onThis = (kind, pid) => state.view === 'item' && state.viewArg && state.viewArg.kind === kind && (state.viewArg.id === pid);
    const addTo = (pl) => (byIds ? addToPlaylist(pl, libSongs, null) : addToPlaylist(pl, [], uris.length ? uris : o.uri ? [o.uri] : []));
    showMenu(anchorEl, [{ title: o.title || 'Item' },
      { label: 'Play next', icon: 'playnext', run: () => playSongs(o.songs || [], 0, 'next', o.plays_from, anchorEl, o.fallback) },
      { label: 'Add to queue', icon: 'plus', run: () => playSongs(o.songs || [], 0, 'end', o.plays_from, anchorEl, o.fallback) },
      { label: 'Add to collection…', icon: 'playlist', run: () => playlistPicker(addTo, o.title || 'New collection', async (name) => { try { await api('POST', '/playlists', byIds ? { name, track_ids: trackIds } : { name, uris: uris.length ? uris : o.uri ? [o.uri] : [] }); state.playlists = await api('GET', '/playlists'); toast(`Created “${name}”` + (byIds ? stayNote(libSongs) : '')); } catch (e) { toast(e.message, 'error'); } }) },
      ...(o.dl ? dlItem(o.dl.kind, o.dl.id, o.title, o.dl.sub, o.dl.online, o.dl.songs) : single && single.track_id != null ? dlItem('track', single.track_id, single.title, single.artist) : []),
      ...(single ? [{ label: singleFav ? 'Remove from Bookmarks' : 'Add to Bookmarks', icon: singleFav ? 'heart-fill' : 'heart', run: async () => { try { if (await setFavourite(single.track_id != null ? { track_id: single.track_id } : { uri: single.page || uriOf('song', single.id) }, !singleFav)) render(); } catch (e) { toast(e.message, 'error'); } } }] : []),
      // (as a song row's Start radio, trackMenu: the radio goes on for the zone whose queue the song
      // replaced, also when you switch zone while it is sent, and the toast then names that zone;
      // before, the zone switched to got the radio and the one playing the song had none)
      // The song itself plays in that zone, as from a Songs row: radio follows a zone's queue, so a
      // page that plays on a Spotify Connect device does not send it there (before: Spotify played
      // the song and the zone's radio went on beside it, saying similar music follows)
      ...(single && trackIds.length === 1 && !isBrowser() ? [{ label: 'Start radio', icon: 'radio', run: async () => { const zid = state.zone; try { if (await playSongs([single], 0, 'replace', null, anchorEl, o.fallback) === false) return; await api('PUT', `/zones/${encodeURIComponent(zid)}/radio`, { on: true }); const zs = await api('GET', `/zones/${encodeURIComponent(zid)}`); state.zones[zs.id] = zs; const away = state.zone !== zid; toast(`Radio from “${single.title}”${away ? ` on ${zs.name || 'the other device'}` : ''}: similar music follows`); if (state.view === 'queue' && !away) render(); } catch (e) { toast(e.message, 'error'); } } }] : []),
      ...(single && single.id && !onThis('song', single.id) ? [{ label: 'Go to song page', icon: 'compass', run: () => setView('item', { kind: 'song', id: single.id }) }] : []),
      ...(goArtists(o.credits) || (o.artistId ? [{ label: 'Go to artist', icon: 'artist', run: () => setView('item', { kind: 'artist', id: o.artistId }) }] : o.artistName ? [{ label: `Search “${o.artistName}”`, icon: 'artist', run: () => { search.value = o.artistName; runSearch(); } }] : [])),
      ...(o.albumId && !onThis('album', o.albumId) ? [{ label: 'Go to album', icon: 'disc', run: () => setView('item', { kind: 'album', id: o.albumId }) }] : []),
      // Where it opens elsewhere, one row: "Open in…" lists every site in a second menu; one site is a direct row
      ...(() => {
        const name = (v) => SERVICE_NAMES[v.source] || (v.label || '').split(' · ')[0] || (() => { try { return new URL(v.url).hostname.replace(/^www\./, ''); } catch { return 'the web'; } })();
        const outs = (o.openOn || []).filter((v) => webUrl(v.url));
        if (!outs.length) return [];
        if (outs.length === 1) return [{ label: `Open in ${name(outs[0])}`, icon: 'external', run: () => openWeb(outs[0].url) }];
        // (the second menu opens once this one's history entry is popped, so it gets its own Back step)
        return [{ label: 'Open in…', icon: 'external', run: () => { setTimeout(async () => { if (popPending) await popPending; showMenu(anchorEl.isConnected ? anchorEl : ($('#imore') || anchorEl), [{ title: 'Open in' }, ...outs.map((v) => ({ label: name(v), svc: srcSvc(v.source), run: () => openWeb(v.url) }))]); }, 0); } }];
      })(),
      ...(o.alsoHere && o.alsoHere.length ? [{ label: 'Also on…', icon: 'compass', run: () => { setTimeout(async () => { if (popPending) await popPending; showMenu(anchorEl.isConnected ? anchorEl : ($('#imore') || anchorEl), [{ title: 'Also on' }, ...o.alsoHere.map((x) => ({ label: x.label, svc: x.svc, run: () => o.switchTo(x.k) }))]); }, 0); } }] : []),
      ...(o.extra || []),
      // The owner's library: edit tags on the node, take a song out (Your library › Node library)
      ...(isOwner() && single && single.track_id != null ? [{ label: 'Edit tags…', icon: 'notes', run: () => editTrackTags(single.track_id) }, { label: 'Remove from library', icon: 'trash', run: () => removeFromLibrary(single.track_id, single.title, anchorEl) }] : []),
      ...(isOwner() && /^\/album\/lib-\d+$/.test(o.path || '') ? [{ label: 'Rename album…', icon: 'disc', run: () => editAlbumTags(Number(o.path.split('-').pop()), o.title) }] : []),
      // A library song's or album's match corrected: the Confirm matches view for its songs
      ...(isOwner() && trackIds.length && /^\/(song|album)\//.test(o.path || '') ? [{ label: 'Wrong match?…', icon: 'search', run: () => setView('identify', { fix: trackIds.slice(0, 500) }) }] : []),
      ...(o.path ? [{ label: 'Copy link', icon: 'copy', run: () => copyLink(o.path) }] : []),
      // Library files only: services' songs are not the user's files
      ...(state.pubkyUser && trackIds.length ? [{ label: trackIds.length === 1 ? 'Copy to my homeserver' : `Copy ${countOf(trackIds.length, 'songs')} to my homeserver`, icon: 'upload', run: () => startUpload({ track_ids: trackIds }) }] : []),
    ]);
  };
  // One song row: title → song page, artist → artist page, album → album page, ⋯ = the item menu.
  // A collection item that doesn't play (a web page, a post): its Pinakes card in the list, with
  // Open instead of Play; filled in by fillCards (GET /v1/cards).
  // An import's line that matched nothing is a MusicBrainz search for its words: shown with its
  // words, and never asked of Pinakes (they are a private playlist's)
  const MB_SEARCH = 'https://musicbrainz.org/search?type=recording&query=';
  const mbSearchWords = (uri) => { if (!String(uri || '').startsWith(MB_SEARCH)) return null; try { return new URL(uri).searchParams.get('query') || null; } catch { return null; } };
  const searchRow = (t, k, opts) => `<div class="list-row srow card-row${opts.rowClass ? ' ' + opts.rowClass(t, k) : ''}" data-srow="${k}" data-search><span class="num muted small">${opts.numbers === 'index' ? k + 1 : (t.no ?? k + 1)}</span><span class="thumb card-thumb" aria-hidden="true">${icon('search')}</span><div class="grow"><div class="name">${esc(t.title)}</div><div class="sub card-sub">Not matched to a song yet</div></div><a class="btn ghost sm" href="${esc(t.page)}" target="_blank" rel="noopener" aria-label="Find “${esc(t.title)}” on MusicBrainz">${icon('external')} Find on MusicBrainz</a>${opts.extra ? opts.extra(t, k) : ''}<button aria-haspopup="menu" aria-expanded="false" class="icon-btn" data-cmore="${k}" title="More" aria-label="${esc(rowWords.more(t.title))}">${icon('more')}</button></div>`;
  const cardRow = (t, k, opts) => t.search ? searchRow(t, k, opts) : `<div class="list-row srow card-row${opts.rowClass ? ' ' + opts.rowClass(t, k) : ''}" data-srow="${k}" data-card="${esc(t.page)}"><span class="num muted small">${opts.numbers === 'index' ? k + 1 : (t.no ?? k + 1)}</span><span class="thumb card-thumb" aria-hidden="true">${icon('compass')}</span><div class="grow"><div class="name">${esc(t.title)}</div><div class="sub card-sub">Reading the page…</div></div>${webUrl(t.page) ? `<a class="btn ghost sm" href="${esc(webUrl(t.page))}" target="_blank" rel="noopener" aria-label="Open ${esc(t.title)} (new tab)">${icon('external')} Open</a>` : ''}${opts.extra ? opts.extra(t, k) : ''}<button aria-haspopup="menu" aria-expanded="false" class="icon-btn" data-cmore="${k}" title="More" aria-label="${esc(rowWords.more(t.title))}">${icon('more')}</button></div>`;
  // At most two at a time: the node's other calls (play, the zone) must not wait behind them
  const fillCards = (box) => {
    const rows = $$('[data-card]', box); let next = 0;
    const one = async (row) => {
      const sub = $('.card-sub', row);
      for (let tries = 0; tries < 3; tries++) {
        let c; try { c = await api('GET', `/cards?uri=${encodeURIComponent(row.dataset.card)}`); } catch { break; }
        if (!row.isConnected) return;
        if (c.status === 'found' || (c.status === 'reading' && c.name)) {
          // (its buttons are named after it too: "More for Liner notes", not the address it had)
          if (c.name) { $('.name', row).textContent = c.name; const m = $('[data-cmore]', row); if (m) m.setAttribute('aria-label', rowWords.more(c.name)); const o = $('a[aria-label$="(new tab)"]', row); if (o) o.setAttribute('aria-label', `Open ${c.name} (new tab)`); $$('[data-wd]', row).forEach((b) => { const w = rowWords[b.dataset.wd]; if (w) b.setAttribute('aria-label', w(c.name)); }); }
          sub.textContent = [c.site, c.description].filter(Boolean).join(' · ');
          const img = webUrl(c.image); if (img) $('.card-thumb', row).outerHTML = `<img class="thumb" src="${esc(img)}" alt="" loading="lazy" data-err="hide">`;
        }
        if (c.status !== 'reading') { if (c.status === 'unknown') sub.textContent = 'A link'; return; }
        await new Promise((r) => setTimeout(r, 3000));
      }
      // Never read (or the node didn't answer): say what it is, not "Reading" for good
      if (row.isConnected && sub.textContent === 'Reading the page…') sub.textContent = 'A link';
    };
    const worker = async () => { while (next < rows.length) { const row = rows[next++]; if (row.isConnected) await one(row); } };
    worker(); worker();
  };
  // A song kept in more than one copy (#268: a homeserver copy, FLAC and MP3, hi-res and CD) is one row
  // that plays the best copy; a quiet "2 versions" in its second line opens the copies to choose from
  // (owner: "maybe have some indication or version chooser, very subtle").
  const versionMark = (t, k) => (Array.isArray(t.versions) && t.versions.length > 1 ? `<button class="lnk vmark" data-sver="${k}" aria-label="${t.versions.length} versions of ${esc(t.title)}: choose which to play" title="Choose which copy plays">${t.versions.length} versions</button>` : '');
  // The row's second line; with a marker, the words shrink to an ellipsis and the marker keeps its width
  // (none when it has nothing to say)
  const subLine = (html, mark) => (!html && !mark ? '' : mark ? `<div class="sub subv"><span class="subtext">${html}</span>${html ? '<span aria-hidden="true">·</span>' : ''}${mark}</div>` : `<div class="sub">${html}</div>`);
  const VERSION_FROM = { library: 'Your library', homeserver: 'Your homeserver' };
  const versionLabel = (v) => [VERSION_FROM[v.source] || v.source, v.format || (v.container || '').toUpperCase(), v.size_bytes ? fileSize(v.size_bytes) : ''].filter(Boolean).join(' · ');
  // (`data-np`: what the row plays, so the playing one is marked; `opts.oneArtist`: the album's songs
  // are all by its artist, said once in the header and not on every row)
  // (`opts.pageArtist`: an artist page's own { name, id }; a row credited to that artist alone says
  // only its album, as the page's title already says who; a row with other artists too keeps all its
  // credits. The page's artist is its name and, when both the credit and the page know one, its id:
  // another artist of the same name keeps the credit. A library row with only its flat credit
  // ('wait') says nothing of it until its structured read (showPageCredits): that read may name a
  // co-artist, or another artist of that name)
  const soleCredit = (c, page) => c.length === 1 && c[0].name === page.name && !(c[0].id && page.id && c[0].id !== page.id);
  const pageCredit = (t, page) => (!page || !page.name || !soleCredit(artistCredits(t), page) ? 'show' : creditsFlat(t) != null ? 'wait' : 'hide');
  // A song row's second line: its artists and album, and its versions marker
  const songSub = (t, k, opts) => {
    const credit = (t.artist || t.artists) && !opts.oneArtist ? pageCredit(t, opts.pageArtist) : 'hide';
    const album = t.album && opts.showAlbum !== false ? (t.album_id ? `<a class="lnk" data-salbum="${k}" href="${esc(pathFor('item', { kind: 'album', id: t.album_id }))}"${rowLinkTab()}>${esc(t.album)}</a>` : esc(t.album)) : '';
    const wait = credit === 'wait' ? `<span data-credit-track="${esc(creditsFlat(t))}" data-page-artist="${esc(opts.pageArtist.name)}" data-page-artist-id="${esc(opts.pageArtist.id || '')}"></span>` : '';
    return wait + subLine([credit === 'show' ? artistLinksHtml(t, rowLinkTab()) : '', album].filter(Boolean).join(' · '), versionMark(t, k));
  };
  const songNpKey = (t) => (t.track_id != null ? `track:${t.track_id}` : t.uri || '');
  // On phones a song row is one button (a tap plays it; its names don't take taps, app.css): its song,
  // artist and album links are not separate Tab stops there either (its ⋯ has their pages), in the
  // library's lists (.track) as in the others (.srow)
  const rowLinkTab = () => (shellMode() === 'phone' ? ' tabindex="-1"' : '');
  // Whether a song row's song is one of your favourites: by its library track, by its page (a
  // canonical URI) or by the URI its page id stands for — the three keys /favourites answers with
  // (the same three `itemMenu` uses for a single song, so the row and its ⋯ menu always agree).
  const songFav = (t) => !!(state.favUris && ((t.track_id != null && state.favUris.has(`track:${t.track_id}`)) || (t.page && state.favUris.has(t.page)) || (t.id && state.favUris.has(uriOf('song', t.id) || ''))));
  // A song row's buttons say which song they act on, as its heart does ("Play next" alone, read 40
  // times down a list, names none); the short tooltip stays
  const rowWords = { play: (title) => `Play ${title || 'this song'}`, next: (title) => `Play ${title || 'this song'} next`, add: (title) => `Add ${title || 'this song'} to the queue`, more: (title) => `More for ${title || 'this song'}`,
    // a queue or playlist row's own edits ("Move X up"); data-wd on a playlist row's button says which, so a link row's are renamed with its title (fillCards)
    up: (title) => `Move ${title || 'this song'} up`, down: (title) => `Move ${title || 'this song'} down`, unqueue: (title) => `Remove ${title || 'this song'} from the queue`, unlist: (title) => `Remove ${title || 'this song'} from this collection`, drag: (title) => `Drag ${title || 'this song'} to reorder (or Alt+↑/↓)` };
  const heartWords = (on, title) => `${on ? 'Remove' : 'Add'} ${title || 'this song'} ${on ? 'from' : 'to'} Bookmarks`;
  // A song row's heart, the heart a row on Songs has: filled when the song is bookmarked (shown
  // then without hover, on a touch screen too), an outline you reach by hovering the row with a
  // mouse. One click to bookmark where the ⋯ menu used to take two. It says which song it is for.
  const songHeart = (t, k) => `<button class="icon-btn heart${songFav(t) ? ' on' : ''}" data-sfav="${k}" aria-pressed="${songFav(t)}" title="${songFav(t) ? 'Remove from Bookmarks' : 'Add to Bookmarks'}" aria-label="${esc(heartWords(songFav(t), t.title))}">${icon(songFav(t) ? 'heart-fill' : 'heart')}</button>`;
  // The heart turned over without redrawing anything around it
  const setHeart = (b, on, title) => { b.classList.toggle('on', on); b.setAttribute('aria-pressed', String(on)); b.innerHTML = icon(on ? 'heart-fill' : 'heart'); b.title = on ? 'Remove from Bookmarks' : 'Add to Bookmarks'; b.setAttribute('aria-label', heartWords(on, title)); };
  // Under a song row's cover: the song's initials on its gradient, as the mini player and queue draw a
  // song without art, so a cover that doesn't load (a library without artwork: the album cover 404s)
  // leaves the monogram, not an empty slot with a lone badge in it. `badged`: the source badge sits in
  // the slot's lower left, so the initials move into the corner it leaves free.
  // A hero's picture (item and person pages) lies over its monogram, so a picture that fails to load (removed) leaves the monogram, as on the tiles
  const heroImg = (src, round) => `<img src="${esc(imgSrc(src))}" alt="" style="width:100%;border-radius:${round ? '50%' : '12px'}" data-err="remove">`;
  const rowMono = (title, badged) => { const h = hue(title); return `<span class="thumb mono${badged ? ' badged' : ''}" aria-hidden="true" style="background:linear-gradient(135deg, hsl(${h} 38% 30%), hsl(${(h + 40) % 360} 42% 18%))">${esc(initials(title))}</span>`; };
  const songRows = (songs, opts = {}) => `<div class="list${opts.listClass ? ' ' + opts.listClass : ''}">${songs.map((t, k) => t.card ? cardRow(t, k, opts) : `<div class="list-row clickable srow${opts.rowClass ? ' ' + opts.rowClass(t, k) : ''}${songNpKey(t) && songNpKey(t) === npUri() ? ' cur" aria-current="true' : ''}" data-srow="${k}"${songNpKey(t) ? ` data-np="${esc(songNpKey(t))}"` : ''}><span class="num muted small">${opts.numbers === 'index' ? k + 1 : (t.no ?? k + 1)}</span><button class="icon-btn" data-splay="${k}" title="Play from here" aria-label="${esc(rowWords.play(t.title))}">${icon('play')}</button>${t.image && opts.covers !== false ? `<span class="thumbwrap">${rowMono(t.title, !t.sources && coverBadge(coverSvc(t)))}<img class="thumb" src="${esc(imgSrc(t.image))}" alt="" loading="lazy" data-err="remove">${t.sources ? '' : coverBadge(coverSvc(t))}</span>` : ''}<div class="grow"><div class="name">${t.id ? `<a class="lnk" data-ssong="${k}" href="${esc(pathFor('item', { kind: 'song', id: t.id }))}"${rowLinkTab()}>${esc(t.title)}</a>` : `<span class="ntext">${esc(t.title)}</span>`}</div>${songSub(t, k, opts)}</div><span class="muted small">${t.duration_ms ? fmt(t.duration_ms) : ''}</span>${opts.extra ? opts.extra(t, k) : ''}${songHeart(t, k)}<button aria-haspopup="menu" aria-expanded="false" class="icon-btn" data-smore="${k}" title="More" aria-label="${esc(rowWords.more(t.title))}">${icon('more')}</button></div>`).join('')}</div>`;
  // The list being selected from (while the bar is up): its page, its part of the page, each song's
  // key and the ticks. The same list drawn again where it stands (a playlist's sources answering, a
  // pin, the next search source, See all songs) takes Select over with the same songs ticked;
  // another page ends it
  let selNow = null;
  const selOn = () => (selBar && selNow) || null;
  const selPage = () => pathFor(state.view, state.viewArg) || state.view;
  const selWhere = (section) => section.id || (section.dataset.skind ? `skind:${section.dataset.skind}` : '');
  // (after a redraw: a selection whose list went and that no new list took over ends)
  const dropStaleSelect = () => { if (selOn() && !selNow.section.isConnected) endSelect(); };
  // The bar's Remove running (one DELETE per song): its list, and that list drawn again meanwhile,
  // take no presses (a row's own Remove or Move, a second Remove) until it is done, so no index
  // goes stale under it
  let selRemoving = null;
  // A playlist change that goes by a song's place (a row's Remove, Move up or down, a pin, the
  // bar's Remove) holds that playlist's songs from the press until a draw that read the list after
  // the change has drawn them: a second press on the list still on screen would go by an old place
  // and change another song. One that doesn't answer in 30 s is cut, and its hold stays until the next library
  // change (the node may still make it) or 30 s more; a hold still on 30 s after its change
  // answered is read again. A read under a hold that doesn't answer in 15 s shows "Couldn't load
  // this playlist" with Try again, so the list is never left inert. (one hold per playlist;
  // plDraws: each playlist draw's number)
  const plHolds = new Map(); let plDraws = 0; const PL_HOLD_MS = 30000; const PL_READ_MS = 15000;
  let plShown = null; // (the playlist whose rows were drawn last, its songs' uris in their order)
  const plHeld = (id) => plHolds.has(id);
  const plHoldShow = () => { $$('#plsongs, #pllocal').forEach((x) => { x.inert = true; }); if (selBar) $$('button', selBar).forEach((b) => { b.disabled = true; }); };
  // (the control that had focus in the list: the list going inert takes the focus away, the new
  // list gives it back. In a song's row, to that song's row wherever the new list has it (its uri,
  // or a local song's id, and which copy of it: a hold means the node may have another song at
  // that place now, and a second Enter there would change that song), the same kind of control;
  // nowhere if the song is gone. Elsewhere in the list, by its data attributes and its place
  // among its likes)
  const plKeysOf = (box) => (plShown && (box === 'pllocal' ? plShown.local : plShown.uris)) || [];
  const plSongFocus = (box, keys, k, kind, alt) => (k >= 0 && k < keys.length ? { box, song: keys[k], copy: keys.slice(0, k).filter((u) => u === keys[k]).length, kind, alt } : null);
  // (a song's × from the keyboard: the focus goes on to the song after it, else the one before.
  // `hid`: the places a source chip hides; it skips those (the next song that shows, else the shown
  // one before). When none shows any more the chip goes off, and it's the next song again)
  const plNextFocus = (box, keys, k, kind, alt, hid) => {
    const rest = keys.map((_, i) => i).filter((i) => i !== k); const shows = (i) => !hid || !hid.has(i);
    let j = rest.findIndex((i) => i > k && shows(i)); if (j < 0) j = rest.findLastIndex((i) => i < k && shows(i)); if (j < 0) j = k < rest.length ? k : k - 1;
    return plSongFocus(box, rest.map((i) => keys[i]), j, kind, alt);
  };
  const plFocusOf = (a) => {
    const box = a && a.closest ? a.closest('#plsongs, #pllocal') : null; if (!box) return null;
    const row = a.closest('[data-srow]');
    if (row && box.contains(row)) { const f = plSongFocus(box.id, plKeysOf(box.id), Number(row.dataset.srow), a.tagName.toLowerCase() + [...a.attributes].filter((x) => x.name.startsWith('data-')).map((x) => (x.value === row.dataset.srow ? `[${x.name}]` : `[${x.name}="${CSS.escape(x.value)}"]`)).join(''), a.dataset.pup != null ? 'button[data-pdown]' : a.dataset.pdown != null ? 'button[data-pup]' : undefined); if (f) { if (a === row) f.row = true; return f; } } // (Move up at the top: Move down; f.row: the row itself)
    const sel = a.tagName.toLowerCase() + [...a.attributes].filter((x) => x.name.startsWith('data-')).map((x) => `[${x.name}="${CSS.escape(x.value)}"]`).join('');
    return { box: box.id, sel, i: $$(sel, box).indexOf(a) };
  };
  // (a focus taken by a draw the person didn't cause, a late answer for where the songs play: one
  // passed on to another song lands on its ⋯ or the row, never its × or Move, so a key already on
  // its way changes no other song; plFocusIn)
  const plCalm = (f) => (f ? { ...f, calm: true } : f);
  // (a hold still on: the next draw of its playlist ends it)
  const plHoldEnd = (hold) => { if (plHolds.get(hold.id) !== hold) return; hold.cut = false; hold.after = Math.min(hold.after, plDraws + 1); if (state.view === 'playlist' && state.viewArg === hold.id) render(); };
  // Runs `run(signal)` (the change) under the hold, then draws the playlist again; false when it
  // failed or another change of this playlist still holds it. `focus`: where the focus goes in the
  // list that ends the hold once the change is made (Move: the moved song's button), else (and when
  // it failed) back where it was
  const plWrite = async (id, run, focus) => {
    if (plHeld(id)) return false;
    const hold = { id, after: Infinity, focus: plFocusOf(document.activeElement) }; plHolds.set(id, hold); plHoldShow();
    const cut = new AbortController(); const limit = setTimeout(() => { cut.signal.why = new Error('The node didn’t answer in time; the collection updates once it does'); cut.abort(); }, PL_HOLD_MS);
    let ok = false;
    try { await run(cut.signal); ok = true; if (focus) hold.focus = focus; } catch (e) { toast(e.message, 'error'); } finally { clearTimeout(limit); }
    if (plHolds.get(id) === hold) { if (cut.signal.aborted) hold.cut = true; else hold.after = plDraws + 1; setTimeout(() => plHoldEnd(hold), PL_HOLD_MS); }
    render();
    return ok;
  };

  // Multi-select (UX review, next tier): "Select" on a song list turns on checkboxes and a bar
  // with what to do to all of them; Escape or Cancel ends it. `opts.remove(indices)` on playlists.
  let selBar = null;
  // While selecting, the page ends the bar's height lower (--selbar), so the last songs scroll out
  // from under it and can be ticked too, and a song reached by Tab stops above the bar (app.css).
  const endSelect = () => { if (selBar) { selBar.ro.disconnect(); selBar.remove(); selBar = null; document.documentElement.style.removeProperty('--selbar'); } $$('.selecting').forEach((b) => b.classList.remove('selecting')); $$('.selbox').forEach((c) => c.remove()); $$('[data-selstart]').forEach((b) => { b.setAttribute('aria-pressed', 'false'); }); };
  const enableSelect = (section, songs, opts = {}) => {
    if (!section || !songs.length) return;
    const head = $('.section-head', section); if (!head || $('[data-selstart]', head)) return;
    head.insertAdjacentHTML('beforeend', `<button class="btn ghost sm selstart" data-selstart aria-pressed="false">${icon('check')}<span>Select</span></button>`);
    const sel = new Set();
    // (the songs on screen: not the rows the playlist's source filter hides)
    const shown = () => { const hid = new Set($$('.list-row.filtered[data-srow]', section).map((r) => Number(r.dataset.srow))); return songs.map((_, k) => k).filter((k) => !hid.has(k)); };
    const draw = () => {
      if (!selBar) { selBar = document.createElement('div'); selBar.className = 'selbar'; selBar.setAttribute('role', 'toolbar'); selBar.setAttribute('aria-label', 'Selected songs'); document.body.appendChild(selBar); const bar = selBar; bar.ro = new ResizeObserver(() => { if (bar.isConnected) document.documentElement.style.setProperty('--selbar', `${bar.offsetHeight}px`); }); bar.ro.observe(bar); }
      const picked = [...sel].sort((a, b) => a - b).map((k) => songs[k]);
      selBar.innerHTML = `<span class="selcount">${countOf(sel.size, 'songs')} selected</span><button class="btn sm primary" data-sel="play" ${sel.size ? '' : 'disabled'}>${icon('play')} Play</button><button class="btn sm" data-sel="queue" ${sel.size ? '' : 'disabled'}>${icon('plus')} Add to queue</button><button class="btn sm" data-sel="playlist" ${sel.size ? '' : 'disabled'}>${icon('playlist')} Add to collection…</button><button class="btn sm" data-sel="tag" ${sel.size ? '' : 'disabled'}>Tag…</button>${opts.remove ? `<button class="btn sm danger" data-sel="remove" ${sel.size ? '' : 'disabled'}>${icon('trash')} Remove</button>` : ''}<button class="btn sm ghost" data-sel="all">${sel.size === shown().length ? 'None' : 'All'}</button><button class="icon-btn" data-sel="cancel" aria-label="Stop selecting">${icon('close')}</button>`;
      $$('[data-sel]', selBar).forEach((b) => b.onclick = async () => {
        const what = b.dataset.sel;
        if (what === 'cancel') return endSelect();
        if (what === 'all') { if (sel.size === shown().length) sel.clear(); else shown().forEach((k) => sel.add(k)); $$('.selbox input', section).forEach((c) => { c.checked = sel.has(Number(c.dataset.k)); }); return draw(); }
        // Picked cards (web pages, posts) don't play: the songs among the picked ones do
        const songsPicked = picked.filter((t) => !t.card);
        if ((what === 'play' || what === 'queue') && !songsPicked.length) { toast('Nothing to play: links and posts open, they don’t play'); return; }
        if (what === 'play') { await playSongs(songsPicked, 0, 'replace', opts.plays_from, b, opts.fallback); return endSelect(); }
        if (what === 'queue') { await playSongs(songsPicked, 0, 'end', opts.plays_from, b, opts.fallback); return endSelect(); }
        if (what === 'playlist') {
          if (!state.playlists.length) { try { state.playlists = await api('GET', '/playlists'); } catch { /* keep */ } }
          const ids = picked.filter((t) => t.track_id != null).map((t) => t.track_id); const uris = picked.filter((t) => t.track_id == null).map((t) => t.page || (t.id && uriOf('song', t.id))).filter(Boolean);
          const add = async (pl) => { try { if (ids.length) await api('POST', `/playlists/${encodeURIComponent(pl.id)}/items`, { track_ids: ids }); if (uris.length) await api('POST', `/playlists/${encodeURIComponent(pl.id)}/items`, { uris }); toast(`${countOf(picked.length, 'songs')} added to “${pl.name}”`); endSelect(); } catch (e) { toast(e.message, 'error'); } };
          return playlistPicker(add, 'New collection', async (name) => { try { await api('POST', '/playlists', { name, track_ids: ids, uris }); state.playlists = await api('GET', '/playlists'); toast(`Created “${name}”`); endSelect(); } catch (e) { toast(e.message, 'error'); } });
        }
        if (what === 'tag') {
          const r = await choiceDialog({ title: `Tag ${countOf(picked.length, 'songs')}`, text: 'One Pubky tag on each of them (public). Songs not matched yet keep it until they are.', field: { label: 'Tag', value: '' }, choices: [{ label: 'Tag them', value: true, primary: true }] });
          if (!r || !r.text) return;
          let n = 0; for (const t of picked) { const ref = t.track_id != null ? { track_id: t.track_id } : (t.page || t.id) ? { uri: t.page || uriOf('song', t.id) } : null; if (!ref) continue; try { await api('PUT', '/tags', { ...ref, label: r.text }); n++; } catch { /* skip that one */ } }
          toast(`Tagged ${countOf(n, 'songs')} #${r.text.toLowerCase()}`); return endSelect();
        }
        // (while the songs go, one by one, the bar waits and a redraw meanwhile ends Select rather
        // than handing its ticks to the half-changed list)
        // (a failure part way ends Select: the ticks no longer point at the same songs; the list is
        // drawn again by opts.remove)
        if (what === 'remove') { const job = { page: selNow.page, where: selNow.where, sel, held: [section] }; selRemoving = job; section.inert = true; draw(); try { await opts.remove([...sel].sort((a, b) => b - a)); } catch (e) { toast(e.message, 'error'); } finally { if (selRemoving === job) selRemoving = null; if (!(opts.held && opts.held())) job.held.forEach((x) => { x.inert = false; }); if (selOn() && selNow.sel === sel) endSelect(); } }
      });
      if ((selRemoving && selRemoving.sel === sel) || (opts.held && opts.held())) $$('button', selBar).forEach((b) => { b.disabled = true; });
    };
    // (a song's key: opts.keys, else its library id, page or address, so the same list drawn again
    // with its sources found still has the same keys)
    const keys = songs.map((t, k) => (opts.keys && opts.keys[k]) ?? t.track_id ?? t.page ?? t.uri ?? t.id ?? t.title);
    // Select starts with nothing ticked (or with the ticks a redraw carries over)
    const start = (btn, ticks) => {
      endSelect(); sel.clear(); section.classList.add('selecting'); btn.setAttribute('aria-pressed', 'true');
      selNow = { section, page: selPage(), where: selWhere(section), keys, sel };
      // (a carried tick on a row the playlist's filter hides is dropped: Remove would take it unseen)
      $$('.srow[data-srow]', section).forEach((row) => { const k = Number(row.dataset.srow); row.insertAdjacentHTML('afterbegin', `<label class="selbox" title="Select"><input type="checkbox" data-k="${k}" aria-label="Select ${esc(songs[k] ? songs[k].title : '')}"></label>`); const c = $('.selbox input', row); if (ticks && ticks.has(k) && !row.classList.contains('filtered')) { c.checked = true; sel.add(k); } c.onclick = (ev) => ev.stopPropagation(); c.onchange = () => { if (c.checked) sel.add(k); else sel.delete(k); draw(); }; });
      draw();
    };
    $('[data-selstart]', head).onclick = (e) => { if (section.classList.contains('selecting')) return endSelect(); start(e.currentTarget, null); };
    // This list in place of the one being selected from: Select goes on here with the ticked songs
    // that are still in it, found by their key and by which copy of it they were (the 2nd copy of
    // a song ticked is the 2nd copy now). A song whose copies came or went has its tick dropped,
    // as which copy was ticked isn't sure; the bar counts what's left
    const was = selOn();
    const busy = selRemoving && selRemoving.page === selPage() && selRemoving.where === selWhere(section);
    if (busy) { section.inert = true; selRemoving.held.push(section); } else if (was && !was.section.isConnected && was.page === selPage() && was.where === selWhere(section)) {
      const copies = (list) => { const m = new Map(); list.forEach((x, j) => { if (!m.has(x)) m.set(x, []); m.get(x).push(j); }); return m; };
      const then = copies(was.keys); const now = copies(keys); const ticks = new Set();
      was.sel.forEach((k) => { const x = was.keys[k]; const a = then.get(x); const b = now.get(x); if (b && b.length === a.length) ticks.add(b[a.indexOf(k)]); });
      start($('[data-selstart]', head), ticks);
    }
  };
  document.addEventListener('keydown', (e) => { if (e.key === 'Escape' && selBar) endSelect(); });
  // A compact header once the page's big one scrolls away: the title and Play stay at hand
  const stickyHead = (title, onPlay) => {
    const hero = $('.hero', main); if (!hero || !window.IntersectionObserver) return;
    main.insertAdjacentHTML('afterbegin', `<div class="stickyhead" hidden><strong>${esc(title)}</strong><button class="btn sm primary" data-sticky-play>${icon('play')} Play</button></div>`);
    const bar = $('.stickyhead', main);
    $('[data-sticky-play]', bar).onclick = (e) => onPlay(e.currentTarget);
    const io = new IntersectionObserver(([en]) => { if (!document.body.contains(bar)) { io.disconnect(); return; } bar.hidden = en.isIntersecting; }, { root: main, threshold: 0 });
    io.observe(hero);
  };
  // (the click before this one: clickWas, by rowSpot, above bindTrackRows)
  const bindSongRows = (box, all, ctx = {}) => {
    $$('[data-srow]', box).forEach((r) => { const t = all[Number(r.dataset.srow)]; if (t && t.track_id != null) r.dataset.tid = t.track_id; else if (t && (t.uri || t.page)) r.dataset.suri = t.uri || t.page; });
    markDownloads(box);
    // Cards (web pages, posts) and unmatched lines don't play: play the songs only, from the one
    // clicked; a click on one that doesn't play does nothing
    const songs = all.filter((t) => !t.card); const at = (k) => Math.max(0, songs.indexOf(all[k]));
    $$('[data-splay]', box).forEach((b) => b.onclick = (e) => { e.stopPropagation(); playSongs(songs, at(Number(b.dataset.splay)), 'replace', ctx.plays_from, b, ctx.fallback); });
    // While selecting, a tap on a row ticks it (on phones a long press starts selecting, and the
    // next taps add songs): it never plays and replaces the queue then. Its box ticks itself.
    // The second click of a double click doesn't start the song again and selects no word of the row,
    // and a drag that selects the row's text doesn't start it, as on a library song row. Only a selection
    // with words in it that starts or ends in the row holds it (a mouse drag's click has both ends there):
    // on a touch screen the rows can't be selected, so a selection made elsewhere stays, even one across
    // the rows or an empty one inside a row, and a tap on the row plays.
    // A quick tap on the next row can come as a double tap's second (a phone counts taps up to 100 dp
    // apart, more than a row), so only a second click on the row the first one hit is held. The
    // mousedown guard stays on every row: a word selected on the next row would hold its click too
    $$('[data-srow]:not([data-card]):not([data-search])', box).forEach((r) => {
      r.onmousedown = (e) => { if (e.detail > 1 && !e.target.closest('a, button, input, select, textarea, label')) e.preventDefault(); };
      r.onclick = (e) => {
        const again = e.detail > 1 && clickWas === srowSpot(r);
        if (e.target.closest('a,button')) return;
        if (r.closest('.selecting')) { const c = $('.selbox input', r); if (c && !e.target.closest('.selbox')) { c.checked = !c.checked; c.dispatchEvent(new Event('change', { bubbles: true })); } return; }
        const s = getSelection(); if (again || (String(s) !== '' && (r.contains(s.anchorNode) || r.contains(s.focusNode)))) return;
        state.startAnchor = null; // (a tap on the row, not a press of a Play: no spinner on one pressed before)
        playSongs(songs, at(Number(r.dataset.srow)), 'replace', ctx.plays_from, r, ctx.fallback);
      };
    });
    fillCards(box);
    // A card's ⋯: open it, copy its link, and the row's own moves (on phones the only way to them)
    $$('[data-cmore]', box).forEach((b) => b.onclick = (e) => {
      e.stopPropagation(); const k = Number(b.dataset.cmore); const t = all[k]; const web = webUrl(t.page);
      showMenu(b, [{ title: t.title }, ...(web ? [{ label: 'Open', icon: 'external', run: () => window.open(web, '_blank', 'noopener') }] : []),
        { label: 'Copy link', icon: 'copy', run: async () => { try { await navigator.clipboard.writeText(t.page); toast('Link copied'); } catch { toast(t.page); } } },
        ...(ctx.rowExtra ? ctx.rowExtra(k, b.closest('.srow')) : [])]);
    });
    // A song's copies: the default first; picking one plays that copy from this row on (the default stays)
    $$('[data-sver]', box).forEach((b) => b.onclick = (e) => {
      e.stopPropagation(); const k = Number(b.dataset.sver); const t = all[k];
      const vs = [...t.versions].sort((x, y) => Number(y.best) - Number(x.best));
      // Every listener may play any copy: { all } keeps the homeserver's copies in the list for them too
      showMenu(b, [{ title: `${t.title}: versions` }, ...vs.map((v) => ({ label: versionLabel(v) + (v.best ? ' · plays by default' : ''), icon: v.best ? 'check' : 'play',
        run: () => playSongs(songs.map((x) => (x === t ? { ...x, track_id: v.track_id } : x)), at(k), 'replace', ctx.plays_from, b, ctx.fallback) }))], { all: true });
    });
    // The row's heart: saves and turns over in place, so a long list never redraws under you.
    // Its press goes through busy() like every other (the spinner, one save per press, and the
    // keyboard keeps the button — disabling it mid-press would drop focus to the top of the page).
    $$('[data-sfav]', box).forEach((b) => { heartSong.set(b, all[Number(b.dataset.sfav)]); b.onclick = async (e) => {
      e.stopPropagation(); e.preventDefault();
      const t = all[Number(b.dataset.sfav)]; if (!t) return;
      const ref = t.track_id != null ? { track_id: t.track_id } : { uri: t.page || (t.id ? uriOf('song', t.id) : null) };
      if (ref.uri == null && ref.track_id == null) { toast('This song has no address to save yet'); return; }
      const on = !b.classList.contains('on');
      if (await setFavourite(ref, on)) setHeart(b, on, t.title);
    }; });
    $$('[data-ssong]', box).forEach((a) => a.onclick = (e) => { e.stopPropagation(); if (opensElsewhere(e)) return; e.preventDefault(); setView('item', { kind: 'song', id: all[Number(a.dataset.ssong)].id }); });
    hydrateArtistCredits(box);
    $$('[data-salbum]', box).forEach((a) => a.onclick = (e) => { e.stopPropagation(); if (opensElsewhere(e)) return; e.preventDefault(); setView('item', { kind: 'album', id: all[Number(a.dataset.salbum)].album_id }); });
    $$('[data-smore]', box).forEach((b) => b.onclick = (e) => { e.stopPropagation(); const t = all[Number(b.dataset.smore)]; itemMenu(b, { title: t.title, songs: [t], plays_from: ctx.plays_from, fallback: ctx.fallback, artistId: t.artist_id, artistName: t.artist, credits: artistCredits(t), albumId: t.album_id, openOn: t.page ? [{ source: serviceOfUrl(t.page) || (t.uri || '').split(':')[0], url: t.page, label: '' }] : [], path: t.id ? `/song/${t.id}` : null, extra: ctx.rowExtra ? ctx.rowExtra(Number(b.dataset.smore), b.closest('.srow')) : [] }); });
  };
  // One tile for albums and artists: click opens the item page, the hover Play plays it.
  // Tiles while a grid loads: the same grid, square covers and the two text lines a tile has
  const tileSkeleton = (n) => (n ? `<div class="grid" aria-hidden="true">${'<div class="tile sk-tile"><div class="art skeleton"></div><div class="t"><span class="skeleton sk-text">&nbsp;</span></div><div class="a"><span class="skeleton sk-text short">&nbsp;</span></div></div>'.repeat(n)}</div>` : '<p class="muted small"><span class="skeleton sk-text">&nbsp;</span></p>');
  // Song rows while a list loads: a song row's own parts (number, play, cover, two lines, length,
  // More), so each is as tall as the row that replaces it
  const rowSkeleton = (n) => (n ? `<div class="list" aria-hidden="true">${'<div class="list-row srow sk-srow"><span class="num muted small">&nbsp;</span><span class="icon-btn"></span><span class="thumbwrap"><span class="thumb skeleton"></span></span><div class="grow"><div class="name"><span class="skeleton sk-text">&nbsp;</span></div><div class="sub"><span class="skeleton sk-text short">&nbsp;</span></div></div><span class="muted small">&nbsp;</span><span class="icon-btn"></span></div>'.repeat(n)}</div>` : '');
  // How many tiles (or rows) a Home row had last time (`n` given: remember it)
  const homeCount = (k, n, first = 4) => {
    let all = {}; try { all = JSON.parse(localStorage.getItem('pm.homeCounts') || '{}') || {}; } catch { /* none kept */ }
    if (n == null) return Number.isInteger(all[k]) ? all[k] : first;
    if (all[k] !== n) { all[k] = n; try { localStorage.setItem('pm.homeCounts', JSON.stringify(all)); } catch { /* private window */ } }
    return n;
  };
  const itemTileGrid = (tiles, kind) => `<div class="grid">${tiles.map((a, k) => `<button class="tile" data-tile="${k}" title="${esc(a.title)}"><div class="art" style="background:linear-gradient(135deg, hsl(${hue(a.title)} 38% 30%), hsl(${(hue(a.title) + 40) % 360} 42% 18%))"><span>${esc(initials(a.title))}</span>${a.image ? `<img loading="lazy" alt="" src="${esc(imgSrc(a.image))}" data-err="remove">` : ''}${coverBadge(coverSvc(a), true)}<span class="tile-play" data-tileplay="${k}" role="button" title="Play">${icon('play')}</span></div><div class="t">${esc(a.title)}</div><div class="a">${esc(a.year || '')}</div>${coverFrom(coverSvc(a))}</button>`).join('')}</div>`;
  const bindItemTiles = (box, tiles, kind) => {
    $$('[data-tile]', box).forEach((b) => b.onclick = (e) => { if (e.target.closest('[data-tileplay]')) return; setView('item', { kind, id: tiles[Number(b.dataset.tile)].id }); });
    $$('[data-tileplay]', box).forEach((b) => b.onclick = async (e) => { e.stopPropagation(); try { const pg = await api('GET', `/item/${kind}/${tiles[Number(b.dataset.tileplay)].id}`); playSongs(pg.songs, 0, 'replace', pg.plays_from, b, { artist: pg.artist && pg.artist.name, album: pg.title, image: pg.image }); } catch (er) { toast(er.message, 'error'); } });
  };

  // An artist's releases (owner's report on /artist/tidal-4805908: one "Albums" grid of every
  // single and remix): Albums, then EPs & singles, then Other releases, by the type the service
  // gives each release (the node's `release_type`; never guessed from a title or a track count).
  // A compilation, or a release the service doesn't type, is Other. Each section is newest first
  // (the node's order), shows its first 12 and has View all: a paged grid with its own path.
  const REL_SECTIONS = [['albums', 'Albums', 'albums'], ['eps', 'EPs & singles', 'eps-singles'], ['other', 'Other releases', 'other']];
  const REL_PREVIEW = 12; const REL_PAGE = 48;
  const relKey = (a) => (a.compilation || !a.release_type ? 'other' : a.release_type === 'album' ? 'albums' : a.release_type === 'ep' || a.release_type === 'single' ? 'eps' : 'other');
  const relGroups = (albums) => { const g = { albums: [], eps: [], other: [] }; for (const a of albums || []) g[relKey(a)].push(a); return g; };
  // With nothing typed (a library artist: files carry no release type) the one section is "Releases"
  const relTitle = (key, g) => (key === 'other' && !g.albums.length && !g.eps.length ? 'Releases' : (REL_SECTIONS.find(([k]) => k === key) || [, 'Releases'])[1]);
  // A tile says what the service says besides its section: EP or Single, Compilation, and MusicBrainz's other secondary types (Live, Remix…)
  const relTile = (a, key) => ({ ...a, year: [a.year, a.compilation ? 'Compilation' : key === 'eps' ? (a.release_type === 'ep' ? 'EP' : 'Single') : '', ...(a.secondary_types || []).filter((t) => t !== 'Compilation')].filter(Boolean).join(' · ') });
  // A list that isn't the whole catalogue (a page failed, or the fetch stopped at its bound) counts "at least"
  const relCount = (n, cat) => `${n}${cat && cat.complete === false && n ? '+' : ''}`;
  const relSvc = (pg) => { const id = (pg.catalogue && pg.catalogue.source) || (pg.plays_from && pg.plays_from.service) || ''; return SERVICE_NAMES[id] || (id === 'musicbrainz' ? 'MusicBrainz' : id) || 'the service'; };
  // What the list is, said plainly: still coming, failed, partial, cut at the bound; else nothing
  const relStatus = (pg, stuck) => {
    const cat = pg.catalogue; const n = (pg.albums || []).length; const svc = esc(relSvc(pg));
    const retry = '<button class="btn sm" id="irelretry">Retry</button>';
    if (pg.pending && !n) return stuck ? `<p class="muted small relnote" id="irelstatus" role="status">${svc} is still listing this artist’s releases. ${retry}</p>` : `<p class="muted small relnote" id="irelstatus" role="status"><span class="spin"></span> Getting the releases from ${svc}…</p>`;
    if (!cat) return '';
    if (cat.error && !n) return `<div class="notice relnote" id="irelstatus" role="alert">${svc} didn’t list the releases: ${esc(cat.error)} ${retry}</div>`;
    if (cat.error) return `<p class="muted small relnote" id="irelstatus" role="status">Only ${n} releases arrived: ${svc} stopped answering (${esc(cat.error)}). ${retry}</p>`;
    if (cat.truncated) return `<p class="muted small relnote" id="irelstatus">${svc} lists more than ${n} releases for this artist; these are the first ${n} it lists, newest first.</p>`;
    if (cat.complete === false) return `<p class="muted small relnote" id="irelstatus">${svc} didn’t say whether this is the whole list: it may not be every release.</p>`;
    return '';
  };
  const relSections = (pg) => {
    const g = relGroups(pg.albums);
    return REL_SECTIONS.filter(([k]) => g[k].length).map(([k, , slug]) => `<section class="section relsec" data-rel="${k}" aria-labelledby="irel-${k}"><div class="section-head"><div class="relhead"><h3 id="irel-${k}">${esc(relTitle(k, g))}</h3><span class="muted small relcount">${relCount(g[k].length, pg.catalogue)}</span></div>${g[k].length > REL_PREVIEW ? `<button class="link relall" data-relall="${k}" data-slug="${slug}" aria-label="View all ${esc(relTitle(k, g))}">View all</button>` : ''}</div>${itemTileGrid(g[k].slice(0, REL_PREVIEW).map((a) => relTile(a, k)), 'album')}</section>`).join('');
  };
  const relSkeleton = () => `<div class="grid">${Array.from({ length: 6 }, () => '<div class="tile"><div class="art skeleton"></div><div class="skeleton relskel"></div></div>').join('')}</div>`;
  const bindRelSections = (box, pg, id) => {
    const g = relGroups(pg.albums);
    for (const [k] of REL_SECTIONS) { const sec = $(`[data-rel="${k}"]`, box); if (sec) bindItemTiles(sec, g[k].slice(0, REL_PREVIEW), 'album'); }
    $$('[data-relall]', box).forEach((b) => b.onclick = () => setView('releases', { id, section: b.dataset.slug }));
  };


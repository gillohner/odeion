        case 'home': {
          if (!state.sources) { try { state.sources = await api('GET', '/sources'); } catch { state.sources = []; } }
          const win = state.homeWindow || '7d';
          const sec = (name, title, opts = {}) => `<section class="section" id="h-${name}"><div class="section-head"><h3>${title}</h3>${opts.windows ? `<span class="chips win" ${win !== '7d' || homeCount('w:' + name, undefined, 1) ? '' : 'hidden'}>${WINDOWS.map(([w, l]) => `<button class="chip ${w === win ? 'on' : ''}" data-win="${w}">${l}</button>`).join('')}</span>` : ''}<button class="link" data-seeall="${name}" ${homeCount('s:' + name, undefined, 1) ? '' : 'hidden'}>See all</button></div><div class="hbody"><div class="skeleton" style="height:${homeCount('h:' + name, undefined, opts.h || 160)}px"></div></div></section>`;
          await pubkyUser();
          // Rows you can hide and reorder (UX review #10): your own layout (Settings › You › Home), over the node's
          const HOME_ROWS = [['listening', 'Listening now'], ['played', 'Recently played'], ['added', 'Recently added'], ['forgotten', 'Forgotten bookmarks'], ['lb', 'Recommended for you'], ['trending-songs', 'Trending songs', { windows: true, h: 300 }], ['trending-albums', 'Trending albums', { windows: true }], ['trending-artists', 'Trending artists', { windows: true }], ['following', 'From people you follow', { h: 120 }], ['tags', 'Tags people use for music', { h: 60 }], ['concerts', 'Concerts near you', { h: 80 }], ['podcasts', 'Podcasts'], ['yours', 'Yours on Pubky', { h: 120 }], ['new', 'New on Pubky', { h: 120 }]];
          // Your Home (Settings › You): your own layout where you set one, else the node's default
          if (!state.homeLayout) {
            const lay = await api('GET', '/me/prefs').then((p) => (p.home_layout && p.home_layout.value) || { order: [], hidden: [] }).catch(() => api('GET', '/home/layout').catch(() => ({ order: [], hidden: [] })));
            if (state.view !== 'home') return; // they went elsewhere while the homeserver answered
            state.homeLayout = lay; state.homeSaved = lay;
          }
          const lay = state.homeLayout; const known = HOME_ROWS.map(([k]) => k);
          const ordered = [...(lay.order || []).filter((k) => known.includes(k)), ...known.filter((k) => !(lay.order || []).includes(k))];
          const showRow = (k) => !(lay.hidden || []).includes(k);
          const rowOf = (k) => HOME_ROWS.find(([x]) => x === k);
          // While the library answers, a row shows the tiles it had last time (4 the first time), each
          // the size of a real tile, so the rows below don't move when the covers come (owner's recording)
          const localSec = (k, title) => `<section class="section" id="h-${k}" ${homeCount(k, undefined, k === 'forgotten' ? 3 : 4) === 0 && (k === 'forgotten' || state.setupShown) ? 'hidden' : ''}><div class="section-head"><h3>${title}</h3></div><div class="hbody">${k === 'forgotten' ? rowSkeleton(homeCount(k, undefined, 3)) : tileSkeleton(homeCount(k))}</div></section>`;
          const rowHtml = (k) => { const [, title, opts] = rowOf(k); if (k === 'lb') return '<div id="h-lbwrap"></div>'; if (k === 'listening') return '<div id="h-listenwrap"></div>'; if (['played', 'added', 'forgotten'].includes(k)) return localSec(k, title); return sec(k, title, opts || {}); };
          const editPanel = state.homeEdit ? `<section class="section homeedit" id="h-edit"><div class="section-head"><h3>Rows on Home</h3><button class="btn sm primary" id="hdone">Done</button></div><div class="list compact">${ordered.map((k, i) => `<div class="list-row"><label class="grow"><input type="checkbox" data-hshow="${k}" ${showRow(k) ? 'checked' : ''}> ${esc(rowOf(k)[1])}</label><button class="icon-btn" data-hup="${i}" ${i === 0 ? 'disabled' : ''} aria-label="Move ${esc(rowOf(k)[1])} up">${icon('chevron', 'ic up')}</button><button class="icon-btn" data-hdown="${i}" ${i === ordered.length - 1 ? 'disabled' : ''} aria-label="Move ${esc(rowOf(k)[1])} down">${icon('chevron')}</button></div>`).join('')}</div></section>` : '';
          main.innerHTML = pageHead('Home', null, `<button class="btn ghost sm" id="hedit" aria-pressed="${!!state.homeEdit}">${icon('settings')} Edit Home</button>`) + editPanel + '<div id="h-setup"></div>'
            + ordered.filter(showRow).map(rowHtml).join('');
          // Saves go one at a time and the newest layout is the one that ends up saved: while one is
          // on its way, a later change waits and is sent when it lands (each save takes seconds)
          const saveLayout = (next) => {
            if (!state.homeSaved) state.homeSaved = state.homeLayout;
            state.homeLayout = next; render();
            state.homePending = next;
            if (state.homeSaving) return;
            state.homeSaving = (async () => {
              while (state.homePending) {
                const send = state.homePending; state.homePending = null;
                try {
                  const p = await api('PUT', '/me/prefs', { home_layout: send });
                  state.homeSaved = (p.home_layout && p.home_layout.value) || send;
                } catch (e) {
                  toast(e.message, 'error');
                  if (!state.homePending) { state.homeLayout = state.homeSaved; if (state.view === 'home') render(); }
                }
              }
              state.homeSaving = null;
            })();
          };
          $('#hedit').onclick = () => { state.homeEdit = !state.homeEdit; render(); };
          // Listening now: what is known at once, then the node's latest
          renderListening(); refreshAlong();
          const hd = $('#hdone'); if (hd) hd.onclick = () => { state.homeEdit = false; render(); };
          $$('[data-hshow]', main).forEach((c) => c.onchange = () => { const hidden = new Set(lay.hidden || []); if (c.checked) hidden.delete(c.dataset.hshow); else hidden.add(c.dataset.hshow); saveLayout({ order: ordered, hidden: [...hidden] }); });
          const move = (i, d) => { const o = [...ordered]; const [x] = o.splice(i, 1); o.splice(i + d, 0, x); saveLayout({ order: o, hidden: lay.hidden || [] }); };
          $$('[data-hup]', main).forEach((b) => b.onclick = () => move(Number(b.dataset.hup), -1));
          $$('[data-hdown]', main).forEach((b) => b.onclick = () => move(Number(b.dataset.hdown), 1));
          $$('[data-win]', main).forEach((b) => b.onclick = () => { state.homeWindow = b.dataset.win; render(); });
          $$('[data-seeall]', main).forEach((b) => b.onclick = () => setView('home-section', { section: b.dataset.seeall, window: state.homeWindow || '7d' }));
          const setupBox = $('#h-setup');
          // This drawing's answers fill this drawing's rows only: Home draws itself again (a Trending
          // window, Edit Home), and an earlier drawing's late answer (last window's songs) would find
          // the new drawing's row by its id
          const here = () => setupBox.isConnected && state.view === 'home';
          const setupDrawn = renderSetup().catch((e) => console.error(e));
          // Without a contact Pubky has nothing to show: its rows go at once, and one line in their
          // place (once, where the first was) says why, with Add it (Settings › Discover). With Get
          // started hidden or done, Home said nothing about the missing rows (Home is the owner's
          // only). While Get started shows its contact step that step says how: the line waits for
          // it, hidden, and shows when there is no such step or when Get started is hidden
          const pubkyRows = ordered.filter(showRow).filter((k) => HOME_TITLES[k]);
          const noContact = (s) => {
            // (an earlier Home's answer: this Home's own handler takes care of its rows)
            if (!here()) return;
            if (!$('#h-needcontact')) {
              const first = pubkyRows.map((k) => document.getElementById('h-' + k)).find(Boolean) || s;
              first.insertAdjacentHTML('beforebegin', `<section class="section fadein" id="h-needcontact" hidden><div class="card small">Trending music, tags and picks from people on Pubky show up here once the node has your email or website. <button class="btn sm" type="button">Add it</button></div></section>`);
              const nc = $('#h-needcontact'); $('button', nc).onclick = openContact;
              setupDrawn.then(() => { if (nc.isConnected && !$('[data-step="contact"]:not(.done)', setupBox)) nc.hidden = false; });
            }
            s.remove();
          };
          // Local rows first: recently played, recently added, forgotten favourites
          // A row that didn't answer says so with Try again (retryOn); the library's three rows come in one answer
          const LOCAL = ['played', 'added', 'forgotten'];
          const loadLocal = (kept) => api('GET', '/library/home?limit=12').then((h) => {
            if (!here()) return;
            const albumRow = (k, list, emptyText) => {
              const el = $(`#h-${k} .hbody`); if (!el) return;
              const albums = (list || []).filter((a) => a.id != null).slice(0, 12);
              homeCount(k, albums.length);
              if (!albums.length && state.setupShown) { el.parentElement.remove(); return; }
              el.innerHTML = albums.length ? itemTileGrid(albums.map((a) => ({ id: 'lib-' + a.id, title: a.title, image: `/v1/library/albums/${a.id}/cover`, year: [a.album_artist, a.year].filter(Boolean).join(' · ') })), 'album') : `<p class="muted small">${emptyText}</p>`;
              el.parentElement.hidden = false; el.parentElement.classList.add('fadein');
              if (albums.length) bindItemTiles(el, albums.map((a) => ({ id: 'lib-' + a.id, title: a.title })), 'album');
            };
            albumRow('played', h.recently_played, 'Albums you play show up here.');
            albumRow('added', h.recently_added, 'New music in your folders shows up here.');
            const fe = $('#h-forgotten .hbody');
            if (fe) {
              const ts = h.forgotten || [];
              homeCount('forgotten', ts.length);
              if (!ts.length) { fe.parentElement.remove(); } else {
                const songs = ts.map((t, k) => ({ no: k + 1, title: t.title, artist: artists(t), album: t.album && t.album.title, album_id: t.album && t.album.id != null ? 'lib-' + t.album.id : null, duration_ms: t.duration_ms, track_id: t.id, id: trackPageId(t), image: t.album && t.album.id != null ? `/v1/library/albums/${t.album.id}/cover` : null }));
                fe.innerHTML = songRows(songs, { numbers: 'index' }); fe.parentElement.hidden = false; fe.parentElement.classList.add('fadein'); bindSongRows(fe, songs, {});
              }
            }
          }).catch(() => {
            if (!here()) return;
            for (const k of LOCAL) {
              const el = $(`#h-${k} .hbody`);
              if (el && el.querySelector('[aria-hidden="true"], .skeleton')) {
                el.innerHTML = '<p class="muted small hfail">The library did not answer. <button class="btn sm" type="button" data-hretry>Try again</button></p>'; el.parentElement.hidden = false;
                retryOn(el, (kept) => {
                  for (const j of LOCAL) { const b = $(`#h-${j} .hbody`); if (b && b.querySelector('[data-hretry]')) holdRow(b); }
                  loadLocal(kept ? k : null);
                });
              }
            }
          }).then(() => { if (kept && here()) focusRetry($(`#h-${kept} .hbody`) || $('#h-played .hbody'), true); });
          loadLocal(null);
          // ListenBrainz's picks for the user, when connected (Settings › ListenBrainz).
          api('GET', '/listenbrainz').then(async (lb) => {
            if (!lb.connected || !here()) return;
            const recs = await api('GET', '/listenbrainz/recommendations?limit=10');
            const wrap = $('#h-lbwrap'); if (!wrap || !here() || !recs.length) return;
            const songs = recs.map((x, k) => ({ no: k + 1, title: itemName(x), artist: x.artist, image: x.image, page: x.page || x.uri, uri: x.playable || null, id: idOf(x.uri) }));
            wrap.innerHTML = `<section class="section fadein" id="h-lb"><div class="section-head"><h3>Recommended for you</h3><span class="muted small">from your ListenBrainz listens</span></div>${songRows(songs, { numbers: 'index' })}</section>`;
            bindSongRows($('#h-lb'), songs, {});
          }).catch(() => { /* not connected or not answering: no section */ });
          // Every Pubky section in parallel, each rendered as it arrives.
          const loadPubky = (name, kept) => loadHomeSection(name, name.startsWith('trending') ? win : null).then((sc) => {
              const el = $(`#h-${name} .hbody`); if (!el || !here()) return;
              if (sc.needs === 'contact') { noContact(el.parentElement); return; }
              el.innerHTML = homeBody(sc, name, false); el.parentElement.classList.add('fadein'); bindHome(el, sc);
              // A Trending row that didn't answer on This week shows its error and Try again only: its
              // window chips (they ask the whole Home again) come back once it answers. On another
              // window they stay, the way back to This week. The next visit's placeholder draws them
              // as they were last (a row failing again doesn't move the rows below)
              const wins = $(`#h-${name} .section-head .chips.win`);
              if (wins) {
                const had = wins.contains(document.activeElement);
                wins.hidden = !!sc.error && win === '7d'; homeCount('w:' + name, sc.error ? 0 : 1);
                if (had && wins.hidden) { const b = $('[data-hretry]', el); if (b) b.focus(); }
              }
              retryOn(el, (kept) => { holdRow(el); loadPubky(name, kept); });
              focusRetry(el, kept);
              const see = $(`#h-${name} [data-seeall]`); if (see) see.hidden = !(sc.items.length > 6 || sc.tags.length > 20);
              // Its height and its See all for the next visit's placeholder (these rows mix songs,
              // covers and events; See all can take the header a second line)
              homeCount('h:' + name, Math.round(el.offsetHeight)); if (see) homeCount('s:' + name, see.hidden ? 0 : 1);
            });
          for (const name of ['trending-songs', 'trending-albums', 'trending-artists', 'following', 'tags', 'concerts', 'podcasts', 'yours', 'new'].filter(showRow)) loadPubky(name, false);
          break;
        }
        case 'home-section': {
          const { section, window } = state.viewArg || {};
          main.innerHTML = backBtn('home', 'Home') + pageHead(HOME_TITLES[section] || 'Home') + (section && section.startsWith('trending') ? `<div class="chips win">${WINDOWS.map(([w, l]) => `<button class="chip ${w === (window || '7d') ? 'on' : ''}" data-win="${w}">${l}</button>`).join('')}</div>` : '') + '<div id="hsfull"><div class="skeleton" style="height:400px"></div></div>';
          $('[data-back]', main).onclick = () => goBack('home');
          $$('[data-win]', main).forEach((b) => b.onclick = () => setView('home-section', { section, window: b.dataset.win }, 'replace'));
          // (an answer for a window since left, or a page since left, draws nothing)
          const shown = state.viewArg;
          const fill = async (kept) => {
            const sc = await loadHomeSection(section, section.startsWith('trending') ? window || '7d' : null, 60);
            const el = $('#hsfull'); if (state.view !== 'home-section' || state.viewArg !== shown || !el) return;
            el.innerHTML = homeBody(sc, section, true); el.classList.add('fadein'); bindHome(el, sc);
            retryOn(el, (k) => { holdRow(el); fill(k); });
            focusRetry(el, kept);
          };
          await fill(false);
          break;
        }
        case 'tag': {
          // Pinakes matches all of a tag page's labels (match=all), and takes at most 3 that way
          const MATCH_ALL_MAX = 3;
          // (a label written twice in a link counts once)
          const labels = [...new Set(String(state.viewArg || '').split(',').filter(Boolean))];
          const over = labels.length > MATCH_ALL_MAX;
          // The picked tags, whatever the answer is (loading, an error, nothing found): tap one for
          // only that tag, × takes it out (owner, 2026-11-12: "allow removing tags being looked for")
          const picked = labels.length > 1 ? `<div class="chips" id="tagpicked" role="group" aria-label="Tags looked for" style="margin-bottom:14px">${labels.map((l) => tagChip(l, { attrs: `data-tag-solo="${esc(l)}"`, title: `Only ${l}`, del: `data-tag-rm="${esc(l)}"` })).join('')}</div>` : '';
          main.innerHTML = backBtn('home', 'Home') + pageHead(labels.map((l) => '#' + l).join(' + ')) + picked + '<div id="tagcotags"></div><div id="tagbody">' + (over ? `<p class="muted small">A tag page matches up to ${MATCH_ALL_MAX} tags.</p>` : '<div class="skeleton" style="height:400px"></div>') + '</div>';
          $('[data-back]', main).onclick = () => goBack('home');
          // Either redraws the page: the keyboard stays there, on the next × (the one before for the
          // last), or on the heading once one tag is left and the row is gone
          const redraw = (b, rest) => {
            const focused = document.activeElement === b; const i = labels.indexOf(b.dataset.tagRm);
            return setView('tag', rest.join(','), 'replace').then(() => {
              if (!focused || document.activeElement !== document.body || state.view !== 'tag') return;
              const next = rest.length > 1 && i >= 0 ? rest[Math.min(i, rest.length - 1)] : null;
              const to = (next != null && $$('[data-tag-rm]', main).find((x) => x.dataset.tagRm === next)) || $('.page-head h2', main);
              if (!to) return;
              if (to.tagName === 'H2') to.tabIndex = -1;
              to.focus({ preventScroll: true });
            });
          };
          $$('[data-tag-rm]', main).forEach((b) => b.onclick = () => redraw(b, labels.filter((l) => l !== b.dataset.tagRm)));
          $$('[data-tag-solo]', main).forEach((b) => b.onclick = () => redraw(b, [b.dataset.tagSolo]));
          // More labels than Pinakes takes (an old link): not asked; one can be removed above
          if (over) break;
          // A read that didn't answer has Try again, as Home's rows: it asks again in place (the
          // picked tags stay; an answer for a page since left or drawn again draws nothing)
          const el = $('#tagbody'); const cot = $('#tagcotags');
          const fill = async (kept) => {
            const sc = await api('GET', `/home/tag?${new URLSearchParams({ labels: labels.join(','), limit: 60 })}`).catch((e) => ({ items: [], tags: [], error: e.message }));
            if (state.view !== 'tag' || !el.isConnected) return;
            sc.items = sc.items || []; sc.tags = sc.tags || [];
            // Narrowing further only while another label fits
            const co = labels.length < MATCH_ALL_MAX ? sc.tags.filter(([l]) => !labels.includes(l)).slice(0, 24) : [];
            cot.innerHTML = co.length ? `<div class="chips" style="margin-bottom:14px"><span class="muted small">Also tagged:</span>${co.map(([l, c]) => tagChip(l, { count: c, attrs: `data-tag-add="${esc(l)}"`, title: `Narrow to ${labels.join(', ')} and ${l}` })).join('')}</div>` : '';
            $$('[data-tag-add]', cot).forEach((b) => b.onclick = () => setView('tag', [...labels, b.dataset.tagAdd].join(',')));
            el.innerHTML = homeBody(sc, 'tag', true); el.classList.add('fadein'); bindHome(el, sc);
            retryOn(el, (k) => { holdRow(el); fill(k); });
            focusRetry(el, kept);
          };
          await fill(false);
          break;
        }
        case 'favourites': {
          // (drawn again where it stands, a library change or a heart: no skeleton, the scroll and
          // the focus kept, keepPlace)
          const again = redrawing('favourites'); if (!again) loading();
          const f = await loadFavourites();
          if (state.view !== 'favourites') return;
          const none = !f.albums.length && !f.tracks.length && !f.artists.length && !f.other.length;
          // (an empty page says whether the owner's are kept on their homeserver: opened first, before
          // the node was asked whether it is signed in with Pubky, it asks, for 1.5 s at most)
          if (none && !isListener() && state.pubkyUser === undefined) {
            await Promise.race([pubkyUser(), new Promise((r) => setTimeout(r, 1500))]);
            if (state.view !== 'favourites') return;
          }
          // A listener's are on their own homeserver: connect it, approve it again, or what waits
          const hs = f.homeserver;
          const hsNote = !hs ? '' : !hs.connected ? `<div class="notice" id="hsnote"><p>Your bookmarks are kept on your own homeserver. Connect it once (one approval in Pubky Ring), then the hearts here are yours.</p><button class="btn primary" data-hsconnect>Connect your homeserver</button></div>`
            : hs.lost ? `<div class="notice warn" role="alert" id="hsnote"><p>Your homeserver no longer lets this node in. These are the node’s copy; approve it again to change them.</p><button class="btn" data-hsconnect>Approve again</button></div>`
              : hs.pending ? `<p class="muted small" id="hsnote" role="status">${countOf(hs.pending, 'changes')} wait on this node until your homeserver answers.</p>`
                : hs.stale ? '<p class="muted small" id="hsnote" role="status">Your homeserver didn’t answer just now: this is the node’s copy.</p>' : '';
          // The count names only the kinds there are, in the order of the sections below (none on an
          // empty page: its empty state says so), with the other services' ones too
          const favCount = [[f.albums, 'albums'], [f.artists, 'artists'], [f.tracks, 'songs']].filter(([l]) => l.length).map(([l, w]) => countOf(l.length, w))
            .concat(f.other.length ? [`${f.other.length} from other services`] : []).join(' · ') || null;
          const place = again ? keepPlace() : null;
          // (drawn again: the other services' rows stay as they are until their new ones come)
          const oldOther = again ? $('#favother', main) : null; const favMine = state.favDraw = (state.favDraw || 0) + 1;
          main.innerHTML = pageHead('Bookmarks', favCount, f.tracks.length ? `<button class="btn primary" id="favplay">${icon('play')} Play songs</button>` : '')
            + hsNote
            + (none && !(hs && !hs.connected) ? emptyState('heart', 'No bookmarks yet', favHowTo(hs), favWayOn(), true) : '')
            + (f.albums.length ? `<section class="section"><div class="section-head"><h3>Albums</h3></div>${albumGrid(f.albums)}</section>` : '')
            + (f.artists.length ? `<section class="section"><div class="section-head"><h3>Artists</h3></div><div class="chips">${f.artists.map((a) => `<button class="chip" data-artist="${a.id}">${esc(a.name)}</button>`).join('')}</div></section>` : '')
            + (f.tracks.length ? `<section class="section"><div class="section-head"><h3>Songs</h3></div>${trackRows(f.tracks, { key: 'fav' })}</section>` : '')
            + (f.other.length ? `<section class="section"><div class="section-head"><h3>From other services</h3></div><div id="favother"><div class="skeleton" style="height:56px"></div></div></section>` : '');
          lists.fav = f.tracks; bindTrackRows(main, lists); // (its rows work while the rest is still coming)
          if (oldOther) { const nb = $('#favother', main); if (nb) nb.replaceWith(oldOther); }
          const hc = $('[data-hsconnect]', main); if (hc) hc.onclick = async () => { if (await homeserverDialog(!!(hs && hs.lost))) render(); };
          const fp = $('#favplay'); if (fp) fp.onclick = () => playTracks(f.tracks, true, 'end', { shuffle: false });
          state.drawnPage = 'favourites'; if (place) place();
          // (an older draw's answer that comes after a newer draw's is not put in)
          // (the keyboard on one of those rows stays on it when the new ones come)
          if (f.other.length) { const items = await describe(f.other); const box = $('#favother'); if (box && state.view === 'favourites' && state.favDraw === favMine) { const back = box.contains(document.activeElement) ? keepPlace() : null; box.innerHTML = discoverRows(items); bindDiscoverRows(items); if (back) back(); } }
          break;
        }
        case 'albums': {
          const mine = listMine = ++pageGen.albums;
          const again = redrawing('albums'); if (!again) loading();
          const albums = await albumsAll();
          if (listStale('albums', mine)) return;
          // No albums but songs: their files have no album tags (Settings' folders wouldn't help)
          const songs = albums.length ? 0 : await api('GET', '/library/stats').then((st) => st.tracks || 0, () => 0);
          if (listStale('albums', mine)) return;
          const lp = listPref('albums');
          // Favourites on a cold start: the list waits for the favourites (not "No favourite albums"),
          // 3 s at most (a listener's first go to their homeserver can take longer), then is drawn
          // again once they are in
          let favLate = false; let favFailed = null;
          if (lp.fav && favLoad) {
            const fl = favLoad;
            // (once is enough: drawn again meanwhile, by a sort or a chip, it doesn't wait again)
            if (favRedraw !== fl) await Promise.race([fl.catch(() => {}), new Promise((ok) => setTimeout(ok, 3000))]);
            if (listStale('albums', mine)) return;
            favLate = !fl.done; favFailed = fl.error || null;
            // Drawn again when they are in, while it still shows Favourites (those listed already
            // can be joined by new ones; drawn where it stands, keepList)
            if (favLate && favRedraw !== fl) { favRedraw = fl; const again = () => { if (state.view === 'albums' && listPref('albums').fav) render(); }; fl.then(again, again); }
          }
          pageShown.albums = mine;
          const sorted = sortList('albums', albums.filter((a) => inFocus(a, state.focus) && (!lp.fav || albumFav(a))), lp.sort || 'name');
          const shown = sorted; const kept = keepList('albums', again, '#albumgrid .tile');
          const letters = (lp.sort || 'name') === 'name' && shown.length > 40 ? [...new Set(shown.map((a) => firstLetter(a.title)))] : (lp.sort === 'artist' && shown.length > 40 ? [...new Set(shown.map((a) => firstLetter(a.album_artist)))] : []);
          // While the favourites are still coming the page says so, without a count ("0 of 45" would
          // say none are hearted, "45" that all of them are)
          const waiting = favLate && !shown.length;
          // They couldn't be read (the node or a listener's homeserver didn't answer): it says so, with
          // Try again and no count ("0 of 45" and "No favourite albums" said none were hearted)
          const failed = !!favFailed && !shown.length;
          main.innerHTML = pageHead('Albums', waiting || failed || !albums.length ? null : state.focus || lp.fav ? `${fmtCount(shown.length)} of ${fmtCount(albums.length)}` : albums.length, (albums.length > 12 ? filterBox('Filter albums', !shown.length) : '') + headAddMusic(albums.length || songs)) + (albums.length ? focusChips(albums) + listControls('albums', lp, letters) : '') + (shown.length ? '<div class="grid" id="albumgrid"></div>' : albums.length ? (waiting ? `<div id="albumswait">${emptyState('disc', 'Getting your bookmarks…', 'They show here as soon as they are in.')}</div>` : failed ? `<div id="albumsfavfail" role="alert">${emptyState('disc', 'Couldn’t get your bookmarks', favFailed.message || 'The node didn’t answer.', { id: 'albumsfavretry', label: 'Try again' })}</div>` : emptyState('disc', ...albumsNone(lp.fav, state.focus, lp.fav && albums.some(albumFav)))) : songs ? emptyState('disc', 'No albums yet', `${songs === 1 ? 'The song in the library has no album name in its tags' : `The ${fmtCount(songs)} songs in the library have no album name in their tags`}. Albums come from the tags of scanned files.`, { view: 'tracks', label: 'Open Songs' })
            : emptyState('disc', 'No albums yet', ...noMusicYet('Albums')));
          $$('[data-focus]', main).forEach((b) => b.onclick = () => { state.focus = b.dataset.focus; return renderSortedList(b, `[data-focus="${b.dataset.focus}"]`); });
          const ls = $('[data-lsort]', main); if (ls) ls.onchange = () => { setListPref('albums', { ...lp, sort: ls.value }); return renderSortedList(ls); };
          const lf = $('[data-lfav]', main); if (lf) lf.onclick = () => { setListPref('albums', { ...lp, fav: !lp.fav }); return renderSortedList(lf, '[data-lfav]'); };
          // (failing again, the keyboard stays on Try again)
          const refocus = favRetryFocus; favRetryFocus = false;
          const fr = $('#albumsfavretry', main); if (fr) { if (refocus) fr.focus(); fr.onclick = () => { favRetryFocus = document.activeElement === fr; fr.disabled = true; loadFavourites().catch(() => {}); render(); }; }
          const grid = $('#albumgrid');
          if (grid) {
            let stop = () => {};
            const draw = (list) => { stop(); grid.innerHTML = ''; stop = growItems(grid, list, (part) => part.map(tile).join(''), bindTiles, 120, Math.max(120, kept ? kept.n : 0)); };
            draw(shown);
            const none = filterNone(grid, 'album', kept && kept.none);
            const f = $('#filter');
            const matches = () => { const q = (f?.value || '').trim().toLowerCase(); return q ? shown.filter((a) => `${a.title} ${a.album_artist || ''} ${a.year || ''}`.toLowerCase().includes(q)) : shown; };
            // A–Z: the matches from that letter on (the grid draws as you scroll)
            const az = azJump(lp.sort === 'artist' ? (a) => firstLetter(a.album_artist) : (a) => firstLetter(a.title), matches, draw, none);
            if (f) {
              f.oninput = () => { const q = f.value.trim(); const list = matches(); draw(list); az(list); none(q, list.length); const c = $('.page-head .count'); if (c) c.dataset.filtered = q ? `${fmtCount(list.length)} shown` : ''; };
              f.onkeydown = (e) => { if (e.key === 'Escape') { f.value = ''; f.oninput(); f.blur(); } };
            }
          }
          state.drawnPage = 'albums'; if (kept) kept.done();
          break;
        }
        case 'artists': {
          const mine = listMine = ++pageGen.artists;
          const again = redrawing('artists'); if (!again) loading();
          const artistList = await api('GET', '/library/artists');
          if (listStale('artists', mine)) return;
          pageShown.artists = mine; state.artists = artistList;
          const alp = listPref('artists');
          const artistsSorted = sortList('artists', state.artists, alp.sort || 'name');
          const aletters = (alp.sort || 'name') === 'name' && artistsSorted.length > 40 ? [...new Set(artistsSorted.map((a) => firstLetter(a.name)))] : [];
          const kept = keepList('artists', again, '#artistlist [data-artist]');
          main.innerHTML = pageHead('Artists', state.artists.length, state.artists.length > 12 ? filterBox('Filter artists') : '') + (state.artists.length ? listControls('artists', alp, aletters) + '<div class="list" id="artistlist"></div>' : emptyState('artist', 'No artists yet', ...noMusicYet('Artists')));
          const alist = $('#artistlist');
          if (alist) {
            const row = (a) => `<div class="list-row clickable" data-artist="${a.id}"><div class="avatar" style="background:hsl(${hue(a.name)} 35% 28%)">${esc(initials(a.name))}</div><div class="grow"><div class="name">${esc(a.name)}</div><div class="sub">${[a.albums ? countOf(a.albums, 'albums') : '', countOf(a.tracks, 'songs')].filter(Boolean).join(' · ')}</div></div>${icon('chevron', 'ic right')}</div>`;
            const bindRows = () => $$('[data-artist]', alist).forEach((el) => el.onclick = () => setView('artist', Number(el.dataset.artist)));
            let stop = () => {};
            const draw = (list) => { stop(); alist.innerHTML = ''; stop = growItems(alist, list, (part) => part.map(row).join(''), bindRows, 150, Math.max(150, kept ? kept.n : 0)); };
            draw(artistsSorted);
            const none = filterNone(alist, 'artist', kept && kept.none);
            const f = $('#filter');
            const matches = () => { const q = (f?.value || '').trim().toLowerCase(); return q ? artistsSorted.filter((a) => a.name.toLowerCase().includes(q)) : artistsSorted; };
            const az = azJump((a) => firstLetter(a.name), matches, draw, none);
            const als = $('[data-lsort]', main); if (als) als.onchange = () => { setListPref('artists', { ...alp, sort: als.value }); return renderSortedList(als); };
            if (f) {
              f.oninput = () => { const q = f.value.trim(); const list = matches(); draw(list); az(list); none(q, list.length); const c = $('.page-head .count'); if (c) c.dataset.filtered = q ? `${fmtCount(list.length)} shown` : ''; };
              f.onkeydown = (e) => { if (e.key === 'Escape') { f.value = ''; f.oninput(); f.blur(); } };
            }
          }
          state.drawnPage = 'artists'; if (kept) kept.done();
          break;
        }
        case 'identify': {
          // "Wrong match?" from a song or album menu: those songs' matches
          if (state.viewArg && Array.isArray(state.viewArg.fix) && state.viewArg.fix.length) { await renderFixMatches(state.viewArg.fix); break; }
          // Album review: each folder's songs together, every song with its own options and
          // evidence. A song with one strong option is offered ticked; a song whose options
          // disagree waits for the owner's pick. Confirm stores the album's choices in one go,
          // or none when one went stale (409: reload).
          // Every await checks this is still the view it drew (leaving, or leaving and coming
          // back, starts a new one): an answer that comes late draws nothing
          const gen = state.idGen = (state.idGen || 0) + 1;
          const live = () => state.view === 'identify' && state.idGen === gen;
          // Something at once while the first page comes (owner, 2026-11-04: no answer for long),
          // in the page's own shape: its header, the line that explains it, rows where the list comes
          // (owner, 2026-11-08: a grid of covers first, then a list, then the header pushed it down)
          const idHow = (files, done = false) => `<details class="idhow small"><summary class="muted">How confirming works</summary><p class="muted${done ? ' idintro' : ''}">Each folder as an album, every song with what MusicBrainz knows about it. Ticked songs are only proposed: nothing is stored until you press Confirm. A confirmed match becomes the song's identity.${files ? ` ${esc(files)}` : ''}</p></details>`;
          main.innerHTML = pageHead('Confirm matches', null, '<button class="btn ghost" id="idskipall" disabled title="Leave every unsure song as it is">Skip all unsure</button>') + idHow('') + '<p class="muted small idloading" role="status">Loading the matches to confirm…</p>' + rowSkeleton(6);
          const [page, st] = await Promise.all([api('GET', '/library/suggestions/albums?limit=50'), api('GET', '/library/identify').catch(() => null)]);
          if (!live()) return;
          // Paged by folder: "Show more" asks for the albums after the last one shown, so albums
          // confirmed meanwhile don't make it skip any (an older node pages by offset)
          const cursor = (p) => (p.next_after != null ? `after=${encodeURIComponent(p.next_after)}` : p.next != null ? `offset=${p.next}` : null);
          const albums = page.albums.slice(); let next = cursor(page);
          // The owner's choice per song, kept while more albums load (a song not touched shows what is offered)
          const choices = new Map();
          let listGen = 0; // bumped when the list is cleared (Skip all): older answers are dropped
          // One change at a time per album (Leave as is, Confirm, Not this), and none while Skip all runs
          // (nor Skip all while one does): two answers would take the same songs off the count, and the
          // later toast would replace the first one's Undo. The other buttons wait meanwhile; the pressed
          // one keeps the focus (busy() marks it)
          const locked = new Map(); // album index, or 'all', → the button pressed
          const isLocked = (ai) => locked.has(ai) || locked.has('all');
          const lockSec = (sec) => { const ai = Number(sec.dataset.album); const by = locked.get(ai) || locked.get('all'); $$('[data-skip],[data-reject]', sec).forEach((b) => { b.disabled = !!by && b !== by; }); if (sec.relabel) sec.relabel(); };
          const hold = (k, by) => {
            if (by) locked.set(k, by); else locked.delete(k);
            if (!live()) return;
            (k === 'all' ? $$('[data-album]', main) : $$(`[data-album="${k}"]`, main)).forEach(lockSec);
            const ska = $('#idskipall', main); if (ska) ska.disabled = [...locked.values()].some((b) => b !== ska);
          };
          // Undo of a skip: the page is loaded again when it is still the one on screen (any generation)
          const undoSkip = (body) => async () => { const u = await unskipSongs(body); toast(`${countOf(u.restored, 'songs')} back for review`); if (state.view === 'identify') { setView('identify', undefined, 'replace'); focusReview(); } else if ($('#idbox')) renderIdentify(); };
          const MAX_CONFIRM = 500;
          const how = (o) => (o.source === 'fingerprint' ? 'audio fingerprint' : o.source === 'album' ? 'the folder as an album' : 'search by tags');
          const holdWhy = { search: 'found by its title and artist only', weak: 'a weak match', other_release: 'on another release than the rest of the album' };
          const isChecked = (g, value) => (choices.has(g.track.id) ? choices.get(g.track.id) === value : String(g.pick == null ? '' : g.pick) === value);
          // What Confirm does to the files depends on "Write IDs into my files": said as it is set
          const files = !st ? 'With "Write IDs into my files" on (Library › Identify), its MusicBrainz IDs are also written into the file.'
            : st.write_ids ? '"Write IDs into my files" is on, so its MusicBrainz IDs are also written into the file (its tags are backed up first).'
              : 'It is stored in the node; your files stay as they are ("Write IDs into my files" is off).';
          const albumHtml = (a, ai) => {
            const head = `${esc(a.album || a.key.split('/').filter(Boolean).slice(-1)[0] || 'Library')}${a.artist ? ` <span class="muted small">${esc(a.artist)}</span>` : ''}`;
            const parts = [countOf(a.songs.length, 'songs'), a.ready ? `${a.ready} ready` : '', a.conflicts ? `${a.conflicts} need${a.conflicts === 1 ? 's' : ''} your pick` : ''].filter(Boolean).join(' · ');
            const rel = a.release ? `<p class="muted small">${a.release.songs} of ${a.songs.length} have an option on <a href="https://musicbrainz.org/release/${esc(a.release.mbid)}" target="_blank" rel="noopener">${esc(a.release.title || 'one release')} ${icon('external')}</a>. Nothing is chosen for you by it.</p>` : '';
            const songs = a.songs.map((g) => {
              const t = g.track; const name = `pick-${t.id}`;
              // The radio is named by its label, which holds the option alone; the link and
              // "Not this" sit beside it, not inside it
              const opts = g.options.map((o) => `<div class="idopt idpick"><input type="radio" id="idopt-${o.id}" name="${name}" value="${o.id}" data-rec="${esc(o.recording_mbid)}" ${isChecked(g, String(o.id)) ? 'checked' : ''}><label class="grow" for="idopt-${o.id}"><span class="name">${esc(o.title || 'Recording')} <span class="muted small">${esc(o.artist || '')}</span></span><span class="sub">${o.release_title ? esc(o.release_title) + ' · ' : ''}${how(o)} · ${Math.round(o.score * 100)}%</span></label><a class="idmb small" href="https://musicbrainz.org/recording/${esc(o.recording_mbid)}" target="_blank" rel="noopener" aria-label="${esc(`${o.title || 'Recording'} on MusicBrainz`)}">MusicBrainz ${icon('external')}</a><button class="btn ghost sm" type="button" data-reject="${o.id}" data-track="${t.id}" data-rec="${esc(o.recording_mbid)}" aria-label="${esc(`Not ${o.title || 'this recording'}`)}">Not this</button></div>`).join('');
              const note = g.status === 'conflict' ? '<p class="small warn-text idhold">These disagree: pick one, or leave it for later.</p>' : g.hold ? `<p class="small muted idhold">Not ticked: ${esc(holdWhy[g.hold] || g.hold)}.</p>` : '';
              return `<div class="idrow" data-track="${t.id}" data-status="${esc(g.status)}"><div class="idfile"><div class="name">${t.track_no ? `<span class="muted">${t.track_no}.</span> ` : ''}${esc(t.title)}</div><div class="sub">${esc([artists(t), fmt(t.duration_ms)].filter(Boolean).join(' · '))}</div>${t.file.path ? `<details class="idpath small muted"><summary title="${esc(t.file.path)}">${esc(t.file.path.split('/').pop())}</summary><span>${esc(t.file.path)}</span></details>` : ''}${note}</div><div class="idopts" role="radiogroup" aria-label="${esc(t.title)}">${opts}<div class="idopt idpick idlater"><input type="radio" id="idlater-${t.id}" name="${name}" value="" ${isChecked(g, '') ? 'checked' : ''}><label class="grow small muted" for="idlater-${t.id}">Leave for later</label></div></div></div>`;
            }).join('');
            if (!a.songs.length) return '';
            return `<section class="section idalbum" data-album="${ai}"><div class="section-head"><h3>${head}</h3><span class="muted small">${parts}</span></div>${rel}<div class="list">${songs}</div><div class="actions idconfirm"><button class="btn ghost" type="button" data-skip="${ai}" title="Leave these songs as they are: they play and show as usual, and Identify again leaves them alone">Leave as is</button><button class="btn primary" type="button" data-confirm="${ai}"></button><span class="small muted" role="status" data-confirm-out="${ai}"></span></div></section>`;
          };
          let idNote = ''; // a note at the top of the list (the album asked for is not among these)
          const draw = () => {
            // (the note is for the draw it came with: a later one, after Show more or Skip all, leaves it out)
            const note = idNote ? `<p class="notice small idnote" role="status">${esc(idNote)}</p>` : ''; idNote = '';
            main.innerHTML = pageHead('Confirm matches', page.songs, page.songs ? '<button class="btn ghost" id="idskipall" title="Leave every unsure song as it is">Skip all unsure</button>' : '') + (albums.some((a) => a.songs.length) || next != null ? `${idHow(files, true)}${note}${albums.map(albumHtml).join('')}${next != null ? '<div class="actions"><button class="btn" type="button" id="idmore">Show more albums</button></div>' : ''}` : note + emptyState('search', 'Nothing to confirm', 'Unsure matches from Identify (Your library › Node library) show up here.', { id: 'idopen', label: 'Open Identify' }));
            $$('[data-album]', main).forEach(wire);
            const ido = $('#idopen'); if (ido) ido.onclick = openIdentify;
            const ska = $('#idskipall'); if (ska) ska.onclick = (ev) => { if (locked.size) return undefined; hold('all', ska); return busy(ska, async () => { let r; try { r = await api('POST', '/library/suggestions/skip', { all: true }); } catch (e) { hold('all', null); throw noRoute(e); } const ub = toast(`${countOf(r.skipped, 'songs')} left as they are`, '', { label: undoLabel(r), run: undoSkip({ all: true }) }); if (!live()) return; listGen++; albums.forEach((a) => { a.songs = []; }); page.songs = 0; next = null; hold('all', null); draw(); focusUndo(ub); }, ev); };
            // (a "Show more" answer that comes after Skip all is for a list that is gone)
            const more = $('#idmore'); if (more) more.onclick = async () => { more.disabled = true; const gen = listGen; try { const p = await api('GET', `/library/suggestions/albums?limit=50&${next}`); if (!live() || gen !== listGen) return; albums.push(...p.albums.filter((a) => !albums.some((x) => x.key === a.key))); next = cursor(p); draw(); } catch (e) { if (!live() || gen !== listGen) return; toast(e.message, 'error'); more.disabled = false; } };
          };
          // One album drawn again from what is known here, after a confirm or a "Not this" (the
          // rest of the page, its choices and where it is scrolled stay; nothing is fetched; the
          // note at the top was for the first draw and goes)
          const redraw = (ai) => {
            { const n = $('.idnote', main); if (n) n.remove(); }
            const sec = $(`[data-album="${ai}"]`, main); if (!sec) return;
            const html = albumHtml(albums[ai], ai);
            if (!html) { sec.remove(); } else { sec.insertAdjacentHTML('afterend', html); sec.remove(); wire($(`[data-album="${ai}"]`, main)); }
            const count = $('.page-head .count', main); if (count) count.textContent = fmtCount(page.songs);
            if (!albums.some((a) => a.songs.length) && next == null) draw();
            else if (!albums.some((a) => a.songs.length)) { const m = $('#idmore'); if (m) m.click(); }
          };
          const chosen = (sec) => $$('.idrow', sec).map((row) => { const r = $('input[type=radio]:checked', row); return r && r.value ? { track_id: Number(row.dataset.track), suggestion_id: Number(r.value), recording_mbid: r.dataset.rec } : null; }).filter(Boolean);
          const wire = (sec) => {
            const btn = $('[data-confirm]', sec); const out = $('[data-confirm-out]', sec);
            // Leave as is: this album's unsure songs are marked reviewed without an identity; undoable
            const skipBtn = $('[data-skip]', sec); if (skipBtn) skipBtn.onclick = (ev) => { const ai = Number(sec.dataset.album); if (isLocked(ai)) return undefined; hold(ai, skipBtn); return busy(skipBtn, async () => {
              const a = albums[ai]; const key = a.key; const nSongs = a.songs.length;
              let r; try { r = await api('POST', '/library/suggestions/skip', { albums: [key] }); } catch (e) { hold(ai, null); throw noRoute(e); }
              // (the toast and its Undo come even when the page was left meanwhile: the songs are skipped)
              // (the node's unskip of an album brings back every song of it left as it is, earlier ones too)
              const ub = toast(`${countOf(r.skipped != null ? r.skipped : nSongs, 'songs')} left as they are`, '', { label: 'Undo (this album’s skipped songs)', run: undoSkip({ albums: [key] }) });
              if (!live()) return;
              a.songs.forEach((g) => choices.delete(g.track.id)); a.songs = []; a.ready = 0; a.conflicts = 0;
              page.songs = Math.max(0, page.songs - nSongs);
              hold(ai, null); redraw(ai); focusUndo(ub);
            }, ev); };
            // More than one batch takes: the first ones are confirmed (all of them or none), the rest stay
            const label = () => { const n = chosen(sec).length; btn.textContent = !n ? 'Nothing chosen' : n > MAX_CONFIRM ? `Confirm the first ${MAX_CONFIRM} of ${n}` : `Confirm ${countOf(n, 'songs')}`; btn.disabled = !n || isLocked(Number(sec.dataset.album)); };
            // (a list gone stale keeps its "Reload this list"; it waits like the rest)
            sec.relabel = () => { if (sec.dataset.stale) btn.disabled = isLocked(Number(sec.dataset.album)); else label(); };
            lockSec(sec); $$('input[type=radio]', sec).forEach((r) => r.onchange = () => { choices.set(Number(r.name.slice(5)), r.value); label(); });
            btn.onclick = async () => {
              const ai = Number(sec.dataset.album); if (isLocked(ai)) return;
              const batch = chosen(sec).slice(0, MAX_CONFIRM); if (!batch.length) return; hold(ai, btn); btn.disabled = true; out.textContent = 'Confirming…';
              try {
                const r = await api('POST', '/library/suggestions/confirm', { choices: batch });
                const skipped = (r.files && r.files.skipped) || [];
                toast(`${countOf(r.count, 'songs')} identified${skipped.length ? `; IDs not written into ${countOf(skipped.length, 'files')} (${skipped[0].reason})` : ''}${r.files && r.files.error ? `; IDs not written: ${r.files.error}` : ''}`, skipped.length || (r.files && r.files.error) ? 'error' : '');
                if (!live()) return;
                // The confirmed songs leave this album; the page isn't loaded again
                const done = new Set(r.confirmed || batch.map((c) => c.track_id));
                const a = albums[ai];
                a.songs = a.songs.filter((g) => !done.has(g.track.id)); done.forEach((id) => choices.delete(id));
                a.ready = a.songs.filter((g) => g.pick != null).length; a.conflicts = a.songs.filter((g) => g.status === 'conflict').length;
                page.songs = Math.max(0, page.songs - done.size);
                hold(ai, null); redraw(ai);
              } catch (e) {
                if (!live()) return;
                const stale = (e.body && e.body.stale) || [];
                stale.forEach((x) => { const row = $(`.idrow[data-track="${x.track_id}"]`, sec); if (row) row.classList.add('idstale'); });
                if (stale.length) sec.dataset.stale = '1';
                hold(ai, null);
                out.textContent = e.message; out.classList.add('warn-text'); btn.disabled = false;
                if (stale.length) { btn.textContent = 'Reload this list'; btn.onclick = () => render(); }
              }
            };
            // "Not this" names the song and recording the page shows: one changed meanwhile is refused
            $$('[data-reject]', sec).forEach((b) => b.onclick = async (ev) => {
              ev.preventDefault();
              const ai = Number(sec.dataset.album); if (isLocked(ai)) return;
              hold(ai, b);
              try {
                await api('DELETE', `/library/suggestions/${b.dataset.reject}?track_id=${encodeURIComponent(b.dataset.track)}&recording_mbid=${encodeURIComponent(b.dataset.rec)}`);
                if (!live()) return;
                hold(ai, null);
                // That option leaves its song; a song with none left leaves the album
                const a = albums[ai]; const tid = Number(b.dataset.track); const sid = Number(b.dataset.reject);
                const g = a.songs.find((x) => x.track.id === tid);
                if (g) {
                  g.options = g.options.filter((o) => o.id !== sid);
                  if (choices.get(tid) === String(sid)) choices.delete(tid);
                  if (g.pick === sid) g.pick = null;
                  if (g.options.length === 1 && g.status === 'conflict') g.status = 'single';
                  if (!g.options.length) { a.songs = a.songs.filter((x) => x !== g); page.songs = Math.max(0, page.songs - 1); }
                  a.ready = a.songs.filter((x) => x.pick != null).length; a.conflicts = a.songs.filter((x) => x.status === 'conflict').length;
                }
                redraw(ai);
              } catch (e) { hold(ai, null); toast(e.message, 'error'); }
            });
          };
          draw();
          // From an album page ("Match this album to tag it"): to its folder, when it waits here
          // (once: a later redraw of the list stays where the owner is)
          const want = state.viewArg && state.viewArg.tracks;
          if (want && want.length) {
            state.viewArg = { ...state.viewArg, tracks: null };
            try { history.replaceState({ ...(history.state || {}), arg: histArg('identify', state.viewArg) }, '', location.pathname + location.search); } catch { /* sandboxed */ }
            const row = want.map((t) => $(`.idrow[data-track="${Number(t)}"]`, main)).find(Boolean); const sec = row && row.closest('.idalbum');
            if (sec) { sec.scrollIntoView({ block: 'start' }); const h = $('h3', sec); if (h) { h.tabIndex = -1; h.focus({ preventScroll: true }); } }
            else { idNote = next != null ? 'This album isn\'t among the first folders here: Show more at the end lists the rest.' : 'This album has no matches waiting to be confirmed. Identify (Settings) looks for it again.'; draw(); }
          }
          break;
        }
        case 'genres': {
          loading();
          // One tag idea: your Pubky tags and the genres in your files' metadata, side by side
          const [genres, mine, ns] = await Promise.all([api('GET', '/library/genres'), api('GET', '/tags').then((r) => r.tags || []).catch(() => []), api('GET', '/node/settings').catch(() => ({}))]);
          if (state.view !== 'genres') return;
          // No genres: as on Albums, with songs it says their tags have none and opens Songs; with no
          // songs (or no answer about them) the owner gets Add music, as on Songs, Albums and Artists
          const songs = genres.length ? 0 : await api('GET', '/library/stats').then((st) => st.tracks || 0, () => null);
          if (state.view !== 'genres') return;
          // Publishing the files' genres is the owner's setting (the node refuses a listener's), and with
          // no songs there is nothing to publish: then it waits until there is, unless it is already on
          // (so it can still be turned off) or the node didn't say how many songs it has
          const pubBox = isOwner() && (genres.length || songs !== 0 || ns.genre_tags) ? `<label class="small genre-pub"><input type="checkbox" id="genretags" ${ns.genre_tags ? 'checked' : ''}> Publish my files' genres as tags too (public on Pubky, on songs that are matched)</label>` : '';
          const byLabel = new Map(); for (const t of mine) byLabel.set(t.label, (byLabel.get(t.label) || 0) + 1);
          const tagsSec = `<section class="section"><div class="section-head"><h3>Your Pubky tags</h3><span class="muted small">public, on your homeserver</span></div>${byLabel.size ? `<div class="chips">${[...byLabel.entries()].sort((a, b) => b[1] - a[1]).map(([l, n]) => tagChip(l, { own: true, count: n, attrs: `data-mytag="${esc(l)}"`, title: `Your tag · ${countOf(n, 'items')}` })).join('')}</div>` : '<p class="muted small">No tags yet. Add them on any song, album or artist page.</p>'}${pubBox}</section>`;
          main.innerHTML = pageHead('Genres', genres.length) + tagsSec + `<section class="section"><div class="section-head"><h3>From your files</h3><span class="muted small">the genres in your files' metadata</span></div>` + (genres.length ? `<div class="cards">${genres.map((g) => `<button class="card genre" data-genre="${g.id}" style="--h:${hue(g.name)}"><h3 style="margin-bottom:4px">${esc(g.name)}</h3><div class="muted small">${g.albums} album${g.albums === 1 ? '' : 's'} · ${countOf(g.tracks, 'songs')}</div></button>`).join('')}</div>` : songs ? emptyState('genre', 'No genres yet', `${songs === 1 ? 'The song in the library has no genre in its tags' : `The ${fmtCount(songs)} songs in the library have no genre in their tags`}. Genres come from the tags of scanned files.`, { view: 'tracks', label: 'Open Songs' }) : emptyState('genre', 'No genres yet', ...noMusicYet('Genres')));
          main.insertAdjacentHTML('beforeend', '</section>');
          $$('[data-genre]', main).forEach((b) => b.onclick = () => setView('genre', Number(b.dataset.genre)));
          $$('[data-mytag]', main).forEach((b) => b.onclick = () => setView('discover', { tag: b.dataset.mytag }));
          const gt = $('#genretags'); if (gt) gt.onchange = async () => { try { await api('PUT', '/node/settings', { genre_tags: gt.checked }); toast(gt.checked ? 'Your files\u2019 genres are now tags on matched songs; they go to Pubky with the next sync' : 'New genres won\u2019t be published; the tags already made stay (remove them on each song)'); render(); } catch (e) { toast(e.message, 'error'); gt.checked = !gt.checked; } };
          break;
        }
        case 'genre': {
          const d = await api('GET', `/library/genres/${state.viewArg}`);
          if (state.view !== 'genre') return;
          main.innerHTML = backBtn('genres', 'Genres') + `<div class="page-head"><div><div class="eyebrow">Genre</div><h1>${esc(d.genre.name)}</h1><div class="muted">${countOf(d.genre.albums, 'albums')} · ${countOf(d.genre.tracks, 'songs')}</div></div><div class="actions"><button class="btn primary" id="gplay">${icon('play')} Play all</button></div></div>`
            + (d.albums.length ? `<section class="section"><div class="section-head"><h3>Albums</h3></div>${albumGrid(d.albums)}</section>` : '')
            + `<section class="section"><div class="section-head"><h3>Songs</h3></div>${trackRows(d.tracks, { key: 'genre' })}</section>`;
          lists.genre = d.tracks;
          $('#gplay').onclick = () => playTracks(d.tracks, true, 'end', { shuffle: false });
          break;
        }
        case 'tracks': {
          // Drawn again where it stands (the library changed: a scan ended, a playlist was edited
          // on another device): the songs that were listed are read again and put in at once, a
          // filter's matches too, so the page is neither emptied nor back at its top
          const again = redrawing('tracks');
          // One read at a time while it is drawn again: a change that comes during a read is drawn
          // once that read is in (thrown away for each newer one, a read slower than the changes
          // come would never show)
          // (a read of an earlier visit, cut off when another page was opened, never ends: it doesn't count)
          if (again && songsReading && songsReading.nav === navGen) { songsReading.again = true; return; }
          if (!again) { loading(); songsReading = null; }
          const drawn = ++songsDraw; const reading = again ? (songsReading = { again: false, nav: navGen }) : null;
          drawStale = () => drawn < songsShown || state.view !== 'tracks';
          // Pages from the node as the list scrolls; the filter is a library search on the node.
          const PAGE = 300;
          const findSongs = (q) => api('GET', `/library/search?${new URLSearchParams({ q, sources: 0, limit: 500 })}`);
          // (what the page on screen lists and has in its filter, when it is Songs drawn already)
          const listed = () => (redrawing('tracks') ? $$('#tracklist .track', main).length : 0);
          const typed = () => (redrawing('tracks') ? ($('#filter', main)?.value || '').trim() : '');
          const q0 = typed();
          // (the first `n` songs, a page at least, a thousand per answer, and whether more follow;
          // `from`: going on from what was read)
          const readSongs = async (n, from = { tracks: [], more: true }) => {
            const tracks = from.tracks; let more = from.more;
            while (more && tracks.length < n && !drawStale()) {
              const limit = Math.min(1000, Math.max(PAGE, n - tracks.length));
              const r = await api('GET', `/library/tracks?limit=${limit}${tracks.length ? `&after=${tracks[tracks.length - 1].id}` : ''}`);
              tracks.push(...r.tracks); more = r.tracks.length === limit;
            }
            return { tracks, more };
          };
          let first; let stats; let found;
          try {
            [first, stats, found] = await Promise.all([readSongs(q0 ? PAGE : Math.max(PAGE, listed())), api('GET', '/library/stats').catch(() => null), q0 ? findSongs(q0).catch(() => null) : null]);
            // (the list grew on the page while it was read: up to there too, so the new page is not shorter)
            for (let i = 0; i < 3 && !drawStale() && !typed() && first.more && listed() > first.tracks.length; i++) first = await readSongs(listed(), first);
          } catch (e) {
            // Drawn again in place and the read failed: the page stays as it is and says why (the
            // error page would take the list and its place); a change that waited is read a second later
            if (!reading || drawStale()) throw e;
            toast(e.message, 'error');
            if (reading.again) setTimeout(() => { if (state.view === 'tracks') render(); }, 1000);
            return;
          } finally { if (reading && songsReading === reading) songsReading = null; }
          // (a newer draw is on the page, or another page: this answer is not put in; a change
          // that waited for this read is drawn by a read of its own)
          if (drawStale()) { if (reading && reading.again && state.view === 'tracks') setTimeout(() => { if (state.view === 'tracks') render(); }); return; }
          songsShown = drawn;
          // (in place whenever Songs is on the page by now, also when an older read drew it meanwhile)
          const inPlace = redrawing('tracks');
          const oldFilter = inPlace ? $('#filter', main) : null; const place = inPlace ? keepPlace() : null;
          const nowTyped = oldFilter ? oldFilter.value.trim() : '';
          // (a count with no songs in the answer is nothing to list or play: the empty state)
          const total = first.tracks.length ? Math.max(stats ? stats.tracks : 0, first.tracks.length) : 0;
          main.innerHTML = pageHead('Songs', total, (total > 12 ? filterBox('Filter songs') : '') + (total ? `<button class="btn primary" id="shuffle">${icon('play')} Play all</button>` : '') + headAddMusic(total)) + (total ? '<div class="tracks" data-list="all" id="tracklist"></div><p class="muted small" id="tracknote" hidden></p>' : emptyState('notes', 'No songs yet', ...noMusicYet('Songs')));
          const box = $('#tracklist');
          if (box) {
            const shown = []; lists.all = shown;
            let stop = () => {}; let run = 0;
            const reset = () => { stop(); stop = () => {}; box.innerHTML = ''; shown.length = 0; };
            const append = (rows) => { box.insertAdjacentHTML('beforeend', trackRows(rows, { key: 'all', start: shown.length, bare: true })); shown.push(...rows); };
            const browse = () => {
              reset(); append(first.tracks); $('#tracknote').hidden = true;
              if (first.more) {
                const mine = run;
                stop = growList(box, async () => {
                  const r = await api('GET', `/library/tracks?limit=${PAGE}&after=${shown[shown.length - 1].id}`);
                  if (state.view !== 'tracks' || mine !== run) return false;
                  append(r.tracks); return r.tracks.length === PAGE;
                });
              }
            };
            const f = $('#filter'); let timer = null;
            const matched = (r) => {
              reset(); append(r.tracks);
              const note = $('#tracknote'); note.hidden = r.tracks.length < 500; note.textContent = 'The first 500 matches: type more to narrow them down.';
              const c = $('.page-head .count'); if (c) c.dataset.filtered = `${r.tracks.length}${r.tracks.length === 500 ? '+' : ''} shown`;
            };
            // (drawn again with a filter typed: its text, and its matches as read with the page)
            // (the box emptied while they were read: the whole list, not the old matches)
            const refound = !!(f && found && nowTyped);
            if (f && oldFilter && oldFilter.value) f.value = oldFilter.value;
            if (refound) matched(found); else browse();
            if (f) {
              const none = filterNone(box, 'song');
              if (refound && nowTyped === q0) none(q0, found.tracks.length);
              // (the matches could not be read again, or the box says something else by now: the
              // filter asks for what it says, and says so if that fails)
              if (nowTyped !== (refound ? q0 : '')) setTimeout(() => { if (f.isConnected) f.oninput(); });
              // Each keystroke empties the no-match line and outdates the search in flight, so only the
              // answer for the text in the box can say "No song matches".
              f.oninput = () => {
                clearTimeout(timer); const mine = ++run; none('', 1);
                timer = setTimeout(async () => {
                  const q = f.value.trim(); const c = $('.page-head .count');
                  if (!q) { browse(); none('', 0); if (c) c.dataset.filtered = ''; return; }
                  const r = await findSongs(q).catch((e) => { toast(e.message, 'error'); return null; });
                  if (!r && mine === run) none('', 1);
                  // (the page drawn again meanwhile: this filter box is the old page's)
                  if (!r || mine !== run || state.view !== 'tracks' || !f.isConnected) return;
                  matched(r); none(q, r.tracks.length);
                }, 200);
              };
              f.onkeydown = (e) => { if (e.key === 'Escape') { f.value = ''; f.oninput(); f.blur(); } };
            }
            // Play all: what is shown when filtered, else every track (fetched on the press).
            // (one load at a time, also across the page drawn again in place while it runs: the new
            // button is drawn pressed then)
            const sh = $('#shuffle');
            if (sh && state.playAllLoading) sh.disabled = true;
            if (sh) sh.onclick = async () => {
              if (state.playAllLoading) return;
              if ((f && f.value.trim()) || shown.length >= total) return playTracks(shown, true, 'end', { shuffle: false });
              // (the message stays while a big library loads, and goes when the songs play; a quick load
              // shows none, as Which song is this?'s lookup)
              // It counts up as each thousand comes (unless another message replaced it), and gives up
              // after 30 s without an answer: Play all can be pressed again
              let loading = `Loading ${countOf(total, 'songs')}…`; sh.disabled = true; state.playAllLoading = true;
              const slow = setTimeout(() => toast(loading, 'wait'), 400);
              const onPage = (n) => { const t = toastBox; const was = loading; loading = `Loading ${countOf(total, 'songs')}… ${fmtCount(n)} so far`; if (t.classList.contains('show') && t.textContent === was) toast(loading, 'wait'); };
              try { await playTracks(await pageAll('/library/tracks', (t) => t.id, (r) => r.tracks, { onPage, wait: 30000 }), true, 'end', { shuffle: false }); } catch (e) { toast(e.message, 'error'); }
              clearTimeout(slow); toastDone(loading); state.playAllLoading = false; sh.disabled = false;
              { const now = $('#shuffle'); if (now && now !== sh && state.drawnPage === 'tracks') now.disabled = false; }
            };
          }
          state.drawnPage = 'tracks';
          if (place) {
            place(); if (state.placeHold) state.placeHold();
            const f = $('#filter', main);
            if (oldFilter && f && f.value && document.activeElement === f) { try { f.setSelectionRange(oldFilter.selectionStart, oldFilter.selectionEnd, oldFilter.selectionDirection || 'none'); } catch { /* no caret */ } }
          }
          if (reading && reading.again) setTimeout(() => { if (state.view === 'tracks') render(); });
          break;
        }
        case 'search': {
          const a = state.viewArg || {};
          // Search with no words while a search still runs (Back, or the Search tab with an empty box):
          // no progress line and no combobox over results; that search's end changes nothing here
          if (!a.q) {
            $('#sprog').hidden = true; searchKeys.reset();
            search.setAttribute('aria-expanded', 'false'); search.removeAttribute('aria-controls');
            main.innerHTML = emptyState('search', 'Search', phoneMode() ? 'Type in the box above: your music services at once. Paste a link to open it.' : 'Type in the box above: your library, your services and Pubky at once. Paste a link to open it.'); break;
          }
          const scope = a.in || 'everywhere'; state.searchScope = scope;
          if (search.value.trim() !== a.q) search.value = a.q;
          const run = startSearch(a.q, scope, true);
          main.innerHTML = `<div class="page-head shead"><h2>Results for “${esc(a.q)}”</h2></div><div class="sbar" id="sbar"><div class="scopechips" id="scopechips"></div></div><div id="sresults"></div><div id="sresults-said" class="sr-only" aria-live="polite"></div>`;
          search.setAttribute('aria-controls', 'sresults'); search.setAttribute('aria-expanded', 'true');
          searchChanged(run);
          break;
        }
        case 'dashboard': {
          // A castable now-playing screen (UX review, next tier): /now-playing?zone=<id>
          const zid = state.viewArg;
          if (zid && (zid === BROWSER || state.zones[zid])) state.zone = zid;
          // A way back for whoever opened it from a link (a TV in kiosk use simply doesn't tap it)
          main.innerHTML = '<a class="dash-home" href="/" id="dashhome">‹ Odeion</a><div class="dash" id="dash" aria-live="polite"></div>';
          $('#dashhome').onclick = (e) => { e.preventDefault(); setView('home'); };
          state.dashKey = null; drawDashboard();
          break;
        }
        case 'smart': {
          // A smart playlist: its songs right now
          loading();
          const sid = state.viewArg;
          let d; try { d = await api('GET', `/smart/${encodeURIComponent(sid)}/tracks`); } catch (e) { main.innerHTML = backBtn('playlists', 'Collections') + failedHtml(e); break; }
          if (state.view !== 'smart' || state.viewArg !== sid) return;
          state.itemTitle = d.playlist.name;
          const ts = d.tracks || [];
          const songs = ts.map((t, k) => ({ no: k + 1, title: t.title, artist: artists(t), artists: t.artists, artist_id: t.artists && t.artists[0] && t.artists[0].id != null ? 'lib-' + t.artists[0].id : null, album: t.album && t.album.title, album_id: t.album && t.album.id != null ? 'lib-' + t.album.id : null, duration_ms: t.duration_ms, track_id: t.id, id: trackPageId(t) }));
          const covers = ts.map((t) => (t.album && t.album.id != null ? `/v1/library/albums/${t.album.id}/cover` : null));
          main.innerHTML = backBtn('playlists', 'Collections') + `<div class="hero">${mosaic(covers, d.playlist.name)}<div><div class="eyebrow">Smart collection · songs change as your library does</div><h1>${esc(d.playlist.name)}</h1><p class="sub">${countOf(ts.length, 'songs')}${d.matched > ts.length ? ` of ${d.matched} that match` : ''} · ${esc(smartSummary(d.playlist))}</p>
              <div class="actions" style="margin-top:12px"><button class="btn primary" id="smplay" ${ts.length ? '' : 'disabled'}>${icon('play')} Play</button><button class="btn" id="smshuffle" ${ts.length ? '' : 'disabled'}>${icon('shuffle')} Shuffle</button><button class="btn ghost" id="smedit">${icon('settings')} Edit rules</button></div></div></div>`
            + `<section class="section" id="smsongs"><div class="section-head"><h3>Songs</h3></div>${ts.length ? songRows(songs, { numbers: 'index' }) : '<p class="muted small">No song matches these rules right now.</p>'}</section>`;
          $('[data-back]', main).onclick = () => goBack('playlists');
          if (ts.length) { bindSongRows($('#smsongs'), songs, {}); enableSelect($('#smsongs'), songs); }
          $('#smplay').onclick = () => playTracks(ts, true, 'end', { shuffle: false });
          $('#smshuffle').onclick = () => playTracks(ts, true, 'end', { shuffle: true });
          $('#smedit').onclick = () => setView('smart-edit', sid);
          break;
        }
        case 'smart-edit': {
          // The rules, with a live preview of what they pick
          const sid = state.viewArg;
          let cur = { name: '', rules: [{ field: 'genre', op: 'contains', value: '' }], matches: 'all', sort: 'random', limit: 100, seeds: [], minutes: null, exclude: {} };
          if (sid && sid !== 'new') { try { cur = (await api('GET', `/smart/${encodeURIComponent(sid)}/tracks`)).playlist; } catch (e) { toast(e.message, 'error'); } }
          if (state.view !== 'smart-edit') return;
          const draw = () => {
            main.innerHTML = backBtn('playlists', 'Collections') + pageHead(sid === 'new' ? 'New smart collection' : 'Edit smart collection')
              + `<div class="smartedit"><label class="field"><span>Name</span><input class="input" id="smname" value="${esc(cur.name)}" placeholder="Late-night techno"></label>
                <div class="field"><span>Start from</span><div class="chips" id="smseeds">${(cur.seeds || []).map((sd, i) => `<span class="chip">${esc(sd.label || `${sd.kind} ${sd.id}`)} <button class="icon-btn" data-smseedrm="${i}" aria-label="Remove ${esc(sd.label || sd.kind)}" title="Remove">${icon('close')}</button></span>`).join('')}<input class="input sm" id="smseedq" placeholder="a song, artist, album or genre…" autocomplete="off" aria-label="Add a seed" style="min-width:220px"></div><div id="smseedhits" class="list"></div><p class="muted small">With seeds, the collection grows from songs like these (same genres, then the same artists); the rules below then narrow it.</p></div>
                <p class="small">Songs that match <select class="input sm" id="smmatch" aria-label="Match"><option value="all" ${cur.matches !== 'any' ? 'selected' : ''}>all</option><option value="any" ${cur.matches === 'any' ? 'selected' : ''}>any</option></select> of these rules:</p>
                <div id="smrules">${cur.rules.map((r, i) => smartRuleRow(r, i)).join('')}</div>
                <button class="btn sm ghost" id="smadd">${icon('plus')} Add a rule</button>
                <div class="toolbar" style="margin-top:12px"><label class="small">Order <select class="input sm" id="smsort">${SMART_SORTS.map(([k, l]) => `<option value="${k}" ${cur.sort === k ? 'selected' : ''}>${l}</option>`).join('')}</select></label><label class="small">At most <input class="input sm" id="smlimit" type="number" min="1" max="1000" value="${cur.limit || 100}" style="width:90px"> songs</label><label class="small">or <input class="input sm" id="smminutes" type="number" min="0" max="6000" value="${cur.minutes || ''}" placeholder="any" style="width:90px"> minutes</label></div>
                <div class="toolbar"><label class="small">Leave out artists <input class="input sm" id="smexart" value="${esc(((cur.exclude || {}).artists || []).join(', '))}" placeholder="comma-separated" style="width:200px"></label><label class="small">genres <input class="input sm" id="smexgen" value="${esc(((cur.exclude || {}).genres || []).join(', '))}" placeholder="comma-separated" style="width:160px"></label><label class="small">songs played in the last <input class="input sm" id="smexdays" type="number" min="0" value="${(cur.exclude || {}).played_within_days || ''}" placeholder="0" style="width:70px"> days</label></div>
                <div class="toolbar"><button class="btn primary" id="smsave">${icon('check')} Save</button>${sid !== 'new' ? `<button class="btn danger" id="smdel">${icon('trash')} Delete</button>` : ''}</div></div>
              <section class="section"><div class="section-head"><h3>Preview</h3><span class="muted small" id="smcount"></span></div><div id="smprev"><p class="muted small">Add a rule to see which songs match.</p></div></section>`;
            $('[data-back]', main).onclick = () => goBack('playlists');
            const readRules = () => $$('[data-smrule]', main).map((row) => { const g = (k) => { const el = $(`[data-k="${k}"]`, row); return el ? el.value : undefined; }; const field = g('field'); const r = { field, op: g('op') || '' }; if (field === 'favourite') r.value = g('value') !== 'no'; else if (['plays', 'last_played'].includes(field)) r.value = Number(g('value') || 0); else if (field === 'year') { r.value = Number(g('value') || 0) || null; r.to = Number(g('to') || 0) || null; } else r.value = g('value') || ''; return r; });
            const list = (id) => $(id).value.split(',').map((x) => x.trim()).filter(Boolean);
            const sync = () => { const days = Number($('#smexdays').value) || 0; cur = { ...cur, name: $('#smname').value, matches: $('#smmatch').value, sort: $('#smsort').value, limit: Number($('#smlimit').value) || 100, minutes: Number($('#smminutes').value) || null, exclude: { artists: list('#smexart'), genres: list('#smexgen'), ...(days ? { played_within_days: days } : {}) }, rules: readRules() }; };
            // A rule left blank (the editor's starting row) counts for nothing
            const filled = () => cur.rules.filter((r) => !(typeof r.value === 'string' && !r.value.trim()));
            const body = () => ({ name: cur.name, rules: filled(), matches: cur.matches, sort: cur.sort, limit: cur.limit, seeds: cur.seeds || [], ...(cur.minutes ? { minutes: cur.minutes } : {}), exclude: cur.exclude || {} });
            let st = null;
            $('#smseedq').oninput = () => { clearTimeout(st); const q = $('#smseedq').value.trim(); const hits = $('#smseedhits'); if (q.length < 2) { hits.innerHTML = ''; return; } st = setTimeout(async () => { try { const r = await api('GET', `/library/search?q=${encodeURIComponent(q)}&limit=5`); if (!$('#smseedhits')) return; const cands = [...(r.artists || []).map((a) => ({ kind: 'artist', id: a.id, label: `${a.name} (artist)` })), ...(r.albums || []).map((a) => ({ kind: 'album', id: a.id, label: `${a.title} (album)` })), ...(r.genres || []).map((g) => ({ kind: 'genre', id: g.id, label: `${g.name} (genre)` })), ...(r.tracks || []).map((t) => ({ kind: 'track', id: t.id, label: `${t.title}${artists(t) ? ' · ' + artists(t) : ''}` }))].filter((c) => c.id != null).slice(0, 8); hits.innerHTML = cands.map((c, k) => `<button class="list-row clickable" data-smseed="${k}"><div class="grow"><div class="name">${esc(c.label)}</div></div></button>`).join(''); $$('[data-smseed]', hits).forEach((b) => b.onclick = () => { sync(); cur.seeds = [...(cur.seeds || []), cands[Number(b.dataset.smseed)]]; draw(); preview(); }); } catch { hits.innerHTML = ''; } }, 250); };
            $$('[data-smseedrm]', main).forEach((b) => b.onclick = () => { sync(); cur.seeds.splice(Number(b.dataset.smseedrm), 1); draw(); preview(); });
            let t = null;
            const preview = () => { clearTimeout(t); t = setTimeout(async () => { if (state.view !== 'smart-edit' || !$('#smname')) return; sync(); const box = $('#smprev'); if (!box) return; if (!filled().length && !(cur.seeds || []).length) { box.innerHTML = '<p class="muted small">Add a rule or a seed to see which songs match.</p>'; $('#smcount').textContent = ''; return; } try { const r = await api('POST', '/smart/preview', { ...body(), limit: Math.min(cur.limit, 20) }); $('#smcount').textContent = `${countOf(r.matched, 'songs')} match`; const songs = r.tracks.map((x, k) => ({ no: k + 1, title: x.title, artist: artists(x), duration_ms: x.duration_ms, track_id: x.id, id: trackPageId(x) })); box.innerHTML = songs.length ? songRows(songs, { numbers: 'index' }) : '<p class="muted small">No song matches yet.</p>'; if (songs.length) bindSongRows(box, songs, {}); } catch (e) { box.innerHTML = `<p class="muted small">${esc(e.message)}</p>`; } }, 350); };
            main.oninput = (e) => { if (state.view !== 'smart-edit') { main.oninput = null; return; } if (e.target.closest('.smartedit')) preview(); };
            main.onchange = (e) => { if (state.view !== 'smart-edit') { main.onchange = null; return; } if (e.target.dataset && e.target.dataset.k === 'field') { sync(); const i = Number(e.target.closest('[data-smrule]').dataset.smrule); cur.rules[i] = { field: e.target.value, op: SMART_FIELDS[e.target.value].ops[0][0], value: '' }; draw(); } preview(); };
            $('#smadd').onclick = () => { sync(); cur.rules.push({ field: 'genre', op: 'contains', value: '' }); draw(); };
            $$('[data-smrm]', main).forEach((b) => b.onclick = () => { sync(); cur.rules.splice(Number(b.dataset.smrm), 1); draw(); });
            $('#smsave').onclick = async () => { sync(); if (!cur.name.trim()) { toast('Give it a name', 'error'); $('#smname').focus(); return; } try { const saved = sid === 'new' ? await api('POST', '/smart', body()) : await api('PUT', `/smart/${encodeURIComponent(sid)}`, body()); toast(`Saved “${saved.name}”`); setView('smart', saved.id); } catch (e) { toast(e.message, 'error'); } };
            const del = $('#smdel'); if (del) del.onclick = async () => { if (!del.dataset.armed) { del.dataset.armed = '1'; del.innerHTML = `${icon('trash')} Really delete?`; return; } try { await api('DELETE', `/smart/${encodeURIComponent(sid)}`); toast('Smart collection deleted'); setView('playlists'); } catch (e) { toast(e.message, 'error'); } };
            preview();
          };
          draw();
          break;
        }

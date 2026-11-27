        case 'playlists': {
          // A listener's are their own, on their own homeserver (no imports or smart playlists: those are the owner's)
          const mine = isListener();
          // (each draw numbered: one that answers after a newer one was drawn draws nothing, its error
          // neither, so the newer list stays; one that answers first is drawn meanwhile)
          const plDraw = state.plDraw = (state.plDraw || 0) + 1; const stale = () => plDraw < (state.plShown || 0);
          let got; try { got = await Promise.all([api('GET', '/playlists'), mine ? [] : api('GET', '/smart').catch(() => []), mine ? api('GET', '/me/homeserver').catch(() => null) : null]); } catch (e) { if (stale()) return; throw e; }
          const [lists, smarts, hs] = got;
          if (state.view !== 'playlists' || stale()) return;
          state.playlists = lists; state.plShown = plDraw;
          // Drawn again where it stands (the library changed, an import or a new playlist came in):
          // what is typed in the name and link boxes and its caret, the scroll and the focus stay.
          // Opening the page draws it from the start.
          const again = redrawing('playlists'); const place = again ? keepPlace() : null;
          const typed = again ? ['plname', 'plshared'].map((id) => document.getElementById(id)).filter((el) => el && main.contains(el)).map((el) => ({ id: el.id, value: el.value, sel: [el.selectionStart, el.selectionEnd, el.selectionDirection || 'none'] })) : [];
          const hsNote = !hs ? '' : !hs.connected ? `<div class="notice" id="hsnote"><p>Your collections are kept on your own homeserver. Connect it once (one approval in Pubky Ring), then the ones you make here are yours.</p><button class="btn primary" data-hsconnect>Connect your homeserver</button></div>`
            : hs.lost ? `<div class="notice warn" role="alert" id="hsnote"><p>Your homeserver no longer lets this node in. These are the node’s copy; approve it again to change them.</p><button class="btn" data-hsconnect>Approve again</button></div>`
              : hs.pending ? `<p class="muted small" id="hsnote" role="status">${countOf(hs.pending, 'changes')} ${hs.pending === 1 ? 'waits' : 'wait'} on this node until your homeserver answers.</p>`
                : hs.stale ? '<p class="muted small" id="hsnote" role="status">The node couldn’t read all of your homeserver just now (it didn’t answer, was slow, or has more posts than the node reads at once): this list is the node’s copy.</p>' : '';
          main.innerHTML = pageHead('Collections', state.playlists.length) + hsNote + `
            <div class="toolbar"><input class="input" id="plname" aria-label="New collection name" placeholder="New collection name"><button class="btn" id="plcreate">${icon('plus')} Create</button>${mine ? '' : `<label class="btn file-btn">${icon('upload')} Import M3U, CSV or Roon export<input type="file" id="plimport" accept=".m3u,.m3u8,.csv,.xlsx"></label><span id="plimportout" class="sr-only" aria-live="polite"></span>`}</div>
            <div class="toolbar"><input class="input" id="plshared" aria-label="Shared collection link" placeholder="Shared collection: paste its pubky:// link or a Pubky key"><button class="btn" id="plopenshared">${icon('external')} Open</button></div>`
            + (state.playlists.length ? `<div class="list">${state.playlists.map((p) => `<div class="list-row clickable" data-pl="${esc(p.id)}" tabindex="0" role="button">${mosaic(p.covers, p.name, 'pl-thumb')}<div class="grow"><div class="name">${esc(p.name)}</div><div class="sub">${plCount(p)}${p.description ? ' · ' + esc(p.description) : ''}</div></div><span class="pill ${p.synced ? 'ok' : ''}">${p.synced ? 'synced' : mine ? 'waiting' : 'local'}</span><button class="icon-btn" data-plplay="${esc(p.id)}" title="Play" aria-label="Play ${esc(p.name || 'this collection')}">${icon('play')}</button></div>`).join('')}</div>` : hs && !hs.connected ? '' : emptyState('playlist', 'No collections yet', mine ? 'Create one, or add songs from any ⋯ menu. They’re kept on your own homeserver.' : 'Create one, import a file from another player, or add songs from any list.'))
            + (mine ? '' : `<section class="section"><div class="section-head"><h3>Smart collections</h3><button class="btn sm" id="smartnew">${icon('plus')} New smart collection</button></div><p class="muted small">Songs picked by rules over your library and your tags; they change as your library does.</p>${smarts.length ? `<div class="list">${smarts.map((x) => `<div class="list-row clickable" data-smart="${esc(x.id)}">${mosaic([], x.name, 'pl-thumb')}<div class="grow"><div class="name">${esc(x.name)}</div><div class="sub">${esc(smartSummary(x))}</div></div><span class="pill">smart</span></div>`).join('')}</div>` : ''}</section>`);
          typed.forEach((t) => { const el = document.getElementById(t.id); if (el) el.value = t.value; });
          const hc = $('[data-hsconnect]', main); if (hc) hc.onclick = async () => { if (await homeserverDialog(!!(hs && hs.lost))) render(); };
          const sn = $('#smartnew'); if (sn) sn.onclick = () => setView('smart-edit', 'new');
          $$('[data-smart]', main).forEach((r) => r.onclick = () => setView('smart', r.dataset.smart));
          // (once made, the box is emptied; pressed from the keyboard, the keyboard goes to it for the next one.
          // A tap doesn't pull the on-screen keyboard up: the focus goes to the new playlist's row (scrolled
          // to if need be), not lost on the page when Create goes grey. The rows carry their tabindex in the
          // markup, so a redraw in place (the node's "library changed" right after) puts the focus back on it.)
          $('#plcreate').onclick = pageAct(async (ev) => { const n = $('#plname').value.trim(); if (!n) { $('#plname').focus(); return; } try { const made = await api('POST', '/playlists', { name: n }); toast(`Created “${n}”`); const box = $('#plname'); const nowhere = () => { const here = document.activeElement; return !here || here === document.body || here === $('#plcreate'); }; const tap = ev.pointerType === 'touch'; if (box) { if (box.value.trim() === n) box.value = ''; if (!tap && nowhere()) box.focus(); } const drawn = render(); const id = made && made.summary && made.summary.id; if (tap && id != null) drawn.then(() => { const row = state.view === 'playlists' && nowhere() && $(`#main [data-pl="${CSS.escape(String(id))}"]`); if (row) row.focus(); }, () => {}); } catch (e) { toast(e.message, 'error'); } });
          $('#plname').onkeydown = (e) => { if (e.key === 'Enter') $('#plcreate').click(); };
          needsText($('#plname'), $('#plcreate')); needsText($('#plshared'), $('#plopenshared'));
          const openShared = pageAct(async () => { try { const t = await api('GET', `/shared/resolve?link=${encodeURIComponent($('#plshared').value)}`); setView('shared', t.id ? { user: t.user, id: t.id } : { user: t.user }); } catch (e) { toast(e.message, 'error'); } });
          $('#plopenshared').onclick = openShared; $('#plshared').onkeydown = (e) => { if (e.key === 'Enter' && $('#plshared').value.trim()) $('#plopenshared').click(); };
          bindPlaylistFile($('#plimport'), 'plimportout');
          $$('[data-pl]', main).forEach((el) => el.onclick = (e) => { if (e.target.closest('[data-plplay]')) return; setView('playlist', el.dataset.pl); });
          $$('[data-plplay]', main).forEach((b) => b.onclick = () => playPlaylist(b.dataset.plplay));
          state.drawnPage = 'playlists'; if (place) place();
          typed.forEach((t) => { const el = document.getElementById(t.id); if (el && document.activeElement === el) { try { el.setSelectionRange(...t.sel); } catch { /* no caret */ } } });
          break;
        }
        case 'person': {
          // A person on Pubky: their own profile.json (via Nexus), the music they tag, what they share
          loading();
          const pid = state.viewArg;
          let pg; try { pg = await api('GET', `/profiles/${pid}`); } catch (e) { main.innerHTML = backBtn('home', 'Home') + failedHtml(e); break; }
          // Their homeserver may not answer: said as such, not as "no shared playlists"
          let sharedErr = null; let sharedPartial = false;
          const shared = await api('GET', `/shared/${pid}/playlists`, undefined, { onHeaders: (h) => { sharedPartial = h.get('x-partial') === '1'; } }).catch((e) => { sharedErr = e.message; return []; });
          if (state.view !== 'person' || state.viewArg !== pid) return;
          state.itemTitle = pg.name;
          const links = (pg.links || []).map((l) => `<a class="chip" href="${esc(webUrl(l.url))}" target="_blank" rel="noopener">${icon('external')} ${esc(l.title || l.url.replace(/^https?:\/\//, ''))}</a>`).join('');
          const tagged = pg.tagged || [];
          main.innerHTML = backBtn('home', 'Home')
            + `<div class="hero person"><div class="art ${pg.avatar ? 'round' : 'placeholder'} monogram" style="background:linear-gradient(135deg, hsl(${hue(pid)} 35% 34%), hsl(${hue(pid)} 35% 20%));border-radius:50%" aria-hidden="true"><span>${esc(initials(pg.name))}</span>${pg.avatar ? heroImg(pg.avatar, true) : ''}</div><div><div class="eyebrow">${['Person on Pubky', pg.you ? 'you' : '', pg.followed_by ? 'follows you' : ''].filter(Boolean).join(' · ')}</div><h1>${esc(pg.name)}</h1><p class="sub">${keyHtml(pid)}</p>${pg.status ? `<p class="muted">${esc(pg.status)}</p>` : ''}${pg.bio ? `<p class="blurb">${esc(pg.bio)}</p>` : ''}
              <div class="actions" style="margin-top:12px">${pg.you || !can('owner_music') ? '' : `<button class="btn ${pg.following ? '' : 'primary'}" id="pfollow" aria-pressed="${!!pg.following}">${pg.following ? `${icon('check')} Following` : `${icon('plus')} Follow`}</button>`}<a class="btn ghost" href="${esc(webUrl(pg.profile_url))}" target="_blank" rel="noopener">${icon('external')} Profile on pubky.app</a></div>
              ${links ? `<div class="chips" style="margin-top:10px">${links}</div>` : ''}<p class="muted small">${countOf(pg.followers || 0, 'followers')} · ${pg.following_count || 0} following</p></div></div>`
            + `<section class="section"><div class="section-head"><h3>Tags they use</h3><span class="muted small">public on Pubky</span></div>${(pg.tags || []).length ? `<div class="chips">${pg.tags.map(([l, c]) => `<button class="chip" data-ptag="${esc(l)}">#${esc(l)} <span class="muted">${c}</span></button>`).join('')}</div>` : '<p class="muted small">No music tags from them on Pubky yet.</p>'}</section>`
            + (tagged.length ? `<section class="section"><div class="section-head"><h3>Recently tagged</h3></div><div class="list">${tagged.map((h, k) => `<div class="list-row clickable" data-ptagged="${k}">${h.image ? `<img class="thumb" src="${esc(imgSrc(h.image))}" alt="" loading="lazy" data-err="hide">` : svcBadge('pubky')}<div class="grow"><div class="name">${esc(h.title)}</div><div class="sub">${esc([{ song: 'Song', album: 'Album', artist: 'Artist' }[h.kind] || h.kind, h.subtitle].filter(Boolean).join(' · '))}</div></div></div>`).join('')}</div></section>` : '')
            + `<section class="section"><div class="section-head"><h3>Collections they share</h3><span class="muted small">${shared.length || ''}</span></div>${shared.length ? `<div class="list">${shared.map((p) => `<div class="list-row clickable" data-pshared="${esc(p.id)}"><div class="avatar" style="background:hsl(${hue(p.name)} 35% 28%)">${icon('playlist')}</div><div class="grow"><div class="name">${esc(p.name)}</div><div class="sub">${plCount(p)}</div></div></div>`).join('')}</div>` : sharedErr ? `<div class="notice warn small" role="status">${esc(sharedErr)}</div>` : '<p class="muted small">No shared collections.</p>'}${sharedPartial ? `<p class="muted small" role="status">${esc(SHARED_PARTIAL)}</p>` : ''}</section>`;
          $('[data-back]', main).onclick = () => goBack('home');
          $$('[data-ptag]', main).forEach((b) => b.onclick = () => setView('discover', { tag: b.dataset.ptag }));
          $$('[data-ptagged]', main).forEach((r) => r.onclick = () => { const h = tagged[Number(r.dataset.ptagged)]; setView('item', { kind: h.kind === 'artist' ? 'artist' : h.kind === 'album' ? 'album' : 'song', id: h.id }); });
          $$('[data-pshared]', main).forEach((r) => r.onclick = () => setView('shared', { user: pid, id: r.dataset.pshared }));
          const fb = $('#pfollow'); if (fb) fb.onclick = async () => { const on = fb.getAttribute('aria-pressed') !== 'true'; fb.disabled = true; try { await api(on ? 'PUT' : 'DELETE', `/profiles/${pid}/follow`); profileCache.delete(pid); toast(on ? `Following ${pg.name}` : `Not following ${pg.name} any more`); render(); } catch (e) { toast(e.message, 'error'); fb.disabled = false; } };
          break;
        }
        case 'shared': {
          // Someone's shared playlists, or one of them: read-only here; "Save a copy" makes it yours
          // (shown to who may save one: can('owner_music'), as Follow on a person's page).
          loading();
          const a = state.viewArg || {}; const who = shortKey(a.user);
          if (!a.id) {
            let list; let partial = false; try { list = await api('GET', `/shared/${a.user}/playlists`, undefined, { onHeaders: (h) => { partial = h.get('x-partial') === '1'; } }); } catch (e) { main.innerHTML = backBtn('playlists', 'Collections') + failedHtml(e); break; }
            if (state.view !== 'shared') return;
            state.itemTitle = `Shared by ${who}`;
            main.innerHTML = backBtn('playlists', 'Collections') + pageHead(`Shared by ${who}`, list.length) + (partial ? `<p class="muted small" role="status">${esc(SHARED_PARTIAL)}</p>` : '') + (list.length ? `<div class="list">${list.map((p) => `<div class="list-row clickable" data-shpl="${esc(p.id)}"><div class="avatar" style="background:hsl(${hue(p.name)} 35% 28%)">${icon('playlist')}</div><div class="grow"><div class="name">${esc(p.name)}</div><div class="sub">${plCount(p)}${p.description ? ' · ' + esc(p.description) : ''}</div></div></div>`).join('')}</div>` : emptyState('playlist', 'Nothing shared yet', 'This person has no shared collections on Pubky.'));
            $$('[data-shpl]', main).forEach((r) => r.onclick = () => setView('shared', { user: a.user, id: r.dataset.shpl }));
            break;
          }
          let sp; try { sp = await api('GET', `/shared/${a.user}/playlists/${a.id}`); } catch (e) {
            if (state.view !== 'shared' || state.viewArg !== a) return;
            // A link pasted into search that names a post but no playlist (a comment, an ordinary post,
            // or one in a form this node can't read: 422): searched as text, as any other text, in this
            // page's place (Back goes where search was opened from). A playlist gone or a homeserver that
            // doesn't answer still says so here. (the box changed meanwhile: its own text decides; emptied
            // on a phone, the Search tab's empty page, as emptying it there gives)
            if (e.status === 422) notPlaylists.set(`${a.user}/${a.id}`, Date.now());
            if (a.q && e.status === 422) {
              const sb = $('#search'); const typed = sb && document.activeElement === sb ? sb.value.trim() : a.q;
              if (typed === a.q) {
                if (sb) sb.value = a.q;
                // (opened over a search with a query, the entry before: Back to it, which then takes this search, searchOver)
                const cur = history.state;
                // (one opened in that search's place, here, takes its search in its own entry: that Back would leave a search behind it)
                if (!a.here && navDepth > 0 && cur && cur.v === 'shared' && !cur.transient && typeof cur.fromPath === 'string' && cur.fromPath.startsWith('/search?')) {
                  if (searchOver) return; // (another draw of this page already went Back)
                  // (this entry, left as Forward, keeps no search: Forward onto it says the post is no playlist, never Back again)
                  try { history.replaceState({ ...cur, arg: { user: a.user, id: a.id } }, '', location.pathname + location.search); } catch { /* sandboxed */ }
                  const over = { q: a.q, in: a.in || 'everywhere', depth: (cur.depth || 0) - 1 }; searchOver = over;
                  const ask = { at: Date.now() }; overBack = ask; history.back();
                  // (no Back came in 3 s, the history still on this entry: the box's text decides in this entry; the Back landing later is undone)
                  setTimeout(() => {
                    if (searchOver !== over) return; searchOver = null; if (overBack === ask) overDone();
                    if (state.view !== 'shared' || state.viewArg !== a || !history.state || history.state.v !== 'shared') return;
                    overDecide(over);
                  }, 3000);
                  return;
                }
                startSearch(a.q, a.in || 'everywhere'); setView('search', { q: a.q, in: a.in || 'everywhere' }, 'replace'); return;
              }
              if (!typed && shellMode() === 'phone') { setView('search', undefined, 'replace'); return; }
            }
            main.innerHTML = backBtn('playlists', 'Collections') + failedHtml(e); break;
          }
          if (state.view !== 'shared') return;
          state.itemTitle = sp.summary.name;
          // A web page or a post plays nowhere: its card, in its place (review of #155, M4)
          const cardOf = (k) => { const e = sp.entries && sp.entries[k]; if (!e || (e.kind !== 'link' && e.kind !== 'post')) return null; return { no: k + 1, card: true, title: (() => { try { return new URL(e.uri).hostname.replace(/^www\./, ''); } catch { return e.uri; } })(), page: e.uri }; };
          const songs = sp.play_from.items.map((r, k) => { const card = cardOf(k); if (card) return card; const lib = r.plays_from && r.plays_from.service === 'library'; return { no: k + 1, title: itemName({ title: r.title, site: r.site }), artist: r.artist, image: r.image, duration_ms: r.duration_ms, track_id: lib ? r.track_id : null, uri: r.stream_uri || null, page: r.page || r.uri, id: idOf(r.uri), plays: r.plays_from, pinned: !!(r.pin && !r.pin_unavailable), unavailable: r.plays_from ? null : (r.reason || 'Not on your connected services') }; });
          const playable = songs.filter((t) => !t.unavailable && !t.card);
          const songCount = songs.filter((t) => !t.card).length; const linkCount = songs.length - songCount;
          const chip = (t) => t.unavailable ? `<span class="chip srcchip none" title="${esc(t.unavailable)}">${icon('close')}<span>Unavailable</span></span>` : t.plays ? `<span class="chip srcchip${t.pinned ? ' pinned' : ''}" title="${esc(t.plays.label)}${t.pinned ? ' · the author’s pick' : ''}">${svcBadge(t.plays.service)}<span>${esc(srcName(t.plays.service))}</span>${t.pinned ? icon('pin') : ''}</span>` : '';
          main.innerHTML = backBtn('playlists', 'Collections') + `<div class="hero">${mosaic((sp.play_from.items || []).map((r) => r.image), sp.summary.name)}<div><div class="eyebrow">Shared collection · by <code>${esc(who)}</code></div><h1>${esc(sp.summary.name)}</h1><p class="sub">${countOf(songCount, 'songs')}${linkCount ? ` · ${countOf(linkCount, 'links')}` : ''} · ${playable.length} play here</p>
              <div class="actions" style="margin-top:12px"><button class="btn primary" id="shplay" ${playable.length ? '' : 'disabled title="None of these songs is on your services"'}>${icon('play')} Play</button>${can('owner_music') ? `<button class="btn" id="shcopy">${icon('plus')} Save a copy</button>` : ''}<button class="btn ghost" id="shlink" title="Copy the pubky:// link">${icon('copy')} Copy link</button></div></div></div>`
            + `<section class="section" id="shsongs"><div class="section-head"><h3>Songs</h3><span class="muted small">where each plays from on your node</span></div>${songs.length ? songRows(songs, { numbers: 'index', rowClass: (t) => (t.unavailable ? 'unavail' : ''), extra: (t) => chip(t) }) : emptyState('playlist', 'This collection is empty', '')}</section>`
            + (sp.summary.description ? `<section class="section"><div class="section-head"><h3>About</h3></div><p>${esc(sp.summary.description)}</p></section>` : '');
          bindSongRows($('#shsongs'), songs, {});
          $('#shplay').onclick = (e) => playSongs(playable, 0, 'replace', null, e.currentTarget);
          const shc = $('#shcopy'); if (shc) shc.onclick = async () => { try { const d = await api('POST', `/shared/${a.user}/playlists/${a.id}/copy`); state.plPf = null; toast(`Saved “${d.summary.name}” to your collections`); setView('playlist', d.summary.id); } catch (e) { toast(e.message, 'error'); } };
          $('#shlink').onclick = async () => { try { await navigator.clipboard.writeText(sp.summary.link); toast('Link copied'); } catch { toast(sp.summary.link); } };
          break;
        }
        case 'playlist': {
          loadSvcRank(); // for the "Play this song from" menu's order
          // (drawn again where it stands, its sources found, a pin or a move: no skeleton, the
          // scroll and the focus kept, keepPlace)
          const again = redrawing('playlist'); if (!again) loading();
          const plRead = ++plDraws; // (plWrite: a change's hold ends with a draw that read after it)
          const plId = state.viewArg;
          const plMine = ++plGen; drawStale = () => plMine !== plGen || state.view !== 'playlist' || state.viewArg !== plId;
          // (under a hold: a read that doesn't answer ends in Try again after 15 s, plHolds; the
          // Pubky user's question gives up after 15 s of its own and the read goes on without it,
          // pubkyUser, so whichever 15 s end first decides. Drawn again (another device changed the
          // list), it doesn't wait for a user not known yet: it draws without it, and again once
          // the node answers, however late, pubkyKnown; Share asks for itself. A first draw whose
          // question gave up after its 15 s draws again on the late answer the same way)
          const plCut = plHolds.has(plId) ? new AbortController() : null; let plCutAt = null;
          if (plCut) { plCut.signal.why = new Error('Couldn’t load this collection: the node didn’t answer in time.'); plCutAt = setTimeout(() => plCut.abort(plCut.signal.why), PL_READ_MS); }
          // (the 15 s timer ends with the read, whichever way it ends)
          const plCutDone = () => clearTimeout(plCutAt);
          const drawOnPubky = () => { const stale = drawStale; pubkyKnown(stale).then(() => { if (!stale()) render(); }); };
          if (again && state.pubkyUser === undefined) drawOnPubky();
          else {
            try { await (plCut ? Promise.race([pubkyUser(), new Promise((_, no) => plCut.signal.addEventListener('abort', () => no(plCut.signal.why), { once: true }))]) : pubkyUser()); } catch (e) { plCutDone(); throw e; }
            if (state.pubkyUser === undefined && !isListener()) drawOnPubky();
          }
          // Where songs play from can take a while the first time (each service asked by ISRC):
          // the songs show at once, the chips follow.
          // (lastPf: the latest answer whatever its age, for the names of songs moved meanwhile; the chips
          // reuse it only under 30 s, cachedPf)
          const lastPf = state.plPf && state.plPf.id === plId ? state.plPf.data : null;
          const cachedPf = lastPf && Date.now() - state.plPf.at < 30000 ? lastPf : null;
          const pfFailed = !cachedPf && state.plPf && state.plPf.id === plId && state.plPf.failed && Date.now() - state.plPf.at < 30000;
          // A listener's collection plays what the library has: no play-from orders or pins (those are the owner's)
          const mine = isListener();
          // (prev: the answer before, for a song the next one leaves out)
          const prevPf = () => (state.plPf && state.plPf.id === plId ? state.plPf.data || state.plPf.prev : null);
          const askPf = () => api('GET', `/playlists/${encodeURIComponent(plId)}/play-from`).then((x) => { state.plPf = { id: plId, at: Date.now(), data: x, prev: prevPf() }; return x; }).catch(() => { state.plPf = { id: plId, at: Date.now(), data: null, failed: true, prev: prevPf() }; return null; });
          let pfP = mine ? Promise.resolve(null) : cachedPf ? Promise.resolve(cachedPf) : pfFailed ? Promise.resolve(null) : askPf();
          const d = await api('GET', `/playlists/${encodeURIComponent(plId)}`, undefined, plCut ? { signal: plCut.signal } : {}).finally(plCutDone);
          // The answer kept from before goes by place too: if the list changed meanwhile (another
          // device moved or removed a song), a row would show one song and remove another, so it is
          // asked for again. A list whose order differs from the rows on screen (plShown) is drawn
          // at once with its chips still coming, whatever the age of the answer and whether one came
          // yet (the old rows would stay up to 1.2 s, each Remove going by place). Not under a hold:
          // its list takes no presses till the draw (plWrite)
          const fitsList = (x) => !!x && x.items.length === d.playlist.items.length && x.items.every((r, k) => r.uri === d.playlist.items[k].uri);
          const pfFits = fitsList(cachedPf);
          const shown = again && plShown && plShown.id === plId ? plShown.uris : null;
          const pfMoved = !mine && !pfFits && !pfFailed && !plHolds.has(plId) && !!shown && (shown.length !== d.playlist.items.length || shown.some((u, k) => u !== d.playlist.items[k].uri));
          // (reread: the list was already read again after this answer, which still doesn't fit; it
          // is used as it is, by song, and not asked for again: a node naming songs otherwise than
          // the list can't keep the draw asking. Only while the list is the one the answer was
          // checked against: a read that also brings another device's change asks again, so a song
          // added in it gets its name and where it plays. A page opened afresh asks again)
          const listIs = (uris) => !!uris && uris.length === d.playlist.items.length && uris.every((u, k) => u === d.playlist.items[k].uri);
          const pfReread = again && !!cachedPf && !pfFits && !mine && listIs(state.plPf.reread);
          const pfAfterRead = !!cachedPf && !pfFits && !mine && !pfReread;
          if (pfAfterRead) pfP = askPf();
          let pf = (pfFits && cachedPf) || (pfMoved ? undefined : await Promise.race([pfP, new Promise((r) => setTimeout(() => r(undefined), 1200))]));
          if (drawStale()) return;
          // (an answer asked before the list was read may be for the old places, another device having
          // changed it meanwhile: the rows wait for one that fits. One asked after the read that
          // doesn't fit says the list moved past this read: each row takes its own song's answer, by
          // uri and copy (never another song's by place), and the list is read once more, as for a
          // list moved under the rows on screen, so Remove and Move go by the places the node has)
          if (pf && !fitsList(pf) && !pfAfterRead && !pfReread) pf = undefined;
          // (every answer asked after the read that doesn't fit holds, whatever its names: the node
          // answers each song by the list's own uri (play_from.rs), so one that doesn't fit is a list
          // changed under the read, a song moved or put in by another device)
          const pfReadAgain = !!pf && pfAfterRead && !fitsList(pf);
          const pfPending = pf === undefined;
          // (the mark holds for the one read after it: an answer drawn otherwise loses it, so a list
          // moved again later asks for where its songs play again)
          // (reread: the list this answer was checked against, its songs' uris)
          const readUris = d.playlist.items.map((it) => it.uri);
          if (!pfPending && state.plPf && state.plPf.id === plId) state.plPf.reread = pfReadAgain && state.plPf.data === pf ? readUris : null;
          // (and the rows of this read go by places the node no longer has: until the read after
          // it has drawn them they take no presses, × / Move / a pin / the bar's Remove, as under a
          // change's hold, plWrite; that read ends the hold, or its 15 s end in Try again)
          // (the focus in the rows on screen, before this draw replaces them: the read that ends the hold gives it back)
          const holdForReread = (focus) => { if (!plHolds.has(plId)) { plHolds.set(plId, { id: plId, after: plDraws + 1, focus }); plHoldShow(); } };
          const focusWas = pfReadAgain ? plFocusOf(document.activeElement) : null;
          if (pfReadAgain) { const stale = drawStale; setTimeout(() => { if (!stale()) render(); }, 0); }
          // (a slow answer asked after the read that doesn't fit marks the next draw's read the same way.
          // One asked before the read that doesn't fit, dropped above, holds the rows too: the list
          // changed between the two, and which came first isn't known, so the rows on screen may go by
          // places the node no longer has. The read it asks for ends the hold, plWrite)
          if (pfPending) { pf = null; let pfDone = false; pfP.then((x) => { pfDone = true; const here = state.view === 'playlist' && state.viewArg === plId; if (!pfReread && x && !fitsList(x) && state.plPf && state.plPf.id === plId && state.plPf.data === x) { if (pfAfterRead) state.plPf.reread = readUris; if (here && plShown && plShown.id === plId && plShown.uris.join('\n') === readUris.join('\n')) holdForReread(plFocusOf(document.activeElement)); } if (here) render(); }); setTimeout(() => { const ps = $('#plsongs'); if (ps && state.view === 'playlist' && state.viewArg === plId) stillLoading(ps, 'Still finding where these songs play… services answer slowly the first time.', () => !pfDone); }, 0); }
          state.itemTitle = d.summary.name;
          // Each song as it plays here: the library file, a service, or unavailable (play_from.rs).
          // (a song listed twice: its k-th copy takes the earlier answer's k-th copy, each place its own pin)
          const lastByUri = new Map(); if (pfPending && lastPf) lastPf.items.forEach((x) => { if (!lastByUri.has(x.uri)) lastByUri.set(x.uri, []); lastByUri.get(x.uri).push(x); });
          const copyNo = []; const copiesSeen = new Map(); d.playlist.items.forEach((it, k) => { const n = copiesSeen.get(it.uri) || 0; copyNo[k] = n; copiesSeen.set(it.uri, n + 1); });
          // (an answer that doesn't fit the list, asked after the read: the list changed again
          // meanwhile, or the node names a song otherwise. Each row takes its own song's answer, by
          // uri and copy as lastByUri, never the one at its place; a song it leaves out keeps the
          // earlier answer's, as a pending draw does)
          const pfAt = (() => {
            if (!pf || fitsList(pf)) return (k) => pf && pf.items[k];
            const byUriOf = (x) => { const m = new Map(); (x ? x.items : []).forEach((r) => { if (!m.has(r.uri)) m.set(r.uri, []); m.get(r.uri).push(r); }); return m; };
            const earlier = state.plPf && state.plPf.id === plId && state.plPf.data === pf ? state.plPf.prev : lastPf !== pf ? lastPf : null;
            const byUri = byUriOf(pf); const byUriWas = byUriOf(earlier);
            return (k) => { const u = d.playlist.items[k].uri; return (byUri.get(u) || [])[copyNo[k]] || (byUriWas.get(u) || [])[copyNo[k]]; };
          })();
          const songs = d.playlist.items.map((it, k) => {
            // A web page or a post plays nowhere: it shows as its card, in its place (B3)
            const kind = d.entries && d.entries[k] && d.entries[k].kind;
            if (kind === 'link' && mbSearchWords(it.uri)) return { no: k + 1, card: true, search: true, title: mbSearchWords(it.uri), page: it.uri };
            if (kind === 'link' || kind === 'post') return { no: k + 1, card: true, title: (() => { try { return new URL(it.uri).hostname.replace(/^www\./, ''); } catch { return it.uri; } })(), page: it.uri };
            // A listener's library album shows as itself, in its place; Play and Shuffle play its
            // songs there (listenerSongs), a row's own play skips it as it does any card
            const ea = mine && kind === 'album' && d.entries[k].album;
            if (ea) return { no: k + 1, card: true, title: ea.album_artist ? `${ea.title} · ${ea.album_artist}` : ea.title, page: it.uri };
            const t = d.tracks[k]; const r = pfAt(k);
            const base = r ? { plays: r.plays_from, pinned: !!(r.pin && !r.pin_unavailable), pinLost: !!r.pin_unavailable, unavailable: r.plays_from ? null : (r.reason || 'Not on your connected services'), sources: r.sources, connect: r.connect } : {};
            const lib = r && r.plays_from && r.plays_from.service === 'library';
            if (t && pfPending) return { pending: true, no: k + 1, title: t.title, artist: artists(t), artists: t.artists, duration_ms: t.duration_ms, track_id: t.id, id: trackPageId(t) };
            if (t && (!r || lib)) return { ...base, no: k + 1, title: t.title, artist: artists(t), artists: t.artists, artist_id: t.artists && t.artists[0] && t.artists[0].id != null ? 'lib-' + t.artists[0].id : null, album: t.album && t.album.title, album_id: t.album && t.album.id != null ? 'lib-' + t.album.id : null, duration_ms: t.duration_ms, track_id: t.id, id: trackPageId(t) };
            if (r) return { ...base, no: k + 1, title: itemName({ title: r.title, site: r.site }), artist: r.artist, image: r.image, duration_ms: r.duration_ms, track_id: lib ? r.track_id : null, uri: r.stream_uri || null, page: r.page || it.uri, id: idOf(it.uri) };
            // (a song moved by another device keeps what the earlier answer gave it, its name and its
            // stream, while it's asked again: it plays and shows as playing meanwhile)
            const was = pfPending && lastPf && (lastByUri.get(it.uri) || [])[copyNo[k]];
            if (was) return { no: k + 1, title: itemName({ title: was.title, site: was.site }), artist: was.artist, image: was.image, duration_ms: was.duration_ms, uri: was.stream_uri || null, pending: true, page: was.page || it.uri, id: idOf(it.uri) };
            return { no: k + 1, title: pfPending ? 'Loading…' : itemName({ title: null, site: null }), pending: pfPending, page: it.uri, id: idOf(it.uri) };
          });
          const localKeys = (d.local_tracks || []).map((t) => String(t.id)); // (each local song's own id, plFocusOf)
          const localSongs = (d.local_tracks || []).map((t, k) => ({ no: k + 1, title: t.title, artist: artists(t), artists: t.artists, artist_id: t.artists && t.artists[0] && t.artists[0].id != null ? 'lib-' + t.artists[0].id : null, album: t.album && t.album.title, album_id: t.album && t.album.id != null ? 'lib-' + t.album.id : null, duration_ms: t.duration_ms, track_id: t.id, id: trackPageId(t) }));
          const total = [...songs, ...localSongs].reduce((a, t) => a + (t.duration_ms || 0), 0);
          const songCount = songs.filter((t) => !t.card).length; const cardCount = songs.length - songCount;
          // How many are in this library: the source chips above the songs say it when they show; a listener (no chips)
          // or a failed source lookup gets it on the top line, as a shared playlist's "N play here"
          const libNote = songCount + localSongs.length && (mine || (!pf && !pfPending)) ? ` · ${d.tracks.filter((t, k) => t && !(songs[k] && songs[k].card)).length + localSongs.length} in this library` : '';
          const empty = (t) => `<p class="muted small">${t}</p>`;
          // Summary bar: "12 songs · 8 Library · 3 TIDAL · 1 unavailable", each part a filter.
          const counts = new Map(); songs.forEach((t) => { const key = t.unavailable ? 'none' : t.plays ? t.plays.service : null; if (key) counts.set(key, (counts.get(key) || 0) + 1); });
          // (a chip whose songs are all gone, removed or now playing from elsewhere, goes off: the
          // whole playlist shows again with "N songs" on, never an empty list with no chip on. Only an
          // answer that came says so: a failed one keeps the chip chosen, the rows showing unfiltered
          // with "Try again" until an answer comes, and the chip is on again then)
          if (state.plFilter && state.plFilter.id === plId && pf && !counts.has(state.plFilter.key)) state.plFilter = null;
          const filt = (pf || pfPending) && state.plFilter && state.plFilter.id === plId ? state.plFilter.key : null;
          // While the play-from answer is on its way, its chip row keeps its place, so the songs don't jump (second review #20)
          const summaryBar = pfPending && songs.length ? '<div class="chips plsummary" aria-hidden="true"><span class="chip skeleton chip-sk"></span><span class="chip skeleton chip-sk"></span></div>' : pf && songs.length ? `<div class="chips plsummary" role="group" aria-label="Filter by source"><button class="chip ${filt ? '' : 'on'}" data-pfilter="" aria-pressed="${!filt}">${songCount} song${songCount === 1 ? '' : 's'}</button>${[...counts.entries()].sort((a, b) => (a[0] === 'none') - (b[0] === 'none') || b[1] - a[1]).map(([key, n]) => `<button class="chip ${filt === key ? 'on' : ''} ${key === 'none' ? 'warnchip' : ''}" data-pfilter="${esc(key)}" aria-pressed="${filt === key}">${key === 'none' ? '' : svcBadge(key)}${n} ${key === 'none' ? 'unavailable' : esc(srcName(key))}</button>`).join('')}</div>` : '';
          const pfNote = !mine && !pf && !pfPending && songs.length ? '<p class="muted small pfnote" role="status">Couldn’t find where these songs play. <button class="link" id="pfretry">Try again</button></p>' : '';
          // Whose homeserver a shared copy is on: the owner's Pubky identity, or the listener's own
          const sharer = mine ? state.me && state.me.key : state.pubkyUser;
          const sharingActions = d.summary.visibility === 'public'
            ? `${sharer ? `<button class="btn hero-zone" id="plcopylink" aria-label="Copy link" title="Copy link">${icon('copy')}<span class="lbl"> Copy link</span></button>` : ''}<button class="btn ghost hero-zone" id="plunshare" aria-label="Make private" title="Make private">${icon('eye-off')}<span class="lbl"> Make private</span></button>`
            : `<button class="btn hero-zone" id="plshare" aria-label="Share on Pubky" title="Share on Pubky">${icon('upload')}<span class="lbl"> Share on Pubky</span></button>`;
          const sharingNote = d.summary.visibility === 'public'
            ? (d.summary.synced ? 'Public on Pubky. Anyone with the link can open it.' : mine ? 'Public copy pending: shared once your homeserver answers.' : 'Public copy pending: shared on the next sync with your homeserver.')
            : (d.summary.synced ? 'Private on your homeserver.' : mine ? 'Private; it waits on this node until your homeserver answers.' : 'Private on this node; saved to your homeserver on the next sync.');
          // (a song's control that had focus before this draw: put back on that same song wherever it
          // is now, or passed on, plFocusIn, never by its place in the list (keepPlace's way): another
          // device removing or moving a song above it would put it on another song's ×, Move or row)
          const rowWas = again ? plFocusOf(document.activeElement) : null;
          // (whether that song's row, or under a change's hold the row pressed (else the one the hold
          // gives the focus to), was wholly in sight before this draw: only then is the song the focus
          // goes on to scrolled into sight; one they scrolled away from on purpose stays where it is.
          // Drawn afresh (Try again), there were no rows to scroll away from: it follows. Looked at
          // before keepPlace, which ends an earlier redraw's hold: its spacer going can scroll the page)
          const seenWas = (f) => { if (!f || f.song == null) return false; const j = plPlaceOf(plKeysOf(f.box), f.song, f.copy); const row = j < 0 ? null : $(`#${f.box} [data-srow="${j}"]`, main); return !!row && row.getClientRects().length > 0 && inSight(row); };
          const rowSeen = seenWas(rowWas); const heldWas = plHolds.get(plId); const heldSeen = !again || (!!heldWas && seenWas(heldWas.from || heldWas.focus));
          const place = again ? keepPlace(!!rowWas && rowWas.song != null) : null;
          // (the eyebrow is the kind alone: who sees the playlist and where it is kept is the sharing
          // note's sentence under the buttons, said once, at every width)
          main.innerHTML = backBtn('playlists', 'Collections') + `<div class="hero">${mosaic(d.summary.covers, d.summary.name)}<div><div class="eyebrow">Collection</div><h1>${esc(d.summary.name)}</h1><p class="sub">${countOf(songCount + localSongs.length, 'songs')}${cardCount ? ` · ${countOf(cardCount, 'links')}` : ''}${total ? ' · ' + mins(total) : ''}${libNote}</p>
              <div class="actions" style="margin-top:12px"><button class="btn primary" id="plplay">${icon('play')} Play</button><button class="btn" id="plshuffle">${icon('shuffle')} Shuffle</button><button class="btn hero-zone" id="plzone" aria-label="Play on…" title="Play on…">${icon('zone')}<span class="lbl"> Play on…</span></button>${sharingActions}<button aria-haspopup="menu" aria-expanded="false" class="btn ghost" id="plmore" title="More">${icon('more')}</button><button class="btn danger pldel" id="pldel">${icon('trash')} Delete</button></div><p class="muted small" role="status">${sharingNote}</p></div></div>`
            // (a collection whose songs are all on this node only lists them below: no Songs card
            // saying "This collection is empty" over them; it stays in the page, hidden, for its bindings)
            + `<section class="section" id="plsongs"${!songs.length && localSongs.length ? ' hidden' : ''}><div class="section-head"><h3>Songs</h3></div>${summaryBar}${pfNote}${songs.length ? songRows(songs, { numbers: 'index', rowClass: (t) => `${t.unavailable ? 'unavail' : ''}${filt && !t.pending && (t.unavailable ? 'none' : t.plays && t.plays.service) !== filt ? ' filtered' : ''}`, extra: (t, k) => `${sourceChip(t, k)}<span class="row-actions q-actions"><button class="icon-btn" data-pup="${k}" data-wd="up" title="Move up" aria-label="${esc(rowWords.up(t.title))}" ${k === 0 ? 'disabled' : ''}>${icon('chevron', 'ic up')}</button><button class="icon-btn" data-pdown="${k}" data-wd="down" title="Move down" aria-label="${esc(rowWords.down(t.title))}" ${k === songs.length - 1 ? 'disabled' : ''}>${icon('chevron')}</button><button class="icon-btn" data-rm="${k}" data-wd="unlist" title="Remove" aria-label="${esc(rowWords.unlist(t.title))}">${icon('close')}</button></span>` }) : emptyState('playlist', 'This collection is empty', mine ? 'Add songs from any ⋯ menu.' : 'Add songs from any ⋯ menu, or import a file.')}</section>`
            + (localSongs.length ? `<section class="section" id="pllocal"><div class="section-head"><h3>On this node only</h3><span class="muted small">${countOf(localSongs.length, 'songs')}</span></div><p class="muted small">Not identified yet, so they stay on this node and aren't synced. Pick which song each one is (the ? in its menu) and it joins the collection on your homeserver.</p>${songRows(localSongs, { numbers: 'index', extra: (t, k) => `<button class="icon-btn" data-lrm="${k}" title="Remove from this collection" aria-label="Remove ${esc(t.title)} from this collection">${icon('close')}</button>` })}</section>` : '')
            + `<section class="section"><div class="section-head"><h3>Description</h3><button class="link" id="pldesc">${d.playlist.description ? 'Edit' : 'Add one'}</button></div>${d.playlist.description ? `<p>${esc(d.playlist.description)}</p>` : '<p class="muted small">A few words about this collection, shown with it when you share it.</p>'}</section>`;
          $('[data-back]', main).onclick = () => goBack('playlists');
          // Phones hide a row's Move up / Move down / Remove buttons (no room next to the title):
          // the row's ⋯ menu offers them and presses the same buttons
          const rowActs = (k, row) => (row && $('.q-actions', row) && getComputedStyle($('.q-actions', row)).display === 'none' ? [['pup', 'Move up', 'upload'], ['pdown', 'Move down', 'download'], ['rm', 'Remove from collection', 'trash']].filter(([a]) => { const b = $(`[data-${a}]`, row); return b && !b.disabled; }).map(([a, label, ic]) => ({ label, icon: ic, run: () => $(`[data-${a}]`, row).click() })) : []);
          bindSongRows($('#plsongs'), songs, { rowExtra: rowActs });
          const pfr = $('#pfretry'); if (pfr) pfr.onclick = () => { state.plPf = null; render(); };
          // A change by place still waiting for a read made after it holds this list too (plWrite),
          // its bar too; the read after it ends the hold (before Select carries over, so its bar takes presses),
          // and the focus the hold took goes back to its place (heldFocus, below)
          let heldFocus = null;
          const plHoldNow = plHolds.get(plId);
          if (plHoldNow) {
            if (plRead < plHoldNow.after) plHoldShow();
            else { if (again || plHoldNow.retry === navGen) heldFocus = plHoldNow.retry != null ? plSettle(plHoldNow.focus) : plHoldNow.focus; plHolds.delete(plId); } // (a fresh open keeps its own place; Try again, on this visit, is in place. A focus handed on by the press keeps its kind in the read that ends its hold, not after Try again, a second press: plSettle)
          }
          // (after a change's hold this read ended: the read again takes its focus on)
          if (pfReadAgain && !plHolds.has(plId)) { holdForReread(plSettle(heldFocus) || focusWas); heldFocus = null; }
          // Playlist rows can also be removed together (highest index first, so positions hold)
          enableSelect($('#plsongs'), songs, { keys: d.playlist.items.map((it) => it.uri), held: () => plHeld(plId), remove: (idx) => plWrite(plId, async (signal) => { try { for (const k of idx) await api('DELETE', `/playlists/${d.summary.id}/items/${k}`, undefined, { signal }); toast(`Removed ${countOf(idx.length, 'songs')}`); } finally { state.plPf = null; } }) });
          if (localSongs.length) bindSongRows($('#pllocal'), localSongs, {});
          // (the last one there: to the Songs card's last song that shows, its ⋯)
          $$('[data-lrm]', main).forEach((b) => b.onclick = (e) => { e.stopPropagation(); const keys = d.playlist.items.map((it) => it.uri); return plWrite(plId, (signal) => api('DELETE', `/playlists/${d.summary.id}/local/${b.dataset.lrm}`, undefined, { signal }), document.activeElement === b ? plNextFocus('pllocal', localKeys, Number(b.dataset.lrm), 'button[data-lrm]') || plSongFocus('plsongs', keys, keys.length - 1, PL_MORE) : null); });
          // Cards (web pages, posts) play nowhere: Play and Shuffle play the songs around them
          const playable = (list) => list.filter((t) => !t.unavailable && !t.card);
          // In this browser only a library song or a stream plays: a row with neither (a Spotify
          // Connect pick, a song still being found) is left out and said, as the list's Play does,
          // never played alone in place of the rest (review of #258); a zone (Shuffle on a speaker) keeps them, as before
          const playAll = async (shuffle, anchorEl) => {
            // A listener's: what the library has of each item, albums as their songs, on any zone
            if (mine) {
              const { songs: list, out } = await listenerSongs(d);
              if (!list.length) return toast('None of its songs is in this library', 'error');
              if (out) toast(`${countOf(out, 'items')} not in this library; left out of the queue`);
              return playSongs(list, 0, 'replace', null, anchorEl, undefined, { shuffle });
            }
            const all = [...playable(songs), ...localSongs]; const list = isBrowser() ? all.filter((t) => t.track_id != null || t.uri) : all; const out = all.length - list.length;
            if (!list.length) return toast(out ? `${countOf(out, 'songs')} ${out === 1 ? 'plays' : 'play'} only elsewhere; nothing here plays ${hereIn()}` : 'Nothing here plays on your services', 'error');
            if (out) toast(`${countOf(out, 'songs')} left out of ${hereName().toLowerCase()}'s queue: ${out === 1 ? 'it plays' : 'they play'} only elsewhere`);
            return playSongs(list, 0, 'replace', null, anchorEl, undefined, { shuffle });
          };
          $('#plplay').onclick = (e) => (isBrowser() ? playAll(false, e.currentTarget) : playPlaylist(d.summary.id));
          stickyHead(d.summary.name, (el) => (isBrowser() ? playAll(false, el) : playPlaylist(d.summary.id)));
          $('#plshuffle').onclick = (e) => playAll(true, e.currentTarget);
          $('#plzone').onclick = (e) => showMenu(e.currentTarget, [{ title: 'Play on' }, ...zoneList().map((z) => ({ label: z.name, icon: 'zone', run: () => { state.zone = z.id; renderZoneName(); renderBar(); if (isBrowser()) playAll(false); else playPlaylist(d.summary.id); } }))]);
          // Name and description (second review: a playlist couldn't be renamed or described)
          const editField = async (key, label, value) => {
            const r = await choiceDialog({ title: key === 'name' ? 'Rename collection' : 'Collection description', field: { label, value: value || '' }, choices: [{ label: 'Save', value: 'save', primary: true }] });
            if (!r || (key === 'name' && !r.text)) return;
            try { await api('PUT', `/playlists/${d.summary.id}`, { [key]: r.text }); state.playlists = await api('GET', '/playlists'); toast(key === 'name' ? `Renamed to “${r.text}”` : 'Description saved'); render(); } catch (er) { toast(er.message, 'error'); }
          };
          $('#pldesc').onclick = () => editField('description', 'Description', d.playlist.description);
          $('#plmore').onclick = (e) => itemMenu(e.currentTarget, { title: d.summary.name, songs, path: `/playlist/${encodeURIComponent(d.summary.id)}`, dl: { kind: 'playlist', id: d.summary.id, online: songs.filter((t) => t.track_id == null && !t.card).length }, extra: [{ label: 'Rename…', icon: 'notes', own: true, run: () => editField('name', 'Name', d.summary.name) }, { label: d.playlist.description ? 'Edit description…' : 'Add a description…', icon: 'notes', own: true, run: () => editField('description', 'Description', d.playlist.description) },
            // Delete is beside Play only in a wide desktop window (CSS, so it follows a resize); else here
            ...($('#pldel') && $('#pldel').getClientRects().length ? [] : [{ label: 'Delete collection…', icon: 'trash', own: true, run: async () => { const ok = await choiceDialog({ title: 'Delete this collection?', text: `“${d.summary.name}” goes from this node and your homeserver. The songs stay where they are.`, choices: [{ label: 'Delete', value: true, primary: true, danger: true }] }); if (!ok) return; try { await api('DELETE', `/playlists/${d.summary.id}`); toast('Collection deleted'); setView('playlists'); } catch (er) { toast(er.message, 'error'); } } }])] });
          // (a held Enter arms it once: its repeats don't say Really delete? yes)
          if ($('#pldel')) $('#pldel').onkeydown = (e) => { if (e.repeat && (e.key === 'Enter' || e.key === ' ')) e.preventDefault(); };
          if ($('#pldel')) $('#pldel').onclick = async () => { if (!$('#pldel').dataset.armed) { $('#pldel').dataset.armed = '1'; $('#pldel').innerHTML = `${icon('trash')} Really delete?`; setTimeout(() => { if ($('#pldel')) { delete $('#pldel').dataset.armed; $('#pldel').innerHTML = `${icon('trash')} Delete`; } }, 3000); return; } try { await api('DELETE', `/playlists/${d.summary.id}`); toast('Collection deleted'); setView('playlists'); } catch (e) { toast(e.message, 'error'); } };
          // (Remove chosen from the keyboard in the row's ⋯ menu, as phones offer it: the menu gave the
          // focus back to that ⋯, so it goes on to the next song's ⋯, a card's if that one is a card)
          const viaMenu = (b) => { const fa = document.activeElement; const row = b.closest('[data-srow]'); return keyedLast && fa && row && row.contains(fa) && fa.matches('button[data-smore], button[data-cmore]'); };
          // (the Songs card's last song: on to the first song on this node only, its ⋯)
          $$('[data-rm]', main).forEach((b) => b.onclick = (e) => { e.stopPropagation(); const keys = d.playlist.items.map((it) => it.uri); const k = Number(b.dataset.rm); const hid = new Set($$('#plsongs [data-srow].filtered', main).map((row) => Number(row.dataset.srow))); const kb = document.activeElement === b; const menu = !kb && viaMenu(b);
            const onward = () => (localKeys.length ? plSongFocus('pllocal', localKeys, 0, PL_MORE) : null);
            const next = kb ? plNextFocus('plsongs', keys, k, 'button[data-rm]', undefined, hid) || onward() : menu ? plNextFocus('plsongs', keys, k, 'button[data-smore]', 'button[data-cmore]', hid) || onward() : null;
            return plWrite(plId, async (signal) => { await api('DELETE', `/playlists/${d.summary.id}/items/${b.dataset.rm}`, undefined, { signal }); state.plPf = null; }, next); });
          // A held Enter or Space acts once: the focus handed on to the next song's × or ⋯ (either
          // box) doesn't go on removing songs or open that song's menu (as Identify's Bring back all;
          // a menu or dialog it opens ignores the repeats itself)
          $$('#plsongs, #pllocal', main).forEach((box) => box.onkeydown = (e) => { if (e.repeat && (e.key === 'Enter' || e.key === ' ') && e.target.closest('button[data-rm], button[data-lrm], button[data-smore], button[data-cmore]')) e.preventDefault(); });
          // (pressed from the keyboard, the focus follows the song to its new place, so a second
          // press moves it on rather than moving the song that took its place back; not when the
          // focus went elsewhere while the move was saved; the list that ends the hold takes it there, heldFocus)
          const pmove = (i2, j2) => { const uris = d.playlist.items.map((it) => it.uri); if (j2 < 0 || j2 >= uris.length) return; const [u] = uris.splice(i2, 1); uris.splice(j2, 0, u); const from = document.activeElement; const had = from && from.matches('#plsongs [data-pup], #plsongs [data-pdown]') ? (j2 < i2 ? 'pup' : 'pdown') : null; const other = had === 'pup' ? 'pdown' : 'pup'; return plWrite(plId, async (signal) => { await api('PUT', `/playlists/${d.summary.id}`, { uris }, { signal }); state.plPf = null; }, had ? plSongFocus('plsongs', uris, j2, `button[data-${had}]`, `button[data-${other}]`) : null); };
          $$('[data-pup]', main).forEach((b) => b.onclick = (e) => { e.stopPropagation(); pmove(Number(b.dataset.pup), Number(b.dataset.pup) - 1); });
          $$('[data-pdown]', main).forEach((b) => b.onclick = (e) => { e.stopPropagation(); pmove(Number(b.dataset.pdown), Number(b.dataset.pdown) + 1); });
          // Source chip → every source this song has here, plus Auto; picking one pins it.
          const pin = (k, body, msg) => plWrite(plId, async (signal) => { await api('PUT', `/playlists/${d.summary.id}/items/${k}/source`, body, { signal }); state.plPf = null; toast(msg); });
          $$('[data-psrc]', main).forEach((b) => b.onclick = (e) => {
            e.stopPropagation(); const k = Number(b.dataset.psrc); const t = songs[k]; const pinnedNow = t.pinned || t.pinLost;
            const opts = rankSvc([...(t.sources || [])], (x) => x.service).sort((a, b) => (b.available ? 1 : 0) - (a.available ? 1 : 0)).map((x) => ({ label: x.name, note: x.available ? x.label.replace(/^[^·]*·\s*/, '') || null : x.mode === 'preview' ? '30 s previews only' : 'not connected', svc: x.service, on: t.pinned && x.chosen, disabled: !x.available, run: () => pin(k, { service: x.service, uri: x.url || null }, `Pinned to ${x.name}`) }));
            showMenu(b, [{ title: t.unavailable ? t.unavailable : 'Play this song from' }, { label: 'Auto (follow your Settings)', icon: 'shuffle', on: !pinnedNow, note: t.plays && !pinnedNow ? `now: ${srcName(t.plays.service)}` : null, run: () => pin(k, { service: null }, 'Follows your Settings again') }, ...opts,
              ...(t.connect && t.connect.length ? [{ label: `Connect ${t.connect.map((c) => SERVICE_NAMES[c] || c).join(' or ')}`, icon: 'plus', run: () => setView('sources', connectArg(t.connect)) }] : []),
              ...(t.id ? [{ label: 'Find another version', icon: 'disc', run: () => setView('item', { kind: 'song', id: t.id }) }] : [])]);
          });
          const setVis = async (pub) => { try { const r = await api('PUT', `/playlists/${d.summary.id}/visibility`, { public: pub }); toast(mine ? (pub ? 'Shared' : 'Private again: the public copy is removed') + (r.summary.synced ? '' : ' once your homeserver answers') : pub ? 'Shared: it goes public on the next sync' : 'Private again: the public copy is removed on the next sync'); render(); } catch (e) { toast(e.message, 'error'); } };
          // Going public asks first, and says what happens without a Pubky sign-in (second review)
          const sh = $('#plshare'); if (sh) sh.onclick = async () => {
            const who = mine ? sharer : await pubkyUser();
            // (no answer in 15 s: whether you're signed in isn't known, so it doesn't say you aren't)
            if (!mine && state.pubkyUser === undefined) { toast('Couldn’t check your Pubky sign-in: the node didn’t answer. Try again in a moment.', 'error'); return; }
            const go = await choiceDialog({ title: `Share “${d.summary.name}”?`, text: who
              ? (mine ? 'A public copy goes to your homeserver' : 'A public copy goes to your homeserver with the next sync (in a few minutes)') + ': the songs and their order, never audio. Anyone with the link can open it, and people on Pubky can find it. You can make it private again at any time.'
              : 'You aren\'t signed in to Pubky yet, so there is nowhere to share it from. Sign in with Pubky Ring first (Settings › My accounts); the collection stays on this node until then.',
              choices: who ? [{ label: 'Share', value: 'share', primary: true }] : [{ label: 'Open Accounts', value: 'accounts', primary: true }] });
            if (go === 'share') setVis(true); else if (go === 'accounts') setView('settings', 'accounts');
          };
          const us = $('#plunshare'); if (us) us.onclick = () => setVis(false);
          const cl = $('#plcopylink'); if (cl) cl.onclick = async () => { const t = `pubky://${sharer.replace(/^pubky/, '')}/pub/social/v1/posts/${d.summary.id}`; try { await navigator.clipboard.writeText(t); toast('Link copied'); } catch { toast(t); } };
          $$('[data-pfilter]', main).forEach((b) => b.onclick = () => { state.plFilter = b.dataset.pfilter ? { id: plId, key: b.dataset.pfilter } : null; render(); });
          state.drawnPage = 'playlist'; plShown = { id: plId, uris: d.playlist.items.map((it) => it.uri), local: localKeys }; if (place) place();
          // (after the page's own place: focus they moved elsewhere meanwhile stays there; a song's
          // control goes to that song's row, plFocusOf)
          // One rule wherever the focus is given back, by the draw that ends a change's hold or by one
          // the person didn't cause (a late answer for where the songs play, the source chip then hiding
          // the row): on the same song, the same control (one hidden at this width or disabled, a
          // phone's × or Move up at the top, gives way to `alt`, then to the song's ⋯). A song gone or
          // hidden passes it on to the next song that shows, else the shown one before, as × does
          // (plNextFocus), and there to its ⋯ (a card's), never its row, × or Move: a key already on its
          // way changes no song the person didn't pick. (A row without a ⋯ that takes it is passed over)
          const plFocusIn = (f) => {
            const can = (x) => !!x && !x.disabled && !x.closest('[inert]') && x.getClientRects().length > 0;
            const shown = (id) => $$(`#${id} [data-srow]`, main).filter((row) => row.getClientRects().length);
            // (no song to hand it on to found: never to the page. Its box still shows songs (the whole
            // list replaced at once): the song now at its old place, else the last, its ⋯. None shows
            // any more, another device removing the last song on this node only (the section goes) or
            // the Songs card's last one (it hides): to the other box, as × does, the Songs card's last
            // song that shows, or the first song on this node only, its ⋯)
            const across = () => {
              if (f.song == null) return null;
              let rows = shown(f.box); let row;
              if (rows.length) { const at = f.was ? plPlaceOf(f.was, f.song, f.copy) : -1; row = rows[Math.min(Math.max(at, 0), rows.length - 1)]; } else { rows = shown(f.box === 'pllocal' ? 'plsongs' : 'pllocal'); row = f.box === 'pllocal' ? rows[rows.length - 1] : rows[0]; }
              const m = row && $(PL_MORE, row); return can(m) ? { b: m } : null;
            };
            const box = $(`#${f.box}`, main); if (!box) return across();
            if (f.song == null) { let b = $$(f.sel, box)[f.i]; if (!can(b) && f.alt) b = $(f.alt, box); return can(b) ? { b } : null; }
            const keys = f.box === 'pllocal' ? localKeys : d.playlist.items.map((it) => it.uri);
            let j = plPlaceOf(keys, f.song, f.copy); let gone = false;
            // (the song itself gone, another device removing it: on to the song after it in the list it
            // was in that is still here, else the one before, its ⋯)
            if (j < 0 && f.was) {
              const w = f.was; const at = plPlaceOf(w, f.song, f.copy);
              const near = at < 0 ? [] : [...w.keys()].filter((i) => i > at).concat([...w.keys()].filter((i) => i < at).reverse());
              for (const i of near) { const n = plPlaceOf(keys, w[i], w.slice(0, i).filter((u) => u === w[i]).length); if (n >= 0) { j = n; gone = true; break; } }
            }
            if (j < 0) return across();
            const order = [...keys.keys()].filter((i) => i >= j).concat([...keys.keys()].filter((i) => i < j).reverse());
            for (const i of order) {
              const row = $(`[data-srow="${i}"]`, box); if (!row || !row.getClientRects().length) continue;
              if (i !== j || gone) { const m = $(PL_MORE, row); if (can(m)) return { b: m }; continue; }
              let b = row.matches(f.kind) ? row : $(f.kind, row); if (b === row && row.matches(UNTAGGED)) keyable(row);
              if (!can(b) && f.alt) b = $(f.alt, row); if (!can(b)) b = $(PL_MORE, row);
              if (can(b)) return { b };
            }
            return across();
          };
          const nowhere = () => { const now = document.activeElement; return !now || now === document.body || now === main; };
          const give = heldFocus || (rowWas && rowWas.song != null ? rowWas : null);
          // (passed on to another song, or its song moved far up or down the list by another device, it
          // is scrolled into sight when the row it was on was in sight: preventScroll keeps the place
          // otherwise. Not a control of the list itself, a chip: it was never followed)
          if (give && nowhere()) { const to = plFocusIn(give); if (to) { to.b.focus({ preventScroll: true }); if (give.song != null && (heldFocus ? heldSeen : rowSeen) && !inSight(to.b)) to.b.scrollIntoView({ block: 'nearest', behavior: 'instant' }); } }
          break;
        }

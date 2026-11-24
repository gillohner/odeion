        case 'queue': {
          state.queueKey = null; // (set once the page is drawn)
          const own = await refreshQueue(false, true);
          if (state.view !== 'queue') return;
          // The page's own fetch overtaken by a newer one (a zone event just after the page was drawn
          // again for another device): the songs in hand are then another zone's, and were drawn under
          // this one's name, a row's Play playing this zone's song at that place and Remove sending the
          // other queue's ids here. The page waits for the newer answer, which draws it: the Queue page
          // on screen stays as it was (its taps are for the zone it shows), another page gives way to the
          // head and a loader, still loading for the navigation. (asked again if no newer fetch was:
          // the zone changed meanwhile)
          if (!own && !isBrowser() && state.queueZone !== state.zone) {
            if (!state.qPage || state.qPage.parentElement !== main) { const zw = currentZone(); main.innerHTML = pageHead('Queue', zw ? zw.name : '') + skelRows(4); state.qPage = null; }
            if (own === null) refreshQueue();
            return;
          }
          const z = currentZone(); const q = isBrowser() ? state.browserQueue : state.queue;
          // The keyboard keeps its place across the page's redraw (an edit, Radio, another device's
          // change): a head button keeps the focus, a row's control stays with its song, on the row the
          // song is on now (after Remove, the row in its place). Before, only Shuffle kept it.
          const QKEYS = ['qup', 'qdown', 'qrm', 'qgrip', 'qi', 'qmore'];
          const fa = document.activeElement; const fk = fa && main.contains(fa) && fa.dataset ? QKEYS.find((k) => fa.dataset[k] != null) : null;
          // (the phone's ⋯ menu has closed by now, the focus with it: it goes back to the song's ⋯;
          // not once another page has been opened, nor out of a menu open now)
          const moved = state.qMoved && state.qMoved.gen === navGen ? state.qMoved : null; state.qMoved = null;
          const place = fk ? { k: fk, i: Number(fa.dataset[fk]) } : fa && main.contains(fa) && /^q(radio|shuffle|transfer|save|clear|mine|menu)$/.test(fa.id) ? { id: fa.id }
            : fa && main.contains(fa) && fa.matches('.page-head h2') ? { head: true }
            : moved && (!fa || fa === document.body || (menuEl.hidden && menuEl.contains(fa))) ? { k: moved.k, i: moved.i } : null;
          // (a song is its queue entry on a node zone, the item itself in this browser's queue; one not
          // in the queue any more, a removed one, leaves the focus on the row in its place)
          const qkey = (it) => (it && !isBrowser() && it.id != null ? it.id : it);
          if (place && place.k) { const was = (state.qDrawn || [])[place.i]; const at = was ? q.findIndex((it) => qkey(it) === qkey(was)) : -1; if (at >= 0) place.i = at; }
          state.qDrawn = q.slice();
          // The page's buttons and rows are for the zone it shows. The zone can change before the page
          // is drawn again (following the host to another device redraws only once its queue is in,
          // the zone gone, the Android app's USB DAC plugged in): a tap then sends nothing (it would
          // have cleared, saved, moved or played another device's queue), says why and draws the page.
          // (one redraw per page: a second tap while the new zone's queue is on its way would start a
          // second draw, and the first, its answer dropped, drew the old songs under the new name)
          const drawnZone = state.zone; let redrawn = false;
          const zoneMoved = () => {
            if (state.zone === drawnZone) return false;
            toast(`Nothing changed: the Queue now shows ${state.zone === BROWSER ? hereName().toLowerCase() : (state.zones[state.zone] || {}).name || 'another device'}`);
            if (!redrawn) { redrawn = true; render(); }
            return true;
          };
          const forDrawn = (f) => (...a) => (zoneMoved() ? undefined : f(...a));
          main.innerHTML = pageHead('Queue', z ? z.name : '', (isBrowser() ? '' : `<button class="btn ${z && z.radio ? 'primary' : ''}" id="qradio" title="When the queue ends, keep playing similar music from the library">${icon('radio')} Radio ${z && z.radio ? 'on' : 'off'}</button>`) + (q.length > 1 ? `<button class="btn" id="qshuffle" data-repeat aria-pressed="false">${icon('shuffle')} Shuffle</button>` : '') + (q.length && zoneList().filter((o) => o.id !== state.zone).length ? `<button class="btn" id="qtransfer" title="Move this queue, the song where it is, repeat and radio to another device">${icon('zone')} Move to…</button>` : '') + (q.length && !isListener() ? `<button class="btn" id="qsave" title="A new private collection from this queue">${icon('playlist')} Save as collection</button>` : '') + (q.length && !isBrowser() ? `<button class="btn" id="qclear">${icon('trash')} Clear</button>` : '') + (q.length && (zoneList().some((o) => o.id !== state.zone) || !isListener() || !isBrowser()) ? `<button aria-haspopup="menu" aria-expanded="false" class="btn ghost" id="qmenu" title="More" aria-label="More for this queue">${icon('more')}</button>` : ''))
            + (q.length ? `<div class="list queue">${q.map((it, i) => `<div class="list-row ${z && z.queue_index === i ? 'cur' : ''}" data-qrow="${i}"${z && z.queue_index === i ? ' aria-current="true"' : ''}><span class="qgrip" data-qgrip="${i}" role="button" tabindex="0" aria-label="${esc(rowWords.drag(itemName(it)))}" title="Drag to reorder">⋮⋮</span><button class="icon-btn" data-qi="${i}" title="Play from here" aria-label="${esc(rowWords.play(itemName(it)))}">${z && z.queue_index === i ? icon('play') : `<span class="num">${i + 1}</span>`}</button><div class="grow"><div class="name">${esc(itemName(it))}</div><div class="sub">${esc([it.artist, it.album].filter(Boolean).join(' · '))}</div></div><span class="muted small">${it.duration_ms ? fmt(it.duration_ms) : ''}</span><span class="row-actions q-actions"><button class="icon-btn" data-qup="${i}" title="Move up" aria-label="${esc(rowWords.up(itemName(it)))}" ${i === 0 ? 'disabled' : ''}>${icon('chevron', 'ic up')}</button><button class="icon-btn" data-qdown="${i}" title="Move down" aria-label="${esc(rowWords.down(itemName(it)))}" ${i === q.length - 1 ? 'disabled' : ''}>${icon('chevron')}</button><button class="icon-btn" data-qrm="${i}" title="Remove" aria-label="${esc(rowWords.unqueue(itemName(it)))}">${icon('close')}</button></span><button aria-haspopup="menu" aria-expanded="false" class="icon-btn qmore" data-qmore="${i}" title="More" aria-label="More for ${esc(itemName(it))}">${icon('more')}</button></div>`).join('')}</div>` : !isBrowser() && state.queueError && state.queueZone !== state.zone ? failedHtml({ message: `Couldn’t read the queue: ${state.queueError.message}` }) /* (not known to be empty) */ : emptyState('queue', 'The queue is empty', 'Play an album, or add a song from its ⋯ menu.', canShuffleMine() && queueEmpty(state.zone) ? { id: 'qmine', label: 'Shuffle my music' } : null, true));
          $$('[data-qi]', main).forEach((b) => b.onclick = forDrawn(() => transport({ action: 'play', index: Number(b.dataset.qi) })));
          // An empty queue offers the library shuffled here too, on the zone the page is for (before,
          // only words: the bar's Play said the queue was empty, and its note had the button). Only
          // when the zone is known to be empty, as for the note: a node zone whose queue could not be
          // fetched, or that is paused on a song, draws no rows but still has music to lose.
          const qm = $('#qmine', main); if (qm) qm.onclick = forDrawn(async () => { try { await shuffleIfEmpty(drawnZone); } catch (e) { toast(e.message, 'error'); } });
          // (entry: the song's queue entry, when i was looked up from it just now: a menu's edit)
          const qedit = async (i, op, to, k, entry) => {
            if (zoneMoved()) return;
            // (from the ⋯ menu, whose closing takes the focus: the next draw puts it on the song's row)
            state.qMoved = k ? { k, i, gen: navGen } : null;
            if (isBrowser()) {
              const bq = state.browserQueue; const j = op === 'up' ? i - 1 : op === 'down' ? i + 1 : op === 'to' ? to : null;
              if (op === 'rm') { bq.splice(i, 1); if (state.browserIndex === i) browserPlay(Math.min(i, bq.length - 1)); else if (state.browserIndex > i) state.browserIndex--; }
              else if (j != null && j >= 0 && j < bq.length && j !== i) {
                const [it] = bq.splice(i, 1); bq.splice(j, 0, it);
                // The playing item keeps playing wherever it moves to.
                const c = state.browserIndex;
                if (c === i) state.browserIndex = j; else if (i < c && c <= j) state.browserIndex = c - 1; else if (j <= c && c < i) state.browserIndex = c + 1;
              }
              render(); return;
            }
            const it = entry || q[i]; if (!it) { state.qMoved = null; return; }
            try {
              // (in the zone's turn: a load on its way goes first, so it can't undo the edit afterwards)
              const zid = state.zone; const j = op === 'rm' ? null : op === 'to' ? to : op === 'up' ? i - 1 : i + 1; if (j === i) { state.qMoved = null; return; }
              await zoneOp(zid, (t) => op === 'rm' ? api('DELETE', `/zones/${encodeURIComponent(zid)}/queue/${it.id}`, undefined, { signal: t.signal })
                : api('PUT', `/zones/${encodeURIComponent(zid)}/queue/${it.id}`, { to_index: j }, { signal: t.signal }));
              refreshQueue();
            } catch (e) { state.qMoved = null; toast(e.message, 'error'); }
          };
          $$('[data-qup]', main).forEach((b) => b.onclick = () => qedit(Number(b.dataset.qup), 'up'));
          // Drag to reorder: pointer events, so mouse, touch and pen all work. The row follows the pointer,
          // a line shows where it lands, and the list scrolls near its edges.
          $$('[data-qgrip]', main).forEach((g) => {
            g.onkeydown = (e) => { const i = Number(g.dataset.qgrip); if (e.altKey && e.key === 'ArrowUp' && i > 0) { e.preventDefault(); qedit(i, 'to', i - 1); } if (e.altKey && e.key === 'ArrowDown' && i < q.length - 1) { e.preventDefault(); qedit(i, 'to', i + 1); } };
            g.onpointerdown = (e) => {
              if (e.button !== 0) return;
              e.preventDefault();
              const from = Number(g.dataset.qgrip);
              const rows = $$('.list.queue .list-row', main); const row = rows[from]; if (!row) return;
              g.setPointerCapture(e.pointerId);
              const startY = e.clientY; const startScroll = main.scrollTop; let to = from; let lastY = e.clientY; let raf = null;
              const rest = rows.map((r) => { const b = r.getBoundingClientRect(); return b.top + b.height / 2; });
              row.classList.add('qdragging'); document.body.classList.add('qdrag-active');
              const place = () => {
                const dy = lastY - startY + (main.scrollTop - startScroll);
                row.style.transform = `translateY(${dy}px)`;
                const mid = row.getBoundingClientRect().top + row.offsetHeight / 2;
                to = from;
                rows.forEach((r, k) => { if (k === from) return; const c = rest[k] - (main.scrollTop - startScroll); if (k < from && mid < c) to = Math.min(to, k); if (k > from && mid > c) to = Math.max(to, k); });
                // The rows in between slide aside, so the gap shows where the row lands.
                const h = row.offsetHeight;
                rows.forEach((r, k) => { if (k === from) return; const shift = to > from && k > from && k <= to ? -h : to < from && k >= to && k < from ? h : 0; r.style.transform = shift ? `translateY(${shift}px)` : ''; });
              };
              const scrollLoop = () => { const b = main.getBoundingClientRect(); const edge = 48; if (lastY < b.top + edge) main.scrollTop -= 10; else if (lastY > b.bottom - edge) main.scrollTop += 10; place(); raf = requestAnimationFrame(scrollLoop); };
              raf = requestAnimationFrame(scrollLoop);
              g.onpointermove = (ev) => { lastY = ev.clientY; place(); };
              const end = (commit) => {
                cancelAnimationFrame(raf); g.onpointermove = null; g.onpointerup = null; g.onpointercancel = null;
                row.classList.remove('qdragging'); row.style.transform = ''; document.body.classList.remove('qdrag-active');
                rows.forEach((r) => { r.style.transform = ''; });
                if (commit && to !== from) qedit(from, 'to', to);
              };
              g.onpointerup = () => end(true);
              g.onpointercancel = () => end(false);
            };
          });
          $$('[data-qdown]', main).forEach((b) => b.onclick = () => qedit(Number(b.dataset.qdown), 'down'));
          $$('[data-qrm]', main).forEach((b) => b.onclick = () => qedit(Number(b.dataset.qrm), 'rm'));
          // A row's ⋯ is the song's own menu, as on Songs (Add to playlist…, favourites, Go to album…):
          // a library song's from the node, another song's Go to song page. Phones hide a row's Move
          // up / Move down / Remove (no room beside the title, and no hover to show them), so there
          // the ⋯ offers them first, as a playlist's song row does; so does a row with nothing else.
          // A tap on another ⋯ while one still reads: only the last one's menu opens (before, the
          // slower one's opened over it, its Move/Remove where the other's were), and the first ⋯
          // stops showing its progress. (the count goes on across the page's redraws: a tap on the
          // redrawn page's ⋯ wins over one still reading on the page before)
          $$('[data-qmore]', main).forEach((b) => b.onclick = (e) => {
            e.stopPropagation(); if (zoneMoved()) return; const i = Number(b.dataset.qmore); const it = q[i]; if (!it) return;
            const tap = state.qMenuTap = (state.qMenuTap || 0) + 1; if (state.qMenuDrop) state.qMenuDrop(); state.qMenuDrop = null; let shown = false; const gen = navGen;
            // The page drawn again for the same zone meanwhile (an edit, another device's change, the
            // next song), here or during the menu's own reads: the menu opens on the new row's ⋯ for the
            // same song (the row at its old place if it still has that song, else where it is now), its
            // Move up / Move down for that place. Before, it was dropped without a word. The song gone
            // from the queue: none opens, and a toast says so. Another page opened (the Queue again too)
            // or another zone: none opens. Asked once,
            // just before a menu would show: after that no other path shows one. (gives the ⋯ to open
            // at, or null)
            const live = () => {
              if (shown) return null; shown = true;
              if (tap !== state.qMenuTap) return null;
              // (another page opened, the Queue again too, first: its rows stay in the page until it
              // is drawn, and a late answer opened the menu over the page loading)
              if (state.view !== 'queue' || navGen !== gen) return null;
              if (b.isConnected) return zoneMoved() ? null : b;
              if (state.zone !== drawnZone) return null;
              const cur = state.qDrawn || []; const k = qkey(it);
              const j = cur[i] !== undefined && qkey(cur[i]) === k ? i : cur.findIndex((x) => qkey(x) === k);
              if (j < 0) { toast(`${itemName(it)} isn’t in the queue any more`); return null; }
              const nb = $(`[data-qmore="${j}"]`, main);
              if (!nb) return null;
              const row = nb.closest('.list-row'); const a = row && $('.q-actions', row);
              head.splice(1, head.length - 1, ...(a && getComputedStyle(a).display !== 'none' ? [] : editsAt(j, cur.length)));
              return nb;
            };
            // An edit is for the song the menu was opened for, at its place when the item is picked (the
            // queue can change while the menu is open: another song at the old place)
            const edit = (op) => () => {
              if (zoneMoved()) return;
              const cur = (isBrowser() ? state.browserQueue : state.queue) || [];
              const at = cur.findIndex((x) => qkey(x) === qkey(it));
              if (at < 0) return toast('Nothing changed: that song isn’t in the queue any more');
              if ((op === 'up' && at === 0) || (op === 'down' && at === cur.length - 1)) return toast(`Nothing changed: it’s ${op === 'up' ? 'first' : 'last'} in the queue now`);
              return qedit(at, op, null, 'qmore', isBrowser() ? null : cur[at]);
            };
            const acts = $('.q-actions', b.closest('.list-row'));
            const editsAt = (at, n) => [...(at > 0 ? [{ label: 'Move up', icon: 'upload', run: edit('up') }] : []), ...(at < n - 1 ? [{ label: 'Move down', icon: 'download', run: edit('down') }] : []), { label: 'Remove from queue', icon: 'trash', run: edit('rm') }];
            const head = [{ title: itemName(it) }, ...(acts && getComputedStyle(acts).display !== 'none' ? [] : editsAt(i, q.length))];
            const page = npPageId(it.uri);
            const plain = () => { const at = live(); if (at) showMenu(at, [...head, ...(page ? [{ label: 'Go to song page', icon: 'compass', run: forDrawn(() => setView('item', { kind: 'song', id: page })) }] : head.length > 1 ? [] : editsAt(Number(at.dataset.qmore), (at === b ? q : state.qDrawn || q).length))]); };
            const tid = it.track_id ?? trackIdOf(it);
            // (the song's own items too are for the zone the page shows: the zone can change while the
            // menu is open, and Play next, Add to queue or Start radio then do nothing, not add to or
            // replace the other zone's queue)
            const full = (async () => {
              const t = tid != null ? await api('GET', `/library/tracks/${tid}`).catch(() => null) : null;
              if (t && !shown && tap === state.qMenuTap) await trackMenu(b, [t], { head, live, guard: forDrawn });
            })().catch(() => {}).then(plain);
            // (the ⋯ shows its progress until its menu opens, or another ⋯ is tapped. It waits at most
            // 1.5 s for the song's own items: then the menu opens without them, as on a node that
            // doesn't answer, with Go to song page, and on phones the row's Move/Remove)
            return Promise.race([full, new Promise((r) => { state.qMenuDrop = r; }), new Promise((r) => setTimeout(r, 1500)).then(plain)]);
          });
          const qr = $('#qradio'); if (qr) qr.onclick = async () => {
            if (zoneMoved()) return;
            // (in the zone's turn, like a load, which carries the radio as it is when it goes)
            const zid = drawnZone; const on = !(z && z.radio);
            try { await zoneOp(zid, async (t) => { await api('PUT', `/zones/${encodeURIComponent(zid)}/radio`, { on }, { signal: t.signal }); const zs = await api('GET', `/zones/${encodeURIComponent(zid)}`, undefined, { signal: t.signal }); if (zs && zs.id) state.zones[zs.id] = zs; }); toast(on ? 'Radio on: similar music follows the queue' : 'Radio off'); render(); } catch (e) { toast(e.message, 'error'); }
          };
          // The same mode as the bar's Shuffle, with its state
          // (the toggle keeps the focus across the page's redraw)
          const qsh = $('#qshuffle'); if (qsh) { qsh.onclick = forDrawn(toggleShuffle); renderModes(); }
          if (place) {
            // (a control the row no longer offers, e.g. Move down on the last row, gives way to the row's others)
            const n = $$('[data-qrow]', main).length; const i = Math.min(place.i, n - 1);
            const alt = { qup: 'qdown', qdown: 'qup' }[place.k];
            const head = $('.page-head h2', main);
            const el = place.head ? head : place.id ? $('#' + place.id, main) : n ? [place.k, alt, 'qrm', 'qmore', 'qgrip'].filter(Boolean).map((k) => $(`[data-${k}="${i}"]`, main)).find((b) => b && !b.disabled && b.getClientRects().length) : null;
            // (Clear, or removing the last song, takes the button or row away: then the empty page's
            // Shuffle my music, Radio, or the heading, which then keeps the focus on the next draw)
            const to = el || $('#qmine', main) || $('#qradio', main) || head;
            if (to && to === head) head.tabIndex = -1;
            if (to) to.focus();
          }
          state.queueKey = queueKey(); state.qPage = main.firstElementChild;
          // (the queue could not be read: Try again asks again, the button busy meanwhile)
          { const fr = $('[data-failed] [data-retry]', main); if (fr) fr.onclick = (ev) => busy(fr, () => render(), ev); }
          const moveItems = () => zoneList().filter((o) => o.id !== drawnZone).map((o) => ({ label: o.name, icon: 'zone', note: deviceWords(o), run: forDrawn(() => moveTo(o.id)) }));
          // (for the zone the page was drawn for; one save at a time, a second tap while the first is
          // on its way made a second playlist, and one the node hasn't answered yet says so)
          const saveQueue = async (zid) => {
            if (queueSaving) { toast('Still saving the queue as a collection…', 'wait'); return; } queueSaving = true;
            const body = zid === BROWSER ? { uris: state.browserQueue.map((it) => it.uri), name: `Queue · ${hereName().toLowerCase()}` } : { zone: zid };
            try { const r = await api('POST', '/playlists/from-queue', body); state.playlists = await api('GET', '/playlists'); toast(r.skipped.length ? `Saved “${r.playlist.summary.name}”; ${countOf(r.skipped.length, 'songs')} left out: not matched to a song yet (press the ? next to a song to pick which one it is)` : `Saved “${r.playlist.summary.name}”`); } catch (e) { toast(e.message, 'error'); } finally { queueSaving = false; }
          };
          const clearQueue = async (zid) => { try { await zoneOp(zid, (t) => api('DELETE', `/zones/${encodeURIComponent(zid)}/queue`, undefined, { signal: t.signal })); refreshQueue(); } catch (e) { toast(e.message, 'error'); } };
          const qt = $('#qtransfer'); if (qt) qt.onclick = forDrawn(() => showMenu(qt, [{ title: 'Move to' }, ...moveItems()]));
          const qsv = $('#qsave'); if (qsv) qsv.onclick = forDrawn(() => saveQueue(drawnZone));
          const qc = $('#qclear'); if (qc) qc.onclick = forDrawn(() => clearQueue(drawnZone));
          // Phones: Radio and Shuffle stay on the head's one row; Save as playlist, Clear and Move to
          // (also in the Play to menu) are under its ⋯, which the desktop layout hides (app.css)
          // (an item gives the focus back to the ⋯ first, so the keyboard keeps its place: after Clear,
          // which takes the ⋯ away, the next draw puts it on Radio, as the Clear button did)
          // The menu stays open across a redraw: like the buttons, its items are for the zone the page
          // was drawn for and do nothing once the page is for another. While an item runs, the ⋯
          // doesn't open again.
          const qmn = $('#qmenu'); if (qmn) qmn.onclick = () => {
            if (BUSY.has(qmn) || zoneMoved()) return;
            const back = (f) => () => {
              if (qmn.isConnected) qmn.focus();
              return zoneMoved() ? undefined : f(drawnZone);
            };
            const own = [...(qsv ? [{ label: 'Save as collection', icon: 'playlist', run: back(saveQueue) }] : []), ...(qc ? [{ label: 'Clear queue', icon: 'trash', run: back(clearQueue) }] : [])];
            const moves = moveItems().map((it) => ({ ...it, run: back(it.run) }));
            // (the other devices gone since the page was drawn, with nothing else here: no ⋯ any more)
            if (!own.length && !moves.length) { render(); return; }
            showMenu(qmn, [...(own.length ? [{ title: 'Queue' }, ...own] : []), ...(moves.length ? [{ title: 'Move to' }, ...moves] : [])], { all: true });
          };
          break;
        }
        case 'zones': {
          const outs = isListener() ? [] : state.outputs || (await loadOutputs().catch(() => []));
          if (state.view !== 'zones') return;
          const capsOf = (z) => (outs.find((o) => o.id === z.output) || {}).caps || null;
          const zs = zoneList();
          // Two zones with one name get their kind after it; an output id is never the text (UX sweep A14)
          const named = (z) => zs.filter((y) => y.name === z.name).length > 1 ? `${esc(z.name)} <span class="muted small">· ${esc(PROTO_WORDS[String(z.output || z.id).split(':')[0]] || String(z.output || '').split(':')[0])}</span>` : esc(z.name);
          // One device seen two ways (the owner's Linn: AirPlay and UPnP) is one card with a chip per
          // way (owner's recording, 2026-11-06: "same device listed twice"). Zones are of one family
          // when their names agree once the way's words are taken out; the card shows the one of
          // them that is selected, else playing, else first.
          const protoOf = (z) => String(z.output || z.id).split(':')[0];
          const WAY_WORDS = /\b(airplay|upnp(?:[ -]?av)?|dlna|chromecast|cast|sonos|bluos|heos|snapcast|squeezelite|squeezebox|slimproto)\b/gi;
          // (the name without its way words, in any script; a name that is only a way word, or one
          // word of three letters or fewer, is too little to go on)
          const familyOf = (z) => { if (z.id === BROWSER || String(z.id).startsWith('group:')) return ''; const f = String(z.name || '').replace(WAY_WORDS, '').toLowerCase().replace(/[^\p{L}\p{N}]+/gu, ' ').trim(); return f.length >= 4 || /[^\x00-\x7f]/.test(f) ? f : ''; };
          const familyName = (z) => String(z.name || '').replace(WAY_WORDS, '').replace(/[\s:·(),-]+$/g, '').replace(/^[\s:·(),-]+/g, '').replace(/\(\s*\)/g, '').replace(/\s{2,}/g, ' ').trim() || z.name;
          const families = new Map();
          for (const z of zs) { const f = familyOf(z); if (f) families.set(f, [...(families.get(f) || []), z]); }
          const seen = new Set();
          const cards = zs.flatMap((z) => {
            const f = familyOf(z); const fam = f ? families.get(f) : null;
            // (two of them playing at once are two devices, not one seen two ways)
            if (!fam || fam.length < 2 || new Set(fam.map(protoOf)).size < 2 || fam.filter((m) => m.transport === 'playing').length > 1) return [{ z, ways: null }];
            if (seen.has(f)) return []; seen.add(f);
            const lead = fam.find((m) => m.id === state.zone) || fam.find((m) => m.transport === 'playing') || fam[0];
            return [{ z: lead, ways: fam }];
          });
          const phone = shellMode() === 'phone';
          main.innerHTML = pageHead('Zones', cards.length, `<button class="btn iconic" id="zgroup" aria-label="Group outputs">${icon('plus')}<span class="btn-label">Group outputs…</span></button>`) + `<div class="cards">${cards.map(({ z, ways }) => {
            const np = z.now_playing; const it = np && np.item; const caps = capsOf(z); const zv = zoneVolume(z); const hv = caps && caps.hw_volume && caps.hw_volume.kind === 'db' ? caps.hw_volume : null;
            const members = np && np.members && np.members.length ? `<span class="pill">${np.members.length} outputs</span>` : '';
            const title = ways ? esc(familyName(z)) : named(z);
            // (a stopped zone gets no state pill: its "Nothing playing" or its Play button says so)
            // A zone paused by an error (its USB DAC went away) says why where the artist is, as the mini player does
            // (not as an alert, nor a stopped zone's "Couldn’t play": the page is drawn again on every
            // play/pause here, and a screen reader would read it out each time; the mini player announces
            // the zone in use's)
            // On a phone the card's Play to (or "selected") sits on the now-playing line and the
            // volume note is for screen readers only, so the controls are one line under it
            const pick = z.id === state.zone ? '<span class="pill ok">selected</span>' : `<button class="btn sm" data-zsel="${esc(z.id)}" aria-label="Play to ${esc(z.name)}">Play to</button>`;
            const wayChips = ways ? `<div class="chips zc-ways" role="group" aria-label="Ways to play to ${esc(familyName(z))}">${ways.map((m) => `<button class="chip ${m.id === state.zone ? 'on' : ''}" data-zsel="${esc(m.id)}" aria-pressed="${m.id === state.zone}" title="Play to ${esc(m.name)}">${esc(PROTO_WORDS[protoOf(m)] || protoOf(m))}</button>`).join('')}</div>` : '';
            const vol = zv ? `<div class="zc-vol"><button class="icon-btn zc-mute" data-volmute="${esc(z.id)}" aria-pressed="${!!zv.muted}" title="${zv.muted ? 'Unmute' : 'Mute'}" aria-label="Mute ${esc(z.name)}">${icon('volume')}</button><input type="range" data-zvol="${esc(z.id)}" aria-label="Volume of ${esc(z.name)}"><span class="vol-level" data-vollevel="${esc(z.id)}"></span></div>` : `<span class="${phone ? 'sr-only' : 'muted small'}">Volume: set on the device</span>`;
            const ungroup = (String(z.id).startsWith('group:') || /^([a-z]+-)?group:/.test(String(z.output || ''))) && can('zone_group') ? `<button class="btn sm ghost" data-zone-del="${esc(z.id)}">Ungroup</button>` : ''; // (only who may change zones: for anyone else the node refuses it)
            // (a wider card's volume and its Play to go together: beside the transport when the slider
            // gets 80 px there, else under it as one, the slider then the line's width. Before, the
            // volume could stay beside the transport with a 56 px slider and Play to went under it
            // alone. A group's Ungroup, used rarely, comes after them, on a line of its own if need be)
            return `<div class="card zone-card ${z.id === state.zone ? 'on' : ''}" data-zcard="${esc(z.id)}">
              <div class="zc-head">${icon('zone')}<h3>${title}</h3>${members}${zoneStatePill(z)}</div>
              ${wayChips}
              <div class="zc-now">${it ? itemArt(it, 'sm', npSvc(np)) : `<div class="np-art placeholder zc-ph" aria-hidden="true">${icon('zone')}</div>`}<div class="zc-text"><div class="name">${it ? esc(np.stream_title || it.title || it.uri) : '<span class="muted">Nothing playing</span>'}</div><div class="sub">${it && z.transport === 'paused' && z.error ? `<span class="warn-text">${esc(z.error)}</span>` : it ? esc([it.artist, it.album].filter(Boolean).join(' · ')) : `<span title="${esc(z.output || '')}">${esc(deviceWords(z))}</span>`}</div>${!it && z.error && z.transport === 'stopped' ? `<div class="sub warn-text">Couldn’t play: ${esc(z.error)}</div>` : ''}</div>${phone ? pick : ''}</div>
              <div class="zc-controls">
                <button class="icon-btn" data-zt="previous" data-repeat title="Previous">${icon('prev')}</button>
                <button class="icon-btn big" data-zt="toggle" title="${z.transport === 'playing' ? 'Pause' : 'Play'}">${icon(z.transport === 'playing' ? 'pause' : 'play')}</button>
                <button class="icon-btn" data-zt="next" data-repeat title="Next">${icon('next')}</button>
                ${phone ? vol : `<div class="zc-tail">${vol}${pick}</div>`}${ungroup}
              </div></div>`;
          }).join('')}</div>`;
          const zoneTransport = async (zid, action) => {
            if (zid === BROWSER) { const keep = state.zone; state.zone = BROWSER; try { await transport({ action }); } finally { state.zone = keep; } return; }
            if (['toggle', 'next', 'previous'].includes(action) && zoneEmpty(state.zones[zid])) { emptyQueue(zid); return; }
            if (action === 'toggle') action = (state.zones[zid] || {}).transport === 'playing' ? 'pause' : 'play';
            try { const zs2 = await api('POST', `/zones/${encodeURIComponent(zid)}/transport`, { action }, { repeat: action === 'next' || action === 'previous' }); state.zones[zs2.id] = zs2; render(); } catch (e) { toast(e.message, 'error'); }
          };
          $$('[data-zcard]', main).forEach((card) => {
            const zid = card.dataset.zcard;
            $$('[data-zt]', card).forEach((b) => b.onclick = () => zoneTransport(zid, b.dataset.zt));
            // (its volume as in the bar's volume popover: Mute, the slider, the level)
            const v = $('[data-zvol]', card); if (v) { const zoneOf = () => (zid === BROWSER ? browserState() : state.zones[zid]); applyVol(v, zoneVolume(zoneOf())); bindVolSlider(v, zoneOf); syncVolumeSliders(zoneOf()); const mb = $('[data-volmute]', card); if (mb) mb.onclick = () => setZoneMute(zoneOf(), mb.getAttribute('aria-pressed') !== 'true'); }
          });
          $$('[data-zsel]', main).forEach((b) => b.onclick = () => { state.zone = b.dataset.zsel; localStorage.setItem('pm.zone', state.zone); renderZoneName(); renderBar(); refreshQueue(); render(); });
          $$('[data-zone-del]', main).forEach((b) => b.onclick = async () => { try { await api('DELETE', `/zones/${encodeURIComponent(b.dataset.zoneDel)}`); toast('Group removed'); if (state.zone === b.dataset.zoneDel) { ensureZone(); renderZoneName(); } render(); } catch (e) { toast(e.message, 'error'); } });
          $('#zgroup').onclick = () => setView('settings', 'playback');
          // Zones opened on a phone (a tab, a link, a reload; not Back or Forward): the zone in use
          // is on screen, not under the mini player (five zones put it a screen down). Once, when
          // its card is first drawn, and not after they touched or scrolled the page: a redraw,
          // a Play to, or a place they scrolled to stays where it is.
          const inUse = $('.zone-card.on', main);
          if (inUse && !redrawing('zones')) {
            state.drawnPage = 'zones';
            if (phone && state.zoneInView) {
              // (from where the card is in the page)
              const m = main.getBoundingClientRect(); const r = inUse.getBoundingClientRect(); const gap = 14;
              const down = Math.max(0, Math.min(r.bottom - m.top + main.scrollTop + gap - main.clientHeight, r.top - m.top + main.scrollTop - gap));
              try { main.scrollTo({ top: down, behavior: 'instant' }); } catch { main.scrollTop = down; }
            }
            state.zoneInView = false;
          }
          break;
        }
        case 'add-music': {
          // The old page: Your library with "Add music" open
          state.libAsk = true;
          return setView('library', null, 'replace');
        }
        case 'library': {
          const owner = isOwner();
          // (Add music's ask is for this drawing only: a page left before it draws forgets it)
          const askAdd = state.libAsk; state.libAsk = false;
          const tab = !owner ? 'mine' : state.viewArg === 'node' || state.viewArg === 'import' ? state.viewArg : 'mine';
          const tabsHtml = `<div class="chips libtabs" role="tablist" aria-label="Your library">${[['mine', 'My music'], ...(owner ? [['node', 'Node library'], ['import', 'Import']] : [])].map(([k, l]) => `<button class="chip ${tab === k ? 'on' : ''}" role="tab" aria-selected="${tab === k}" aria-controls="libpanel" id="libtab-${k}" data-libtab="${k}">${l}</button>`).join('')}</div>`;
          const head = pageHead('Your library', null, (appDl() ? `<button class="btn" data-dlgo>${icon('download')} Downloads</button>` : '') + (owner ? `<button class="btn primary" id="libadd">${icon('plus')} Add music</button>` : '')) + tabsHtml;
          if (tab === 'mine') {
            if (!owner) {
              main.innerHTML = head + libPanel(tab, '<div class="card"><p>Your own music lives on your Pubky homeserver: the Odeion app keeps it there and plays it here when this node lets you.</p><p class="muted small">This node’s songs, albums and collections are under Albums, Artists, Songs and Collections.</p></div>');
              break;
            }
            const [st, pk, ns] = await Promise.all([api('GET', '/storage').catch(() => null), api('GET', '/pubky').catch(() => null), api('GET', '/node/settings').catch(() => ({}))]);
            if (state.view !== 'library' || (state.viewArg && state.viewArg !== 'mine')) return;
            const hs = (st && st.homeserver) || { mode: 'none' };
            const sv = st && st.saving;
            const room = hs.quota_mb && hs.quota_mb !== 'unlimited' && hs.used_bytes != null ? ` of ${fmtBytes(hs.quota_mb * 1e6)}` : '';
            // Saving: a failed save offers Try again; otherwise Save now (a full sync, off while the node needs re-approval)
            const saveBtn = !sv || !pk || (pk.sync && pk.sync.running) || sv.state === 'saving' ? ''
              : sv.state === 'error' ? ' · <button class="lnk" id="stsave">Try again</button>'
                : ` · <button class="lnk" id="sync" ${pk.needs_resign ? `disabled title="${esc(pk.notice || RESIGN_TEXT)}"` : ''}>Save now</button>`;
            const status = pk && pk.user ? `${resignNotice(pk)}<div class="libstatus" role="status"><span id="libsaved">${savedWords(sv, pk)}</span>${saveBtn}<span class="muted" id="mmused"></span></div>` : '';
            const upOpen = state.libUpload || up.files.length > 0;
            const upload = `<section class="section card addm" id="libupload" ${upOpen ? '' : 'hidden'} aria-labelledby="libuph"><div class="section-head"><h3 id="libuph">${icon('upload')} Upload files</h3><button class="icon-btn" id="libupclose" aria-label="Close upload" title="Close">${icon('close')}</button></div>`
              + '<p class="muted small">To your homeserver, kept privately (never public), then added to your library.</p>'
              + (pk && pk.user ? `<div class="dropzone" id="updrop" tabindex="0" role="button" aria-label="Drop music files or folders here, or press Enter to choose files">${icon('upload')}<p><strong>Drop music files or whole folders here</strong></p><p class="muted small">FLAC, MP3, AAC, ALAC, Ogg, Opus, WAV, AIFF, DSD…</p><div class="row-btns"><button class="btn primary" id="uppick">Choose files</button><button class="btn" id="uppickdir">Choose a folder</button></div><input type="file" id="upfiles" multiple accept="audio/*,.flac,.dsf,.dff,.ape,.wv,.mka" hidden><input type="file" id="updir" webkitdirectory multiple hidden></div><div id="upbox"></div>` : '<p class="muted small">Sign in with Pubky first: the files are kept on your homeserver.</p>')
              + (hs.mode !== 'none' ? '<details class="set-adv"><summary>Or copy a folder from the node’s machine</summary><form class="toolbar" id="impform"><label class="sr-only" for="imppath">Folder to copy</label><input class="input" id="imppath" placeholder="/mnt/usb/Music" autocomplete="off" spellcheck="false"><button class="btn" id="impgo">Check</button></form><div id="impplan"></div></details>' : '')
              + '</section>';
            let body;
            if (!pk || !pk.user) body = '<div class="card"><p>Sign in with Pubky to keep your music, collections and bookmarks on your homeserver.</p><button class="btn primary" data-goto-accounts>Sign in</button></div>';
            else {
              body = '<div id="mmfiles" aria-busy="true"></div>'
                + sGroup('Copy from this node', sRow('Copy to your homeserver', 'Pick folders, an artist, an album or everything; files already there are skipped. It runs on the node in the background.', `<button class="btn" id="pkcopyall">${icon('upload')} Copy…</button>`))
                + sGroup('Tag my library', sBlock('', 'One public tag on every identified song, so the music index matches your songs across services (saved to your homeserver with your other changes).', '<div class="toolbar"><input class="input" id="bulklabel" value="library" aria-label="Tag label"><button class="btn" id="bulktag">Tag</button></div><p class="muted small" id="bulkout" aria-live="polite"></p>'))
                + (hs.mode !== 'bundled' ? `<details class="set-adv"><summary>Copy between this node and your homeserver</summary>${sRow('Bring files here', 'Files on your homeserver this node doesn’t have yet.', `<button class="btn ghost" id="pkmirror">${icon('download')} Bring them</button>`)}${sRow('Copy speed limit', 'Copies share your internet upload with this node\u2019s pages and the music it streams to you away from home. A limit leaves room for them; empty or 0 is no limit.', `<span class="toolbar"><input class="input rateinput" id="copyrate" type="number" min="0" step="100" inputmode="numeric" value="${ns.copy_rate ? Math.round(ns.copy_rate / 1000) : ''}" placeholder="No limit" aria-label="Copy speed limit in kB/s"><span class="muted small">kB/s</span></span>`)}</details>` : '');
            }
            main.innerHTML = head + libPanel(tab, status + upload + body);
            // Files: drawn after the page, paged (mineList)
            if (pk && pk.user) mineList($('#mmfiles'), { bundled: hs.mode === 'bundled', room });
            const sts = $('#stsave'); if (sts) sts.onclick = async () => { sts.disabled = true; try { await api('POST', '/storage/save'); toast('Saving to your homeserver'); } catch (er) { toast(er.message, 'error'); } render(); };
            const sy = $('#sync'); if (sy) sy.onclick = async () => { sy.disabled = true; try { syncToast(await api('POST', '/pubky/sync')); } catch (er) { toast(er.message, 'error'); } render(); };
            bindResign(main);
            if ((sv && sv.state === 'saving') || (pk && pk.sync && pk.sync.running)) { clearTimeout(state.savePoll); state.savePoll = setTimeout(() => { if (state.view === 'library' && (state.viewArg || 'mine') === 'mine') render(); }, 3000); }
            $$('[data-goto-accounts]', main).forEach((b) => b.onclick = () => setView('settings', 'accounts'));
            const cr = $('#copyrate'); if (cr) cr.onchange = async () => { const kb = Math.max(0, Math.round(Number(cr.value) || 0)); try { await api('PUT', '/node/settings', { copy_rate: kb * 1000 }); toast(kb ? `Copies at most ${kb} kB/s` : 'Copies without a speed limit'); } catch (e) { toast(e.message, 'error'); } };
        const bt = $('#bulktag'); if (bt) bt.onclick = async () => {
          const label = $('#bulklabel').value.trim(); if (!label) return; bt.disabled = true;
          try { const r = await api('POST', '/tags/bulk', { label }); $('#bulkout').textContent = `#${r.label}: ${countOf(r.tagged, 'songs')} tagged, ${fmtCount(r.already)} already had it, ${fmtCount(r.skipped_no_identity)} skipped (no MusicBrainz id or ISRC). Sync with Pubky to publish.`; }
          catch (e) { toast(e.message, 'error'); }
          bt.disabled = false;
        };
        const ca = $('#pkcopyall'); if (ca) ca.onclick = () => startUpload();
        const mi = $('#pkmirror'); if (mi) mi.onclick = async () => { mi.disabled = true; await bringFiles(); mi.disabled = false; };
            // Upload: the drop zone, choosing files or a folder, and copying a folder on the machine
            const showUpload = () => { const s2 = $('#libupload'); s2.hidden = false; s2.scrollIntoView({ block: 'start' }); const z = $('#updrop'); if (z) z.focus(); };
            $$('[data-libup]', main).forEach((b) => b.onclick = showUpload);
            $('#libupclose').onclick = () => { state.libUpload = false; $('#libupload').hidden = true; };
            const drop = $('#updrop');
            if (drop) {
              $('#uppick').onclick = (ev) => { ev.stopPropagation(); $('#upfiles').click(); };
              $('#uppickdir').onclick = (ev) => { ev.stopPropagation(); $('#updir').click(); };
              drop.onkeydown = (ev) => { if (ev.key === 'Enter' && ev.target === drop) $('#upfiles').click(); };
              $('#upfiles').onchange = (ev) => { addUploads(ev.target.files); ev.target.value = ''; };
              $('#updir').onchange = (ev) => { addUploads(ev.target.files); ev.target.value = ''; };
              drop.ondragover = (ev) => { ev.preventDefault(); drop.classList.add('over'); };
              drop.ondragleave = () => drop.classList.remove('over');
              drop.ondrop = async (ev) => { ev.preventDefault(); drop.classList.remove('over'); addUploads(await filesFromDrop(ev.dataTransfer)); };
              drawUploads();
            }
            const imp = $('#impform');
            if (imp) imp.onsubmit = async (ev) => {
              ev.preventDefault(); const path = $('#imppath').value.trim(); if (!path) return;
              const box = $('#impplan'); box.innerHTML = '<p class="small" role="status"><span class="spin"></span> Looking at the folder…</p>';
              let plan; try { plan = await api('POST', '/library/imports', { path, dry_run: true }); } catch (er) { box.innerHTML = `<p class="small joberr">${esc(er.message)}</p>`; return; }
              if (!plan.total) { box.innerHTML = '<p class="small">No music files in that folder.</p>'; return; }
              box.innerHTML = `<p class="small">${countOf(plan.total, 'music files')}, ${fmtBytes(plan.bytes_total)}. Files already on your homeserver are skipped while it runs.</p><div class="row-btns"><button class="btn primary" id="impstart">Copy them</button></div>`;
              $('#impstart').onclick = async () => { try { const j = await api('POST', '/library/imports', { path }); watchJob(j.id); box.innerHTML = '<p class="small">Copying. Follow it in the job chip by the player; you can leave this page.</p>'; } catch (er) { toast(er.message, 'error'); } };
            };
          } else if (tab === 'node') {
            const toCard = state.libToIdentify; state.libToIdentify = false; // (Open Identify's, for this draw only: a page left before it loads forgets it)
            const idGen = idRenderGen; // a card drawn since this ask has a fresher answer than it
            const [info, folders, shares, drv, ns, hidden, idSt] = await Promise.all([
              api('GET', '/info'), api('GET', '/library/folders').catch(() => []), api('GET', '/library/webdav').catch(() => []),
              api('GET', '/library/drives').catch(() => ({ providers: [], roots: [] })), api('GET', '/node/settings').catch(() => ({})), api('GET', '/library/hidden').catch(() => []),
              api('GET', '/library/identify').then((st) => ({ st, gen: idGen }), (err) => ({ err, gen: idGen })),
            ]);
            if (state.view !== 'library' || state.viewArg !== 'node') return;
            state.info = info;
            const lsc = info.last_scan;
            const songs = countOf(Number(info.library.tracks || 0).toLocaleString(), 'songs');
            const scanLine = info.scanning ? [`<span class="spin"></span> Scanning… ${songs} so far`]
              : [`${songs} in the library`, ...(lsc ? [`last scan ${ago(lsc.finished_at)}`, ...(lsc.error_count ? [`<button class="lnk" type="button" id="liberrs" aria-controls="scanerrs">${countOf(Number(lsc.error_count).toLocaleString(), 'files')} could not be read</button>`] : [])] : [])];
            main.innerHTML = head + libPanel(tab, `<div class="libstatus lsline" role="status"><span class="lsrow">${scanLine.map((t) => lsPart(t)).join('')}${unsureSlot(idUnsure(idSt.st))}</span></div><p class="muted small">Edit a song’s tags or take it out of the library from its ⋯ menu, an album from the album’s ⋯ menu. Edits stay on this node; files are never changed.</p>`
              + nodeLibraryHtml({ info, folders, ns, shares, drv })
              + (hidden.length ? sGroup('Taken out of the library', hidden.map((h, k) => sRow(`<code class="path">${esc(h.path.split('/').slice(-2).join('/'))}</code>`, `Taken out ${ago(h.hidden_at)}; the file is still there.`, `<button class="btn sm ghost" data-unhide="${k}">Bring back</button>`)).join('')) : ''));
            bindNodeLibrary(idSt);
            // The line's unreadable files: one press opens their list (in Music folders, often a screen
            // down) and goes to it, instead of saying the same count twice with no way between them
            const le = $('#liberrs'); if (le) le.onclick = () => { const d = $('#scanerrs'); const sm = d && $('summary', d); if (!sm) return; d.open = true; sm.scrollIntoView({ block: 'center', behavior: 'instant' }); sm.focus({ preventScroll: true }); }; // (its title centred: a long list stays below it)
            driveReturn(drv);
            if (toCard) showIdentifyCard();
            $$('[data-unhide]', main).forEach((b) => b.onclick = async () => { b.disabled = true; try { const back = hidden[Number(b.dataset.unhide)].path; const r = await api('POST', '/library/hidden/restore', { path: back }); broughtBack(back); toast(r.scan_started ? 'Back: reading it again' : 'Back with the next scan'); watchScan({ scanning: !!r.scan_started }); render(); } catch (er) { toast(er.message, 'error'); b.disabled = false; } });
            if (info.scanning) { clearTimeout(state.libPoll); state.libPoll = setTimeout(() => { if (state.view === 'library' && state.viewArg === 'node') render(); }, 3000); }
          } else {
            main.innerHTML = head + libPanel(tab, importHtml());
            bindImport();
          }
          $$('[data-libtab]', main).forEach((b) => b.onclick = () => { if (document.activeElement === b) state.libTabFocus = true; setView('library', b.dataset.libtab === 'mine' ? null : b.dataset.libtab, 'replace'); });
          $$('[data-view-go]', main).forEach((b) => b.onclick = () => setView(b.dataset.viewGo));
          const la = $('#libadd'); if (la) la.onclick = addMusicDialog;
          if (askAdd && owner) addMusicDialog();
          // Tabs: arrow keys move between them; the chosen tab keeps the keyboard after the page draws
          const tabBtns = $$('[data-libtab]', main);
          tabBtns.forEach((b, k) => { b.tabIndex = b.classList.contains('on') ? 0 : -1; b.onkeydown = (ev) => { if (ev.key !== 'ArrowRight' && ev.key !== 'ArrowLeft') return; ev.preventDefault(); const n = tabBtns[(k + (ev.key === 'ArrowRight' ? 1 : tabBtns.length - 1)) % tabBtns.length]; state.libTabFocus = true; n.click(); }; });
          if (state.libTabFocus) { state.libTabFocus = false; const on = $('[data-libtab].on', main); if (on) on.focus(); }
          break;
        }
        case 'downloads': {
          // What this device keeps (STEWARD-DOWNLOADS.md): offline, from the app's local server
          if (!isOffline() && appDl()) { await drawStorage(main); break; }
          if (!isOffline()) { main.innerHTML = pageHead('Downloads', null) + '<div id="dljob"></div><p class="muted">Songs kept for offline listening live in the Odeion app on your phone or laptop: keep an album or a collection there with its ⋯ menu, and it plays without the node.</p><p><a class="btn" href="https://github.com/gillohner/pubky-music-node/blob/main/docs/guides/desktop-app.md" target="_blank" rel="noopener">' + icon('external') + ' Get the app</a></p>'; drawJobs(); break; }
          let d; try { d = await api('GET', '/offline'); } catch (e) { main.innerHTML = pageHead('Downloads', null) + failedHtml(e); break; }
          if (state.view !== 'downloads') return;
          const byId = new Map(d.tracks.map((t) => [t.id, t]));
          const songsOf = (ids) => ids.map((i) => byId.get(i)).filter(Boolean);
          const facts = [d.synced_at ? `Last synced ${ago(d.synced_at)}` : '', fileSize(d.bytes), d.queued ? `${countOf(d.queued, 'plays and hearts')} to send when your node is back` : ''].filter(Boolean).join(' · ');
          const head = pageHead('Downloads', d.tracks.length || null) + `<p class="muted small dl-note" role="status">Offline: playing what this device keeps. ${esc(facts)}</p>`;
          if (!d.tracks.length) { main.innerHTML = head + '<div class="empty">' + icon('download') + '<h3>Nothing on this device yet</h3><p>When your node is back, keep albums and collections here to play them without it.</p></div>'; break; }
          // (its lists are render's own: the rows are bound to them after the switch)
          const albumOpen = d.albums.find((a) => a.id === state.dlAlbum);
          const albumTile = (a) => `<button class="tile" data-dlalbum="${a.id}" title="${esc(a.title)}">${art(a.id, a.title, '', 'library', true)}<div class="t">${esc(a.title)}</div><div class="a">${countOf(a.track_ids.length, 'songs')}</div>${coverFrom('library')}</button>`;
          let html = head;
          if (albumOpen) {
            lists.album = songsOf(albumOpen.track_ids);
            html += `<section class="section"><div class="section-head"><h3>${esc(albumOpen.title)}</h3><div class="actions"><button class="btn sm primary" data-dlplay="album">${icon('play')} Play</button><button class="btn sm ghost" data-dlclose>All downloads</button></div></div>${trackRows(lists.album, { showAlbum: false, numbers: 'track', key: 'album' })}</section>`;
          }
          if (d.albums.length) html += `<section class="section"><div class="section-head"><h3>Albums <span class="muted small">${d.albums.length}</span></h3></div><div class="grid">${d.albums.map(albumTile).join('')}</div></section>`;
          const pls = d.playlists.filter((p) => p.track_ids.length);
          if (pls.length) html += `<section class="section"><div class="section-head"><h3>Collections</h3></div><div class="list">${pls.map((p, k) => `<div class="list-row"><div class="grow"><div class="name">${esc(p.name || 'Collection')}</div><div class="sub">${p.track_ids.length === p.total ? countOf(p.total, 'songs') : `${fmtCount(p.track_ids.length)} of ${countOf(p.total, 'songs')} on this device`}</div></div><button class="btn sm" data-dlpl="${k}">${icon('play')} Play</button></div>`).join('')}</div></section>`;
          const favs = songsOf(d.favourites.track_ids || []);
          if (favs.length) { lists.fav = favs; html += `<section class="section"><div class="section-head"><h3>Bookmarks</h3><button class="btn sm ghost" data-dlplay="fav">${icon('play')} Play</button></div>${trackRows(favs, { key: 'fav' })}</section>`; }
          lists.all = [...d.tracks].sort((a, b) => String(a.title).localeCompare(String(b.title)));
          html += `<section class="section"><div class="section-head"><h3>All songs <span class="muted small">${d.tracks.length}</span></h3><button class="btn sm ghost" data-dlplay="all">${icon('shuffle')} Shuffle</button></div>${trackRows(lists.all, { key: 'all' })}</section>`;
          main.innerHTML = html;
          $$('[data-dlalbum]', main).forEach((b) => b.onclick = () => { state.dlAlbum = Number(b.dataset.dlalbum); render(); mainTo(0); });
          const dc = $('[data-dlclose]', main); if (dc) dc.onclick = () => { state.dlAlbum = null; render(); };
          $$('[data-dlplay]', main).forEach((b) => b.onclick = () => { const k = b.dataset.dlplay; const ts = lists[k]; return ts && ts.length ? playTracks(ts, true, 'end', { shuffle: k === 'all' }) : undefined; });
          $$('[data-dlpl]', main).forEach((b) => b.onclick = () => playTracks(songsOf(pls[Number(b.dataset.dlpl)].track_ids), true, 'end', { shuffle: false }));
          break;
        }

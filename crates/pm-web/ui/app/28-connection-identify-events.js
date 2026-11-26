        default:
          main.innerHTML = emptyState('compass', 'Unknown view', '');
      }
      bindTiles();
      $$('[data-artist]', main).forEach((el) => el.onclick = () => setView('artist', Number(el.dataset.artist)));
      $$('[data-back]', main).forEach((b) => b.onclick = () => goBack(b.dataset.back));
      $$('[data-empty-action]', main).forEach((b) => b.onclick = () => setView(b.dataset.emptyAction));
      bindResign(main);

      $$('[data-crumb-kind]', main).forEach((b) => b.onclick = () => (b.dataset.asBack ? $('.pagebar > [data-back]', main).click() : setView('item', { kind: b.dataset.crumbKind, id: b.dataset.crumbId })));
      bindTrackRows(main, lists);
      // A read that failed (failedHtml): Try again draws the page from the start, with its loader
      $$('[data-failed] [data-retry]', main).forEach((b) => { if (!b.onclick) b.onclick = (ev) => busy(b, () => { if (state.drawnPage === v) state.drawnPage = null; return render(); }, ev); });
    } catch (e) {
      if ((listMine && listStale(v, listMine)) || (drawStale && drawStale())) return; // (an older list's or playlist draw's error over a newer one)
      const heldPl = v === 'playlist' ? plHolds.get(state.viewArg) : null; // (its Try again draws it in place, on this visit only: a fresh open is a new navGen)
      if (state.drawnPage === v) state.drawnPage = null; // (Albums, Artists: Try again draws it from the start)
      main.innerHTML = failedHtml(e, 'navretry');
      $('#navretry', main).onclick = () => { if (heldPl && plHolds.get(heldPl.id) === heldPl) heldPl.retry = navGen; render(); };
    }
  };

  // ---- connection + events -------------------------------------------------
  // The connection to the node (docs/CONNECTION.md): one WebSocket on /v1/events that reconnects with
  // backoff and jitter, resyncs from the snapshot (seq, boot, library_rev) without a reload, pings for the
  // round-trip time, and holds changes made while it is down for a few seconds.
  const link = { ws: null, up: false, ever: false, attempt: 0, retry: null, retryAt: 0, tick: null, ping: null, pingId: 0, pingSent: 0, awaiting: false, missed: 0, rtt: null, pongs: false, loaded: false, seq: null, boot: null, libRev: null, downSince: 0, waiters: [] };
  const HOLD_MS = 8000;
  const nodeLabel = () => (state.info ? `${state.info.name} · ${countOf(state.info.library.tracks, 'songs')}` : 'Connected');
  // kind: 'ok' (connected) | 'wait' (reconnecting) | 'bad' (offline); true/false for ok/bad.
  const setConn = (kind, text) => {
    if (kind === true) kind = 'ok'; else if (kind === false) kind = 'bad';
    const c = $('#conn');
    c.className = 'node-status ' + kind;
    c.dataset.state = kind === 'ok' ? 'connected' : kind === 'wait' ? 'reconnecting' : 'offline';
    const ms = kind === 'ok' && link.rtt != null ? ` · ${link.rtt} ms` : '';
    $('.node-text', c).textContent = text + ms;
    c.title = kind === 'ok' ? (link.rtt != null ? `${text}. Round trip ${link.rtt} ms.` : text) : `${text}. Click to retry now.`;
    const bar = $('#connbar');
    if (bar) { bar.hidden = kind === 'ok'; bar.className = 'connbar ' + kind; $('.connbar-text', bar).textContent = text; }
  };
  const whenConnected = () => {
    if (!link.ever || link.up) return Promise.resolve();
    linkHint();
    return new Promise((resolve, reject) => {
      const w = { resolve, timer: setTimeout(() => { link.waiters = link.waiters.filter((x) => x !== w); reject(new Error('Not connected to the node, so nothing was changed. Try again when it is back.')); }, HOLD_MS) };
      link.waiters.push(w);
    });
  };
  const flushWaiters = () => { const ws = link.waiters; link.waiters = []; ws.forEach((w) => { clearTimeout(w.timer); w.resolve(); }); };
  // Someone is waiting on the node: retry now rather than at the end of a long backoff.
  const linkHint = () => { if (link.ever && !link.up && link.retry && link.retryAt - Date.now() > 1000) retryNow(); };
  const showDown = () => {
    clearInterval(link.tick);
    const paint = () => {
      if (link.up) { clearInterval(link.tick); return; }
      const secs = Math.max(0, Math.ceil((link.retryAt - Date.now()) / 1000));
      if (!navigator.onLine) setConn('bad', 'Offline: no network');
      else if (link.ever && Date.now() - link.downSince < 20000) setConn('wait', 'Reconnecting…');
      else setConn('bad', secs ? `Node offline · retrying in ${secs} s` : 'Node offline · retrying…');
    };
    paint(); link.tick = setInterval(paint, 1000);
  };
  const scheduleRetry = () => {
    clearTimeout(link.retry);
    // Short retries while it matters: someone waits on a held change, or the node has only just gone
    // (a restart is back within seconds); the long backoff after that spares a node that is really off
    const urgent = link.waiters.length > 0 || (link.downSince && Date.now() - link.downSince < 20000);
    const d = Math.min(urgent ? 1500 : 30000, 500 * 2 ** Math.min(link.attempt, 10));
    const wait = Math.round(d / 2 + Math.random() * d / 2);
    link.attempt = urgent ? Math.min(link.attempt + 1, 2) : link.attempt + 1; // after the fast phase: 2 → 4 → 8 → 16 → 30 s
    link.retryAt = Date.now() + wait;
    const conn = $('#conn'); if (conn) conn.dataset.retryAt = String(link.retryAt); // when the next retry is due (for tests)
    link.retry = setTimeout(() => { link.retry = null; clearRetryAt(); reconnect(); }, wait);
  };
  const goneDown = () => {
    if (signedOut()) return; // the Log in screen, not a dropped link
    link.up = false; clearInterval(link.ping); link.awaiting = false;
    if (!link.downSince) link.downSince = Date.now();
    scheduleRetry(); showDown();
  };
  // Drop the socket without waiting for its close handshake (a dead peer may never answer).
  const dropSocket = () => { const ws = link.ws; link.ws = null; if (ws) { ws.onclose = ws.onmessage = ws.onerror = null; try { ws.close(); } catch {} } };
  const clearRetryAt = () => { const c = $('#conn'); if (c) delete c.dataset.retryAt; };
  const retryNow = () => { clearTimeout(link.retry); link.retry = null; clearRetryAt(); link.attempt = 0; reconnect(); };
  let reconnectGen = 0; // only the latest attempt may report its result
  const reconnect = async () => {
    if (!link.loaded) return connect();
    const gen = ++reconnectGen;
    let info = null; try { info = await api('GET', '/info'); } catch { if (gen === reconnectGen && !link.up) goneDown(); return; }
    if (link.up) return; // a success from an older attempt still counts
    state.info = info;
    openEvents();
  };
  const sendPing = () => {
    const ws = link.ws; if (!ws || ws.readyState !== 1) return;
    if (link.awaiting && link.pongs && ++link.missed >= 2) { dropSocket(); goneDown(); return; }
    link.pingId++; link.pingSent = performance.now(); link.awaiting = true;
    ws.send(JSON.stringify({ type: 'ping', id: link.pingId }));
  };
  // Favourites' and the queue's empty states pick their button by whether the library has songs
  // (favWayOn: Open Songs or Add music, canShuffleMine: Shuffle my music); a new song count from
  // GET /info that turns "no songs" into "songs" or back draws the one on screen again
  const songsChanged = () => { const e = $('#main .empty[data-songs]'); if (e && ['favourites', 'queue'].includes(state.view) && e.dataset.songs !== String(hasSongs())) render(); };
  // The library changed. What was kept from it goes at once (libraryForget); the page on screen is
  // drawn again (libraryRedraw)
  const libraryForget = () => {
    forgetAlbums(); forgetLibrarySearches();
    // (a playlist change cut after 30 s: the node changed something, so the next read ends its hold, plHolds)
    plHolds.forEach((h) => { if (h.cut) { h.cut = false; h.after = Math.min(h.after, plDraws + 1); } });
  };
  const libraryRedraw = () => {
    // (the redraw below comes before the new song count, songsChanged)
    api('GET', '/info').then((i) => { state.info = i; watchScan(i); if (link.up) setConn('ok', nodeLabel()); songsChanged(); }).catch(() => {});
    if (['albums', 'tracks', 'artists', 'home', 'favourites', 'album', 'artist', 'playlists', 'playlist'].includes(state.view)) render();
  };
  const libraryChangedNow = () => { libraryForget(); libraryRedraw(); };
  // The node says so after every write to the library (an import: once per file). A hidden tab
  // draws nothing: once, when it is looked at again. On the library's long lists the first change
  // is drawn at once and those that follow within a second with one redraw at its end; a
  // playlist's or an album's own page answers each one (a playlist's rows wait for the read after it)
  const LIBRARY_LISTS = ['albums', 'tracks', 'artists', 'home', 'favourites'];
  // (by the page's own clock, which never goes back: a wall clock set back would hold every redraw)
  let libActed = -1000; let libLater = null; let libMissed = false; let libLastNav = 0;
  const onLibraryChanged = () => {
    libraryForget();
    if (document.visibilityState === 'hidden') { libMissed = true; return; }
    if (!LIBRARY_LISTS.includes(state.view)) { libraryRedraw(); return; }
    const wait = libActed + 1000 - performance.now();
    if (wait <= 0) { libActed = performance.now(); libraryRedraw(); return; }
    // (another page opened after the last change that waits: that page was drawn after it, nothing
    // is owed; a change that came once that page was open is still drawn)
    libLastNav = navGen;
    if (!libLater) libLater = setTimeout(() => { libLater = null; if (libLastNav !== navGen) return; if (document.visibilityState === 'hidden') { libMissed = true; return; } libActed = performance.now(); libraryRedraw(); }, wait);
  };
  document.addEventListener('visibilitychange', () => { if (document.visibilityState !== 'visible' || !libMissed) return; libMissed = false; clearTimeout(libLater); libLater = null; libActed = performance.now(); libraryRedraw(); });
  const applySnapshot = (ev) => {
    const first = link.boot == null;
    const restarted = !first && ev.boot != null && ev.boot !== link.boot;
    const libChanged = !first && (restarted || (ev.library_rev != null && ev.library_rev !== link.libRev));
    const downFor = link.downSince ? Date.now() - link.downSince : 0;
    link.seq = ev.seq ?? null; link.boot = ev.boot ?? null; link.libRev = ev.library_rev ?? null;
    Object.keys(state.zones || {}).forEach(countEvent); state.zones = Object.fromEntries(ev.zones.map((z) => [z.id, z])); ev.zones.forEach((z) => countEvent(z.id)); state.zonesSeen = true; ensureZone(); renderZoneName();
    const wasUp = link.up;
    link.up = true; link.ever = true; link.attempt = 0; link.downSince = 0; clearInterval(link.tick);
    setConn('ok', nodeLabel()); checkHealth(); mountHelp(); if (isOwner()) findJob(); if (state.browserIndex < 0 && state.browserQueue.length) restoreBrowserQueue(); if (isOwner()) api('GET', '/node/settings').then((ns) => { const was = state.devOutputs; state.devOutputs = !!ns.dev_outputs; if (was !== state.devOutputs && state.view === 'zones') render(); }).catch(() => {});
    if (!wasUp) { clearInterval(link.ping); link.missed = 0; link.awaiting = false; sendPing(); link.ping = setInterval(sendPing, 10000); }
    flushWaiters();
    if (!first) {
      refreshQueue(false);
      if (!overlay.hidden) renderNowPlaying();
      if (state.view === 'zones') render();
      if (libChanged) libraryChangedNow();
      if (downFor > 5000) toast(restarted ? 'Reconnected: the node restarted' : 'Reconnected');
    }
  };
  // Scan indicator: while the node says it is scanning, poll /info until it is done.
  let scanTimer = null;
  const showScanning = (on) => { $('#status').innerHTML = on ? `<span class="scanning" title="Scanning library…"><span class="spin"></span><span class="scan-text">Scanning library…</span></span>` : ''; };
  // ---- identify library (Settings card and the review view) ----
  let idTimer = null; let idRenderGen = 0; // an older card draw that answers last is dropped
  // A run's identified songs by method: " (9 by album, 3 by fingerprint)"
  const idHow = (by) => { const n = (k) => (by || {})[k] || 0; const parts = [[n('album_fit'), 'by album'], [n('fingerprint') + n('fingerprint+release'), 'by fingerprint'], [n('tag_match'), 'by title, artist and length']].filter(([x]) => x); return parts.length ? ` (${parts.map(([x, how]) => `${x} ${how}`).join(', ')})` : ''; };
  // "Bring back N skipped": asked (armed until `until`) and on its way (sending), kept outside the
  // card's markup so any redraw of the card keeps them (a poll's, a saved key's)
  const idUnskip = { armedUntil: 0, sending: false, lastKey: 0 };
  // The card's title: where the focus goes when a press removed its button (a key held down can't
  // press a title), and only when the focus was left on the page
  const focusCardTitle = () => { const blk = $('#idcard') && $('#idcard').closest('.set-block'); const h = blk && blk.querySelector('.set-name'); if (h && focusLeft()) { h.tabIndex = -1; h.focus(); } };
  // Confirm matches with nothing to confirm: "Open Identify" opens Node library with the Identify card
  // in view and its title focused (the card is far down the page; Settings has no Identify)
  const openIdentify = () => { state.libToIdentify = true; setView('library', 'node'); };
  // (the group with its IDENTIFY heading at the top, at once; .main's scroll-padding keeps it 8 px
  // clear of the top, or of the phone's sticky Library chips)
  const showIdentifyCard = () => { const grp = $('#idcard') && $('#idcard').closest('.set-group'); if (!grp || !focusLeft()) return; grp.scrollIntoView({ block: 'start', behavior: 'instant' }); focusCardTitle(); };
  // Node library's top line says the unsure songs too, as the card does (the card is far down the
  // page): one press to the review, no scroll. Drawn with the page from the same answer, so the line
  // doesn't grow after it shows, and a page redrawn by a scan's poll says nothing new
  const idUnsure = (st) => { if (!st || !st.counts) return 0; const running = st.phase ? st.phase === 'running' : !!(st.progress && st.progress.running); return running ? 0 : Number(st.counts.suggested || 0); };
  // Node library's top line comes in parts, each after a "·"; the line clips the dot of a part that
  // starts a row (.lsline), so a wrapped line never starts with "·" and a part moves down whole
  const lsPart = (html, attrs = '') => `<span class="lspart"${attrs}><span class="lsep" aria-hidden="true">· </span><span>${html}</span></span>`;
  const unsureSlot = (k) => k ? lsPart(`<button class="lnk" type="button" id="libreview">Review ${k.toLocaleString()} unsure</button>`, ` id="libunsure" data-n="${k}"`) : `<span id="libunsure" data-n="0"></span>`;
  // `pre`: the answer the page asked for with its other parts ({ st } or { err }), used unless a
  // card was drawn since it was asked
  const renderIdentify = async (pre) => {
    const box = $('#idbox'); if (!box) return;
    if (pre && pre.gen !== idRenderGen) pre = null;
    const g = ++idRenderGen;
    const setUnsure = (k) => {
      // Changed only when the count does, and a new count goes into the same button, so a redraw of
      // the card keeps the focus
      const lu = $('#libunsure'); const was = lu && lu.querySelector('#libreview');
      if (lu && lu.dataset.n !== String(k)) {
        if (was && k) { lu.dataset.n = String(k); was.textContent = `Review ${k.toLocaleString()} unsure`; } else { const had = was && document.activeElement === was; lu.outerHTML = unsureSlot(k); if (had) focusCardTitle(); }
      }
      const lb = $('#libreview'); if (lb) lb.onclick = () => setView('identify');
    };
    let st;
    try { if (pre && pre.err) throw pre.err; st = pre ? pre.st : await api('GET', '/library/identify'); } catch (e) { if (g === idRenderGen) { box.textContent = e.message; setUnsure(0); } return; }
    if (g !== idRenderGen || !box.isConnected) return;
    const c = st.counts; const p = st.progress;
    // One line says where identification stands (st.phase), before the numbers
    const n = (x) => Number(x || 0).toLocaleString(); const matches = (x) => `${n(x)} ${x === 1 ? 'match' : 'matches'}`;
    const ph = st.phase || (p.running ? 'running' : c.suggested ? 'needs_review' : c.unchecked ? 'new' : 'ready');
    const head = {
      running: `Working: ${n(p.done)} of ${n(p.total)}…`,
      needs_review: 'Finished',
      ready: c.unknown ? `Finished — nothing to review (${n(c.unknown)} not found)` : 'Finished — nothing to review',
      new: `${n(c.unchecked)} ${c.unchecked === 1 ? 'song' : 'songs'} not checked yet`,
      interrupted: `The last run was cut off when the node stopped${p.total ? ` (after ${n(p.done)} of ${n(p.total)})` : ''}. Identify carries on from there.`,
      failed: `It stopped after ${n(p.done)} of ${n(p.total)}: ${esc(p.stopped || 'an error')}. Identify carries on from there.`,
      empty: 'No songs yet: add music first.',
    }[ph] || '';
    // Review is optional (TASK-identify-less-review-1106): songs play and show as usual either way,
    // so the unsure ones are a secondary line, not a warning, and Identify stays the main button
    const reviewFirst = false;
    setUnsure(idUnsure(st));
    const unsure = ph === 'needs_review' && c.suggested ? `<p class="small muted idsub" id="idunsure">${n(c.suggested)} ${c.suggested === 1 ? 'song' : 'songs'} unsure (optional): songs play and show as usual, identified or not.</p>` : '';
    // Unsure songs whose files the node can't find now (a share offline, a folder moved): not counted above, as on Confirm matches
    const offline = c.suggested_missing ? `<p class="small muted" id="idmissing">${n(c.suggested_missing)} more unsure ${c.suggested_missing === 1 ? 'song’s file is' : 'songs’ files are'} out of reach now (a share offline or a folder moved); they wait until the files are back.</p>` : '';
    box.innerHTML = `<p class="idphase ${ph === 'failed' || ph === 'interrupted' ? 'warn-text' : ''}" id="idphase" data-phase="${esc(ph)}" role="status">${ph === 'running' ? '<span class="spin"></span> ' : ''}${head}</p>${unsure}${offline}
      ${(() => {
        // One bar, one segment per state (owner, 2026-11-04: half purple, half dark read as "half
        // done, half empty" while the rest was to confirm and not found): identified, to confirm,
        // not found, not checked; the counts under it with the segments' colours
        // (the songs to re-check are among the unsure ones: said with them, not counted twice)
        const parts = [['identified', c.identified, 'identified'], ['suggested', c.suggested, c.outdated ? `unsure (${n(c.outdated)} to re-check)` : 'unsure'], ['skipped', c.skipped, 'left as they are'], ['unknown', c.unknown, 'not found'], ['unchecked', c.unchecked, 'not checked yet']].map(([k, v, w]) => [k, Number(v || 0), w]);
        const said = parts.filter(([, v]) => v).map(([, v, w]) => `${n(v)} ${w}`).join(', ');
        const bar = `<div class="idbar" role="img" aria-label="${esc(c.total ? `${n(c.total)} ${c.total === 1 ? 'song' : 'songs'}: ${said}` : 'No songs yet')}">${parts.filter(([, v]) => v).map(([k, v]) => `<span class="idseg ${k}" style="flex-basis:${((v / Math.max(c.total, 1)) * 100).toFixed(3)}%"></span>`).join('')}</div>`;
        const legend = parts.filter(([, v]) => v).map(([k, v, w]) => `<span class="idkey"><span class="idswatch ${k}" aria-hidden="true"></span>${k === 'suggested' ? `<button class="lnk" type="button" data-view="identify" aria-label="Review ${n(v)} ${w}">${n(v)} ${w}</button>` : `${n(v)} ${w}`}</span>`).join(' ');
        return `${bar}<p class="small idlegend">${c.total ? `${n(c.total)} ${c.total === 1 ? 'song' : 'songs'}:` : ''}${legend ? ` <span class="idkeys">${legend}</span>` : ''}</p>`;
      })()}
      ${c.identified && c.by ? `<p class="small muted" id="idby">${[[c.by.tags, 'by their tags'], [c.by.album, 'by their folder as an album'], [c.by.fingerprint, 'by how they sound'], [c.by.title_artist, 'by their exact title, artist and length'], [c.by.auto, 'picked automatically <span class="pill" data-auto>auto</span>'], [c.by.you, 'by you']].filter(([n]) => n).map(([n, how]) => `${n} ${how}`).join(' · ')}</p>` : ''}
      ${!p.running && p.finished_at ? `<p class="small muted">Last run: ${fmtCount(p.identified)} identified${idHow(p.identified_by)}, ${fmtCount(p.suggested)} to confirm, ${fmtCount(p.unknown)} not found${p.errors ? `, ${fmtCount(p.errors)} errors` : ''}${(() => { const k = Object.values(p.settled || {}).reduce((a, b) => a + b, 0); return k ? `; ${fmtCount(k)} unsure ${k === 1 ? 'song' : 'songs'} picked automatically` : ''; })()}.</p>` : ''}
      ${(() => {
        // The AcoustID key lives here, with its state and a Check (owner, 2026-10-31): Settings
        // keeps no copy. Matching by how songs sound needs it; without it, title and artist only
        const ks = st.acoustid_state || (st.acoustid ? 'ok' : 'missing'); const where = '<a href="https://acoustid.org/new-application" target="_blank" rel="noopener">acoustid.org/new-application</a>';
        const pill = ks === 'ok' ? '<span class="pill ok">OK</span>' : ks === 'refused' ? '<span class="pill bad">Key refused</span>' : '<span class="pill">Key missing</span>';
        const words = ks === 'ok' ? `Matching by how songs sound uses ${st.acoustid_own ? 'your' : 'the node’s'} AcoustID key.`
          : ks === 'refused' ? `AcoustID refused the key (${esc(st.acoustid_detail || p.acoustid_refused || '')}), so songs are matched by title and artist only. Get a free one at ${where} and paste it here.`
          : `Songs are matched by their title and artist only. Matching by how they sound (AcoustID) finds more: get a free key at ${where} (register an application) and paste it here.`;
        return `<div class="set-row" id="acidrow"><div class="set-label"><div class="set-name">AcoustID key ${pill}</div><p class="set-help">${words}</p><form class="toolbar" id="acidform"><input class="input" name="key" id="acidkey" placeholder="${st.acoustid_own || ks === 'refused' ? 'A new key' : 'AcoustID application key'}" autocomplete="off" spellcheck="false" aria-label="AcoustID key"><button class="btn sm" type="button" id="acidcheck">Check</button><button class="btn sm primary" id="acidsave">Save key</button>${st.acoustid_own ? '<button class="btn ghost sm" type="button" id="acidclear">Remove</button>' : ''}<span class="small" id="acidout" role="status"></span></form></div></div>`;
      })()}
      <label class="row small" style="gap:8px; margin:8px 0"><input type="checkbox" id="idauto" ${st.auto_match !== false ? 'checked' : ''}> Match new songs automatically <span class="muted">(after each scan that adds or changes songs, once a week for songs not found, and unsure songs whose best match is clearly ahead settled; on by default)</span></label>
      <label class="row small" style="gap:8px; margin:8px 0"><input type="checkbox" id="idwrite" ${st.write_ids ? 'checked' : ''}> Write IDs into my files <span class="muted">(MusicBrainz recording and release IDs; each file's tags are backed up first; off by default)</span></label>
      <p class="small muted" id="idautohelp">Unsure songs whose best match is clearly ahead are matched automatically (marked auto). Odeion uses those matches for albums, Pubky tags and Pinakes, but writes their IDs into your files only after you confirm them: Wrong match? on the song.</p>
      <div class="actions">${reviewFirst ? `<button class="btn primary" data-view="identify" id="idreview">Review ${matches(c.suggested)}</button>` : ''}<button class="btn ${reviewFirst ? '' : 'primary'}" id="idrun" ${p.running || ph === 'empty' ? 'disabled' : ''}>${c.unchecked ? 'Identify' : 'Identify again'}</button>${c.suggested && !reviewFirst ? `<button class="btn" data-view="identify">Review ${n(c.suggested)} unsure</button>${p.running ? '' : '<button class="btn ghost" id="idskipall" title="Leave every unsure song as it is: it plays and shows as usual, and Identify again leaves it alone">Skip all unsure</button>'}` : ''}${c.skipped ? `<button class="btn ghost" id="idunskip" ${idUnskip.sending ? 'disabled' : ''}>${idUnskip.armedUntil > Date.now() ? `Bring back all ${n(c.skipped)}? Press again` : `Bring back ${n(c.skipped)} skipped`}</button>` : ''}${st.write_ids ? '<button class="btn" id="idwritenow" title="Write the IDs found so far into the files">Write now</button>' : ''}</div>`;
    $$('[data-view="identify"]', box).forEach((b) => b.onclick = () => setView('identify'));
    const acf = $('#acidform'); if (acf) {
      const saveKey = async (key) => { try { await api('PUT', '/library/identify/settings', { acoustid_key: key }); toast(key ? 'AcoustID key saved: the next Identify also listens to the songs' : 'AcoustID key removed'); renderIdentify(); } catch (e) { toast(e.message, 'error'); } };
      needsText($('#acidkey'), $('#acidsave'));
      acf.onsubmit = (e) => { e.preventDefault(); const k = $('#acidkey').value.trim(); return k ? saveKey(k) : undefined; };
      // Check: the key typed, else the one in use; nothing is saved
      const chk = $('#acidcheck'); const out = $('#acidout');
      chk.onclick = async () => { chk.disabled = true; out.textContent = 'Checking…'; out.classList.remove('warn-text'); const k = $('#acidkey').value.trim(); try { const r = await api('POST', '/library/identify/key/check', k ? { acoustid_key: k } : {}); out.textContent = r.detail; out.classList.toggle('warn-text', !r.ok); } catch (e) { out.textContent = e.message; out.classList.add('warn-text'); } chk.disabled = false; };
      const clr = $('#acidclear'); if (clr) clr.onclick = () => saveKey('');
    }
    const au = $('#idauto'); if (au) au.onchange = async () => { try { await api('PUT', '/library/identify/settings', { auto_match: au.checked }); toast(au.checked ? 'New songs are matched after each scan, and clear unsure ones settled' : 'Songs are matched only when you press Identify, and every unsure one waits for you'); renderIdentify(); } catch (e) { toast(e.message, 'error'); au.checked = !au.checked; } };
    const wr = $('#idwrite'); if (wr) wr.onchange = async () => { try { await api('PUT', '/library/identify/settings', { write_ids: wr.checked }); toast(wr.checked ? 'IDs will be written into your files' : 'Your files stay as they are'); renderIdentify(); } catch (e) { toast(e.message, 'error'); wr.checked = !wr.checked; } };
    const wn = $('#idwritenow'); if (wn) wn.onclick = async () => { wn.disabled = true; try { const r = await api('POST', '/library/identify/write'); toast(`${fmtCount(r.written)} file${r.written === 1 ? '' : 's'} updated${r.skipped.length ? `, ${fmtCount(r.skipped.length)} left as they were (${r.skipped[0].reason})` : ''}`); } catch (e) { toast(e.message, 'error'); } wn.disabled = false; };
    const run = $('#idrun'); if (run) run.onclick = async () => { try { await api('POST', '/library/identify', { retry: !c.unchecked }); renderIdentify(); } catch (e) { toast(e.message, 'error'); } };
    // Skip all unsure: marked reviewed without an identity (kept out of re-checks); undoable
    // (the node's unskip of all brings back earlier skips too: the Undo says so when there are any)
    // (the Undo button goes with the click: the focus comes back to the card)
    const sk = $('#idskipall'); if (sk) sk.onclick = (ev) => busy(sk, async () => { let r; try { r = await api('POST', '/library/suggestions/skip', { all: true }); } catch (e) { throw noRoute(e); } const ub = toast(`${countOf(r.skipped, 'songs')} left as they are`, '', { label: undoLabel(r), run: async () => { const u = await unskipSongs({ all: true }); toast(`${countOf(u.restored, 'songs')} back for review`); if (state.view === 'identify') { setView('identify', undefined, 'replace'); focusReview(); return; } await renderIdentify(); focusCardTitle(); } }); renderIdentify().then(() => focusUndo(ub)); }, ev);
    // (a second press to confirm: it undoes every "Leave as is" at once, and the node answers how many
    // came back, not which, so there is no Undo for it)
    const us = $('#idunskip'); if (us) {
      // (a key's press: a click within 100 ms of its keydown; a key held down doesn't press it again)
      us.onkeydown = (e) => { if (e.key === 'Enter' || e.key === ' ') { if (e.repeat) e.preventDefault(); else idUnskip.lastKey = Date.now(); } };
      us.onclick = (ev) => {
        if (idUnskip.sending) return undefined;
        // (the clicks of one double-click are one press, however slow; on a touch screen each tap of a
        // double-tap may come with detail 1, so a pointer press soon after the one that asked counts as
        // part of it; a key's press counts at once)
        if (ev && ev.detail > 1) return undefined;
        const byKey = Date.now() - idUnskip.lastKey < 100;
        if (idUnskip.armedUntil <= Date.now()) {
          // (30 s to press again: time enough for a switch or dwell; the card's redraws keep it)
          idUnskip.armedUntil = Date.now() + 30000; idUnskip.armedAt = Date.now();
          us.textContent = `Bring back all ${n(c.skipped)}? Press again`; toast(`Press again to bring back all ${n(c.skipped)} skipped songs for review`);
          setTimeout(() => { if (!idUnskip.sending && idUnskip.armedUntil <= Date.now()) { const b = $('#idunskip'); if (b) b.textContent = `Bring back ${n(c.skipped)} skipped`; } }, 30100);
          return undefined;
        }
        if (!byKey && Date.now() - idUnskip.armedAt < 500) return undefined;
        idUnskip.armedUntil = 0; idUnskip.sending = true; us.disabled = true;
        return busy(us, async () => {
          let u; try { u = await unskipSongs({ all: true }); } finally { idUnskip.sending = false; }
          toast(`${countOf(u.restored, 'songs')} back for review`); await renderIdentify(); focusCardTitle();
        }, ev).finally(() => { if (!idUnskip.sending) { const b = $('#idunskip'); if (b) { b.disabled = false; if (idUnskip.armedUntil <= Date.now()) b.textContent = `Bring back ${n(c.skipped)} skipped`; } } });
      };
    }
    clearTimeout(idTimer);
    // While a run goes on the card is drawn again; not while a Check waits or a key is being typed
    const idAgain = () => { const k = $('#acidkey'); const ch = $('#acidcheck'); if ((k && k.value) || (ch && ch.disabled)) { idTimer = setTimeout(idAgain, 1500); return; } renderIdentify(); };
    if (p.running) idTimer = setTimeout(idAgain, 1500);
  };
  const watchScan = (info) => {
    const was = state.wasScanning; state.wasScanning = !!(info && info.scanning);
    showScanning(!!(info && info.scanning));
    clearTimeout(scanTimer);
    // A scan that ends says so, with what it could not read (fresh-eyes review)
    if (was && info && !info.scanning && info.library) {
      const ls = info.last_scan; const n = info.library.tracks;
      toast(`Library up to date: ${countOf(n.toLocaleString(), 'songs')}${ls && ls.error_count ? `. ${countOf(ls.error_count, 'files')} could not be read (Your library › Node library)` : ''}`, ls && ls.error_count ? 'error' : '');
      forgetAlbums();
      if (state.view === 'settings' || state.view === 'albums' || state.view === 'tracks') render();
    }
    if (info && info.scanning) scanTimer = setTimeout(async () => { try { const i = await api('GET', '/info'); state.info = i; setConn(true, `${i.name} · ${countOf(i.library.tracks, 'songs')}`); watchScan(i); songsChanged(); } catch { showScanning(false); } }, 2000);
  };
  const connect = async () => {
    try {
      // The page's first three reads go together: one after another, a slow link (remote access, a
      // phone's data) waited three round trips before the first page was asked for. (Logged in only:
      // without a token the first refusal alone opens Log in, with its reason. Each is read where it
      // was before; one that fails with /info is no unhandled rejection.)
      const early = state.token ? { me: api('GET', '/me').then((value) => ({ value }), (error) => ({ error })), zones: api('GET', '/zones') } : null;
      if (early) early.zones.catch(() => {});
      state.info = await api('GET', '/info');
      document.body.classList.toggle('offline', isOffline());
      if (isOffline()) {
        // The app's local server: no zones, no events; the app switches back when the node is reachable
        setConn(true, `Offline · ${countOf(state.info.library.tracks, 'songs')} on this device`);
        if (early) state.me = (await early.me).value || { role: 'owner' };
        else { try { state.me = await api('GET', '/me'); } catch { state.me = { role: 'owner' }; } }
        state.sources = []; state.zones = {}; state.zonesSeen = true; state.zone = BROWSER;
        loadFavourites().catch(() => { state.favUris = new Set(); paintHearts(); });
        renderZoneName(); renderBar();
        link.loaded = true;
        if (!OFFLINE_VIEWS.has(state.view)) setView('downloads', undefined, 'replace'); else render();
        return;
      }
      document.body.classList.toggle('phone', phoneMode());
      if (phoneMode()) {
        // The app's own backend, no node: what it lists only (PHONE_CAPS); no zones, no events
        for (const c of PHONE_CAPS) document.body.classList.toggle(`no-${c}`, !offers(c));
        // The on-device list is the phone's own songs here
        $$('.nav-offline span').forEach((x) => { x.textContent = 'My songs'; });
        setConn(true, state.info.name || 'This phone');
        search.placeholder = 'Search your music services'; search.setAttribute('aria-label', search.placeholder);
        try { state.me = await api('GET', '/me'); } catch { state.me = { role: 'owner' }; }
        state.zones = {}; state.zonesSeen = true; state.zone = BROWSER; state.pubkyUser = null; state.favUris = new Set();
        if (offers('sources')) api('GET', '/sources').then((l) => { state.sources = l; applySourcesOff(); }).catch(() => {}); else state.sources = [];
        if (offers('bookmarks')) loadFavourites().catch(() => {});
        dlLoad(); renderZoneName(); renderBar(); refreshQueue(false);
        link.loaded = true;
        const r = routeFromPath();
        if (r && r.v === 'search' && r.arg && r.arg.q) { search.value = r.arg.q; runSearch('replace', r.arg.in || 'everywhere'); } else if (r) setView(r.v, r.arg, 'replace'); else setView(phoneHome(), undefined, 'replace');
        return;
      }
      setConn(true, `${state.info.name} · ${countOf(state.info.library.tracks, 'songs')}`);
      watchScan(state.info);
      // Who this is. Only a backend without the route (404: one with no roles, where everyone is the
      // owner) is read as the owner. Any other failure is the node out of reach, as for /info above:
      // it says so and tries again, instead of showing the owner's controls to someone who may not be
      if (early) {
        const me = await early.me;
        if (me.error) { if (me.error.status !== 404) throw me.error; state.me = { role: 'owner' }; } else state.me = me.value;
      } else { try { state.me = await api('GET', '/me'); } catch (e) { if (e.status !== 404) throw e; state.me = { role: 'owner' }; } }
      document.body.classList.toggle('listener', isListener());
      drawMe().catch(() => {});
      // Usage insights (usage.js): off unless the owner turned them on; asks the node once
      if (window.PM_USAGE) PM_USAGE.mount({ token: () => state.token, view: () => state.view, shell: shellMode, offline: isOffline, open: () => setView('settings', isListener() ? undefined : 'privacy'), ask: (title, text, label, danger) => choiceDialog({ title, text, choices: [{ label, value: true, primary: true, danger: !!danger }] }).then((v) => v === true) });
      if (isOwner()) {
        loadFavourites().catch(() => {});
        api('GET', '/sources').then((l) => { state.sources = l; applySourcesOff(); }).catch(() => {});
        // The outputs, so the bar names a zone's device (a phone's USB DAC) without the picker first
        refreshOutputs();
      } else {
        state.favUris = new Set(); state.sources = [];
        // A listener's own bookmarks, from their homeserver (none until they connect it)
        loadFavourites().catch(() => {});
      }
      const zones = await (early ? early.zones : api('GET', '/zones'));
      state.zones = Object.fromEntries(zones.map((z) => [z.id, z])); state.zonesSeen = true;
      ensureZone(); renderZoneName(); renderBar(); refreshQueue(false); refreshAlong();
      {
        const r = routeFromPath();
        if (r && r.v === 'search' && r.arg && r.arg.q) {
          search.value = r.arg.q;
          history.replaceState({ v: 'search', arg: r.arg, depth: 0 }, '', location.pathname + location.search);
          // A saved search's source applies to its first request, before the view is drawn.
          runSearch('pop', r.arg.in || 'everywhere');
        } else if (r) setView(r.v, r.arg, 'replace');
        else { const m = missedRoute(); setView(m ? m.v : state.view, undefined, 'replace'); if (m) toast(`No page at ${m.said}`); }
      }
      if (signinBack) { const b = signinBack; signinBack = null; state.signinFailed = b.why ? { svc: b.svc, why: b.why } : null; toast(b.text, b.kind); if (isListener()) setView('settings'); else setView('sources', { source: b.svc }); }
      link.loaded = true;
      openEvents();
    } catch (e) {
      if (signedOut()) return;
      main.innerHTML = `<div class="notice warn">Cannot reach the node: ${esc(e.message)} It retries by itself.</div>`;
      { const said = document.getElementById('navstatus'); if (said && said.textContent === 'Loading…') said.textContent = ''; } // (the first loader's word)
      if (!link.downSince) link.downSince = Date.now();
      scheduleRetry(); showDown();
    }
  };
  const openEvents = () => {
    clearTimeout(link.retry); link.retry = null; clearRetryAt();
    dropSocket();
    const proto = location.protocol === 'https:' ? 'wss' : 'ws';
    const ws = new WebSocket(`${proto}://${location.host}/v1/events${tokenQ()}`);
    link.ws = ws;
    ws.onclose = () => { if (link.ws === ws) { link.ws = null; goneDown(); } };
    ws.onmessage = (m) => {
      let ev; try { ev = JSON.parse(m.data); } catch { return; }
      if (ev.type === 'pong') {
        if (ev.id === link.pingId) { link.rtt = Math.max(1, Math.round(performance.now() - link.pingSent)); link.awaiting = false; link.missed = 0; link.pongs = true; setConn('ok', nodeLabel()); }
        return;
      }
      // A snapshot (connected, reconnected, resynced): an outputs_changed may have been missed
      if (ev.type === 'snapshot') { applySnapshot(ev); renderBar(); followZonePop(); refreshOutputs(); return; }
      // Numbered events: a gap means something was missed, so ask for a fresh snapshot.
      if (typeof ev.seq === 'number' && link.seq != null) {
        if (ev.seq > link.seq + 1) ws.send(JSON.stringify({ type: 'resync' }));
        if (ev.seq > link.seq) link.seq = ev.seq;
      }
      if (ev.type === 'zone_event' && ev.event && ev.event.type === 'error' && state.starting && ev.zone === state.starting.zone) startFail(ev.event.message, true);
      // (another zone's event the bar has already heard of changes nothing it shows; the dashboard may be on that zone)
      let barSame = false;
      if (ev.type === 'zone') {
        const old = state.zones[ev.state.id]; state.zones[ev.state.id] = ev.state; countEvent(ev.state.id);
        barSame = !!old && ev.state.id !== state.zone && state.view !== 'dashboard';
        syncVolumeSliders(ev.state);
        if (state.starting && ev.state.id === state.starting.zone) {
          const fresh = state.starting.oldItem == null || (ev.state.now_playing && ev.state.now_playing.item.id !== state.starting.oldItem);
          if (state.starting.sent !== false && ((ev.state.transport === 'playing' && fresh) || (state.starting.what === 'move' && ev.state.transport !== 'stopped'))) startDone(true);
          // Only an error that is new fails the start: the engine keeps the last error until a track starts
          else if (ev.state.transport === 'stopped' && ev.state.error && ev.state.error !== (old && old.error)) startFail(ev.state.error);
        }
        // A start that failed says so until the zone starts playing (from stopped or paused) something
        // other than the song it was already playing: that song going on, or the next one of its queue, is not that start
        if (ev.state.transport === 'playing' && (!old || old.transport !== 'playing') && state.startError && state.startError.zone === ev.state.id && !(ev.state.now_playing && ev.state.now_playing.item.id === state.startError.still)) state.startError = null;
        if (!old) { renderZoneName(); refreshAlong(); }
        if (ev.state.id === state.zone && (!old || old.queue_version !== ev.state.queue_version)) refreshQueue();
        if (ev.state.id === state.zone && old && old.now_playing && ev.state.now_playing && old.now_playing.item.id !== ev.state.now_playing.item.id && !overlay.hidden) renderNowPlaying();
        if (state.view === 'zones' && (!old || old.transport !== ev.state.transport || old.error !== ev.state.error || (old.now_playing && old.now_playing.item.id) !== (ev.state.now_playing && ev.state.now_playing.item.id))) render();
      }
      // A zone that could not play (or seek) says why, once, for the zone this page plays to
      else if (ev.type === 'zone_event' && ev.event && ev.event.type === 'error' && ev.zone === state.zone) {
        const zn = state.zones[ev.zone]; toast(`${zn ? zn.name : 'This zone'}: ${ev.event.message || 'something went wrong'}`, 'error');
      }
      else if (ev.type === 'zone_removed') {
        // A group of yours went: rebuilt (follow it) or ended (back to your own zone). Ask first,
        // then fall back as for any zone that goes
        const yours = ev.zone === state.zone && (state.along || state.hosting || state.alongHome);
        delete state.zones[ev.zone]; countEvent(ev.zone);
        if (yours) refreshAlong().finally(() => { ensureZone(); renderZoneName(); renderBar(); followZonePop(); refreshQueue(); });
        else { ensureZone(); renderZoneName(); refreshAlong(); }
      }
      else if (ev.type === 'library_changed') { if (link.libRev != null) link.libRev++; onLibraryChanged(); }
      else if (ev.type === 'outputs_changed') refreshOutputs(); // a phone's USB DAC came or went
      else if (ev.type === 'jobs') { if (isOwner()) { jobs.eventAt = Date.now(); onJobs(ev.jobs || ev); } } // the owner's jobs, as the polling
      if (!barSame) renderBar();
      if (ev.type === 'zone' || ev.type === 'zone_removed') followZonePop();
    };
  };
  $('#conn').addEventListener('click', () => { if (!link.up) retryNow(); });
  $('#connretry').addEventListener('click', () => retryNow());
  window.addEventListener('online', () => { if (!link.up) retryNow(); });
  window.addEventListener('offline', () => { if (link.up) sendPing(); else showDown(); });
  // Phones and laptops suspend sockets in the background: check at once when the page is back.
  document.addEventListener('visibilitychange', () => { if (document.visibilityState !== 'visible') return; if (link.up) { link.awaiting = false; sendPing(); } else if (link.ever || link.retry) retryNow(); });
  connect();
})();

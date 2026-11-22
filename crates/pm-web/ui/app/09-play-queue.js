  const hitItem = (h, album) => ({ uri: h.uri || `${h.source || 'tidal'}:track:${h.id.replace(/^track:/, '')}`, title: h.title, artist: h.creator || '', album: album || '', duration_ms: h.duration_ms || null, image: h.image || null });
  // (`press`: a Play press starts it; a song ending, a skip, a retry or a queue change starts none)
  const browserPlay = (i, startAt = 0, press = false) => {
    const it = state.browserQueue[i];
    state.restoredAt = null;
    // A pick-up still due for a song that broke off mid-song is off: this play is the one on. A
    // new play gets its own pick-up; a retry of the same song where it was keeps the one it used.
    if (state.midRetry) { clearTimeout(state.midRetry.timer); state.midRetry.timer = null; if (!(state.midRetry.it === it && startAt > 0)) state.midRetry = null; }
    // The end of the queue: the last song stays on the bar, stopped, ready to play again
    if (!it && i >= state.browserQueue.length && state.browserQueue.length) { audio.pause(); try { audio.currentTime = 0; } catch { /* not loaded */ } renderBar(); return; }
    if (!it) { audio.pause(); if (state.ytOn) { yt.stop(); state.ytOn = false; ytPanel(null); } state.browserIndex = -1; renderBar(); return; }
    state.browserIndex = i;
    const yid = ytOf(it.uri);
    if (yid) {
      if (audioEl.getAttribute('src')) { audioEl.pause(); audioEl.removeAttribute('src'); audioEl.load(); }
      useNative(false);
      state.ytOn = true; ytPanel(it);
      yt.open(yid).catch((e) => toast(e.message, 'error'));
      mediaSessionUpdate(it); renderBar(); queueShown();
      return;
    }
    if (state.ytOn) { yt.stop(); state.ytOn = false; ytPanel(null); }
    if (/^deezer:/.test(it.uri || '') && !state.previewNoted) { state.previewNoted = true; toast('Deezer plays a 30-second preview of each song without a Deezer partner account'); }
    browserSrc(it);
    // (a retry opens the song where it was)
    if (startAt > 0) audio.currentTime = startAt;
    // (counted per song: another song at the same place in a new queue gets its own retry)
    state.browserRetry = state.browserRetry && state.browserRetry.it === it ? state.browserRetry : { it, n: 0 };
    state.browserRetry.gaveUp = null;
    // (the wait goes on only for this same song, as a retry; another song started while one still
    // loads gets its own, so the bar names the song now starting, not the one before)
    const sw = state.starting;
    if (!(sw && sw.zone === BROWSER && sw.it === it)) { startWait('play', BROWSER, it.title || '', false, false, press); state.starting.it = it; }
    nativeAhead();
    audio.play().catch(async (e) => {
      // (the app's own player says its failures through its events: nativefailed, nativerefused)
      if (e.name === 'AbortError') { if (state.starting && state.starting.zone === BROWSER && (audio.paused || state.ytOn)) startDone(); return; }
      // (the bar says it in the message's plain words, not the browser's "Failed to load because
      // no supported source was found"; each step below puts its words there too)
      let why = `its source sent no audio ${hereName().toLowerCase()} can play`;
      const was = state.startError;
      // (only this player's start: another zone's start pressed meanwhile isn't failed with this reason)
      if (state.starting && state.starting.zone === BROWSER) startFail(e.name === 'NotSupportedError' ? why[0].toUpperCase() + why.slice(1) : e.message, true);
      const se = state.startError !== was ? state.startError : null;
      const barSays = (msg) => { if (!se || state.startError !== se) return; se.msg = msg; se.at = Date.now(); renderBar(); setTimeout(renderBar, 10500); };
      if (e.name !== 'NotSupportedError') { toast(`Playback: ${e.message}`, 'error'); return; }
      // Ask what came back: a busy node or a proxy in front of it can answer with an error page
      // instead of the file (owner, 2026-10-22: a library FLAC during a big copy)
      try { const r = await fetch(audio.src, { headers: { range: 'bytes=0-1' } }); const ct = r.headers.get('content-type') || ''; if (!r.ok) why = `the node answered ${r.status}${r.status >= 500 ? ' (busy or not reachable just now)' : ''}`; else if (!/^audio\/|octet-stream|mpegurl|video\/mp2t/.test(ct)) why = `the node sent ${ct.split(';')[0] || 'something'} instead of audio`; } catch { why = 'the node did not answer'; }
      // (only while this song is still the one on: another queue started meanwhile keeps its song)
      const still = () => state.browserIndex === i && state.browserQueue[i] === it;
      if (state.browserRetry.n < 1 && still()) { state.browserRetry.n++; barSays(`Trying again: ${why}`); toast(`“${it.title || 'This song'}” did not start: ${why}. Trying again…`); setTimeout(() => { if (still()) { it.moved = true; browserPlay(i, startAt); } }, 2000); return; }
      // (the place a later Play tries it from again: browserResume; a Play while the reason is
      // still being asked does nothing, as while the retry is due)
      if (still()) state.browserRetry.gaveUp = { at: startAt };
      if (still()) barSays(`Can’t play here: ${why}`);
      toast(`“${it.title || 'This song'}” can't play here: ${why}.${it.track_id != null ? '' : ' Try another version on the song\u2019s page.'}`, 'error');
    });
    // A song moved in from a zone was counted there already
    if (it.track_id != null && !it.moved) api('POST', '/library/history', { track_id: it.track_id, zone: 'browser' }).catch(() => {});
    it.moved = false;
    mediaSessionUpdate(it);
    renderBar();
    queueShown();
  };
  // Hardware media keys and lock-screen controls for browser playback.
  const hasMediaSession = typeof navigator !== 'undefined' && 'mediaSession' in navigator && typeof MediaMetadata === 'function';
  const mediaSessionUpdate = (it) => {
    if (!hasMediaSession) return;
    try {
      const albumId = albumIdOf(it);
      navigator.mediaSession.metadata = new MediaMetadata({ title: it.title || it.uri, artist: it.artist || '', album: it.album || '', artwork: it.image ? [{ src: imgSrc(it.image), sizes: '640x640' }] : albumId ? [{ src: `${location.origin}/v1/library/albums/${albumId}/cover${tokenQ()}`, sizes: '512x512' }] : [] });
    } catch { /* metadata is a nicety */ }
  };
  if (hasMediaSession) {
    // (the bar's steps: right after a reload they go on from the saved song, and Previous past
    // 0:03 starts the song again; before, Next and Previous went from song 1)
    const handlers = { play: () => { if (audio.paused) browserStep({ action: 'toggle' }); }, pause: () => browserPause(), nexttrack: () => browserStep({ action: 'next' }), previoustrack: () => browserStep({ action: 'previous' }), seekto: (d) => { if (d && typeof d.seekTime === 'number') browserSeek(d.seekTime); } };
    for (const [k, f] of Object.entries(handlers)) { try { navigator.mediaSession.setActionHandler(k, f); } catch { /* unsupported action */ } }
    audio.addEventListener('play', () => { try { navigator.mediaSession.playbackState = 'playing'; } catch { /* ignore */ } });
    audio.addEventListener('pause', () => { try { navigator.mediaSession.playbackState = 'paused'; } catch { /* ignore */ } });
  }
  // The end of a song in this browser: repeat one plays it again, repeat all wraps to the start
  audio.addEventListener('ended', () => {
    const r = browserRepeat();
    if (r === 'one') return browserPlay(state.browserIndex);
    if (r === 'all' && state.browserIndex + 1 >= state.browserQueue.length) return browserPlay(0);
    browserPlay(state.browserIndex + 1);
  });
  // A stream that breaks off mid-song (a dropped connection) picks up where it was, once
  audio.addEventListener('error', () => {
    if (state.ytOn) return;
    const i = state.browserIndex; const it = state.browserQueue[i]; const at = audio.currentTime;
    if (i < 0 || !it || !audio.src || !(at > 0)) return;
    // (a song that didn't open at all, as at a resume place: that is the start's own retry)
    if (audio.readyState < 1 || (audio.error && audio.error.code === 4)) return;
    // (once per play of this song: a new play, even of another song at the same place, starts afresh)
    state.midRetry = state.midRetry && state.midRetry.it === it ? state.midRetry : { it, n: 0 };
    const mine = state.midRetry;
    if (mine.n >= 1) return;
    mine.n++;
    // (only while this play is still the one on: a song started meanwhile keeps playing; songs
    // taken out or moved before it in the queue meanwhile don't matter)
    mine.src = audio.src; mine.at = at; mine.timer = setTimeout(() => { mine.timer = null; if (state.browserQueue[state.browserIndex] !== it || (state.natOn && nat.held)) return; audio.src = mine.src; audio.currentTime = at; audio.play().catch(() => {}); }, 1500);
  });
  // A pick-up still due is off when the listener pauses (the bar, a media key, the sleep timer, a
  // Move): before, 1.5 s later the song went back to where it broke off and played. A seek or
  // Previous takes it to the place asked for at once (before, it was pulled back there too)
  const cancelPickUp = () => { const m = state.midRetry; if (!m || !m.timer) return null; clearTimeout(m.timer); m.timer = null; return m; };
  const browserPause = () => { if (cancelPickUp()) state.midRetry = null; audio.pause(); };
  // (a song brought back that hasn't opened, or one that couldn't play here: a seek moves the
  // place Play starts it from)
  const browserSeek = (t) => { const r = state.restoredAt; if (r && r.it === state.browserQueue[state.browserIndex]) r.pos = t; const g = state.browserRetry; if (g && g.gaveUp && g.it === state.browserQueue[state.browserIndex]) g.gaveUp.at = t; const m = cancelPickUp(); const up = m && state.browserQueue[state.browserIndex] === m.it && !(state.natOn && nat.held); if (up) audio.src = m.src; audio.currentTime = t; if (up) audio.play().catch(() => {}); };
  // Play on a song whose stream broke off opens it again where it was: at once when its pick-up is
  // still due (before, Play went on and the pick-up opened it a second time 1.5 s later), and when
  // the pick-up is off or used (a Pause or a failed Move in the wait: before, Play went on with the
  // broken stream, silent while the bar said playing). (A song that never opened, error 4, is the
  // start's own retry while that is due: Play does not open it a second time. Once the start and
  // its retry have both failed, "can't play here", a Play is the listener's own new try.)
  // (`press`: the bar's Play pressed, so a song started again here takes its spinner; Move here, none)
  const browserResume = (press = false) => {
    const i = state.browserIndex; const it = state.browserQueue[i]; const m = cancelPickUp();
    // A song brought back after a reload whose file didn't open (the node busy just then): Play
    // starts it where it was left, with the start's own retry and message (before, Play did
    // nothing and said nothing: no start had run, so there was no retry to wait for)
    const r = state.restoredAt;
    if (it && r && r.it === it && audio.readyState < 1) {
      if (audio.error) { it.moved = true; browserPlay(i, r.pos, press); return; }
      // (its file still opening: if the open then fails, this Play becomes that start, from the
      // place held then; before, it went quiet and only a second Play said anything)
      audio.play().catch((e) => { const h = state.restoredAt; if (e.name !== 'NotSupportedError' || !h || h.it !== it || state.browserQueue[state.browserIndex] !== it) return; it.moved = true; browserPlay(state.browserIndex, h.pos, press); });
      return;
    }
    // A song that couldn't play here, its retry used too: Play tries it once more from where that
    // start was, and says so if it fails again (before, Play did nothing and said nothing)
    const g = state.browserRetry;
    if (it && g && g.it === it && g.gaveUp && audio.error && audio.error.code === 4) { const at = g.gaveUp.at; g.gaveUp = null; it.moved = true; browserPlay(i, at, press); return; }
    const at = m && m.it === it ? m.at : audio.error && audio.error.code !== 4 ? audio.currentTime : null;
    if (it && at != null) { it.moved = true; browserPlay(i, at, press); return; }
    audio.play().catch(() => {});
  };
  // For tests (tools/ui-youtube.js): queue items in the current zone, read the browser engine
  window.pmPlayer = { play: (items) => playSourceItems(items, true), now: () => ({ youtube: !!state.ytOn, index: state.browserIndex, paused: audio.paused, time: audio.currentTime, src: audio.src }) };
  audio.addEventListener('timeupdate', () => { nativeAhead(); if (isBrowser()) renderBar(); });
  // A song that didn't start in the app (the node busy, the link down): once more after 2 s, as
  // for <audio>, then said
  const nativeRetry = (i, why, at = 0) => {
    const it = state.browserQueue[i]; if (!it) return;
    if (state.natOn && nat.held) { toast(`“${it.title || 'This song'}” did not start: ${why}.`); return; } // paused meanwhile: the next play opens it
    // (counted per song: another song at the same place in a new queue gets its own retry)
    state.nativeRetry = state.nativeRetry && state.nativeRetry.it === it ? state.nativeRetry : { it, n: 0 };
    // (the retry is the same play: not counted again; nor run when another song is on by then)
    if (state.nativeRetry.n < 1 && state.browserIndex === i) { state.nativeRetry.n++; toast(`“${it.title || 'This song'}” did not start: ${why}. Trying again…`); setTimeout(() => { if (state.browserIndex !== i || state.browserQueue[i] !== it || (state.natOn && nat.held)) return; it.moved = true; browserPlay(i, at); }, 2000); return; }
    toast(`“${it.title || 'This song'}” can't play here: ${why}.`, 'error');
  };
  nat.addEventListener('nativefailed', (e) => { const d = (e && e.detail) || {}; const why = (typeof d === 'string' ? d : d.why) || 'it did not start'; if (!state.natOn || state.browserIndex < 0) return; if (state.starting && state.starting.zone === BROWSER) startFail(why, true); nativeRetry(state.browserIndex, why, d.at || 0); renderBar(); });
  // Opened, then found unplayable by the app (the song handed ahead): the page plays it itself
  nat.addEventListener('nativerefused', () => { const i = state.browserIndex; const it = state.browserQueue[i]; if (!it || !state.natOn) return; it.nativeNo = true; browserPlay(i); });
  // The Zones page's card for this device is drawn again too: it says playing or paused, and why
  // the app's player paused (its USB DAC went away), as the bar does (it was drawn again only when
  // a node's zone changed). (One redraw for a burst: a song's end and the next one's start.)
  let hereCardT = null;
  // (a card's Play, Next, Previous or Mute that had the focus has it again in the new drawing)
  const hereCardAgain = () => {
    clearTimeout(hereCardT);
    hereCardT = setTimeout(async () => {
      if (state.view !== 'zones') return;
      const a = document.activeElement; const card = a && a.dataset && (a.dataset.zt || a.dataset.volmute != null) && a.closest('#main [data-zcard]');
      const was = card ? [card.dataset.zcard, a.dataset.zt ? `[data-zt="${a.dataset.zt}"]` : '[data-volmute]'] : null;
      await render();
      const now = document.activeElement;
      if (was && state.view === 'zones' && (!now || now === document.body)) { const b = $$('#main [data-zcard]').find((c) => c.dataset.zcard === was[0]); const t = b && $(was[1], b); if (t) t.focus(); }
    }, 150);
  };
  audio.addEventListener('play', () => { renderBar(); hereCardAgain(); });
  audio.addEventListener('pause', () => { renderBar(); hereCardAgain(); });
  // Back on screen after Android stopped the app's audio in the background (its player service
  // gone, as the background probe's part C does on API 37): the element still says it plays, but
  // its time stands still and nothing sounds. A pause and play starts it again; a stream that is
  // only buffering (no data yet) is left to arrive.
  // (the in-app player's own element, whichever zone the page shows; cancelled if the app goes
  // out of sight again, or the listener seeks or pauses meanwhile)
  let stallCheck = null;
  const checkStall = () => {
    clearTimeout(stallCheck); stallCheck = null;
    if (document.visibilityState !== 'visible' || !audioEl.getAttribute('src') || state.ytOn || audioEl.paused || audioEl.ended) return;
    const t0 = audioEl.currentTime;
    stallCheck = setTimeout(() => {
      stallCheck = null;
      if (document.visibilityState === 'visible' && audioEl.getAttribute('src') && !state.ytOn && !audioEl.paused && !audioEl.ended && !audioEl.seeking && audioEl.currentTime === t0 && audioEl.readyState >= 2) {
        audioEl.pause(); audioEl.play().catch(() => {});
      }
    }, 2000);
  };
  // Back on screen: the page's own signal, and the app's (its WebView doesn't always say)
  document.addEventListener('visibilitychange', () => (document.visibilityState === 'visible' ? checkStall() : (clearTimeout(stallCheck), stallCheck = null)));
  window.addEventListener('pmdesktop:resumed', checkStall);
  window.addEventListener('pmdesktop:suspended', () => { clearTimeout(stallCheck); stallCheck = null; });

  const rememberTracks = (tracks) => { for (const t of tracks || []) if (t && t.id != null) state.trackCache.set('track:' + t.id, t); };
  // A node zone's new queue with a mode, in one call: the items in their order, shuffle on or off
  // (on: a random song first, the rest shuffled after it; the node keeps the order for shuffle off);
  // the zone's repeat and radio stay
  const loadZoneQueue = async (items, shuffle, o = {}) => {
    const zid = zoneOf(o);
    const index = shuffle ? Math.floor(Math.random() * items.length) : 0;
    const wait = startWait('play', zid, items[index].title || '', true, true);
    const mine = o.mode || askMode(shufflePending, zid, shuffle);
    try {
      await zoneOp(zid, async (t) => {
        // (the zone's repeat and radio, and the song this replaces, as they are when the load goes)
        startSending(wait, zid);
        const z = state.zones[zid] || {}; const ev0 = evOf(zid);
        const st = await api('POST', `/zones/${encodeURIComponent(zid)}/load`, { items, index, transport: 'playing', shuffle, repeat: z.repeat || 'off', radio: !!z.radio }, { signal: t.signal });
        applyAnswer(st, ev0);
      }, o.turn);
      startSent(wait); refreshQueue(); return true;
    } catch (e) { startFail(e.message, false, wait); return false; } finally { if (!o.mode) doneMode(shufflePending, zid, mine); }
  };
  // A page's Play and Shuffle (`o.shuffle` false or true) load the queue in its order with that mode:
  // Shuffle starts on a random song. Without it a new queue keeps the zone's mode (shuffled after its
  // first song when on). (true once it is in, false when it failed and said so: Start radio then
  // turns no radio on)
  const playTracks = async (tracks, replace = true, at = 'end', o = {}) => {
    tracks = (tracks || []).filter((t) => t && t.id != null);
    if (!tracks.length) return false;
    rememberTracks(tracks);
    if (onBrowser(o)) {
      const items = tracks.map((t) => ({ id: t.id, uri: 'track:' + t.id, track_id: t.id, title: t.title, artist: artists(t), artists: t.artists, album: t.album ? t.album.title : '', duration_ms: t.duration_ms }));
      if (replace) browserLoad(items, o.shuffle);
      else if (at === 'next') state.browserQueue.splice(state.browserIndex + 1, 0, ...items);
      else browserAppend(items);
      if (!replace) toast(items.length === 1 ? (at === 'next' ? 'Playing next' : 'Added to queue') : `${countOf(items.length, 'songs')} added`);
      if (state.view === 'queue') render();
      return true;
    }
    if (replace && o.shuffle !== undefined) return loadZoneQueue(tracks.map((t) => ({ uri: `track:${t.id}`, title: t.title || null, artist: artists(t) || null, album: t.album ? t.album.title : null, duration_ms: t.duration_ms || null })), o.shuffle, o);
    const zid = zoneOf(o); const wait = replace ? startWait('play', zid, tracks[0].title, true, true) : null;
    try {
      await zoneOp(zid, (t) => { if (replace) startSending(wait, zid); return api('POST', `/zones/${encodeURIComponent(zid)}/queue`, { track_ids: tracks.map((t) => t.id), replace, play: replace, at }, { signal: t.signal }); }, o.turn);
      if (replace) startSent(wait);
      if (!replace) toast(tracks.length === 1 ? (at === 'next' ? 'Playing next' : 'Added to queue') : `${countOf(tracks.length, 'songs')} added`);
      refreshQueue(); return true;
    } catch (e) { if (replace) startFail(e.message, false, wait); else toast(e.message, 'error'); return false; }
  };
  // Up to 200 of the library's songs (albums or not: untagged files have none), shuffled, on zone
  // `zid` (default the zone in use): Get started's Shuffle my music, and an empty queue's Play. The
  // zone is the one asked for when it was offered, in its turn: a card on Zones that is not the zone
  // in use, or a switch of zone while the songs are read, can't replace another zone's queue
  // (pressed again while one for the same zone is on its way and still the last change asked for that
  // zone, from the note, Get started or anywhere else: nothing, its songs are coming; before, a node
  // zone's second one replaced the first's. With a change asked after it, an album's Play say, the
  // new press is the last word and goes in its turn, as that Play did. This browser's also waits for
  // any other change of its own on its way)
  const shuffling = new Map(); // (zone → the zone's last change asked when its shuffle was)
  // (offered with a library that has songs, and not offline: the downloads have no list of the
  // library's songs to shuffle)
  // The node's library has songs (GET /info's count; undefined counts as none)
  const hasSongs = () => !!(state.info && state.info.library && state.info.library.tracks > 0);
  const canShuffleMine = () => !isOffline() && hasSongs();
  const shuffleMyMusic = (zid = state.zone) => {
    if ((shuffling.has(zid) && shuffling.get(zid) === zoneOps.get(zid)) || (zid === BROWSER && zoneBusy.get(BROWSER))) return Promise.resolve();
    const run = zoneOp(zid, async (turn) => { const r = await api('GET', '/library/tracks?limit=200', undefined, { signal: turn.signal }); await playTracks(r.tracks, true, 'end', { shuffle: true, turn }); });
    const mine = zoneOps.get(zid); shuffling.set(zid, mine);
    return run.finally(() => { if (shuffling.get(zid) === mine) shuffling.delete(zid); });
  };
  // Play, Previous or Next with nothing in the queue and nothing playing says so (before, nothing
  // happened at all) and offers the library shuffled when there is one (not offline: the
  // downloads have no list of the library's songs to shuffle)
  // (on zone `zid`, the one whose Play was pressed: named whenever there is more than one place to
  // play, so the note still says which zone after a switch, and This browser's card says so too)
  const onZoneWords = (zid) => { const name = zid === BROWSER ? hereName() : (state.zones[zid] || {}).name; return name && zoneList().length > 1 ? ` on ${name}` : ''; };
  // The note's Shuffle my music and the empty Queue page's look again when pressed: songs queued
  // meanwhile (here or from another device) are not replaced by the shuffle
  const shuffleIfEmpty = (zid) => (queueEmpty(zid) ? shuffleMyMusic(zid) : toast(`The queue has songs now${onZoneWords(zid)}: press Play`));
  const emptyQueue = (zid = state.zone) => {
    if (isOffline()) toast('The queue is empty: play one of your downloads');
    // (the note stays a while)
    else if (canShuffleMine()) toast(`The queue is empty${onZoneWords(zid)}`, '', { label: 'Shuffle my music', run: () => shuffleIfEmpty(zid) });
    else toast('The queue is empty: search for something to play');
  };
  // A node zone stopped, with nothing in its queue and nothing playing (the node would only answer
  // "queue is empty"; a playing or paused one still takes Pause). For the zone in use, its queue as
  // fetched must say empty too, should its event be late; with no queue fetched for it yet (just
  // switched to, or another zone's kept while This browser was in use) Play goes to the node, so a
  // false "empty" can't offer a shuffle that replaces songs it has.
  // A queue change of this app's still on its way (Add to queue on an empty zone) counts as songs.
  const zoneEmpty = (z) => !!z && z.transport === 'stopped' && !z.now_playing && !hasQueue(z) && !zoneBusy.get(z.id) && (z.id !== state.zone || (z.id === state.queueZone && !(state.queue || []).length));
  // (a zone gone meanwhile: the shuffle goes, and the node says it is gone)
  const queueEmpty = (zid) => (zid === BROWSER ? !state.browserQueue.length : !state.zones[zid] || zoneEmpty(state.zones[zid]));
  // This browser's own player: the bar's buttons (in This browser) and the keyboard's or lock
  // screen's media keys (whichever zone is in use) take the same steps
  const browserStep = (action) => {
    const steps = ['toggle', 'play', 'next', 'previous'].includes(action.action);
    // (only Play/Pause is the press that starts its song: a Next, Previous or queue tap, also a media
    // key's, starts none, so a Play or Pause pressed just before puts no spinner on the song it starts)
    if (action.action !== 'toggle') state.startAnchor = null;
    // (with Shuffle my music still reading the library, nothing: its songs are on their way)
    if (steps && !state.browserQueue.length) { if (!zoneBusy.get(BROWSER)) emptyQueue(BROWSER); return; }
    // Right after a reload the saved song and place come back when the node's events connect. A
    // press before that (or with the node out of reach) goes on from them: Play plays the saved
    // song from where it was, Next and Previous move from it (before, all three started song 1,
    // and the saver then kept that). (Already counted when it first started: no second play.)
    const at = ['toggle', 'next', 'previous'].includes(action.action) && state.browserIndex < 0 ? savedBrowserAt() : null;
    if (at) {
      const i = action.action === 'next' ? at.index + 1 : action.action === 'previous' && at.pos <= 3 ? Math.max(0, at.index - 1) : at.index;
      if (i === at.index) state.browserQueue[i].moved = true;
      browserPlay(i, action.action === 'toggle' ? at.pos : 0, action.action === 'toggle'); renderBar(); return;
    }
    // Songs queued while nothing played (Add to queue, Play next): Play starts the queue at its
    // first song (before, it played nothing and showed Pause)
    if (action.action === 'toggle' && !state.browserQueue[state.browserIndex]) { browserPlay(0, 0, true); return; }
    if (action.action === 'toggle') { if (audio.paused) browserResume(true); else browserPause(); }
    else if (action.action === 'next') browserPlay(state.browserIndex + 1);
    else if (action.action === 'previous') { if (browserTime() > 3) browserSeek(0); else browserPlay(Math.max(0, state.browserIndex - 1)); }
    else if (action.action === 'seek') browserSeek(action.position_ms / 1000);
    else if (action.action === 'play' && action.index != null) browserPlay(action.index);
    renderBar();
  };
  const transport = async (action) => {
    const steps = ['toggle', 'play', 'next', 'previous'].includes(action.action);
    // (the same on a node's zone: see browserStep)
    if (action.action !== 'toggle') state.startAnchor = null;
    if (isBrowser()) { browserStep(action); return; }
    if (!state.zones[state.zone]) return;
    const zt = state.zones[state.zone];
    if (steps && zoneEmpty(zt)) { emptyQueue(); return; }
    // The button says what it does: pause while it shows pause. A toggle pressed twice, or
    // pressed while the shown state was behind, played again instead of pausing
    if (action.action === 'toggle') action = { action: zt.transport === 'playing' ? 'pause' : 'play' };
    if (action.action === 'play' && zt.queue && zt.queue.length) startWait('play', state.zone, zt.now_playing ? zt.now_playing.item.title || '' : '');
    try {
      const st = await api('POST', `/zones/${state.zone}/transport`, action, { repeat: action.action === 'next' || action.action === 'previous' });
      state.zones[st.id] = st;
      renderBar();
    } catch (e) {
      // (Play pressed on an empty queue while a station or a page's Play is still on its way to the
      // zone: the node has no queue yet and says so; that start's own wait goes on and says how it went)
      if (action.action === 'play' && zoneBusy.get(zt.id) && !(zt.queue && zt.queue.length)) return;
      if (state.starting && state.starting.zone === zt.id) startFail(e.message, true); toast(e.message, 'error');
    }
  };
  const playStation = async (st) => {
    if (isBrowser()) {
      if (location.protocol === 'https:' && /^http:\/\//i.test(st.url)) { toast(`This page is served over HTTPS, so ${hereName().toLowerCase()} will not play an http:// stream. Pick a node zone, or a station with an https stream.`, 'error'); return; }
      state.browserQueue = [{ id: Date.now(), uri: st.url, stream: st.url, title: st.name, artist: [st.country, (st.tags || []).slice(0, 3).join(', ')].filter(Boolean).join(' · '), album: 'Internet radio', duration_ms: 0 }];
      browserPlay(0, 0, true); toast(`Tuning to ${st.name}`); return;
    }
    // (in the zone's turn, like a page's Play: while it is on its way the zone's Play doesn't take the
    // queue for empty and offer a shuffle, and it goes after the zone's changes asked before it)
    const zid = state.zone;
    const wait = startWait('play', zid, st.name, true, true);
    try { await zoneOp(zid, (t) => { startSending(wait, zid); return api('POST', '/radio/play', { zone: zid, station: st }, { signal: t.signal }); }); startSent(wait); toast(`Tuning to ${st.name}`); refreshQueue(); } catch (e) { startFail(e.message, true, wait); toast(e.message, 'error'); }
  };
  // A listener's own collection as what this library has of it, in its order: a song as itself, a
  // library album as its songs (as the node's Play on a zone queues it); `out` counts the items
  // with nothing here. Where songs play from is the owner's: no play-from for a listener.
  const listenerSongs = async (d) => {
    const albums = new Map();
    const ids = [...new Set((d.entries || []).filter((e) => e && e.kind === 'album' && e.album && e.album.id != null).map((e) => e.album.id))];
    await Promise.all(ids.map(async (aid) => { try { albums.set(aid, (await api('GET', `/library/albums/${aid}/tracks`)).tracks || []); } catch { albums.set(aid, []); } }));
    albums.forEach((ts) => rememberTracks(ts));
    rememberTracks(d.tracks);
    const song = (t) => ({ track_id: t.id, title: t.title, artist: artists(t), artists: t.artists, album: t.album && t.album.title, duration_ms: t.duration_ms });
    let out = 0;
    const songs = d.playlist.items.flatMap((it, k) => {
      const t = d.tracks[k]; const e = d.entries && d.entries[k];
      const here = t ? [t] : e && e.kind === 'album' && e.album ? albums.get(e.album.id) || [] : [];
      if (!here.length) out += 1;
      return here.map(song);
    });
    return { songs, out };
  };

  const playPlaylist = async (id) => {
    try {
      if (isBrowser() && isListener()) {
        const d = await api('GET', `/playlists/${encodeURIComponent(id)}`);
        const { songs, out } = await listenerSongs(d);
        if (!songs.length) return toast('None of its songs is in this library', 'error');
        if (out > 0) toast(`${countOf(out, 'items')} not in this library; left out of ${hereName().toLowerCase()}'s queue`);
        await playSongs(songs, 0, 'replace', undefined, undefined, undefined, { shuffle: false });
        return;
      }
      if (isBrowser()) {
        // The list's Play shortcut uses the same resolved sources as the detail
        // page: d.tracks alone omits all service-only entries. Only a library song or a
        // stream plays here: a pick with neither (Spotify Connect plays on Spotify's own
        // devices) has just its canonical page, which is never audio, so it is left out and
        // said so. Every other row keeps its place, duplicates included.
        const finding = 'Finding where these songs play…'; const slow = setTimeout(() => toast(finding), 800);
        let d, pf;
        try { [d, pf] = await Promise.all([api('GET', `/playlists/${id}`), api('GET', `/playlists/${encodeURIComponent(id)}/play-from`)]); } finally { clearTimeout(slow); toastDone(finding); }
        rememberTracks(d.tracks); rememberTracks(d.local_tracks);
        const elsewhere = new Map();
        const songs = pf.items.flatMap((r, k) => {
          if (['link', 'post'].includes(d.entries && d.entries[k] && d.entries[k].kind)) return [];
          if (!r.plays_from) { elsewhere.set('', (elsewhere.get('') || 0) + 1); return []; }
          const lib = r.plays_from.service === 'library'; const t = lib ? d.tracks[k] : null;
          const song = { uri: r.stream_uri || null, track_id: lib ? (r.track_id ?? (t && t.id)) : null, artists: t ? t.artists : undefined, title: r.title, artist: r.artist, album: r.album, duration_ms: r.duration_ms, image: r.image };
          if (song.track_id != null || song.uri) return [song];
          const svc = r.plays_from.service; const name = `${SERVICE_NAMES[svc] || srcName(svc)}${r.plays_from.mode === 'connect' ? ' Connect' : ''}`;
          elsewhere.set(name, (elsewhere.get(name) || 0) + 1); return [];
        });
        const local = (d.local_tracks || []).map((t) => ({ track_id: t.id, title: t.title, artist: artists(t), artists: t.artists, album: t.album && t.album.title, duration_ms: t.duration_ms }));
        const left = [...elsewhere].map(([name, n]) => (name ? `${countOf(n, 'songs')} ${n === 1 ? 'plays' : 'play'} only through ${name}` : `${countOf(n, 'songs')} ${n === 1 ? 'plays' : 'play'} nowhere you are connected`)).join('; ');
        if (!songs.length && !local.length) return toast(left ? `${left}; nothing here plays ${hereIn()}` : 'Nothing here plays on your services', 'error');
        // Say what was omitted before starting: a playback failure must remain the last notice.
        if (left) toast(`${left}; left out of ${hereName().toLowerCase()}'s queue`);
        await playSongs([...songs, ...local], 0, 'replace', undefined, undefined, undefined, { shuffle: false });
        return;
      }
      // Play is in the list's order: shuffle goes off first (Shuffle loads through playSongs); both on
      // the zone in use when pressed
      const zid = state.zone;
      const wait = startWait('play', zid, (state.playlists.find((x) => x.id === id) || {}).name || '', true, true);
      // (its shuffle off is the zone's pending mode, as a page's Play would be)
      const mine = askMode(shufflePending, zid, false);
      try {
        // (asked whenever the node has the mode, not only when this app sees it on: what the zone's
        // earlier changes left may not be in yet; the node does nothing when it is off already)
        const r = await zoneOp(zid, async (t) => { startSending(wait, zid); if ('shuffle' in (state.zones[zid] || {})) await setZoneShuffle(false, zid, t.signal); return api('POST', `/playlists/${id}/play`, { zone: zid }, { signal: t.signal }); });
        startSent(wait);
        toast(`Queued ${r.queued} song${r.queued === 1 ? '' : 's'}${r.missing.length ? `; ${r.missing.length} not playable here` : ''}`);
      } catch (e) { startFail(e.message, true, wait); toast(e.message, 'error'); } finally { doneMode(shufflePending, zid, mine); }
    } catch (e) { startFail(e.message, true); toast(e.message, 'error'); }
  };
  // The queue of the zone in use, fetched: only the last one asked counts, and only while its zone
  // is still in use (an older answer, a zone switched from, landing late keeps the newer one). A
  // queue fetched for another zone, or before a time in This browser, no longer counts as this one's.
  let queueAsk = 0, queueDropped = false;
  // What the Queue page shows besides which row is the song on. When only the song changes (the next
  // one starts here, the zone moves on), the page is not drawn again: the mark moves to its row, so a
  // focused row button keeps the focus and a drag in progress still lands. Other changes redraw it.
  const queueKey = () => {
    const z = currentZone(); const q = isBrowser() ? state.browserQueue : state.queue;
    return JSON.stringify([state.zone, z ? z.name : '', !!(z && z.radio), isListener(), zoneList().some((o) => o.id !== state.zone), q.map((it) => [it.id, it.uri, itemName(it), it.artist, it.album, it.duration_ms])]);
  };
  const markQueue = () => {
    const z = currentZone(); const at = z ? z.queue_index : null;
    $$('#main .list.queue [data-qrow]').forEach((r) => {
      const i = Number(r.dataset.qrow); const on = at === i;
      if (r.classList.contains('cur') === on) return;
      r.classList.toggle('cur', on);
      if (on) r.setAttribute('aria-current', 'true'); else r.removeAttribute('aria-current');
      const b = $('[data-qi]', r); if (b) b.innerHTML = on ? icon('play') : `<span class="num">${i + 1}</span>`;
    });
  };
  const queueShown = () => { if (state.view !== 'queue') return; if (state.queueKey === queueKey()) markQueue(); else render(); };
  // (ownDraw: the Queue page asks as it is drawn, and draws what this brings itself. A dropped answer
  // gives false when a newer fetch was asked, null when none was: the zone changed meanwhile)
  const refreshQueue = async (rerender = true, ownDraw = false) => {
    if (state.queueZone !== state.zone) state.queueZone = null;
    if (!isBrowser()) {
      const zid = state.zone; const ask = ++queueAsk;
      // (a failed fetch shows no songs, but says nothing of the zone's queue: Play still goes to the node)
      let q = null; let failed = null; try { q = await api('GET', `/zones/${zid}/queue`); } catch (e) { failed = e; /* said on the Queue page, not shown as an empty queue */ }
      if (ask !== queueAsk || zid !== state.zone) { queueDropped = true; return ask === queueAsk ? null : false; }
      state.queue = q || []; state.queueZone = q ? zid : null; state.queueError = q ? null : failed;
      // (the Queue page drew what it had when its own, older answer was dropped: drawn again now,
      // unless this answer is the page's own, as it is drawn: that draw has it, a second would take
      // the focus from a row)
      if (queueDropped) { queueDropped = false; if (!ownDraw) rerender = true; }
    }
    if (rerender) queueShown();
    if (!$('#nowplaying').hidden) renderNowPlaying();
    return true;
  };

  // Shuffle and repeat are modes of the zone in use, as in any player: the node's come with its state
  // (on /v1/events, so another open app sees them too); this browser keeps its own next to its queue.
  // Repeat goes off → all → one. Shuffle on plays what comes next in a random order, the current song
  // staying; off puts the queue back in its original order, the current song in its place.
  const REPEATS = ['off', 'all', 'one'];
  // Each zone's states from the node's events, counted: an answer to a request is newer than the
  // zone's state here only when no event for that zone came while it was on its way
  const zoneEvents = new Map();
  const evOf = (id) => zoneEvents.get(id) || 0;
  const countEvent = (id) => zoneEvents.set(id, evOf(id) + 1);
  const applyAnswer = (st, ev0) => { if (st && st.id && state.zones[st.id] && evOf(st.id) === ev0) state.zones[st.id] = st; };
  // One change of a zone's mode or queue at a time from this app, in the order asked (a toggle,
  // a page's Play or Shuffle, a playlist's shuffle off): the last one asked is the node's last word
  const zoneOps = new Map();
  const zoneBusy = new Map(); // (per zone: how many of them are asked and not done yet)
  // `fn(turn)` runs when the ones asked before it are done. What it goes on to load (discovery finding
  // songs, the library read for a page's rows) runs inside its turn, given `turn`, rather than waiting
  // on itself. A turn over 60 s is cut: its requests take `turn.signal`, so none of its answers lands
  // here after the ones that follow (the node may still act on one; its next event says)
  const TURN_MS = 60000;
  const zoneOp = (zid, fn, turn) => {
    if (turn && turn.zid === zid && turn.open) return Promise.resolve().then(() => fn(turn));
    const mine = { zid, open: false, signal: null };
    const run = (zoneOps.get(zid) || Promise.resolve()).then(() => {
      const ac = new AbortController(); mine.signal = ac.signal; mine.open = true;
      const why = new Error('The node did not answer in 60 seconds'); ac.signal.why = why; // (older engines pass no reason)
      const t = setTimeout(() => ac.abort(why), TURN_MS);
      return Promise.resolve().then(() => fn(mine)).finally(() => { clearTimeout(t); mine.open = false; });
    });
    zoneBusy.set(zid, (zoneBusy.get(zid) || 0) + 1);
    const done = () => { const n = zoneBusy.get(zid) - 1; if (n > 0) zoneBusy.set(zid, n); else zoneBusy.delete(zid); };
    run.then(done, done);
    zoneOps.set(zid, run.catch(() => {})); return run;
  };
  // The zone a change goes to: its turn's when it runs inside one (the zone in use may have changed
  // meanwhile), else the zone in use. `onBrowser(o)` likewise
  const zoneOf = (o) => (o && o.turn ? o.turn.zid : state.zone);
  const onBrowser = (o) => (o && o.turn ? o.turn.zid === BROWSER : isBrowser());
  // The mode a zone was last asked for (a toggle, a page's Play or Shuffle, a playlist's Play), until
  // it is in: a press turns from it, and an older change can't put its own back over a newer press
  const shufflePending = new Map(); const repeatPending = new Map();
  const askMode = (map, zid, value) => { const token = { value }; map.set(zid, token); renderModes(); return token; };
  const doneMode = (map, zid, token) => { if (map.get(zid) === token) { map.delete(zid); renderModes(); } };
  // A page's Play or Shuffle on a node zone: its mode is asked for when its turn is, and goes with the
  // turn to what it loads (the library read, the finding of songs, the load itself)
  const withTurn = (zid, o, fn) => { const mode = o.shuffle !== undefined && !o.mode ? askMode(shufflePending, zid, o.shuffle) : null; return zoneOp(zid, (turn) => fn({ ...o, turn, mode: o.mode || mode })).finally(() => { if (mode) doneMode(shufflePending, zid, mode); }); };
  const browserRepeat = () => { try { return localStorage.getItem('pm.repeat') || 'off'; } catch { return 'off'; } };
  const zoneRepeat = () => (isBrowser() ? browserRepeat() : (state.zones[state.zone] || {}).repeat || 'off');
  const browserShuffle = () => { try { return localStorage.getItem('pm.shuffle') === 'on'; } catch { return false; } };
  const setBrowserShuffle = (on) => { try { if (on) localStorage.setItem('pm.shuffle', 'on'); else localStorage.removeItem('pm.shuffle'); } catch { /* no storage */ } };
  const zoneShuffle = () => (isBrowser() ? browserShuffle() : !!(state.zones[state.zone] || {}).shuffle);
  // Fisher-Yates on a[from…]
  const shuffleFrom = (a, from = 0) => { for (let k = a.length - 1; k > from; k--) { const j = from + Math.floor(Math.random() * (k - from + 1)); [a[k], a[j]] = [a[j], a[k]]; } return a; };
  // This browser's original order while shuffled: each item's `ord`. One added with "Add to queue"
  // ends it (browserAppend); one with no `ord` ("Play next", or put in by hand) follows the item
  // before it in the play order, so "Play next" is next in both orders. Returns the queue in that
  // order (and numbers it again)
  const browserOrder = () => {
    const q = state.browserQueue; let prev = -1;
    q.forEach((it) => { if (typeof it.ord !== 'number') it.ord = prev + 1e-6; prev = it.ord; });
    const orig = q.map((it, k) => [it, k]).sort((a, b) => a[0].ord - b[0].ord || a[1] - b[1]).map(([it]) => it);
    orig.forEach((it, k) => { it.ord = k; });
    return orig;
  };
  const browserAppend = (items) => { const top = state.browserQueue.reduce((m, it) => (typeof it.ord === 'number' ? Math.max(m, it.ord) : m), -1); items.forEach((it, k) => { it.ord = top + 1 + k; }); state.browserQueue.push(...items); };
  // A new queue here: in its order, shuffled after the first song when shuffle is on (`shuffle`, from a
  // page's Play or Shuffle, sets the mode first; Shuffle starts on a random song)
  const browserLoad = (items, shuffle) => {
    if (shuffle !== undefined) setBrowserShuffle(shuffle);
    items.forEach((it, k) => { it.ord = k; });
    if (browserShuffle()) shuffleFrom(items, shuffle ? 0 : 1);
    state.browserQueue = items; browserPlay(0, 0, true); renderModes();
  };
  // (`ifChanged`: asked by the bar's draw, several times a second: nothing to do while both modes are
  // as they were; whoever draws mode buttons anew (Now playing, the Queue page) asks without it)
  let modesShown = '';
  const renderModes = (ifChanged) => {
    const r = !isBrowser() && repeatPending.has(state.zone) ? repeatPending.get(state.zone).value : zoneRepeat();
    const sh = !isBrowser() && shufflePending.has(state.zone) ? shufflePending.get(state.zone).value : zoneShuffle();
    { const now = `${r} ${sh}`; if (ifChanged && now === modesShown) return; modesShown = now; }
    const rWords = { off: 'Repeat: off', all: 'Repeat: the whole queue', one: 'Repeat: this song' }[r]; const sWords = `Shuffle: ${sh ? 'on' : 'off'}`;
    // (on: the accent and a dot under the icon, not colour alone; the state is in the label too)
    const mark = (b, on, words, ic) => { if (ic && b.dataset.ic !== ic) { b.innerHTML = icon(ic); b.dataset.ic = ic; } b.classList.toggle('on', on); b.setAttribute('aria-pressed', String(on)); b.setAttribute('aria-label', words); b.title = words; };
    $$('#repeatbtn, [data-npt="repeat"]').forEach((b) => mark(b, r !== 'off', rWords, r === 'one' ? 'repeat-one' : 'repeat'));
    $$('#shufflebtn, [data-npt="shuffle"]').forEach((b) => mark(b, sh, sWords));
    const qs = $('#qshuffle'); if (qs && qs.dataset.on !== String(sh)) { qs.dataset.on = String(sh); qs.classList.toggle('primary', sh); qs.setAttribute('aria-pressed', String(sh)); qs.innerHTML = `${icon('shuffle')} Shuffle ${sh ? 'on' : 'off'}`; qs.title = sh ? 'Shuffle is on: press to play the queue in its order again' : 'Play what comes next in a random order; the playing song stays'; }
  };
  const cycleRepeat = async () => {
    if (isBrowser()) { const next = REPEATS[(REPEATS.indexOf(zoneRepeat()) + 1) % REPEATS.length]; try { localStorage.setItem('pm.repeat', next); } catch { /* no storage */ } renderModes(); return; }
    // (in the zone's turn, like a load, which carries the repeat as it is when it goes)
    const zid = state.zone; const cur = repeatPending.has(zid) ? repeatPending.get(zid).value : zoneRepeat();
    const next = REPEATS[(REPEATS.indexOf(cur) + 1) % REPEATS.length];
    const mine = askMode(repeatPending, zid, next);
    // (a 204 says the node took it, not that nothing changed since: another app may have set repeat
    // after it, its event already here. The zone is read again in the same turn, so a load after it
    // carries the node's repeat; the read is kept only if no event came while it was on its way)
    try {
      await zoneOp(zid, async (t) => {
        const ev0 = evOf(zid); const st = await api('PUT', `/zones/${encodeURIComponent(zid)}/repeat`, { repeat: next }, { signal: t.signal });
        if (st && st.id) { applyAnswer(st, ev0); return; }
        // (a read that fails leaves the change made: it is taken here unless an event came since the PUT)
        const ev1 = evOf(zid); const zs = await api('GET', `/zones/${encodeURIComponent(zid)}`, undefined, { signal: t.signal }).catch(() => null);
        if (zs && zs.id) applyAnswer(zs, ev1); else if (state.zones[zid] && evOf(zid) === ev0) state.zones[zid].repeat = next;
      });
    } catch (e) { toast(e.message, 'error'); } finally { doneMode(repeatPending, zid, mine); }
  };
  // The node's mode, set (a node from before modes answers 404/405: it says to update)
  const setZoneShuffle = async (on, zid = state.zone, signal) => {
    const ev0 = evOf(zid);
    let st; try { st = await api('PUT', `/zones/${encodeURIComponent(zid)}/shuffle`, { shuffle: on }, { signal }); } catch (e) { throw noRoute(e); }
    if (st && st.id) applyAnswer(st, ev0); else if (state.zones[zid] && evOf(zid) === ev0) state.zones[zid].shuffle = on;
  };
  // (two presses before the node answers, on two copies: the second turns the first's choice back)
  // (the mode buttons carry data-repeat: a press while the last one is still on its way turns the
  // choice back, so every press runs; the busy guard would drop it)
  const toggleShuffle = async () => {
    if (isBrowser()) {
      const on = !browserShuffle();
      const q = state.browserQueue; const cur = q[state.browserIndex];
      if (on) { q.forEach((it, k) => { it.ord = k; }); shuffleFrom(q, state.browserIndex + 1); } else { state.browserQueue = browserOrder(); if (cur) state.browserIndex = state.browserQueue.indexOf(cur); }
      // (Up next in Now playing and the Queue page show the new order: their rows play by position)
      setBrowserShuffle(on); renderModes(); if (state.view === 'queue') render(); if (!$('#nowplaying').hidden) renderNowPlaying(); return;
    }
    // (one write at a time per zone, in the order pressed: the last press is the node's last word)
    const zid = state.zone; const on = !(shufflePending.has(zid) ? shufflePending.get(zid).value : zoneShuffle());
    const mine = askMode(shufflePending, zid, on); // (the press shows at once)
    try { await zoneOp(zid, (t) => setZoneShuffle(on, zid, t.signal)); refreshQueue(); } catch (e) { toast(e.message, 'error'); } finally { doneMode(shufflePending, zid, mine); }
  };
  $('#repeatbtn').onclick = cycleRepeat;
  $('#shufflebtn').onclick = toggleShuffle;
  $('#toggle').onclick = () => transport({ action: 'toggle' });
  // Next and Previous pressed twice skip twice: they don't wait for the last press (data-repeat)
  $('#next').dataset.repeat = ''; $('#prev').dataset.repeat = '';
  $('#next').onclick = () => transport({ action: 'next' });
  $('#prev').onclick = () => transport({ action: 'previous' });
  const seek = $('#seek');
  seek.addEventListener('input', () => { seek.dataset.dragging = '1'; setFill(seek); });
  // (a drag let go on its start value sends no change: the bar's seek let go too)
  for (const ev of ['pointerup', 'pointercancel', 'touchend', 'touchcancel']) seek.addEventListener(ev, () => { setTimeout(() => { delete seek.dataset.dragging; }, 0); }, { passive: true });
  seek.addEventListener('change', () => {
    delete seek.dataset.dragging;
    const z = currentZone(); const d = z && z.now_playing ? z.now_playing.duration_ms : 0;
    transport({ action: 'seek', position_ms: Math.round((seek.value / 1000) * d) });
  });

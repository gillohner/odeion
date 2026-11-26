  // ---- zones & playback ----------------------------------------------------
  // The browser zone plays through one of two engines: the <audio> element, or YouTube's
  // official player (IFrame Player API) for YouTube links. `audio` stands for whichever is in use,
  // so the player bar, transport, media keys and repeat drive both the same way.
  wordHere();
  const audioEl = $('#browser');
  const ytOf = (u) => { const m = /^youtube:video:([A-Za-z0-9_-]{11})$/.exec(u || '') || /^(?:https?:\/\/)?(?:www\.|m\.|music\.)?(?:youtube\.com\/(?:watch\?(?:[^#]*&)?v=|shorts\/|embed\/|live\/)|youtu\.be\/)([A-Za-z0-9_-]{11})/.exec(u || ''); return m ? m[1] : null; };
  const yt = (() => {
    const ev = new EventTarget(); const fire = (t, detail) => ev.dispatchEvent(new CustomEvent(t, { detail }));
    let player = null; let ready = null; let vid = null; let st = -1; let vol = 1; let tick = null;
    const api = () => (ready || (ready = new Promise((res, rej) => {
      if (window.YT && window.YT.Player) return res();
      const prev = window.onYouTubeIframeAPIReady; window.onYouTubeIframeAPIReady = () => { if (prev) prev(); res(); };
      const sc = document.createElement('script'); sc.src = 'https://www.youtube.com/iframe_api'; sc.async = true;
      sc.onerror = () => { ready = null; sc.remove(); rej(new Error('YouTube\u2019s player did not load (offline, or blocked on this network)')); };
      document.head.appendChild(sc);
    })));
    const has = (f) => player && typeof player[f] === 'function';
    return {
      addEventListener: (t, f) => ev.addEventListener(t, f),
      removeEventListener: (t, f) => ev.removeEventListener(t, f),
      get paused() { return st !== 1 && st !== 3; },
      get ended() { return st === 0; },
      get currentTime() { return has('getCurrentTime') ? player.getCurrentTime() || 0 : 0; },
      set currentTime(v) { if (has('seekTo')) player.seekTo(v, true); },
      get duration() { return has('getDuration') ? player.getDuration() || NaN : NaN; },
      get volume() { return vol; },
      set volume(v) { vol = v; if (has('setVolume')) player.setVolume(Math.round(v * 100)); },
      get src() { return vid ? `https://www.youtube.com/watch?v=${vid}` : ''; },
      set src(_) { /* chosen through open() */ },
      async open(id) {
        await api();
        vid = id; st = -1;
        if (!player) {
          await new Promise((res) => {
            player = new window.YT.Player('ytframe', { videoId: id, width: '100%', height: '100%', playerVars: { autoplay: 1, playsinline: 1, rel: 0, origin: location.origin }, events: {
              onReady: () => { player.setVolume(Math.round(vol * 100)); player.playVideo(); res(); },
              onStateChange: (e) => { st = e.data; if (st === 1) fire('play'); else if (st === 2) fire('pause'); else if (st === 0) fire('ended'); },
              onError: (e) => fire('yterror', e.data),
            } });
          });
        } else player.loadVideoById(id);
        clearInterval(tick); tick = setInterval(() => { if (st === 1) fire('timeupdate'); }, 500);
      },
      play() { if (has('playVideo')) player.playVideo(); return Promise.resolve(); },
      pause() { if (has('pauseVideo')) player.pauseVideo(); },
      stop() { clearInterval(tick); if (has('stopVideo')) player.stopVideo(); vid = null; st = -1; },
    };
  })();
  // The Android app plays the node's streams itself (`window.pmDesktop.nativePlayer`): decoded in
  // the app and played through the phone's output the way the phone's own zone plays (bit-perfect
  // or Exclusive USB to a USB DAC), not through Android's mixer. The same surface as <audio>; the
  // app says where it is in `pmdesktop:native` events. What it can't play (a playlist stream,
  // another codec) the page plays itself (`nativerefused`).
  const tauriInvoke = () => (window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke) || (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke) || null;
  // A phone-mode command to the app (Add songs, Connect to a node): busy at once, sent once (keep:
  // still busy after it went, as the app is leaving this page)
  const phoneCmd = (b, cmd, keep = false) => {
    if (!b || b.disabled || b.getAttribute('aria-busy') === 'true') return Promise.resolve(false);
    const f = tauriInvoke(); if (!f) { toast('That needs the Odeion app', 'error'); return Promise.resolve(false); }
    b.disabled = true; b.setAttribute('aria-busy', 'true'); b.classList.add('is-busy', 'is-spinning');
    const done = () => { b.disabled = false; b.removeAttribute('aria-busy'); b.classList.remove('is-busy', 'is-spinning'); };
    return Promise.resolve().then(() => f(cmd, {})).then(() => { if (!keep) done(); return true; }, (e) => { done(); toast(String((e && e.message) || e || 'That didn’t work; try again.'), 'error'); return false; });
  };
  // Add songs (phone mode): the app picks them; the library page shows them after
  const nativeOk = () => !!(window.pmDesktop && window.pmDesktop.nativePlayer && tauriInvoke());
  const nat = (() => {
    const ev = new EventTarget(); const fire = (t, detail) => ev.dispatchEvent(new CustomEvent(t, { detail }));
    const call = (cmd, args) => { const f = tauriInvoke(); return f ? f(cmd, args || {}) : Promise.reject(new Error('the app’s player is off')); };
    const abs = (u) => { try { return new URL(u, location.href).href; } catch { return ''; } };
    // st: the app's last status (when it came); src: what the page set; at: where to start it
    // (restore, a seek before it plays); live: the app has src; adopt: the app already went on
    // to the song handed ahead; want: play pressed, the app not there yet; ahead: last handed
    let st = null; let stAt = 0; let src = ''; let at = 0; let live = false; let adopt = false; let want = false; let done = false; let vol = 1; let ahead = null; let held = false; let seq = 0; let fresh = false; let opening = false;
    let nativeCommand = 0; let external = null;
    window.addEventListener('pmdesktop:native-command', (e) => {
      const d = e.detail || {}; if (!(d.id > nativeCommand)) return;
      // A seek or queue notice cancels an older direct-command error, but the pending local
      // open still owns its reply (including the timer before native_load). A new local play
      // supplies its own token; only an explicit system pause cancels and settles this open.
      nativeCommand = d.id; external = null;
      if (d.action === 'pause') ++seq;
      if (d.action === 'play') { if (st && st.url === src) at = st.position_ms / 1000; held = false; want = true; external = { id: d.id, seq }; }
      else if (d.action === 'pause') { held = true; want = false; opening = false; fire('pause'); }
      else if (d.action === 'resume') held = false;
    });
    window.addEventListener('pmdesktop:native-failed', (e) => {
      const d = e.detail || {}, err = d.error || {};
      if (!external || d.id !== external.id || seq !== external.seq || d.url !== src || held || err.overtaken) return;
      external = null; opening = false; want = false; live = false; at = Math.max(0, Number(d.at) || 0);
      fire('pause'); fire(err.refused ? 'nativerefused' : 'nativefailed', { why: String(err.message || 'it did not start'), at: d.at || 0 });
    });
    const mine = () => live && !!st && st.url === src;
    const playing = () => mine() && st.transport === 'playing';
    window.addEventListener('pmdesktop:native', (e) => {
      const was = st; st = e.detail || null; stAt = Date.now(); if (!st) return;
      if (!live) return;
      // The song opens again (the same one tapped while it played): the old copy's updates and
      // its end wait
      if (opening && (st.url === src || (was && was.url === src))) return;
      // (an end of the song before this one, still playing while this one opens, is not this one's)
      const ended = was && st.ended > was.ended && was.url === src;
      if (ended) {
        // Handed ahead: the app already plays the next one (the same one for repeat one); the
        // page's next src is it
        adopt = !!st.url && st.advanced > was.advanced; want = false;
        if (!adopt) done = true;
        fire('ended'); return;
      }
      if (st.url !== src) return;
      if (st.refused) { live = false; want = false; fire('nativerefused', st.refused); return; }
      // A refused resume can stay paused (for example, its DAC went away). This is a
      // completed play request, not a stream failure to retry on another output.
      if (st.transport === 'paused' && st.error && !st.broke) { want = false; fire('pause'); return; }
      // A break mid-song: the page's mid-song retry, whatever the page heard before it (at its
      // very start: the song's one retry)
      if (st.broke && !(was && was.url === src && was.broke) && !st.taken) { fire('pause'); if (st.position_ms > 0) fire('error'); else fire('nativefailed', { why: st.error || 'the stream broke off', at: 0 }); return; }
      // (a song opened again while it played: its start is a start, for the bar and the spinner)
      const before = !fresh && was && was.url === src ? was.transport : 'stopped';
      if (st.transport === 'playing') fresh = false;
      if (st.transport === 'playing' && before !== 'playing') { want = false; done = false; fire('play'); fire('playing'); }
      // A break mid-song: the page's mid-song retry; a song that didn't start: its one retry; a
      // device gone (a DAC unplugged): paused, said in the bar
      else if (st.transport !== 'playing' && before === 'playing') { fire('pause'); if (st.error && !st.taken && !st.broke && st.transport === 'stopped' && st.position_ms === 0) fire('nativefailed', st.error); }
      // The song handed ahead didn't open (the node busy): the page's one retry
      else if (st.error && !st.taken && st.transport === 'stopped' && (!was || !was.error) && st.position_ms === 0) fire('nativefailed', st.error);
      if (was && was.duration_ms !== st.duration_ms) fire('durationchange');
      if (!was || was.position_ms !== st.position_ms) fire('timeupdate');
    });
    return {
      addEventListener: (t, f, o) => ev.addEventListener(t, f, o),
      removeEventListener: (t, f) => ev.removeEventListener(t, f),
      get paused() { return !(want || playing()); },
      get ended() { return done; },
      // A pause pressed since the last play (a retry doesn't start it again)
      get held() { return held; },
      get currentTime() {
        if (!mine()) return at;
        const run = st.transport === 'playing' ? Math.min(1000, Date.now() - stAt) : 0;
        return (st.position_ms + run) / 1000;
      },
      set currentTime(v) { external = null; const ms = Math.max(0, Math.round((Number(v) || 0) * 1000)); done = false; if (mine()) { st = { ...st, position_ms: ms }; stAt = Date.now(); call('native_seek', { ms }).catch(() => {}); } else at = ms / 1000; },
      get duration() { return mine() && st.duration_ms ? st.duration_ms / 1000 : NaN; },
      // Never a gain on the samples here: the level is Android's (or the DAC's own)
      get volume() { return vol; },
      set volume(v) { vol = v; },
      get muted() { return false; },
      set muted(_) { /* Android's */ },
      get src() { return src; },
      set src(v) {
        external = null; const u = abs(v);
        if (adopt && st && u === st.url) {
          adopt = false; src = u; live = true; done = false;
          if (st.transport === 'playing') setTimeout(() => { if (src === u) { fire('play'); fire('playing'); } }, 0);
          return;
        }
        adopt = false; src = u; at = 0; live = false; want = false; done = false;
        // As <audio> does once it knows the song: the page may seek it now (restore)
        setTimeout(() => { if (src === u) fire('loadedmetadata'); }, 0);
      },
      set preload(_) { /* opened when it plays */ },
      load() { /* opened when it plays */ },
      async play() {
        external = null; if (!src) throw new Error('nothing to play');
        done = false; held = false;
        if (mine()) {
          if (st.transport === 'playing') return;
          // Carry on (it reopens the song when the node took the output, it broke, or it ended):
          // a failure goes to the page's own handlers, as a failed start does
          // An open under way (a load, or a play that reopens) carries the outcome: this press
          // only says play
          if (opening) { want = true; call('native_play').catch(() => {}); return; }
          want = true; const tok = ++seq; const pos = this.currentTime; const prior = st;
          // (a play can reopen the song: after the node took the output, a break, the end)
          opening = true;
          try { await call('native_play'); } catch (e) {
            if (tok !== seq) return;
            opening = false; want = false; if (e && e.overtaken) return;
            live = false; fire(e && e.refused ? 'nativerefused' : 'nativefailed', { why: String((e && e.message) || e), at: pos });
            return;
          }
          if (tok !== seq) return;
          // (a load this play overtook won't end `opening`: this answer does)
          if (opening) {
            opening = false;
            if (src && st && st.url === src && st.transport === 'playing' && !held) { fresh = false; want = false; done = false; fire('play'); fire('playing'); }
            else if (st !== prior && mine() && st.transport === 'paused' && st.error && !st.broke) { want = false; fire('pause'); }
            // (the output didn't open: said, as for a failed start)
            else if (src && st && st.url === src && st.transport === 'stopped' && st.error && !st.broke) { want = false; live = false; fire('pause'); fire('nativefailed', { why: st.error, at: pos }); return; }
          }
          if (held) call('native_pause').catch(() => {});
          return;
        }
        // Its first events may come before the answer. Only the latest play() changes anything
        // once its answer comes (play pressed twice while a song opens)
        want = true; live = true; fresh = true; const u = src; const tok = ++seq; const prior = st;
        // A seek the page makes as the song loads (moving here from a zone, a restore) comes
        // first: the song opens there
        await new Promise((r) => setTimeout(r, 0));
        if (src !== u || tok !== seq) return;
        fire('waiting');
        opening = true;
        try { await call('native_load', { url: u, atMs: Math.round(at * 1000), ahead: ahead || null }); } catch (e) {
          if (tok === seq) opening = false;
          if (src !== u || tok !== seq || (e && e.overtaken)) return;
          want = false; live = false;
          // Said through the engine's own events, whoever pressed play (the bar, the lock screen,
          // a retry): only what the app can't play itself goes to <audio>; anything else may pass
          // (the page tries once more)
          fire('pause'); fire(e && e.refused ? 'nativerefused' : 'nativefailed', { why: String((e && e.message) || e), at: at });
          return;
        }
        if (tok !== seq) return;
        opening = false;
        // Its start, as the app says it now (the updates waited while it opened)
        if (src === u && st && st.url === u && st.transport === 'playing' && !held) { fresh = false; want = false; done = false; fire('play'); fire('playing'); }
        else if (st !== prior && mine() && st.transport === 'paused' && st.error && !st.broke) { want = false; fire('pause'); }
        else if (src === u && st && st.url === u && st.transport === 'stopped' && st.error && !st.broke) { want = false; live = false; fire('pause'); fire('nativefailed', { why: st.error, at: at }); return; }
        // Paused while it opened
        if (held && src === u) call('native_pause').catch(() => {});
      },
      pause() { external = null; want = false; held = true; if (live) call('native_pause').catch(() => {}); },
      stop() { external = null; want = false; adopt = false; done = false; opening = false; if (live || src) call('native_stop').catch(() => {}); live = false; src = ''; at = 0; ahead = null; },
      // What follows what plays now (the page's next song), for a gapless join
      ahead(u) { const a = u ? abs(u) : null; if (a === ahead) return; ahead = a; if (live) call('native_ahead', { url: a }).catch(() => {}); },
      status: () => (mine() ? st : null),
    };
  })();
  const audio = new Proxy({}, {
    get: (_, k) => {
      if (k === 'addEventListener') return (t, f, o) => {
        // Once: on the engine that loads the song, and then gone from the other two (it would
        // seek a later song there)
        if (o && o.once) { const off = () => { audioEl.removeEventListener(t, w); yt.removeEventListener(t, w); nat.removeEventListener(t, w); }; const w = (e) => { off(); f(e); }; audioEl.addEventListener(t, w); yt.addEventListener(t, w); nat.addEventListener(t, w); return; }
        audioEl.addEventListener(t, f, o); yt.addEventListener(t, f); nat.addEventListener(t, f, o);
      };
      const e = state.ytOn ? yt : state.natOn ? nat : audioEl; const v = e[k];
      return typeof v === 'function' ? v.bind(e) : v;
    },
    set: (_, k, v) => { if (k === 'volume') { audioEl.volume = v; yt.volume = v; nat.volume = v; } else (state.ytOn ? yt : state.natOn ? nat : audioEl)[k] = v; return true; },
  });
  // The URL the page's player opens for a queue item
  const itemUrl = (it) => (it.stream ? it.stream : streamUrlFor(it.uri) || `/v1/library/tracks/${it.track_id}/file${tokenQ()}`);
  // Whether the app plays it itself: in the app, a stream of the node's (this page's origin), not
  // one it already couldn't play
  const nativeFor = (it, url) => { if (!nativeOk() || !it || it.nativeNo || ytOf(it.uri)) return false; try { return new URL(url, location.href).origin === location.origin; } catch { return false; } };
  const useNative = (on) => {
    if (on === !!state.natOn) return;
    if (on) { if (audioEl.getAttribute('src')) { audioEl.pause(); audioEl.removeAttribute('src'); audioEl.load(); } } else nat.stop();
    state.natOn = on;
  };
  // The page's player on a queue item: its engine, then its URL
  const browserSrc = (it) => { const u = itemUrl(it); useNative(nativeFor(it, u)); audio.src = u; };
  // The song after this one as the page will play it (repeat one: the same), handed to the app ahead
  const nativeAhead = () => {
    if (!state.natOn) return;
    const q = state.browserQueue; const r = browserRepeat(); const i = state.browserIndex;
    const n = r === 'one' ? i : i + 1 < q.length ? i + 1 : r === 'all' && q.length ? 0 : -1;
    const it = q[n]; const u = it && itemUrl(it);
    nat.ahead(it && nativeFor(it, u) ? u : null);
  };
  // ---- Downloads in the Android app: songs, albums and playlists kept on this phone ----
  // The app keeps them (pm-offline) and says what it has (`pmdesktop:downloads`); online, the
  // Downloads view is its storage screen. Only the library's own files: a streaming service's
  // songs get no Download.
  const appDl = () => nativeOk() && !isOffline() && offers('downloads');
  const dlCall = (cmd, args) => { const f = tauriInvoke(); return f ? f(cmd, args || {}) : Promise.reject(new Error('the app’s downloads are off')); };
  let dlRevision = 0; let dlRead = 0;
  const dlSet = (d) => { dlRevision += 1; state.dl = d || null; state.dlTracks = new Set((d && d.track_ids) || []); state.dlAlbums = new Set((d && d.album_ids) || []); };
  const dlMark = () => `<span class="dl-mark" role="img" aria-label="On this phone" title="On this phone">${icon('download')}</span>`;
  const dlHas = (id) => !!(state.dlTracks && state.dlTracks.has(Number(id)));
  const dlAlbum = (id) => !!(state.dlAlbums && state.dlAlbums.has(Number(id)));
  // The mark on song rows and album tiles on this phone: drawn with them, and kept up to date here
  // when what the phone has changes
  const markDownloads = (root = document) => {
    if (!state.dl) return;
    const put = (el, on) => { const m = el && $('.dl-mark', el); if (on && el && !m) el.insertAdjacentHTML('beforeend', dlMark()); else if (!on && m) m.remove(); };
    for (const r of $$('.track[data-tid]', root)) put($('.title', r), dlHas(r.dataset.tid));
    for (const r of $$('[data-srow][data-tid]', root)) put($('.name', r), dlHas(r.dataset.tid));
    for (const b of $$('.tile[data-album]', root)) put($('.t', b), dlAlbum(b.dataset.album));
  };
  const dlLoad = () => { document.body.classList.toggle('app-dl', appDl()); const revision = dlRevision; const read = ++dlRead; return appDl() ? dlCall('downloads_state').then((d) => { if (revision === dlRevision && read === dlRead) { dlSet(d); markDownloads(); } return state.dl; }).catch(() => null) : Promise.resolve(null); };
  window.addEventListener('pmdesktop:downloads', (e) => { dlSet(e.detail); markDownloads(); if (state.view === 'downloads' && !isOffline() && e.detail) storageUpdate(e.detail); });
  // Download (or Remove download) for a song, a library album or a playlist, in its ⋯ menu
  const dlKeep = async (kind, id, on, title, sub, online, songs) => {
    try {
      const d = await dlCall('downloads_keep', { kind, id: String(id), on, title: title || null, sub: sub || null, songs: songs || null });
      dlSet(d); markDownloads();
      if (state.view === 'downloads' && state.dl) storageUpdate(state.dl);
      // A song also kept with its album, a playlist or the favourites stays
      if (!on && d && d.still_kept) { toast(`“${title}” stays on this phone: its album, a collection or your bookmarks keep it there`); return; }
      toast(on ? `Downloading “${title}” to this phone${online ? `; ${online === 1 ? '1 song not in your library stays' : `${online} songs not in your library stay`} online` : ''}` : `“${title}” removed from this phone`);
    } catch (e) { toast(String((e && e.message) || e), 'error'); }
  };
  const dlItem = (kind, id, title, sub, online, songs) => {
    if (!appDl() || id == null || id === '' || (kind === 'playlist' && !isOwner())) return [];
    const kept = !!(state.dl && state.dl.items.some((x) => x.kind === kind && x.id === String(id)));
    // On the phone through its album or a playlist: removed there
    if (!kept && kind === 'track' && state.dlTracks && state.dlTracks.has(Number(id))) return [];
    return [{ label: kept ? 'Remove download' : 'Download', icon: kept ? 'trash' : 'download', run: () => dlKeep(kind, id, !kept, title, sub, online, songs) }];
  };
  document.addEventListener('click', (e) => { const b = e.target.closest && e.target.closest('[data-dlgo]'); if (b) setView('downloads'); });
  setTimeout(dlLoad, 0);
  // The storage screen (Downloads in the app, online): what this phone keeps, its size against
  // the limit, the downloads under way, Remove per item and Remove all (the phone's copies only)
  const DL_LIMITS = [2e9, 5e9, 10e9, 20e9, 50e9, null];
  // Each draw has a number: an older one finishing late (its fetch answered after an event) draws
  // nothing
  let storageDraws = 0;
  // What the screen's lists show (redrawn only when this changes)
  const storageKey = (d) => JSON.stringify([d.items, d.bytes, d.other_bytes, d.limit, d.songs]);
  const storageFacts = (d) => {
    const p = d.progress || {};
    const facts = [`${fileSize(d.bytes + (d.other_bytes || 0)) || '0 bytes'}${d.limit ? ` of ${fileSize(d.limit)}` : ''} on this phone${d.other_bytes ? ` (${fileSize(d.other_bytes)} from another node)` : ''}`, countOf(d.songs, 'songs'), d.free != null ? `${fileSize(d.free)} free` : ''].filter(Boolean).join(' · ');
    const at = (p.done || 0) + (p.failed || 0) + (p.preparing || 0);
    const now = p.running ? `Downloading ${Math.min(at + 1, p.wanted)} of ${p.wanted}${p.current ? `: ${esc(p.current)}` : ''}` : p.full ? esc(p.full) : p.failed ? `${countOf(p.failed, 'songs')} didn’t download; they are tried again at the next sync.` : '';
    return `${esc(facts)}${now ? `<br>${now}` : ''}`;
  };
  // An event while the screen shows: the status line in place; the lists only when they changed
  const storageUpdate = (d) => {
    const main = $('#main'); const line = main && $('.dl-note', main);
    if (!line) { if (main) drawStorage(main, d); return; }
    line.innerHTML = storageFacts(d);
    // The lists again when they changed (a song done); the limit row and its picker stay
    if (main.dataset.dlKey !== storageKey(d)) drawStorage(main, d, true);
  };
  const drawStorage = async (main, given, listsOnly) => {
    const mine = ++storageDraws;
    const d = given || await dlLoad();
    if (state.view !== 'downloads' || mine !== storageDraws) return;
    if (!d) { main.innerHTML = pageHead('Downloads', null) + '<div class="notice">The app’s downloads can’t be read just now. Try again in a moment.</div>'; return; }
    main.dataset.dlKey = storageKey(d);
    const sub = (i) => [i.sub, i.kind === 'track' ? '' : i.songs ? `${i.here} of ${countOf(i.songs, 'songs')} here` : `${countOf(i.here, 'songs')} here`, i.online ? `${i.online} stay online` : '', i.bytes ? fileSize(i.bytes) : i.here ? '' : 'waiting'].filter(Boolean).map(esc).join(' · ');
    const rows = (kind) => d.items.map((x, k) => [x, k]).filter(([x]) => x.kind === kind).map(([x, k]) => `<div class="list-row"><div class="grow"><div class="name">${esc(x.title)}</div><div class="sub">${sub(x)}</div></div><button class="btn sm ghost" data-dlrm="${k}">Remove</button></div>`).join('');
    const section = (kind, title) => { const r = rows(kind); return r ? `<section class="section"><div class="section-head"><h3>${title}</h3></div><div class="list">${r}</div></section>` : ''; };
    const limit = `<label class="set-row"><span class="set-label"><span class="set-name">Limit</span><span class="set-help">Downloads stop there, and always leave ${fileSize(d.keep_free)} free on this phone.</span></span><select class="input" id="dllimit" aria-label="Downloads limit">${DL_LIMITS.map((v) => `<option value="${v == null ? '' : v}" ${(d.limit == null ? v == null : v === d.limit) ? 'selected' : ''}>${v == null ? 'No limit' : fileSize(v)}</option>`).join('')}</select></label>`;
    const body = d.items.length
      ? section('album', 'Albums') + section('playlist', 'Collections') + section('track', 'Songs')
        + `<div class="actions"><button class="btn ghost" id="dlrmall">${icon('trash')} Remove all downloads</button></div>`
      : `<div class="empty">${icon('download')}<h3>Nothing on this phone yet</h3><p>Download an album, a collection or a song from its ⋯ menu: it plays here without your node, and saves mobile data.</p></div>`;
    const lists = $('#dllists', main);
    if (listsOnly && lists) { lists.innerHTML = body; const h = $('.page-head', main); if (h) h.outerHTML = `<div class="page-head"><h2>Downloads${d.songs ? `<span class="count">${esc(fmtCount(d.songs))}</span>` : ''}</h2><div class="actions"></div></div>`; }
    else main.innerHTML = pageHead('Downloads', d.songs || null) + `<p class="muted small dl-note" role="status">${storageFacts(d)}</p><div id="dllists">${body}</div><div class="set-group">${limit}</div>`;
    $$('[data-dlrm]', main).forEach((b) => b.onclick = () => { const x = d.items[Number(b.dataset.dlrm)]; return x ? dlKeep(x.kind, x.id, false, x.title) : undefined; });
    const all = $('#dlrmall', main); if (all) all.onclick = async () => { const ok = await choiceDialog({ title: 'Remove all downloads?', text: 'The copies on this phone go. Your library and your collections stay as they are.', choices: [{ label: 'Remove all', value: true, primary: true, danger: true }] }); if (!ok) return; try { dlSet(await dlCall('downloads_remove_all')); markDownloads(); toast('Downloads removed from this phone'); if (state.view === 'downloads') drawStorage(main, state.dl); } catch (e) { toast(String((e && e.message) || e), 'error'); } };
    if (listsOnly && lists) return;
    $('#dllimit', main).onchange = async (e) => { const v = e.target.value; try { dlSet(await dlCall('downloads_limit', { bytes: v === '' ? null : Number(v) })); if (state.view === 'downloads') storageUpdate(state.dl); } catch (er) { toast(String((er && er.message) || er), 'error'); } };
  };

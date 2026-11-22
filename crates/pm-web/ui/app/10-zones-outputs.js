  // The bar's volume slider is bound with the other volume sliders (zoneVolume, above).

  // Zone picker.
  const zonePop = $('#zonepop');
  let zOpener = '#zonebtn'; // the button that opened it (the bar's, or Now playing's) gets the focus back
  const zoneAnchor = () => { const op = $(zOpener); return op && op.getClientRects().length ? op : $('#zonebtn'); };
  const closePop = (fromBack) => { if (zonePop.hidden) return; const had = focusWasIn(zonePop); zonePop.hidden = true; $('#scrim').hidden = true; if (fromBack !== true) popTransient(); focusBack(had, zoneAnchor()); zOpener = '#zonebtn'; };
  // What a device is, in words (owner, 2026-10-29: the line showed raw ids like airplay:9056…):
  // the protocol of its output id, and the device's maker and model when the node knows them
  // (`model` in GET /v1/outputs); the id itself stays as the tooltip.
  const PROTO_WORDS = { alsa: 'ALSA DAC', cpal: 'System audio', null: 'Null output, for developers', pull: 'Pull stream', upnp: 'UPnP', airplay: 'AirPlay', cast: 'Chromecast', slim: 'Squeezelite', snapcast: 'Snapcast', mpd: 'MPD', net: 'The app', group: 'Group', rec: 'Recording', wav: 'WAV file', clock: 'Clock', bluos: 'BluOS', heos: 'HEOS', get browser() { return hereName(); } };
  // A USB DAC on the app's own output (a phone): the node reports it as that output's `device`
  // (GET /v1/outputs). Android's mixer still plays it ("shared"); a mode other than shared shows
  // only when the node says the phone confirmed it for what plays now.
  const DAC_MODE = { bit_perfect: 'Bit-perfect', exclusive: 'Exclusive USB', shared: 'Shared' };
  const DAC_HELP = { shared: 'Android plays it through its own mixer, which mixes in other sounds and usually converts the music to 48 kHz.' };
  const deviceOf = (z) => {
    // The app's own player: the device it plays to, as it says
    if (z && z.id === BROWSER) { const n = state.natOn ? nat.status() : null; return (n && n.device) || null; }
    const o = z && (state.outputs || []).find((x) => x.id === z.output); return (o && o.device) || null;
  };
  // Anything but shared only when confirmed: bit-perfect is said only for mode bit_perfect with
  // confirmed true; an unconfirmed or unknown mode reads as shared
  const dacMode = (d) => (d.confirmed === true && (d.mode === 'bit_perfect' || d.mode === 'exclusive') ? d.mode : 'shared');
  const dacShort = (d) => ({ bit_perfect: 'bit-perfect', exclusive: 'exclusive USB', shared: 'shared' })[dacMode(d)];
  const dacUnconfirmed = (d) => (d.mode === 'bit_perfect' && d.confirmed !== true ? 'Android has not confirmed bit-perfect for this format.' : d.mode === 'exclusive' && d.confirmed !== true ? 'Exclusive USB mode is not confirmed for this format.' : '');
  const dacWords = (d) => `${d.name} · ${dacShort(d)}`;
  // What the zone's DAC does with what plays now: bit-perfect only when the phone confirmed it for
  // this very format (the device's `format`) and the node's own path changes nothing (no volume,
  // DSP or conversion); otherwise shared, and why when the node knows
  const dacNow = (z) => {
    const d = deviceOf(z); if (!d) return null;
    const np = z && z.now_playing; const f = np && np.output_format;
    if (!f) return { bp: false, d, why: '' };
    const same = !!(d.format && d.format.rate_hz === f.rate_hz && d.format.bits === f.bits && d.format.channels === f.channels);
    const clean = !!(np.signal_path && headline(np.signal_path) === 'bit_perfect');
    // Exclusive USB: the app's own driver, confirmed when the DAC's clock read back this rate
    const ex = d.mode === 'exclusive';
    if (dacMode(d) !== 'shared' && same && clean) return { bp: true, ex, d, f };
    const why = !same ? '' : dacMode(d) !== 'shared' ? 'the node changes the samples (volume, DSP or a conversion)'
      : (dacUnconfirmed(d) || d.note || '').replace(/\.$/, '');
    return { bp: false, ex, d, f, why };
  };
  // The path as the listener hears it: the node's stages, then what the phone does with its DAC.
  // Bit-perfect only as dacNow says (confirmed for this format, nothing changed on the way);
  // otherwise Android's mixer, which may convert and mixes in other sounds: every headline, badge
  // and label built from this path follows, so none says bit-perfect for a shared DAC
  const zoneNp = (z, np) => {
    const n = np && np.signal_path && dacNow(z);
    if (!n) {
      // The system's shared output (a phone without a DAC, or one the node no longer knows; a
      // computer's system audio): its mixer is in the path too, so it is never bit-perfect
      const out = z && z.id === BROWSER ? (nat.status() || {}).output : z && z.output;
      if (!np || !np.signal_path || !/(^|\/)cpal:/.test(String(out || ''))) return np;
      const of = np.output_format;
      const st = { name: 'system mixer', in_format: of, out_format: of, quality: 'lossless', note: 'shared: the system may convert it and mixes in other sounds' };
      return { ...np, signal_path: { ...np.signal_path, stages: [...np.signal_path.stages, st] } };
    }
    const of = np.output_format;
    const stage = n.bp
      ? { name: 'DAC', in_format: of, out_format: of, quality: 'bit_perfect', note: n.ex ? dacExLine(n) : `Android confirmed bit-perfect to ${n.d.name}` }
      // The app's own driver, not (yet) confirmed for this format: never said bit-perfect
      : n.ex ? { name: 'USB driver', in_format: of, out_format: of, quality: 'lossless', note: `Exclusive USB · ${n.d.name}: not confirmed for this format${n.why && !/not confirmed/.test(n.why) ? `; ${n.why}` : ''}` }
      : { name: 'Android mixer', in_format: of, out_format: of, quality: 'lossless', note: `shared: Android may convert it and mixes in other sounds${n.why ? `; ${n.why}` : ''}` };
    return { ...np, signal_path: { ...np.signal_path, stages: [...np.signal_path.stages, stage] } };
  };
  // "Exclusive USB · FiiO KA17 · 96 kHz / 24 bit"
  const dacExLine = (n) => `Exclusive USB · ${n.d.name} · ${kHz(n.f.rate_hz)} kHz / ${n.f.bits} bit`;
  // Who sets a USB DAC's level on its bit-perfect paths, as its descriptors tell (`own_volume`;
  // nothing said while that is unknown): its own volume, which the slider, the volume keys and
  // Android's panel move; or, without one, the DAC or amplifier itself (the slider is gone then).
  // Nothing once it played shared: Android's mixer applies the volume there
  const dacVolumeWords = (d) => (d.mode === 'shared' && d.format ? '' : d.own_volume === true ? 'Volume: the DAC’s own' : d.own_volume === false ? 'Volume: set it on the DAC' : '');
  // "FiiO KA17 · bit-perfect" or "· shared", for the picker and the pill
  const dacWordsFor = (z) => { const n = dacNow(z); return n ? `${n.d.name} · ${n.bp ? (n.ex ? 'exclusive USB' : 'bit-perfect') : 'shared'}` : ''; };
  // "Bit-perfect · FiiO KA17 · 96 kHz / 24 bit", or "Shared · FiiO KA17 · why"
  const dacLine = (z) => {
    const n = dacNow(z); if (!n) return '';
    if (n.bp) return n.ex ? dacExLine(n) : `Bit-perfect · ${n.d.name} · ${kHz(n.f.rate_hz)} kHz / ${n.f.bits} bit`;
    if (n.ex && n.d.format) return `Exclusive USB · ${n.d.name} · not confirmed${n.why && !/not confirmed/.test(n.why) ? ` · ${n.why}` : ''}`;
    return `Shared · ${n.d.name}${n.why ? ` · ${n.why}` : ''}`;
  };
  // Now playing › Details, Output: where the sound goes and, when it is not bit-perfect, why.
  // A zone with a USB DAC: dacLine. The Android app's in-app player (This phone) always plays
  // through Android's mixer, which runs at its own rate (usually 48 kHz, what a USB DAC then
  // shows): bit-perfect plays only on the phone's own zone, which the node has while it reaches
  // the phone's player directly (not through remote access). [value, why]
  const outputFacts = (z) => {
    if (!z) return null;
    if (z.id !== BROWSER) { const l = deviceOf(z) ? dacLine(z) : ''; return l ? [l, ''] : null; }
    if (appPlatform() !== 'android') return null;
    // The app plays it itself: the same words as the phone's own zone
    if (state.natOn && !state.ytOn && z.now_playing && z.now_playing.signal_path) {
      const l = deviceOf(z) ? dacLine(z) : ''; const n = nat.status();
      // A song downloaded here plays from its copy: no stream, no mobile data
      return [l || 'Shared · Android mixer', n && n.from_device ? 'From this phone: the downloaded copy plays, not a stream.' : ''];
    }
    if (nativeOk()) {
      const cur = state.browserQueue[state.browserIndex];
      if (state.ytOn) return ['Android mixer · usually 48 kHz', 'Not bit-perfect: YouTube plays in its own player, through Android’s mixer.'];
      return cur && cur.nativeNo ? ['Android mixer · usually 48 kHz', 'Not bit-perfect: the app can’t play this stream itself, so it goes through Android’s mixer.'] : null;
    }
    const id = window.pmDesktop.output;
    const own = id ? Object.values(state.zones || {}).find((x) => x.output === id) : null;
    const dac = appOutput();
    const value = `${dac ? `${dac.device.name} · ` : ''}Android mixer · usually 48 kHz`;
    return [value, own
      ? `Not bit-perfect: ${hereName()} always plays through Android’s mixer. For bit-perfect, play on ${own.name}.`
      : `Not bit-perfect: ${hereName()} always plays through Android’s mixer. Bit-perfect plays on the phone’s own zone, which the node adds when it can reach this phone directly (on the same network, not through remote access).`];
  };
  const dacCaps = (d) => [d.max_rate ? `PCM up to ${kHz(d.max_rate)} kHz` : '', d.bits && d.bits.length ? `${d.bits.join(', ')} bit` : '', d.channels ? `${d.channels} channels` : '', d.dsd ? ({ native: 'DSD (native)', dop: 'DSD (DoP)' })[d.dsd] || 'DSD' : '', d.hardware_volume ? 'its own volume' : ''].filter(Boolean).join(' · ');
  // This app's own output (the app tells the page after it registers its player), with its device
  const appOutput = () => { const id = window.pmDesktop && window.pmDesktop.output; return id ? (state.outputs || []).find((o) => o.id === id && o.device) || null : null; };
  const hereDevice = () => (window.pmDesktop && window.pmDesktop.platform === 'android' ? 'This phone' : 'This computer');
  // A DAC plugged in or out: the app registers its player again and says so
  // (also once after each page load), and the node tells every controller (`outputs_changed`):
  // the outputs again, then the bar, the Sound panel and the picker; Settings draws again when it
  // shows another device than this app's
  const appDevice = () => JSON.stringify((appOutput() || {}).device || null);
  // One loader for GET /v1/outputs, used by every refresh and Settings: one request at a time,
  // one more after it when asked meanwhile (the answer in flight may predate the change), and an
  // answer older than the newest installed is never installed
  const outputsLoad = { seq: 0, applied: 0, inflight: null, again: false };
  const loadOutputs = () => {
    if (outputsLoad.inflight) { outputsLoad.again = true; return outputsLoad.inflight; }
    const seq = ++outputsLoad.seq;
    outputsLoad.inflight = api('GET', '/outputs')
      .then((o) => { if (seq > outputsLoad.applied) { outputsLoad.applied = seq; state.outputs = o; } return state.outputs; })
      .finally(() => { outputsLoad.inflight = null; if (outputsLoad.again) { outputsLoad.again = false; refreshOutputs(); } });
    return outputsLoad.inflight;
  };
  // A USB DAC on the phone (the Android app): "this phone" plays through the phone's own zone, the
  // path that reaches the DAC unchanged (Exclusive USB, else bit-perfect); the page's own player
  // always goes through Android's mixer. Once per DAC, and never while the page's player plays.
  let dacZoneFor = null;
  const preferPhoneZone = () => {
    // The native engine already owns this output. A paused DAC must not change the route.
    if (window.pmDesktop && window.pmDesktop.nativePlayer) return;
    const o = appOutput();
    if (!o || appPlatform() !== 'android') { dacZoneFor = null; return; }
    const key = `${o.id}|${o.device.name}`; if (dacZoneFor === key) return;
    const z = Object.values(state.zones || {}).find((x) => x.output === o.id); if (!z) return;
    dacZoneFor = key;
    if (state.zone !== BROWSER || !audio.paused) return;
    state.zone = z.id; try { localStorage.setItem('pm.zone', z.id); } catch { /* storage unavailable */ }
    renderZoneName(); renderBar(); queueShown();
    toast(`${o.device.name}: this phone plays through ${z.name}, straight to the DAC`);
  };
  const refreshOutputs = () => { if (isOwner()) loadOutputs().then(() => { preferPhoneZone(); renderBar(); followZonePop(); if (state.view === 'settings' && state.dacDrawn !== undefined && appDevice() !== state.dacDrawn) render(); }).catch(() => {}); };
  window.addEventListener('pmdesktop:output', refreshOutputs);
  // A zone's line under its name (Zones cards, Play to, Move to, the Sound panel, Settings). The page's
  // own player's is short, so it fits beside "selected" or Play to on a 360 px phone's card
  const deviceWords = (z) => {
    if (z.id === BROWSER) return appPlatform() ? 'Plays in the app' : 'Plays in this browser';
    if (deviceOf(z)) return dacWordsFor(z);
    const id = String(z.output || ''); const o = (state.outputs || []).find((x) => x.id === id);
    return [PROTO_WORDS[id.split(':')[0]] || id.split(':')[0], o && o.model].filter(Boolean).join(' · ');
  };
  // Something to move: the zone's queue, or the browser's.
  const hasQueue = (z) => !!z && (z.id === BROWSER ? state.browserQueue.length > 0 : !!(z.queue && z.queue.length));
  // The browser takes over a queue at a song and a position; paused stays paused.
  const browserResumeAt = (index, posMs, paused) => {
    const it = state.browserQueue[index]; if (!it) { state.browserIndex = -1; renderBar(); return; }
    if (ytOf(it.uri)) { browserPlay(index); return; } // YouTube's player starts from the top
    if (state.ytOn) { yt.stop(); state.ytOn = false; ytPanel(null); }
    if (!paused) {
      // The seek is for this song only: a later song (this one never loaded) is not moved
      audio.addEventListener('loadedmetadata', () => { if (state.browserIndex !== index) return; try { if (posMs > 1000) audio.currentTime = posMs / 1000; } catch { /* not seekable */ } renderBar(); }, { once: true });
      it.moved = true; browserPlay(index); return; // browserPlay says why when it can't play
    }
    // Paused: held at its place, as after a reload (before, a file that didn't open left the bar at
    // 0:00 and Play did nothing and said nothing)
    state.browserIndex = index;
    holdPlace(it, posMs > 1000 ? posMs / 1000 : 0);
    browserSrc(it);
    audio.preload = 'metadata'; audio.load();
    mediaSessionUpdate(it); renderBar();
  };
  // Whether this browser can play a queue item: a library file, a stream it reaches, YouTube
  const browserCanPlay = (it) => !!it && (/^track:\d+$/.test(it.uri || '') || !!ytOf(it.uri) || !!streamUrlFor(it.uri));
  // Move what plays to another device (owner, 2026-10-29): the song at its position, the queue,
  // repeat and radio; paused stays paused. Zone to zone is the node's transfer. This browser is no
  // zone of the node's, so out of it the target loads the browser's queue (POST /zones/{id}/load)
  // and the browser stops; into it the browser takes the zone's queue and the zone empties (radio
  // is the node's: it does not come along). The node's errors (403: a zone not open to listeners;
  // 400: an empty queue) show in the menu when it is open, else as a toast.
  let queueSaving = false; // one Save as playlist at a time (the Queue page's)
  let moving = false; // one move at a time (review: a second Move here sent the same queue twice)
  let moveTarget = null; // where that move goes: its Move here still says "Moving…" when the menu is drawn again
  const moveTo = async (target) => {
    const from = state.zone;
    if (target === from || moving) return;
    moving = true; moveTarget = target; $$('[data-move]', zonePop).forEach((b) => { b.disabled = true; });
    // Zone to zone can take a while (the node opens the track first): say so at once when the menu is closed
    // (the message goes when the node answers: the menu opened meanwhile is drawn again instead of a
    // toast, and the message stayed over it, saying it still moved)
    let said = '';
    if (zonePop.hidden) { const tz0 = target === BROWSER ? { name: hereName() } : state.zones[target]; said = `Moving to ${(tz0 && tz0.name) || 'the zone'}…`; toast(said); }
    let resume = null; // the browser, paused for the move: back on when the move fails
    const cur = currentZone(); const curTitle = cur && cur.now_playing ? cur.now_playing.item.title || '' : '';
    const wasPlaying = cur && cur.transport === 'playing';
    try {
      if (from === BROWSER) {
        // (shuffle on: the items in their original order, so the node keeps it; it draws the rest again)
        const cur = state.browserIndex >= 0 ? state.browserQueue[state.browserIndex] : null;
        let queue = browserShuffle() ? browserOrder() : state.browserQueue;
        // YouTube plays in this page: a zone takes it only with yt-dlp on (as playSourceItems)
        const ytZones = (state.sources || []).some((x) => x.id === 'youtube' && x.enabled !== false && /zones/.test(x.status));
        if (!ytZones && queue.some((q) => ytOf(q.uri))) {
          if (cur && ytOf(cur.uri)) throw new Error(`YouTube plays only ${hereOnly()}`);
          const rest = queue.filter((q) => !ytOf(q.uri));
          toast(`${countOf(queue.length - rest.length, 'YouTube items')} left out: YouTube plays only ${hereOnly()}`);
          queue = rest;
        }
        const items = queue.map((it) => ({ uri: nodeRel(it.uri), title: it.title || null, artist: it.artist || null, album: it.album || null, duration_ms: it.duration_ms || null, image: nodeRel(it.image || null), limit: it.limit || null }));
        if (!items.length) throw new Error('Nothing to move: the queue is empty');
        const index = cur ? queue.indexOf(cur) : -1;
        const transport = !cur ? 'stopped' : browserTransport();
        // Pause here first: the node may take seconds to open the track, and both would play meanwhile
        if (transport === 'playing') { browserPause(); resume = true; }
        const zs = await zoneOp(target, (t) => api('POST', `/zones/${encodeURIComponent(target)}/load`, { items, index: index >= 0 ? index : null, position_ms: Math.round((browserTime() || 0) * 1000), transport, repeat: browserRepeat(), shuffle: browserShuffle(), radio: false }, { signal: t.signal }));
        state.zones[zs.id] = zs;
        if (transport !== 'stopped' && zs.transport === 'stopped' && zs.error) throw new Error(zs.error);
        resume = null;
        if (state.ytOn) { yt.stop(); state.ytOn = false; ytPanel(null); } useNative(false); audioEl.removeAttribute('src'); audioEl.load();
        state.browserQueue = []; state.browserIndex = -1; try { localStorage.removeItem('pm.browserAt'); } catch { /* storage unavailable */ }
      } else if (target === BROWSER) {
        const z = await api('GET', `/zones/${encodeURIComponent(from)}`);
        if (!z.queue || !z.queue.length) throw new Error('Nothing to move: the queue is empty');
        const np = z.now_playing; const index = np ? np.index : (z.queue_index != null ? z.queue_index : -1); const pos = np ? np.position_ms || 0 : 0;
        // Refused before the zone empties: the song in hand must be one this browser can play
        const cur = index >= 0 ? z.queue[index] : null;
        if (cur && z.transport !== 'stopped' && !browserCanPlay(cur)) throw new Error(`“${cur.title || 'This song'}” plays on the node’s zones only; it stays on ${z.name}`);
        await api('DELETE', `/zones/${encodeURIComponent(from)}/queue`);
        if (state.zones[from]) { state.zones[from] = { ...state.zones[from], queue: [], now_playing: null, transport: 'stopped' }; }
        state.browserQueue = z.queue.map((it, k) => { const m = /^track:(\d+)$/.exec(it.uri || ''); return { id: Date.now() + k, uri: it.uri, track_id: m ? Number(m[1]) : undefined, stream: m ? null : streamUrlFor(it.uri) || it.uri, title: it.title, artist: it.artist, album: it.album, duration_ms: it.duration_ms, image: it.image || null, limit: it.limit || null }; });
        try { localStorage.setItem('pm.repeat', z.repeat || 'off'); } catch { /* storage unavailable */ }
        // (shuffle comes along; the zone's original order isn't in its state, so the play order stands for it)
        state.browserQueue.forEach((it, k) => { it.ord = k; }); setBrowserShuffle(!!z.shuffle);
        if (index >= 0 && index < state.browserQueue.length && z.transport !== 'stopped') browserResumeAt(index, pos, z.transport === 'paused'); else state.browserIndex = -1;
      } else {
        const was = state.zones[from];
        const zs = await api('POST', `/zones/${encodeURIComponent(from)}/transfer`, { to: target });
        state.zones[zs.id] = zs;
        if (state.zones[from]) state.zones[from] = { ...state.zones[from], queue: [], now_playing: null, transport: 'stopped' };
        if (was && was.transport !== 'stopped' && zs.transport === 'stopped' && zs.error) throw new Error(zs.error);
      }
      state.zone = target; try { localStorage.setItem('pm.zone', target); } catch { /* storage unavailable */ }
      // The bar says "Moving to …" until the target plays (the node answered, but its player may still be opening the track)
      const tz = state.zones[target];
      if (target !== BROWSER && wasPlaying && tz && tz.transport !== 'playing') startWait('move', target, curTitle, false, false, false);
      renderZoneName(); renderBar(); refreshQueue();
      const name = (currentZone() || {}).name || target;
      moving = false; // before the menu draws again, so its Move here buttons are on
      if (!zonePop.hidden) renderZonePop(); else toast(`Moved to ${name}`);
    } catch (e) {
      moving = false;
      startFail(e.message, true);
      if (resume) browserResume();
      if (!zonePop.hidden) renderZonePop(e.message); else toast(e.message, 'error');
    } finally { moving = false; if (said) toastDone(said); }
  };
  // Listening together on this node (presence.rs): who you listen along with, with Leave, and who
  // sees what you play. The node keeps it; the page asks every 20 s and when zones come or go.
  const personName = (l) => l.name || (l.key === 'owner' ? 'the owner' : shortKey(l.key));
  const VISIBLE = [['hidden', 'Only you', 'nobody else sees what you play'], ['followed', 'People you follow', 'on Pubky'], ['everyone', 'Everyone on this node', '']];
  const renderAlong = () => {
    const a = state.along; $('#along').hidden = !a;
    if (a) $('#alongname').textContent = personName(a);
  };
  // Follow your music: into the group it plays on (you host it or listen along) from your own
  // zone, and back to your own zone once that group is gone. A zone you picked yourself stays.
  const followAlong = () => {
    const g = state.along || state.hosting; const target = g && g.zone ? g.zone.id : null;
    if (g && g.your_zone) state.alongHome = g.your_zone;
    let to = null;
    if (target && state.zones[target] && state.zone !== target && (state.zone === g.your_zone || !state.zones[state.zone])) to = target;
    else if (!g && !state.zones[state.zone] && state.alongHome && state.zones[state.alongHome]) to = state.alongHome;
    if (!g) state.alongHome = null;
    if (!to) return;
    state.zone = to; try { localStorage.setItem('pm.zone', to); } catch { /* storage unavailable */ }
    renderZoneName(); renderBar(); refreshQueue();
  };
  const refreshAlong = async () => {
    if (!state.me || isOffline() || document.hidden) return;
    try {
      const ls = await api('GET', '/presence');
      state.along = ls.find((l) => l.mine) || null; state.hosting = ls.find((l) => l.you && l.hosting) || null; state.presence = ls;
      if (state.presenceVisible == null) state.presenceVisible = (await api('GET', '/me/presence')).visible;
    } catch { return; }
    renderAlong(); followAlong(); renderListening();
  };
  // Home's "Listening now" (TASK-pinakes-social-ux L3): who else on this node is listening, what and
  // where, with Listen along when their zone is open to you. A zone you can't see: just who.
  // The node's own words name the reason (along already, someone else's zone, a maker's group,
  // a zone that is gone); a 403 gets a plain sentence
  const ALONG_ERRORS = { 403: 'That zone isn’t open to you.' };
  const renderListening = () => {
    const wrap = $('#h-listenwrap'); if (!wrap || state.view !== 'home') return;
    const others = (state.presence || []).filter((l) => !l.you);
    if (!others.length) { wrap.innerHTML = ''; return; }
    wrap.innerHTML = `<section class="section" id="h-listening"><div class="section-head"><h3>Listening now</h3></div><div class="list">${others.map((l, k) => {
      const name = personName(l);
      const face = l.avatar ? `<img class="thumb round" src="${esc(imgSrc(l.avatar))}" alt="" loading="lazy" data-err="hide">` : `<div class="avatar" aria-hidden="true">${esc(initials(name))}</div>`;
      const what = l.title ? ` to <b>${esc(l.title)}</b>${l.artist ? ` <span class="muted">· ${esc(l.artist)}</span>` : ''}` : '';
      const joining = state.joiningAlong === l.key;
      const act = l.mine ? '<span class="pill ok">Listening along</span>' : l.zone && l.can_join ? `<button class="btn sm" data-alongjoin="${k}" ${joining ? 'disabled aria-label="Joining…"' : `aria-label="Listen along with ${esc(name)}"`}>${joining ? '<span class="spin"></span> Joining…' : `${icon('zone')} Listen along`}</button>` : '';
      return `<div class="list-row listening">${face}${l.image ? `<img class="thumb" src="${esc(imgSrc(l.image))}" alt="" loading="lazy" data-err="hide">` : ''}<div class="grow"><div class="name">${esc(name)} is listening${what}</div>${l.zone ? `<div class="sub">on ${esc(l.zone.name)}${l.playing === false ? ' · paused' : ''}</div>` : ''}</div>${act}</div>`;
    }).join('')}</div></section>`;
    $$('[data-alongjoin]', wrap).forEach((b) => b.onclick = () => joinAlong(others[Number(b.dataset.alongjoin)], b));
  };
  // Listen along from a zone that is free to join: a node zone (This browser can't join a group),
  // not one someone else is listening on (joining stops what plays there) and not a listen-along
  // group. The zone in use when it is one, else a menu of them. The page only sees listeners who
  // let it, so the node is the guard: it refuses (409) a zone where someone else's music plays,
  // and then the menu offers the others (`skip`: zones refused so far).
  const joinAlong = (l, b, skip = new Set(), quiet = false) => {
    const busy = new Set((state.presence || []).filter((p) => !p.you && p.zone).map((p) => p.zone.id));
    const free = (id) => id !== BROWSER && !!state.zones[id] && !String(id).startsWith('group:') && !busy.has(id) && !skip.has(id) && (!l.zone || id !== l.zone.id);
    if (!skip.size && free(state.zone)) return doJoinAlong(l, state.zone, b, skip);
    const mine = zoneList().filter((z) => free(z.id));
    if (!mine.length) { if (quiet) return; toast(skip.size ? 'Someone else is listening on your other speakers too: Listen along needs free ones.' : `Listening along needs speakers on this node that nobody else is using: ${hereName()} can’t join.`, 'error'); return; }
    showMenu(b, [{ title: skip.size ? 'Someone listens there: listen from' : 'Listen from' }, ...mine.map((z) => ({ label: z.name, icon: 'zone', run: () => doJoinAlong(l, z.id, b, skip) }))], { all: true });
  };
  const doJoinAlong = async (l, zone, b, skip = new Set()) => {
    // The row may be drawn again while this runs (a zone came, the 20 s poll): it stays "Joining…"
    state.joiningAlong = l.key; renderListening();
    try {
      const zs = await api('POST', '/presence/along', { key: l.key, zone });
      state.zones[zs.id] = zs; state.zone = zs.id; try { localStorage.setItem('pm.zone', state.zone); } catch { /* storage unavailable */ }
      renderZoneName(); renderBar(); refreshQueue();
      state.joiningAlong = null;
      toast(`Listening along with ${personName(l)}`);
      await refreshAlong();
    } catch (e) {
      state.joiningAlong = null; renderListening();
      // Someone else's music plays on that zone: pick another of yours
      if (e.status === 409 && /someone else/i.test(e.raw || e.message)) {
        skip.add(zone); toast(e.message, 'error');
        const others = (state.presence || []).filter((p) => !p.you);
        const again = $$('#h-listenwrap [data-alongjoin]').find((x) => (others[Number(x.dataset.alongjoin)] || {}).key === l.key) || b;
        if (again.isConnected) joinAlong(l, again, skip, true);
        return;
      }
      toast(ALONG_ERRORS[e.status] || e.message, 'error');
    }
  };
  const leaveAlong = async () => {
    const a = state.along; const b = $('#alongleave'); if (!a) return;
    b.disabled = true;
    try {
      const zs = await api('DELETE', '/presence/along'); state.zones[zs.id] = zs;
      if (a.your_zone && state.zones[a.your_zone]) { state.zone = a.your_zone; try { localStorage.setItem('pm.zone', state.zone); } catch { /* storage unavailable */ } }
      state.along = null; renderAlong(); renderZoneName(); renderBar(); refreshQueue();
      toast(`You left ${personName(a)}'s music`);
    } catch (e) { toast(e.message, 'error'); } finally { b.disabled = false; }
  };
  $('#alongleave').onclick = leaveAlong;
  setInterval(refreshAlong, 20000);
  const setVisible = async (v) => {
    try { state.presenceVisible = (await api('PUT', '/me/presence', { visible: v })).visible; toast(v === 'hidden' ? 'Only you see what you play' : `${VISIBLE.find((x) => x[0] === v)[1]} can see what you play`); } catch (e) { toast(e.message, 'error'); }
  };
  // A menu opened from the Play to popover waits until the popover's history entry is popped, so it
  // gets its own Back step (opened at once, it found that entry still there and pushed none, and
  // Back then left the URL a page behind)
  const menuAfterPop = (anchor, items) => { const go = () => showMenu(anchor, items); if (popPending) popPending.then(go); else go(); };
  // What the menu shows, and the error it shows (kept when a zone event draws it again)
  let zonePopDrawn = ''; let zonePopErr = null;
  const zonePopHtml = (err) => {
    const movable = hasQueue(currentZone());
    const movingTo = state.starting && state.starting.what === 'move' ? state.starting.zone : moving ? moveTarget : null;
    return `<div class="pop-title">Play to</div>` + zoneList().map((z) => `
      <div class="pop-row"><button class="pop-item ${z.id === state.zone ? 'on' : ''}" data-zone="${esc(z.id)}">
        ${icon('zone')}<span><span class="name">${esc(z.name)}</span><span class="sub" title="${esc(z.id === BROWSER ? '' : z.output)}">${esc(deviceWords(z))}${z.id === BROWSER ? '' : `<span class="sr-only"> (${esc(z.output)})</span>`}</span></span>
        <span class="state">${zoneStateText(z)}</span>
      </button>${movingTo === z.id ? '<span class="pop-act muted"><span class="spin"></span> Moving…</span>' : movable && z.id !== state.zone ? `<button class="pop-item pop-act" ${moving ? 'disabled' : ''} data-move="${esc(z.id)}" title="Move what plays, with the queue, to ${esc(z.name)}" aria-label="Move here: ${esc(z.name)}">Move here</button>` : ''}</div>`).join('')
      + (err ? `<div class="pop-err" role="alert">${esc(err.charAt(0).toUpperCase() + err.slice(1))}</div>` : '')
      // The sleep timer for the zone in use, here too (UX review: it was only in Now playing). Moving
      // is each device's Move here above, one way only (UX sweep A14); the Queue page has Move to…
      + `<div class="pop-title">This zone</div>`
      + `<button class="pop-item" id="zpsleep">${icon('moon')}<span><span class="name">${esc(sleepLabel() === 'Sleep' ? 'Sleep timer' : sleepLabel())}</span><span class="sub">pause after a while</span></span></button>`
      + (isOffline() ? '' : `<div class="pop-title">Listening together</div><button class="pop-item" id="zpvisible">${icon('artist')}<span><span class="name">Who sees what you play</span><span class="sub">${esc((VISIBLE.find((x) => x[0] === (state.presenceVisible || 'hidden')) || VISIBLE[0])[1])}</span></span></button>`);
  };
  const renderZonePop = (err) => {
    zonePopErr = err || null;
    zonePop.innerHTML = zonePopDrawn = zonePopHtml(zonePopErr);
    // (their menu opens at the button the picker was opened with, which has the focus back)
    const zs = $('#zpsleep', zonePop); if (zs) zs.onclick = () => { const at = zoneAnchor(); closePop(); menuAfterPop(at, [{ title: 'Sleep timer' }, ...[15, 30, 45, 60, 90].map((m) => ({ label: `${m} minutes`, icon: 'moon', run: () => setSleep(m * 60) })), { label: 'Off', icon: 'close', run: () => setSleep(null) }]); };
    const zv = $('#zpvisible', zonePop); if (zv) zv.onclick = () => { const at = zoneAnchor(); closePop(); menuAfterPop(at, [{ title: 'Who sees what you play' }, ...VISIBLE.map(([v, label, note]) => ({ label, note, icon: 'artist', on: (state.presenceVisible || 'hidden') === v, run: () => setVisible(v) }))]); };
    $$('[data-move]', zonePop).forEach((b) => b.onclick = () => { b.disabled = true; b.innerHTML = '<span class="spin"></span> Moving…'; moveTo(b.dataset.move); });
    $$('[data-zone]', zonePop).forEach((b) => b.onclick = () => {
      state.zone = b.dataset.zone; localStorage.setItem('pm.zone', state.zone); state.zonePicked = true;
      closePop(); renderZoneName(); renderBar(); refreshQueue();
    });
  };
  // The open menu follows the zones (a zone that went stayed in it, and a tap on it picked a zone no
  // longer there; one that came was missing; a state word stayed old). Drawn again only when what it
  // shows changed, keeping its error, its scroll and the keyboard's place (the first row when that
  // row went).
  const followZonePop = () => {
    if (zonePop.hidden) return;
    const html = zonePopHtml(zonePopErr);
    if (html === zonePopDrawn) return;
    const f = document.activeElement; const had = zonePop.contains(f) ? f : null;
    const key = had && (had.dataset.zone != null ? `[data-zone="${CSS.escape(had.dataset.zone)}"]` : had.dataset.move != null ? `[data-move="${CSS.escape(had.dataset.move)}"]` : had.id ? `#${CSS.escape(had.id)}` : null);
    const top = zonePop.scrollTop;
    renderZonePop(zonePopErr);
    zonePop.scrollTop = top;
    if (had) { const b = (key && $(key, zonePop)) || $('.pop-item.on', zonePop) || $('.pop-item', zonePop); if (b) b.focus({ preventScroll: true }); }
  };
  $('#zonebtn').onclick = () => {
    if (!zonePop.hidden) { closePop(); return; }
    closeJobPop();
    renderZonePop();
    { const a = document.activeElement; zOpener = a && a.id === 'npzone' ? '#npzone' : '#zonebtn'; }
    zonePop.hidden = false; $('#scrim').hidden = false; pushTransient();
    // The devices' models: the owner's output list, fetched once
    if (isOwner() && !state.outputs) loadOutputs().then(() => { followZonePop(); renderBar(); }).catch(() => {});
  };

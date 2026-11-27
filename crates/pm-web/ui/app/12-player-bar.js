  // ---- player bar ----------------------------------------------------------
  const ORDER = { bit_perfect: 0, enhanced: 1, lossless: 2, lossy: 3 };
  const LABEL = { bit_perfect: 'Bit-perfect', enhanced: 'Enhanced', lossless: 'Lossless', lossy: 'Lossy' };
  const Q_COLOUR = { bit_perfect: 'var(--q-bit)', enhanced: 'var(--q-enh)', lossless: 'var(--q-lossless)', lossy: 'var(--q-lossy)' };
  // The signal path as a block: the headline, then one row per stage (Now playing's Details and
  // the sound panel share it)
  const pathHtml = (np, cls = '') => {
    const h = np.signal_path ? headline(np.signal_path) : null;
    const fmtLine = np.signal_path ? fmtFormat(np.source_format) + ' → ' + fmtFormat(np.output_format) : '';
    return `<div class="path${cls ? ' ' + cls : ''}">
      <div class="path-head"><span class="qdot" style="width:10px;height:10px;border-radius:50%;background:${h ? Q_COLOUR[h] : 'var(--muted)'}"></span>${h ? LABEL[h] : playedBy()}<span class="muted" style="font-weight:400;margin-left:auto">${esc(fmtLine)}</span></div>
      ${np.signal_path ? np.signal_path.stages.map((st) => `<div class="path-row"><span class="pd" style="background:${Q_COLOUR[st.quality]}"></span><span class="pn">${esc(st.name)}</span><span class="pf">${esc(fmtFormat(st.in_format))} → ${esc(fmtFormat(st.out_format))}</span>${st.note ? `<span class="pnote">${esc(st.note)}</span>` : ''}</div>`).join('') : ''}
    </div>`;
  };
  // Stages that are DSP, not conversion: the pill says "DSP" when the path has one
  const DSP_STAGES = ['eq', 'convolution', 'crossfeed', 'loudness', 'headroom', 'volume'];
  const dspOn = (np) => !!(np && np.signal_path && np.signal_path.stages.some((s) => DSP_STAGES.includes(s.name)));
  // "24/96", "DSD64": bits and rate of a format, the short way
  const rateBits = (f) => (!f ? '' : f.kind === 'dsd' ? `DSD${Math.round(f.rate_hz / 44100)}` : `${f.bits}/${kHz(f.rate_hz)}`);
  // The pill's words (owner, 2026-10-29): what arrives, what the zone plays, and the verdict:
  // "FLAC 24/96 → 24/96 · Bit-perfect", "AAC 320 kbps → 16/44.1 · Lossy"
  const pathSummary = (np) => {
    const f = np.format; const sf = np.source_format; const of = np.output_format;
    const h = np.signal_path ? headline(np.signal_path) : null;
    const arrive = f ? (f.lossless || !f.codec ? `${f.codec || ''} ${rateBits(sf)}`.trim() : `${f.codec}${f.bitrate_kbps ? ` ${f.bitrate_kbps} kbps` : ''}`) : rateBits(sf);
    const out = of ? rateBits(of) : '';
    return [arrive + (out ? ` → ${out}` : ''), h ? LABEL[h] : ''].filter(Boolean).join(' · ');
  };
  // What is arriving, as people say it (STEWARD-UX-PLAYER-SOURCES §2): "FLAC 44.1/16 · TIDAL",
  // "AAC 320 kbps · Deezer", "MP3 128 kbps · preview", "FLAC 96/24 · local file", and "→ 48 kHz"
  // when the node converts for the zone. Null when the node doesn't say (older nodes).
  const kHz = (hz) => String(Math.round(hz / 100) / 10);
  const streamLabel = (np, long = false) => {
    const f = np && np.format; if (!f) return null;
    const sf = np.source_format; const of = np.output_format;
    const what = sf && sf.kind === 'dsd' ? `DSD${Math.round(sf.rate_hz / 44100)}`
      : f.lossless && sf ? (long ? `${f.codec} ${kHz(sf.rate_hz)} kHz / ${sf.bits} bit` : `${f.codec} ${kHz(sf.rate_hz)}/${sf.bits}`)
      : `${f.codec}${f.bitrate_kbps ? ` ${f.bitrate_kbps} kbps` : ''}`;
    const tier = f.tier ? ` ${f.tier.charAt(0).toUpperCase()}${f.tier.slice(1)}` : '';
    const from = f.preview ? 'preview' : f.service === 'library' ? 'local file' : f.service === 'stream' ? 'stream' : f.service === 'download' ? 'downloaded' : `${SERVICE_NAMES[f.service] || srcName(f.service)}${tier}`;
    const conv = sf && of && sf.kind !== 'dsd' && of.kind !== 'dsd' ? (of.rate_hz !== sf.rate_hz ? ` → ${kHz(of.rate_hz)} kHz` : f.lossless && of.bits < sf.bits ? ` → ${of.bits} bit` : '') : '';
    return `${what}${conv} · ${from}`;
  };
  // Why the node converts, from the signal path's own notes (resample, requantize, DSD to PCM)
  const headline = (p) => p.stages.reduce((w, s) => (ORDER[s.quality] > ORDER[w] ? s.quality : w), 'bit_perfect');
  const albumIdOf = (it) => { const t = state.trackCache.get(it.uri); return t && t.album ? t.album.id : null; };
  // The artists a credit names ("Sefa, Dr. Peacock" → two links), like the node splits them
  // Credits are identities, not comma-delimited text: a real artist can have
  // commas (or an ampersand) in their name. Only structured library metadata
  // separates people; an unstructured display credit is one safe search link.
  const artistCredits = (it) => {
    const tid = it.track_id ?? trackIdOf(it);
    const known = tid != null && state.trackCache.get('track:' + tid);
    const credits = Array.isArray(it.artists) ? it.artists : known && known.artists;
    // A phone without a node has no library artist pages: its songs' artists are a search there
    const libPages = !phoneMode() || offers('library');
    const libId = (id) => libPages || !/^(lib-)?\d+$/.test(String(id));
    if (Array.isArray(credits) && credits.length) return credits.filter((a) => a && a.name).map((a) => ({ name: a.name, id: a.id != null && /^\d+$/.test(String(a.id)) && libPages ? 'lib-' + a.id : null, join: a.join_phrase ?? a.joinphrase ?? ', ' }));
    return it.artist ? [{ name: it.artist, id: it.artist_id && !/[,;&/]/.test(it.artist) && libId(it.artist_id) ? it.artist_id : null, join: '' }] : [];
  };
  // A song's ⋯ on a phone, where its row's names take no taps or Tab (rowLinkTab): one Go to per
  // credited artist when there are two or more ("Go to Sefa", "Go to Dr. Peacock"); else null and
  // the menu's one Go to artist
  const goArtists = (credits) => (shellMode() === 'phone' && credits && credits.length > 1 ? credits.map((a) => (a.id
    ? { label: `Go to ${a.name}`, icon: 'artist', run: () => setView('item', { kind: 'artist', id: a.id }) }
    : { label: `Search “${a.name}”`, icon: 'artist', run: () => { search.value = a.name; runSearch(); } })) : null);
  // (`tab`: a song row's names on a phone are not Tab stops, rowLinkTab)
  // A library song whose row carries no structured artists (nor has them read yet): its track id, as
  // only such a song is read again (hydrateArtistCredits); else null
  const creditsFlat = (it) => { const tid = it.track_id ?? trackIdOf(it); return tid != null && !Array.isArray(it.artists) && !Array.isArray((state.trackCache.get('track:' + tid) || {}).artists) ? tid : null; };
  const artistLinksHtml = (it, tab = '') => {
    const credits = artistCredits(it); const flat = creditsFlat(it);
    return `<span${flat != null ? ` data-credit-track="${esc(flat)}"` : ''}>${credits.map((a, i) => `<a class="lnk" data-artist-credit="${esc(a.name)}"${a.id ? ` data-credit-id="${esc(a.id)}"` : ''} href="${esc(a.id ? '/artist/' + encodeURIComponent(a.id) : '/search?q=' + encodeURIComponent(a.name))}" title="Open ${esc(a.name)}"${tab}>${esc(a.name)}</a>${i < credits.length - 1 ? esc(a.join) : ''}`).join('')}</span>`;
  };
  // A click the browser takes elsewhere (a new tab or window): Ctrl, Cmd, Shift or not the main button.
  // Not Alt: Alt+click on a link downloads the page in Chrome and Edge, so it stays in the app.
  const opensElsewhere = (e) => e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey;
  // A link with an address (Tab reaches it): a plain click or Enter runs `go` inside the app, a click the
  // browser takes elsewhere opens the address in a new tab
  const hrefAttr = (to) => (to ? ` href="${esc(to)}"` : '');
  const inAppClick = (go) => (e) => { if (opensElsewhere(e)) return; e.preventDefault(); go(e); };
  document.addEventListener('click', (e) => {
    const a = e.target.closest && e.target.closest('[data-artist-credit]');
    if (!a || opensElsewhere(e)) return;
    e.preventDefault(); e.stopPropagation();
    // (the page opened replaces the one the focus came from: it goes to the mini player, as in setView)
    if (!overlay.hidden) { npReturn = null; toggleNowPlaying(); }
    if (a.dataset.creditId) setView('item', { kind: 'artist', id: a.dataset.creditId });
    else { search.value = a.dataset.artistCredit; setView('search', { q: a.dataset.artistCredit, in: 'everywhere' }); }
  }, true);
  // Some older item-page responses carry only the flattened credit plus track_id.
  // Hydrate from the node's existing track API, never from an enrichment service. Rows
  // rendered from structured credits are never read. Bounded: two reads at a time, at most
  // ARTIST_READS distinct songs per page visit, each given up after ARTIST_READ_MS; leaving
  // the page drops the queue and aborts reads in flight. A transient failure (5xx, timeout,
  // network) retries with backoff, ARTIST_TRIES in all; any other refusal keeps the safe
  // unsplit search link. Coming back to the page starts afresh.
  const ARTIST_READS = 48; const ARTIST_READ_MS = 6000; const ARTIST_TRIES = 3;
  const hyd = { queue: [], seen: new Map(), active: new Set(), reading: 0, gen: 0 };
  const creditEls = (id) => $$(`[data-credit-track="${id}"]`);
  const cancelArtistReads = () => {
    hyd.gen++; hyd.queue.length = 0;
    for (const s of hyd.seen.values()) clearTimeout(s.retry);
    hyd.seen.clear();
    for (const c of hyd.active) c.abort();
  };
  // A structured answer worth showing: at least one named artist (else the flattened credit stays)
  const namedCredits = (t) => Array.isArray(t && t.artists) && t.artists.some((a) => a && a.name);
  const showCredits = (id, t) => creditEls(id).forEach((el) => {
    if (el.dataset.pageArtist != null) { showPageCredits(el, t); return; }
    const focused = el.contains(document.activeElement);
    el.insertAdjacentHTML('afterend', artistLinksHtml({ track_id: Number(id), artists: t.artists }, el.closest('.srow:not(.card-row), .track') ? rowLinkTab() : ''));
    const fresh = el.nextElementSibling; el.remove();
    if (focused && fresh) { const a = $('[data-artist-credit]', fresh); if (a) a.focus(); }
  });
  // A song row on an artist page whose flat credit was the page's artist (songRows' pageCredit): the
  // read says who. That artist alone stays unsaid; anyone else (a co-artist, another artist of that
  // name) comes in front of the album, in the row's second line (made when it had none)
  const showPageCredits = (el, t) => {
    const grow = el.parentElement; el.remove();
    if (!grow || soleCredit(artistCredits({ artists: t.artists }), { name: el.dataset.pageArtist, id: el.dataset.pageArtistId || null })) return;
    let sub = grow.querySelector(':scope > .sub');
    if (!sub) { grow.insertAdjacentHTML('beforeend', '<div class="sub"></div>'); sub = grow.lastElementChild; }
    const box = sub.querySelector(':scope > .subtext') || sub; const had = box.textContent.trim() !== '';
    box.insertAdjacentHTML('afterbegin', artistLinksHtml({ artists: t.artists }, grow.closest('.srow:not(.card-row)') ? rowLinkTab() : '') + (had ? ' · ' : ''));
    if (box !== sub && !had) box.insertAdjacentHTML('afterend', '<span aria-hidden="true">·</span>');
  };
  const pumpCredits = () => {
    while (hyd.reading < 2 && hyd.queue.length) {
      const id = hyd.queue.shift(); const s = hyd.seen.get(id);
      if (!s) continue;
      if (!creditEls(id).length) { hyd.seen.delete(id); continue; } // no longer on screen
      const gen = hyd.gen; const ctrl = new AbortController(); let late = false;
      const timer = setTimeout(() => { late = true; ctrl.abort(); }, ARTIST_READ_MS);
      hyd.reading++; hyd.active.add(ctrl); s.tries++;
      api('GET', `/library/tracks/${id}`, undefined, { signal: ctrl.signal }).then((t) => {
        rememberTracks([t]);
        if (gen === hyd.gen && namedCredits(t)) showCredits(id, t); // only the page that asked
      }).catch((e) => {
        if (gen !== hyd.gen || (ctrl.signal.aborted && !late)) return; // left the page: not a failure
        const transient = late || !e.status || e.status >= 500 || e.status === 408 || e.status === 429;
        if (!transient || s.tries >= ARTIST_TRIES) return; // keep the unsplit search link
        s.retry = setTimeout(() => { if (gen === hyd.gen) { hyd.queue.push(id); pumpCredits(); } }, 1000 * 2 ** (s.tries - 1));
      }).finally(() => { clearTimeout(timer); hyd.reading--; hyd.active.delete(ctrl); pumpCredits(); });
    }
  };
  const hydrateArtistCredits = (root) => {
    $$('[data-credit-track]', root).forEach((el) => {
      const id = el.dataset.creditTrack; const known = state.trackCache.get('track:' + id);
      if (known && Array.isArray(known.artists)) { if (namedCredits(known)) showCredits(id, known); return; } // read meanwhile
      if (!/^\d+$/.test(id) || hyd.seen.has(id) || hyd.seen.size >= ARTIST_READS) return;
      hyd.seen.set(id, { tries: 0 }); hyd.queue.push(id);
    });
    pumpCredits();
  };
  // A zone's volume slider: the output's hardware range and the zone's level, or this browser's
  // own volume; null when the device sets it
  // Title, artists and album of what plays, each a link to its page (player bar and Now Playing);
  // `plain` is text only, for a bar that opens Now playing as a whole
  const npLinksHtml = (it, live, plain) => {
    if (plain) return { title: esc(live || it.title || it.uri), artistsHtml: esc(live ? [it.title, it.album].filter(Boolean).join(' · ') : [it.artist, it.album].filter(Boolean).join(' · ')) };
    const songPage = npPageId(it.uri);
    const title = !live && songPage ? `<button class="lnk" data-np-song="${esc(songPage)}" title="Open this song's page">${esc(it.title || it.uri)}</button>` : esc(live || it.title || it.uri);
    const artistsHtml = live ? esc([it.title, it.album].filter(Boolean).join(' · ')) : [artistLinksHtml(it), it.album ? (albumIdOf(it) ? `<button class="lnk" data-np-album="${albumIdOf(it)}" title="Open the album">${esc(it.album)}</button>` : esc(it.album)) : ''].filter(Boolean).join(' · ');
    return { title, artistsHtml };
  };
  const bindNpLinks = (root, it, after) => {
    const go = (fn) => (e) => { e.stopPropagation(); if (after) after(); fn(); };
    $$('[data-np-song]', root).forEach((b) => b.onclick = go(() => setView('item', { kind: 'song', id: b.dataset.npSong })));
    $$('[data-np-album]', root).forEach((b) => b.onclick = go(() => setView('item', { kind: 'album', id: 'lib-' + b.dataset.npAlbum })));
    hydrateArtistCredits(root);
  };
  // The song playing now stands out in every track list on screen
  const markPlaying = () => {
    const z = currentZone(); const np = z && z.now_playing; const m = np && /^track:(\d+)$/.exec(np.item.uri || '');
    const id = m ? m[1] : null;
    $$('#main .track[data-tid]').forEach((r) => { const on = r.dataset.tid === id; r.classList.toggle('cur', on); if (on) r.setAttribute('aria-current', 'true'); else r.removeAttribute('aria-current'); });
    const uri = np ? np.item.uri || '' : '';
    $$('#main .srow[data-np]').forEach((r) => { const on = !!uri && r.dataset.np === uri; r.classList.toggle('cur', on); if (on) r.setAttribute('aria-current', 'true'); else r.removeAttribute('aria-current'); });
    // (a service's own album page: its songs are known by their address, not a library id)
    $$('#main .track[data-np]').forEach((r) => { const on = !!uri && r.dataset.np === uri; r.classList.toggle('cur', on); if (on) r.setAttribute('aria-current', 'true'); else r.removeAttribute('aria-current'); });
  };
  const npUri = () => { const z = currentZone(); return z && z.now_playing ? z.now_playing.item.uri || '' : ''; };
  // The dashboard follows the zone: the whole screen when the song changes, else only the time
  const drawDashboard = () => {
    const box = $('#dash'); if (!box || state.view !== 'dashboard') return;
    const z = currentZone(); const np = z && z.now_playing; const raw = np && np.item;
    // A library song queued by another app may come without its details: the library has them
    const tid = trackIdOf(raw); const known = tid != null ? state.trackCache.get('track:' + tid) : null;
    if (tid != null && !known && !state.dashAsked) { state.dashAsked = tid; api('GET', `/library/tracks/${tid}`).then((t) => { rememberTracks([t]); state.dashKey = null; drawDashboard(); }).catch(() => {}).finally(() => { state.dashAsked = null; }); }
    const it = raw && known ? { ...raw, title: raw.title || known.title, artist: raw.artist || artists(known), album: raw.album || (known.album && known.album.title) } : raw;
    const key = it ? `${z.id}|${it.uri}|${it.title}` : `${z ? z.id : ''}|none`;
    if (key !== state.dashKey) {
      state.dashKey = key;
      box.innerHTML = it ? `${it.image || albumIdOf(it) ? `<div class="dash-bg" aria-hidden="true">${itemArt(it)}</div>` : ''}<div class="dash-inner">${itemArt(it, 'dash-art')}<div class="dash-text"><div class="eyebrow">Now playing on ${esc(z.name)}</div><h1>${esc(np.stream_title || it.title || '')}</h1><p class="dash-artist">${esc(it.artist || '')}</p>${it.album ? `<p class="muted">${esc(it.album)}</p>` : ''}<div class="dash-progress"><span id="dashbar"></span></div><div class="dash-times muted"><span id="dashpos"></span><span id="dashdur"></span></div></div></div><div class="dash-brand muted small">Odeion</div>`
        : `<div class="dash-inner empty"><div class="dash-text"><div class="eyebrow">${esc(z ? z.name : '')}</div><h1>Nothing playing</h1><p class="muted">Start something on this zone and it shows here.</p></div></div><div class="dash-brand muted small">Odeion</div>`;
    }
    if (it) {
      const pos = np.position_ms || 0; const dur = np.duration_ms || 0;
      const bar = $('#dashbar'); if (bar) bar.style.width = `${dur ? Math.min(100, (100 * pos) / dur) : 0}%`;
      const ps = $('#dashpos'); if (ps) ps.textContent = fmt(pos); const ds = $('#dashdur'); if (ds) ds.textContent = dur ? fmt(dur) : '';
    }
  };
  // Colour from where it plays (docs/ux/responsive.md §6). The player surfaces take the source's
  // accent: [light, dark], each checked in its theme (as text on the page ≥ 4.5:1, and the play
  // button's ink on it ≥ 4.5:1). Our own files and the homeserver keep ours. A tint only: no logos,
  // layouts or fonts of theirs, and lists keep the app's theme.
  const SRC_ACCENT = {
    spotify: ['#13803B', '#1ED760'], tidal: ['#007C89', '#33E8E0'], qobuz: ['#1F5BD6', '#6E9BFF'],
    deezer: ['#9B35F4', '#B35CFF'], youtube: ['#E00000', '#FF3232'], soundcloud: ['#C94300', '#FF5500'],
    bandcamp: ['#167A94', '#1DA0C3'], plex: ['#936608', '#E5A00D'], jellyfin: ['#9852AF', '#B26BC8'],
    jamendo: ['#CE364D', '#F24560'], audius: ['#C00ED3', '#D741E7'], archive: ['#3774A9', '#428BCA'], subsonic: ['#086ED5', '#0A84FF'],
  };
  const colourOn = (k) => { try { return localStorage.getItem(k) !== 'off'; } catch { return true; } };
  const svcOfItem = (it) => {
    const u = String((it && it.uri) || ''); const m = /^([a-z]+)[:-]/.exec(u);
    if (m && SRC_ACCENT[m[1]]) return m[1];
    if (/^https?:/.test(u)) { try { const k = srcSvc(new URL(u).hostname); return SRC_ACCENT[k] ? k : null; } catch { return null; } }
    return null;
  };
  let lastSrc;
  const setSourceColour = (np) => {
    const root = document.documentElement;
    // the node's own word on the service once now-playing carries it (#94), else the item's uri
    const svc = np && colourOn('pm.srcColour') ? (np.format && np.format.service ? srcSvc(np.format.service) : svcOfItem(np.item)) : null;
    if (svc === lastSrc) return; // renderBar runs several times a second: touch <html> only on a change
    lastSrc = svc;
    const a = svc && SRC_ACCENT[svc];
    if (a) { root.dataset.src = svc; root.style.setProperty('--src-l', a[0]); root.style.setProperty('--src-d', a[1]); }
    else { delete root.dataset.src; root.style.removeProperty('--src-l'); root.style.removeProperty('--src-d'); }
  };
  // Now playing's background: a soft gradient from the cover (its average colour, read through a small
  // canvas; a cover from another site that does not allow that gets none)
  const artTints = new Map();
  const setArtTint = () => {
    const root = document.documentElement; const img = $('#nowplaying .np-big img');
    const clear = () => { delete root.dataset.art; root.style.removeProperty('--np-h'); root.style.removeProperty('--np-s'); };
    if (!img || !colourOn('pm.artColour')) { clear(); return; }
    const src = img.currentSrc || img.src;
    const apply = (c) => { if (c) { root.dataset.art = '1'; root.style.setProperty('--np-h', String(c[0])); root.style.setProperty('--np-s', `${c[1]}%`); } else clear(); };
    if (artTints.has(src)) { const c = artTints.get(src); if (c === 'pending') clear(); else apply(c); return; }
    clear(); artTints.set(src, 'pending');
    if (artTints.size > 200) artTints.delete(artTints.keys().next().value);
    const probe = new Image(); probe.crossOrigin = 'anonymous';
    probe.onload = () => {
      let c = null;
      try {
        const cv = document.createElement('canvas'); cv.width = 12; cv.height = 12; const g = cv.getContext('2d');
        g.drawImage(probe, 0, 0, 12, 12); const d = g.getImageData(0, 0, 12, 12).data;
        let r = 0; let gr = 0; let b = 0; let n = 0;
        for (let i = 0; i < d.length; i += 4) { const hi = Math.max(d[i], d[i + 1], d[i + 2]); const lo = Math.min(d[i], d[i + 1], d[i + 2]); if (hi < 24 || lo > 235) continue; r += d[i]; gr += d[i + 1]; b += d[i + 2]; n += 1; }
        if (n) {
          // hue and saturation only: the lightness is set per theme, so text on it keeps its contrast
          const R = r / n / 255; const G = gr / n / 255; const B = b / n / 255; const hi = Math.max(R, G, B); const lo = Math.min(R, G, B); const l = (hi + lo) / 2; const dl = hi - lo;
          const sat = dl ? dl / (1 - Math.abs(2 * l - 1)) : 0;
          const hue = !dl ? 0 : hi === R ? 60 * (((G - B) / dl) % 6) : hi === G ? 60 * ((B - R) / dl + 2) : 60 * ((R - G) / dl + 4);
          c = [Math.round((hue + 360) % 360), Math.round(Math.min(1, sat) * 100)];
        }
      } catch { c = null; }
      artTints.set(src, c);
      const now = $('#nowplaying .np-big img'); if (now && (now.currentSrc || now.src) === src) apply(c);
    };
    probe.onerror = () => { artTints.set(src, null); const now = $('#nowplaying .np-big img'); if (now && (now.currentSrc || now.src) === src) clear(); };
    probe.src = src;
  };
  // The phone mini player's progress: a 2 px line on its top edge
  const miniProgress = (f) => { const el = $('#miniprog'); if (el) el.style.setProperty('--p', `${Math.max(0, Math.min(1, f || 0)) * 100}%`); };
  // The bar is drawn on every event of its zone (four a second while it plays) and after most
  // presses. Each part is written only when what it shows has changed (`put`: the last value written
  // is kept on the element; only this function writes them), so between two songs the mini player's
  // cover, title and artists stay the same elements (a keyboard's place on them stays too), and a tick
  // writes the time and the seek bar and nothing else. The playing song's mark in the page's lists:
  // here when the song changes, and as rows are drawn (the watch on the page, where `main` is).
  const put = (el, key, val, write) => { if (!el || el[key] === val) return false; el[key] = val; write(val); return true; };
  const putText = (el, s) => put(el, '_barText', s, (v) => { el.textContent = v; });
  const putHtml = (el, s) => put(el, '_barHtml', s, (v) => { el.innerHTML = v; });
  let markedUri = null;
  const renderBar = () => {
    const z = currentZone(); const np = z && z.now_playing;
    { const cur = `${state.zone} ${np ? np.item.uri || '' : ''}`; if (cur !== markedUri) { markedUri = cur; markPlaying(); } }
    renderModes(true); if (state.view === 'dashboard') drawDashboard();
    setSourceColour(np);
    const playing = z && z.transport === 'playing';
    { const tg = $('#toggle'); if (put(tg, '_barIcon', playing ? 'pause' : 'play', (v) => { tg.innerHTML = icon(v); })) tg.setAttribute('aria-label', playing ? 'Pause' : 'Play'); }
    const q = $('#quality'); const npEl = $('#np');
    // (the pill's parts, each written when it changes)
    const pill = (cls, src, words, dsp, short, label, title) => {
      put(q, '_barCls', cls, (v) => { q.className = v; }); putHtml($('.qsrc', q), src); putText($('.qtext', q), words);
      { const d = $('.qdsp', q); put(d, '_barHidden', !dsp, (v) => { d.hidden = v; }); }
      putHtml($('.qshort', q), short); put(q, '_barLabel', label, (v) => { q.setAttribute('aria-label', v); }); put(q, '_barTitle', title, (v) => { q.title = v; });
    };
    // Pressing play answers at once (owner, 2026-10-29): "Starting on <device>…" until the zone
    // plays, or what went wrong, for a few seconds
    const st = state.starting && state.starting.zone === state.zone ? state.starting : null;
    const se = !st && state.startError && state.startError.zone === state.zone && Date.now() - state.startError.at < 10000 ? state.startError : null;
    // (the spinner sits in the cover's square, centred: inline in the title it was cut to an arc)
    const waitHtml = st ? `<div class="np-art placeholder np-art-wait"><span class="spin" aria-hidden="true"></span></div><div class="np-text"><div class="np-title np-wait" role="status">${st.what === 'move' ? 'Moving to' : 'Starting on'} ${esc(z ? z.name : 'the zone')}…</div><div class="np-sub">${esc(st.title || '')}</div></div>` : '';
    const errHtml = se ? `<div class="np-art placeholder"></div><div class="np-text"><div class="np-title">Couldn’t start</div><div class="np-sub"><span class="warn-text" role="alert" title="${esc(se.msg)}">${esc(se.msg)}</span></div></div>` : '';
    if (!np && !state.zonesSeen && !state.zonePicked && barKept().title && !waitHtml && !errHtml) {
      // The node hasn't said yet: the song the bar showed last stays (index.html drew it)
      const k = barKept();
      putHtml(npEl, `<div class="np-art placeholder"></div><div class="np-text"><div class="np-title">${esc(k.title)}</div><div class="np-sub">${esc(k.sub || '')}</div></div>`);
      return;
    }
    if (z && state.zonesSeen) barKeep(np ? { title: np.item.title || '', sub: np.item.artist || '' } : { title: '', sub: '' });
    if (!np) {
      // A zone that stopped on an error says so here, until the next track starts (the node clears it)
      const err = z && z.transport === 'stopped' && z.error;
      putHtml(npEl, waitHtml || errHtml || `<div class="np-art placeholder"></div><div class="np-text"><div class="np-title muted">${err ? 'Stopped' : 'Nothing playing'}</div><div class="np-sub">${err ? `<span class="warn-text" role="alert" title="${esc(z.error)}">Couldn’t play on ${esc(z.name)}: ${esc(z.error)}</span>` : esc(z ? z.name : '')}</div></div>`);
      put(npEl, '_barName', 'Now playing', (v) => { npEl.setAttribute('aria-label', v); });
      putText($('#pos'), '0:00'); putText($('#dur'), '0:00'); seek.value = 0; setFill(seek); miniProgress(0); put(seek, '_barWords', '', () => { seek.removeAttribute('aria-valuetext'); });
      pill('quality idle', '', '—', false, `<span class="qdev">${esc(z ? z.name : '')}</span>`, `Sound. Nothing playing on ${z ? z.name : 'this zone'}. Open the sound panel`, 'Sound: where it plays from, how it gets to you, and DSP');
      syncQPanel();
      return;
    }
    const it = np.item;
    const live = np.stream_title;
    const links = npLinksHtml(it, live, barPlain());
    // A track change cross-fades the mini player
    const wasUri = npEl.dataset.uri || ''; if (wasUri !== (it.uri || '')) npEl.dataset.uri = it.uri || '';
    if (wasUri && wasUri !== (it.uri || '') && !waitHtml && !errHtml) { npEl.classList.remove('swap'); void npEl.offsetWidth; npEl.classList.add('swap'); clearTimeout(npEl.swapT); npEl.swapT = setTimeout(() => npEl.classList.remove('swap'), 500); }
    // (the same song, shown the same way, with the same word on a pause: the elements stay; a link
    // whose artist could not be read yet is asked for again, as each draw did)
    const npHtml = waitHtml || errHtml || `<button class="np-art np-artbtn" id="npart" title="Now playing" aria-label="Open Now playing">${itemArt(it, 'sm', npSvc(np), true)}</button><div class="np-text"><div class="np-title">${links.title}</div><div class="np-sub">${links.artistsHtml}</div></div>`;
    const pausedBy = !waitHtml && !errHtml && z.transport === 'paused' && z.error ? z.error : '';
    const linked = !waitHtml && !errHtml && !barPlain();
    // (its name is the song, not only "Now playing": the label replaces what is inside it for a screen reader)
    put(npEl, '_barName', waitHtml || errHtml ? 'Now playing' : ((w) => (w ? `Now playing: ${w}` : 'Now playing'))([live || it.title, it.artist].filter(Boolean).join(', ')), (v) => { npEl.setAttribute('aria-label', v); });
    if (put(npEl, '_barHtml', `${npHtml}\n${pausedBy}\n${linked}`, () => { npEl.innerHTML = npHtml; })) {
      // On a touch screen the whole bar opens Now playing (the song page is one tap further, from
      // its title there); with a mouse the bar's title and artist are links of their own
      if (linked) bindNpLinks(npEl, it, () => { if (!overlay.hidden) toggleNowPlaying(); });
      // Paused because its device went away (a USB DAC unplugged): said where the artist is
      if (pausedBy) { const sub = $('.np-sub', npEl); if (sub) sub.innerHTML = `<span class="warn-text" role="alert">${esc(pausedBy)}</span>`; }
    } else if (linked && !pausedBy) hydrateArtistCredits(npEl);
    putText($('#pos'), fmt(np.position_ms)); putText($('#dur'), fmt(np.duration_ms));
    // (the seek bar read out as a time, not as a number of a thousand)
    if (!seek.dataset.dragging) put(seek, '_barWords', `${fmt(np.position_ms)} of ${fmt(np.duration_ms)}`, (v) => { seek.setAttribute('aria-valuetext', v); });
    if (!seek.dataset.dragging) { seek.value = np.duration_ms ? Math.round((np.position_ms / np.duration_ms) * 1000) : 0; setFill(seek); }
    miniProgress(np.duration_ms ? np.position_ms / np.duration_ms : 0);
    // The pill (owner, 2026-10-29): where it plays from (the provider's mark), the path in short
    // and whether DSP is on; on phones the device and a quality badge (CSS swaps .qtext for .qshort)
    const zp = zoneNp(z, np);
    const h = zp.signal_path ? headline(zp.signal_path) : null;
    const dsp = dspOn(np);
    const svc = npSvc(np); const svcName = svc === 'library' ? 'your library' : svc ? srcName(svc) : null;
    const dv = deviceOf(z);
    const words = (np.signal_path || np.format ? pathSummary(zp) : playedBy()) + (dv ? ` → ${dacWordsFor(z)}` : '');
    const badge = np.output_format ? rateBits(np.output_format) : np.format ? (np.format.lossless ? 'Lossless' : np.format.codec || '') : '';
    pill('quality ' + (h ? 'q-' + h : '') + (dsp ? ' dsp' : ''), svc ? svcIcon(svc) : '', words, dsp, `<span class="qdev">${esc(z.name)}</span>${badge ? `<span class="qbadge">${esc(badge)}</span>` : ''}`,
      `Sound: ${svcName ? `from ${svcName}, ` : ''}${words}${dsp ? ', DSP on' : ''}. Playing on ${z.name}. Open the sound panel`,
      zp.signal_path ? zp.signal_path.stages.map((s) => `${s.name}: ${fmtFormat(s.in_format)} → ${fmtFormat(s.out_format)} (${s.quality})${s.note ? ' · ' + s.note : ''}`).join('\n') : decodeNote());
    syncQPanel();
  };
  // ---- immediate feedback on play (owner, 2026-10-29) ----
  // The play control just pressed: it shows a spinner until the music starts (capture phase, so
  // every play button on every page counts without each call site knowing)
  // The latest press wins: any other control pressed since (a ⋯ and its menu's rows, such as Start
  // radio) takes the press away, so the start it makes spins no Play pressed just before. (A click
  // on an empty spot leaves it: an album tile's Play still spins when its songs come.)
  // A song row's Play, found again when its row is drawn anew while its answer comes (a picker still
  // to open): the same list (its section's id), the same song (the row's data-np or data-uri) and the
  // same kind of Play. A Play without one, or a key that finds no Play or more than one, spins
  // nothing (not a button no longer on screen)
  const PRESS_ROW = '[data-np], [data-uri]';
  const pressKey = (b) => {
    const attr = ['data-splay', 'data-dplay'].find((a) => b.hasAttribute(a)); const row = attr && b.closest(PRESS_ROW);
    const song = row && (row.dataset.np || row.dataset.uri); const sec = row && row.closest('[id]');
    return song && sec ? { attr, sec: sec.id, song } : null;
  };
  const pressEl = (p) => {
    if (p.el.isConnected || !p.key) return p.el.isConnected ? p.el : null;
    const sec = document.getElementById(p.key.sec);
    const found = sec ? [...sec.querySelectorAll(`[${p.key.attr}]`)].filter((b) => { const r = b.closest(PRESS_ROW); return r && (r.dataset.np || r.dataset.uri) === p.key.song; }) : [];
    return found.length === 1 ? found[0] : null;
  };
  // (`pressBefore`: the press as it stood just before this click, for a picker's pick below)
  document.addEventListener('click', (e) => { if (!e.target.closest) return; state.pressBefore = state.startAnchor; const b = e.target.closest('[data-act="play"], [data-splay], [data-tileplay], [data-dplay], [data-aplay], [data-vplay], #iplay, #ishuffle, #toggle, [data-npt="toggle"]'); if (b) state.startAnchor = { el: b, at: Date.now(), key: pressKey(b) }; else if (e.target.closest('button, a, input, select, summary, label, [role="button"], [role="menuitem"], .clickable')) state.startAnchor = null; }, true);
  // A picker a Play opens to finish it (the 30 s preview of a song not on your services): its pick
  // goes on with that Play's press, so the Play pressed spins while the music starts, as with no
  // picker (the pick's own click took the press away). Wrap the picker's run with it as the picker
  // opens; a press already gone (another control pressed since, or 1.5 s past) stays gone, and the
  // pick lasts the 1.5 s from when it is made. Only the press of that picker's own Play (`anchor`):
  // another Play pressed while this one's answer was still coming is not this picker's. And only
  // when no newer press came while the picker was open (Space, another Play): the press stood just
  // so up to the pick's own click. (Only on This browser: a preview on a speaker zone,
  // and the other pickers a Play opens, YouTube's Play in this browser and the Spotify device list,
  // start no wait, so nothing spins for them, with or without a picker.)
  const pressFor = (run, anchor) => { const p = state.startAnchor && state.startAnchor.el === anchor && Date.now() - state.startAnchor.at < 1500 ? state.startAnchor : null; return (...a) => { if (p && (state.startAnchor === p || state.pressBefore === p)) state.startAnchor = { ...p, at: Date.now() }; return run(...a); }; };
  // `replace`: the queue is being replaced, so the song still playing does not end the wait.
  // `press`: a press of a play control starts this. A start nobody pressed Play for (Move here,
  // Switch source, a song ending or skipped, the playing song taken out of the queue) puts no
  // spinner on a Play pressed just before. It leaves that press alone: a Play whose start comes
  // after a wait (an album tile's songs still loading) still spins when the song before ended
  // in that wait.
  const startWait = (what, zoneId = state.zone, title = '', replace = false, deferred = false, press = true) => {
    startDone(true);
    const a = press && state.startAnchor && Date.now() - state.startAnchor.at < 1500 ? pressEl(state.startAnchor) : null;
    if (a) a.classList.add('starting');
    const zw = state.zones[zoneId]; const oldItem = replace && zw && zw.now_playing ? zw.now_playing.item.id : null;
    // `sent`: a replace waits for the node's answer first, so the old queue moving on by itself
    // (its song ended) isn't taken for the new one starting
    const at = Date.now(); state.starting = { zone: zoneId, what, title, at, anchor: a, oldItem, sent: !replace, deferred }; state.startError = null;
    // (a start waiting its turn behind the zone's other changes: its 20 s run from when its request goes)
    if (!deferred) armStart(at, zoneId);
    renderBar();
    return at;
  };
  const armStart = (at, zoneId) => setTimeout(() => { if (state.starting && state.starting.at === at) { const zn = state.zones[zoneId]; startFail(`${zn ? zn.name : 'The zone'} did not start playing in 20 seconds`); } }, 20000);
  // The node took the replace: from now on the zone playing another song ends the wait (it may
  // already be playing it)
  // The song a replace replaces, as it is when the request goes (a start queued behind others: the
  // song in hand when it was pressed may have changed meanwhile, and ending the wait on it would be early)
  const startSending = (wait, zid) => { const s = state.starting; if (!s || s.at !== wait) return; const z = state.zones[zid]; s.oldItem = z && z.now_playing ? z.now_playing.item.id : null; if (s.deferred) { s.deferred = false; armStart(wait, zid); } };
  // (`wait`, from startWait: a start another start replaced meanwhile is not this one)
  const startSent = (wait) => {
    const s = state.starting; if (!s || s.sent || (wait && s.at !== wait)) return; s.sent = true;
    const z = state.zones[s.zone]; if (z && z.transport === 'playing' && z.now_playing && z.now_playing.item.id !== s.oldItem) startDone();
  };
  const startDone = (quiet) => { const s = state.starting; if (!s) return; if (s.anchor) s.anchor.classList.remove('starting'); state.starting = null; if (!quiet) renderBar(); if (s.what === 'move' && !zonePop.hidden) renderZonePop(); };
  const startFail = (msg, quiet, wait) => {
    const s = state.starting;
    if (wait && (!s || s.at !== wait)) { if (!quiet) toast(msg || 'something went wrong', 'error'); return; }
    if (!s) return; startDone(true);
    // The song still playing (a refused replace leaves it on) doesn't clear the error: a new one does
    const zf = state.zones[s.zone]; const still = zf && zf.transport === 'playing' && zf.now_playing ? zf.now_playing.item.id : null;
    state.startError = { zone: s.zone, msg: msg || 'something went wrong', at: Date.now(), still };
    if (!quiet) toast(`${(state.zones[s.zone] || {}).name || (s.zone === BROWSER ? hereName() : 'The zone')}: ${state.startError.msg}`, 'error');
    renderBar(); setTimeout(renderBar, 10500);
  };
  // The browser's own player: it starts (or fails) by itself
  audio.addEventListener('playing', () => { if (state.starting && state.starting.zone === BROWSER) startDone(); if (state.startError && state.startError.zone === BROWSER) { state.startError = null; renderBar(); } });
  audio.addEventListener('pause', () => { if (state.starting && state.starting.zone === BROWSER) startDone(); });
  // The first Tab stop: past the menu and the top bar, to the page
  { const sk = $('#skip'); if (sk) sk.onclick = () => { main.focus({ preventScroll: true }); }; }
  $('#np').onclick = () => toggleNowPlaying();
  // (Enter or Space on the bar itself; on its title, artist or cover the key is that control's own.
  // A link has no Space of its own: there it opens Now playing, as it did)
  $('#np').onkeydown = (e) => { const inner = e.target !== e.currentTarget; if (inner && !(e.key === ' ' && e.target.closest('a'))) return; if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); toggleNowPlaying(); } };
  // The quality sheet needs to know what is streaming (now_playing.format); without it (This browser,
  // an older node) the label opens Now playing, where the signal path is
  // ---- the sound panel (owner, 2026-10-29): the pill in full ----
  // Zone and device, the source, the signal path, this zone's DSP with quick switches, volume.
  const qPanel = $('#qpanel');
  let qPanelKey = null;
  let qOpener = '#quality'; // the button that opened the Sound panel gets the focus back
  const closeQPanel = (fromBack) => { if (qPanel.hidden) return; const had = focusWasIn(qPanel); qPanel.hidden = true; $('#scrim').hidden = true; $('#quality').setAttribute('aria-expanded', 'false'); $('#volbtn').setAttribute('aria-expanded', 'false'); const op = $(qOpener); if (op) op.setAttribute('aria-expanded', 'false'); focusBack(had, op); qOpener = '#quality'; if (fromBack !== true) popTransient(); };
  const qpKey = () => { const z = currentZone(); const np = z && z.now_playing; const cfg = state.dspCfg && state.dspCfg[state.zone]; return `${state.zone}|${z ? z.transport : ''}|${z && z.error ? z.error : ''}|${np ? np.item.id : ''}|${np && np.signal_path ? np.signal_path.stages.length : ''}|${cfg ? JSON.stringify(cfg) : ''}|${state.outputs ? 1 : 0}|${z ? dacLine(z) : ''}`; };
  // Re-render the open panel only when what it says changed (renderBar runs often while playing)
  const syncQPanel = () => { if (qPanel.hidden) return; const k = qpKey(); if (k !== qPanelKey) renderQPanel(); };
  const loadDsp = async () => { if (isBrowser()) return; const zone = state.zone; try { const cfg = await api('GET', `/zones/${encodeURIComponent(zone)}/dsp`); (state.dspCfg = state.dspCfg || {})[zone] = cfg; } catch (e) { (state.dspCfg = state.dspCfg || {})[zone] = { error: e.message }; } syncQPanel(); };
  // A quick switch: the whole config goes back with one part changed (what was set stays set)
  // One switch at a time (a second one is built on the first's answer), and nothing set is lost:
  // EQ remembers which bands were on (kept in this browser per zone), crossfeed and room correction
  // switch their own `enabled` and keep their values.
  const eqKey = (zone) => `pm.eqOn.${zone}`;
  const toggleDsp = async (what, on) => {
    if (state.dspBusy) return;
    state.dspBusy = true; state.dspFocus = what;
    $$('[data-dsp]', qPanel).forEach((c) => { c.disabled = true; });
    try {
    const zone = state.zone; const cfg = { ...((state.dspCfg || {})[zone] || {}) }; delete cfg.error;
    if (what === 'eq') {
      const bands = cfg.eq || [];
      if (!on) { try { localStorage.setItem(eqKey(zone), JSON.stringify(bands.map((b) => b.enabled !== false))); } catch { /* no storage: all come back on */ } }
      let was = null; if (on) { try { was = JSON.parse(localStorage.getItem(eqKey(zone)) || 'null'); } catch { was = null; } }
      const back = Array.isArray(was) && was.length === bands.length && was.some(Boolean) ? was : null;
      cfg.eq = bands.map((b, i) => ({ ...b, enabled: on ? (back ? back[i] : true) : false }));
    }
    if (what === 'room' && cfg.convolution) cfg.convolution = { ...cfg.convolution, enabled: on };
    if (what === 'loudness') { if (!on && cfg.loudness && cfg.loudness !== 'off') state.loudnessLast = cfg.loudness; cfg.loudness = on ? (state.loudnessLast || 'album') : 'off'; }
    if (what === 'crossfeed') cfg.crossfeed = cfg.crossfeed ? { ...cfg.crossfeed, enabled: on } : on ? { level_db: -6, cutoff_hz: 700, delay_us: 300, enabled: true } : null;
    try { await api('PUT', `/zones/${encodeURIComponent(zone)}/dsp`, cfg); (state.dspCfg = state.dspCfg || {})[zone] = cfg; toast('Applies from the next song'); } catch (e) { toast(e.message, 'error'); }
    await loadDsp();
    } finally { state.dspBusy = false; qPanelKey = null; syncQPanel(); }
  };
  const renderQPanel = () => {
    const z = currentZone(); if (!z) return; const np = z.now_playing; const cfg = (state.dspCfg || {})[state.zone];
    // A re-render (the outputs arrived, a switch answered) keeps the keyboard where it was
    const ae = document.activeElement; let had = qPanel.contains(ae) ? (ae.id ? '#' + ae.id : ae.dataset && ae.dataset.dsp ? `[data-dsp="${ae.dataset.dsp}"]` : null) : null;
    // A switch that answered gets the keyboard back (disabling it while it saved dropped the focus)
    if (!had && state.dspFocus && !state.dspBusy) { had = `[data-dsp="${state.dspFocus}"]`; state.dspFocus = null; }
    const svc = np && npSvc(np);
    const row = (id, label, on, disabled, note) => `<label class="qp-row${disabled ? ' off' : ''}"><span class="grow"><span class="name">${label}</span>${note ? `<span class="sub">${esc(note)}</span>` : ''}</span><input type="checkbox" class="switch" data-dsp="${id}" ${on ? 'checked' : ''} ${disabled ? 'disabled' : ''} aria-label="${esc(label)}"></label>`;
    let dsp;
    if (isBrowser()) dsp = `<p class="qp-note">${hereName()} plays the file as it is: the node’s DSP is for its zones.</p>`;
    else if (!cfg) dsp = '<p class="qp-note"><span class="spin"></span> Reading this zone’s DSP…</p>';
    else if (cfg.error) dsp = `<p class="qp-note">${esc(cfg.error)}</p>`;
    else {
      const eq = cfg.eq || []; const eqOn = eq.some((b) => b.enabled !== false); const conv = cfg.convolution; const cf = cfg.crossfeed; const ro = !isOwner();
      dsp = row('eq', 'Parametric EQ', eq.length > 0 && eqOn, ro || !eq.length, eq.length ? countOf(eq.length, 'bands') : 'no bands set')
        + row('room', 'Room correction', !!(conv && conv.enabled !== false), ro || !conv, conv ? String(conv.path).split('/').pop() : 'no impulse response set')
        + row('loudness', 'Volume leveling', !!cfg.loudness && cfg.loudness !== 'off', ro, cfg.loudness && cfg.loudness !== 'off' ? `ReplayGain by ${cfg.loudness}` : 'ReplayGain off')
        + row('crossfeed', 'Headphone crossfeed', !!(cf && cf.enabled !== false), ro, cf && cf.enabled !== false ? `${cf.level_db} dB below ${cf.cutoff_hz} Hz` : 'off')
        + (isOwner() ? `<button class="btn sm ghost" id="qpdsp">${icon('settings')} All DSP settings</button>` : '');
    }
    const srcLabel = np ? (svc === 'library' ? 'Your library' : svc ? srcName(svc) : (np.stream_title ? 'Live stream' : 'Unknown source')) : 'Nothing playing';
    qPanel.innerHTML = `<div class="pop-title">Playing on</div>
      <div class="qp-zone">${icon('zone')}<span class="grow"><span class="name">${esc(z.name)}</span><span class="sub" title="${esc(z.id === BROWSER ? '' : z.output)}">${esc(deviceOf(z) ? dacLine(z) : deviceWords(z))}</span></span><span class="state">${zoneStateText(z)}</span></div>
      <div class="pop-title">Source</div>
      <div class="qp-src">${svc ? `<span class="src-badge ico" aria-hidden="true">${svcIcon(svc)}</span>` : ''}<span class="grow"><span class="name">${esc(srcLabel)}</span>${np ? `<span class="sub">${esc(streamLabel(np, true) || (isBrowser() ? decodedBy() : ''))}</span>` : ''}</span></div>
      <div class="pop-title">Signal path</div>
      ${np && np.signal_path ? pathHtml(zoneNp(z, np), 'compact') : `<p class="qp-note">${isBrowser() ? decodeNote() : 'Nothing playing.'}</p>`}
      <div class="pop-title">DSP · ${esc(z.name)}</div>
      <div class="qp-dsp">${dsp}</div>`;
    qPanelKey = qpKey();
    $$('[data-dsp]', qPanel).forEach((c) => { if (state.dspBusy) c.disabled = true; c.onchange = () => toggleDsp(c.dataset.dsp, c.checked); });
    const qd = $('#qpdsp', qPanel); if (qd) qd.onclick = () => { closeQPanel(); state.dspOpen = state.zone; setView('settings', 'playback'); };
    if (had) { const back = $(had, qPanel); if (back) back.focus(); }
  };
  // Opened from `opener` (the pill or the volume button): only that one says it is expanded, and
  // the focus goes back to it on close
  const openQPanel = (opener) => {
    closePop(); closeJobPop();
    renderQPanel();
    qPanel.hidden = false; $('#scrim').hidden = false; qOpener = opener; { const op = $(opener); if (op) op.setAttribute('aria-expanded', 'true'); } pushTransient();
    loadDsp();
    if (isOwner() && !state.outputs) loadOutputs().then(() => syncQPanel()).catch(() => {});
    // Its first control; with none (the in-page player: no DSP, no volume here) the panel itself,
    // so Escape and the arrows work and a screen reader lands in it
    const first = $('[data-dsp]:not(:disabled), button', qPanel) || qPanel; first.focus();
  };
  $('#quality').onclick = () => {
    if (!qPanel.hidden) { closeQPanel(); return; }
    openQPanel('#quality');
  };
  // Where the bar has no room for the slider (narrow tablets and desktops): a small volume popover
  $('#volbtn').onclick = () => { if (!volPop().hidden) closeVolPop(); else openVolPop(); };

  // Now Playing's Pubky row: the heart, your tags on this song (added here), and its full page
  // The heart in the title row and the Tags sheet, for the song on screen. The heart draws at once
  // from the known favourites; an answer that comes back after a newer render is dropped.
  let npGen = 0;
  // Now Playing's heart painted again from the known favourites (a heart pressed elsewhere)
  let npHeartPaint = () => {};
  const fillNpPubky = async (it, gen = npGen) => {
    const live = () => gen === npGen;
    const tid = it.track_id ?? (/^track:(\d+)$/.exec(it.uri || '') || [])[1];
    const songPage = npPageId(it.uri);
    const pageUri = songPage && !/^lib-/.test(songPage) ? uriOf('song', songPage) : null;
    let t = null;
    const favNow = () => (t ? isFav(t) : tid != null ? state.favUris.has(`track:${tid}`) : pageUri ? state.favUris.has(pageUri) : false);
    let fav = favNow();
    const ref = () => (tid != null ? { track_id: Number(tid) } : pageUri ? { uri: pageUri } : null);
    const drawHeart = () => {
      const h = $('#npheart', overlay); if (!h || !live()) return;
      if (!ref() || !offers('bookmarks')) { h.innerHTML = ''; return; }
      const word = fav ? 'In Bookmarks' : 'Add to Bookmarks';
      const had = document.activeElement && document.activeElement.id === 'npfav';
      h.innerHTML = `<button class="icon-btn np-heart ${fav ? 'on' : ''}" id="npfav" aria-pressed="${fav}" aria-label="${word}" title="${word}">${icon(fav ? 'heart-fill' : 'heart')}</button>`;
      $('#npfav', overlay).onclick = async () => { const want = !fav; try { if (await setFavourite(ref(), want)) { fav = want; drawHeart(); } } catch (e) { toast(e.message, 'error'); } };
      if (had) $('#npfav', overlay).focus({ preventScroll: true });
      npRestore();
    };
    drawHeart();
    npHeartPaint = () => { if (!live()) return; const on = favNow(); if (on !== fav) { fav = on; drawHeart(); } };
    if (tid != null) { try { t = await api('GET', `/library/tracks/${tid}`); } catch { t = null; } if (!live()) return; if (t) { fav = isFav(t); drawHeart(); } }
    if (!$('#nppubky', overlay)) return;
    const target = t ? { track_id: Number(tid) } : pageUri ? { uri: pageUri } : null;
    const pageRow = songPage && !$('.np-bsheet', overlay) ? `<div class="np-pubky-row"><button class="btn ghost sm" id="nppage">${icon('compass')} Go to song page</button></div>` : '';
    let tags = [];
    if (target) { try { tags = (await api('GET', `/tags?${new URLSearchParams(Object.entries(target).map(([k, v]) => [k, String(v)]))}`)).tags || []; } catch { tags = []; } }
    const box = $('#nppubky', overlay); if (!box || !live()) return;
    box.innerHTML = pageRow + (target
      ? `<div class="np-tags"><span class="eyebrow">Tags</span>${tags.map((x) => `${tagChip(x.label, { own: true, attrs: `data-np-tag="${esc(x.label)}"`, title: `Your tag · open ${x.label}` })}${x.pending ? '<span class="muted small tag-wait" title="Published once this file is matched to a song">waits for a match</span>' : ''}`).join('')}<input class="input taginput" id="nptagadd" placeholder="Add a tag…" maxlength="20" aria-label="Add a tag to this song"></div>`
      : t ? '<p class="muted small">Pick which song this is (the ? on its row) to tag it and keep it in collections.</p>'
        : `<p class="muted">Nothing to tag here: ${it.stream || np2Stream(it) ? 'this is a live stream' : 'this song has no page yet'}.</p>`);
    const pb = $('#nppage', box); if (pb) pb.onclick = () => setView('item', { kind: 'song', id: songPage });
    $$('[data-np-tag]', box).forEach((x) => x.onclick = () => setView('tag', x.dataset.npTag));
    const ta = $('#nptagadd', overlay);
    if (ta) {
      // A half-typed tag survives re-renders while the song stays the same
      if (state.npTagDraft && state.npTagDraft.id === it.id) ta.value = state.npTagDraft.value;
      ta.oninput = () => { state.npTagDraft = { id: it.id, value: ta.value }; };
      npRestore();
      ta.onkeydown = async (e) => { if (e.key !== 'Enter' || !ta.value.trim()) return; try { await api('PUT', '/tags', { ...target, label: ta.value.trim() }); toast(`Tagged #${ta.value.trim().toLowerCase()}`); state.npTagDraft = null; fillNpPubky(it); } catch (er) { toast(er.message, 'error'); } };
    }
  };
  const np2Stream = (it) => !!(it && it.album === 'Internet radio');

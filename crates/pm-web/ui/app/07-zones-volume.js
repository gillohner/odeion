  // The YouTube panel: YouTube's terms want its player visible, so it shows while a YouTube item plays
  const ytPanel = (it) => { const p = $('#ytpanel'); if (!p) return; if (it) { $('#yttitle').textContent = it.title || 'YouTube'; p.hidden = false; } else p.hidden = true; };
  $('#ytmin').onclick = () => { const p = $('#ytpanel'); const small = !p.classList.contains('small'); p.classList.toggle('small', small); $('#ytmin').setAttribute('aria-expanded', String(!small)); $('#ytmin').title = small ? 'Show the video' : 'Make the video smaller'; };
  // A YouTube item that can't be embedded (the uploader turned it off, removed, private): say so and go on
  yt.addEventListener('yterror', (e) => {
    const it = state.browserQueue[state.browserIndex];
    const why = { 2: 'the link is not a valid video', 5: `${hereName().toLowerCase()} cannot play it`, 100: 'the video is gone or private', 101: 'YouTube only plays it on youtube.com (its owner turned that off, or YouTube wants you signed in first: see the player)', 150: 'YouTube only plays it on youtube.com (its owner turned that off, or YouTube wants you signed in first: see the player)' }[e.detail] || `YouTube error ${e.detail}`;
    toast(`\u201c${(it && it.title) || 'This video'}\u201d can't play here: ${why}.`, 'error');
    if (state.browserIndex + 1 < state.browserQueue.length) browserPlay(state.browserIndex + 1);
  });
  const isBrowser = () => state.zone === BROWSER;
  // What the app's own player says of what plays: the path, as a zone reports it
  const nativePath = () => { const n = state.natOn && !state.ytOn ? nat.status() : null; const np = n && n.now_playing; return np ? { signal_path: np.signal_path, source_format: np.source_format, output_format: np.output_format, format: np.format || null } : {}; };
  // Where the bar's song is, in seconds: a song held at its place whose file hasn't opened is
  // there, not at 0:00 (holdPlace)
  const browserTime = () => { const r = state.restoredAt; return r && r.it === state.browserQueue[state.browserIndex] && audio.readyState < 1 ? r.pos : audio.currentTime; };
  const browserTransport = () => (audio.paused ? (browserTime() > 0 && !audio.ended ? 'paused' : 'stopped') : 'playing');
  const browserState = () => {
    const it = state.browserQueue[state.browserIndex];
    return {
      id: BROWSER, name: hereName(), output: 'browser',
      transport: browserTransport(),
      queue_index: state.browserIndex,
      // Why the app's player paused (the node took this phone's output, the DAC went, the stream
      // broke): said in the bar
      error: (() => { const n = state.natOn ? nat.status() : null; return n && n.transport === 'paused' ? n.error : null; })(),
      now_playing: it ? { item: it, position_ms: browserTime() * 1000, duration_ms: (it.duration_ms && (!Number.isFinite(audio.duration) || it.stream)) ? it.duration_ms : (audio.duration || 0) * 1000, signal_path: null, ...nativePath() } : null,
    };
  };
  const currentZone = () => (isBrowser() ? browserState() : state.zones[state.zone]);
  // The Null output is for developers: hidden unless switched on in About this node (it stays
  // listed while it is the zone in use)
  const devOutput = (id) => /^null/.test(String(id || ''));
  const TOKEN_GUIDE = 'https://github.com/gillohner/pubky-music-node/blob/main/docs/guides/troubleshooting.md#it-asks-for-a-token-or-the-token-is-refused';
  // Group outputs (POST /zones): speakers of one kind, kept in sync. Sonos, HEOS and BluOS speakers
  // become that system's own group; outputs the node keeps in time itself (caps.groupable:
  // squeezelite, the app's players on hw: DACs) a node group, of outputs that share a sample
  // rate. Kinds don't mix, and other outputs (which the API would only group loosely) aren't offered.
  const GROUP_KINDS = [['sonos', 'Sonos'], ['heos', 'HEOS (Denon, Marantz)'], ['bluos', 'BluOS (Bluesound, NAD)'], ['node', 'Squeezelite players'], ['net', 'The app\u2019s players on a DAC']];
  const groupKind = (o) => o.group_by || (o.caps.groupable ? (/^net:/.test(o.id) ? 'net' : 'node') : null);
  const groupSync = (picked) => {
    if (picked.length < 2) return '';
    const k = groupKind(picked[0]);
    if (k === 'heos') return 'HEOS groups them itself, in sync (if HEOS doesn’t know one of them, they play loosely synced).';
    return k === 'node' || k === 'net' ? 'In sync: the node keeps them in time sample by sample.' : `${k === 'sonos' ? 'Sonos' : 'BluOS'} groups them itself, in sync.`;
  };
  // Who is using the page (#46): the owner, or a listener with their own Ring sign-in, who plays
  // the shared library on the zones the owner opened and has nothing of the owner's
  const isListener = () => !!(state.me && state.me.role === 'listener');
  const isOwner = () => !isListener();
  // What the page shows and hides goes by what the node says this caller can do (GET /v1/me's
  // `can`: the capability names of crates/pm-api/src/access.rs, in snake case). Without the list
  // (the offline app, a backend that has no roles) it is the role as before: the owner everything,
  // someone with music access what the node gives them.
  const LISTENER_CAN = new Set(['artwork', 'session', 'library', 'zone_list', 'zone_use', 'collections', 'own', 'own_play', 'own_sign_in_state', 'own_sign_in']);
  const can = (cap) => { const me = state.me; return me && Array.isArray(me.can) ? me.can.includes(cap) : !isListener() || LISTENER_CAN.has(cap); };
  // ---- people & access: the words the owner's screen and a person's own Settings share ----
  // "Kitchen", "Kitchen and Den", "Kitchen, Den and Hall", "Kitchen, Den, Hall and 2 more"
  const listWords = (names) => { const n = names.length > 4 ? [...names.slice(0, 3), `${names.length - 3} more`] : names; return n.length < 2 ? n.join('') : `${n.slice(0, -1).join(', ')} and ${n[n.length - 1]}`; };
  // What someone the owner added can and can't do, in plain words: [capability, words]. `o.has(cap)`
  // says whether they can; `o.their`, `o.owners` and `o.zones` fit the words to who reads them.
  // `o.noZone`: what "can't" says when no zone is open to them (then `o.has('zone_use')` is false).
  const CAN_WORDS = [
    ['library', () => 'Play and search this node’s library'],
    ['zone_use', (o) => o.zones],
    ['collections', (o) => `Keep ${o.their} own bookmarks and collections, on ${o.their} own Pubky account`],
    ['own_sign_in', (o) => `Sign in to ${o.their} own TIDAL, Qobuz or Spotify`],
    ['own', () => 'Report a problem'],
  ];
  const CANT_WORDS = [
    ['zone_use', (o) => o.noZone || ''],
    ['settings', () => 'Change settings'],
    ['library_manage', () => 'Add, change or remove music'],
    ['owner_music', (o) => `See ${o.owners} bookmarks, collections, tags or history`],
    ['people', () => 'Add or remove people'],
    ['backup', () => 'Back up or restore'],
    ['zone_manage', () => 'Change zones and speakers'],
  ];
  const canLists = (o) => {
    const col = (title, mark, rows) => (rows.length ? `<div class="pa-col"><h4 class="pa-col-h">${esc(title)}</h4><ul class="pa-points ${mark}">${rows.map(([, words]) => `<li>${icon(mark === 'pa-yes' ? 'check' : 'close')}<span>${esc(words(o))}</span></li>`).join('')}</ul></div>` : '');
    return `<div class="pa-cando">${col(o.canTitle, 'pa-yes', CAN_WORDS.filter(([cap, words]) => o.has(cap) && words(o)))}${col(o.cantTitle, 'pa-no', CANT_WORDS.filter(([cap, words]) => !o.has(cap) && words(o)))}</div>`;
  };
  // A sign-in on this node as its list shows it (GET /v1/me/sign-ins, `sign_ins` in GET /v1/people):
  // what it said it is, since when, the day it was last used, and whether it signed in from
  // somewhere other than the node's own network (remote access, a tunnel, a public address)
  const signInName = (s) => s.label || { browser: 'A browser', app: 'An app' }[s.kind] || 'A browser or app';
  // (`last_used` is the start of a UTC day: read as that day, wherever the reader is)
  const signInNote = (s) => [
    s.created_at ? `since ${new Date(s.created_at * 1000).toLocaleDateString()}` : '',
    s.last_used == null ? '' : s.last_used >= Math.floor(Date.now() / 86400000) * 86400 ? 'last used today' : `last used ${new Date(s.last_used * 1000).toLocaleDateString(undefined, { timeZone: 'UTC' })}`,
    s.remote ? 'signed in away from home' : '',
  ].filter(Boolean).join(' · ');
  // The caller's own sign-ins, drawn into `host` as settings rows: this one first (hereName: "This
  // browser", "This computer", "This phone"), with `control` (the owner's Log out), then every other
  // one with Sign out, and "Sign out all others".
  // Loading, "Only this one", and an error with Retry. `kept`: the caller's `{ list }`, so a page
  // drawn again shows the last answer at once while it asks again. `wire(host)` runs after every draw.
  const mountSignIns = (host, { control = '', kept = {}, wire = null } = {}) => {
    if (!host) return;
    const row = (name, note, ctl, attr = '') => `<div class="set-row" ${attr}><div class="set-label"><div class="set-name">${name}</div>${note ? `<p class="set-help">${note}</p>` : ''}</div>${ctl ? `<div class="set-ctl">${ctl}</div>` : ''}</div>`;
    const here = (list) => {
      const cur = (list || []).find((s) => s.current);
      const note = cur ? esc([cur.label, signInNote(cur)].filter(Boolean).join(' · ')) : !list ? '' : state.token ? 'Logged in with the password for apps.' : 'No log in is needed here: this node has no password for apps.';
      return row(esc(hereName()), note, control, 'data-si-here');
    };
    const draw = (list, error) => {
      const others = (list || []).filter((s) => !s.current);
      host.innerHTML = here(list) + (error
        ? `<div class="set-row" data-si-error><div class="set-label"><p class="set-help pa-err" role="alert">Couldn’t read the other sign-ins: ${esc(error)}</p></div><div class="set-ctl"><button class="btn sm" data-si-retry>Retry</button></div></div>`
        : !list ? '<div class="set-row" data-si-loading aria-busy="true"><div class="skeleton pa-skel"></div></div>'
          : !others.length ? '<div class="set-row" data-si-none><div class="set-label"><p class="set-help">Only this one.</p></div></div>'
            : others.map((s) => row(esc(signInName(s)), esc(signInNote(s)), `<button class="btn sm" data-si-end="${esc(s.id)}" aria-label="Sign out ${esc(signInName(s))}, ${esc(signInNote(s))}">Sign out</button>`, `data-si="${esc(s.id)}"`)).join('')
              + `<div class="set-row" data-si-rest><div class="set-label"><p class="set-help">${others.length === 1 ? 'One other is signed in.' : `${others.length} others are signed in.`}</p></div><div class="set-ctl"><button class="btn sm" data-si-all>Sign out all others</button></div></div>`);
      $$('[data-si-end]', host).forEach((b) => b.onclick = async () => {
        const s = others.find((x) => x.id === b.dataset.siEnd); const ids = others.map((x) => x.id); const at = ids.indexOf(b.dataset.siEnd);
        try { await api('DELETE', `/me/sign-ins/${encodeURIComponent(b.dataset.siEnd)}`); toast(`Signed out: ${s ? signInName(s) : 'that sign-in'}`); } catch (e) { toast(e.message, 'error'); }
        // (found by its value: an id from the node is never made into a selector)
        await load([ids[at + 1], ids[at - 1]].filter(Boolean).map((id) => () => $$('[data-si-end]', host).find((x) => x.dataset.siEnd === id)));
      });
      const all = $('[data-si-all]', host); if (all) all.onclick = async () => {
        const go = await choiceDialog({ title: 'Sign out all other browsers and apps?', text: `Each of them has to log in again with Pubky Ring. ${hereName()} stays logged in.`, choices: [{ label: 'Sign out all others', value: true, primary: true, danger: true }] });
        if (!go) { all.focus(); return; }
        try { const r = await api('DELETE', '/me/sign-ins'); toast(r && r.ended === 1 ? 'Signed out one other browser or app' : `Signed out ${r && r.ended != null ? r.ended : 'the'} other browsers and apps`); } catch (e) { toast(e.message, 'error'); }
        await load([]);
      };
      const again = $('[data-si-retry]', host); if (again) again.onclick = () => load([() => $('[data-si-end]', host), () => $('[data-si-all]', host)]);
      if (wire) wire(host);
    };
    // `focus`: where the focus goes when the control that had it is gone after the draw: functions
    // that find it (the next row's Sign out), else this browser's own control, else the list itself
    const load = async (focus = null) => {
      const had = !!focus && host.contains(document.activeElement);
      let list = null; let error = null;
      try { list = await api('GET', '/me/sign-ins'); if (!Array.isArray(list)) throw new Error('The node’s answer was not a list.'); } catch (e) { list = null; error = e.message; }
      if (!host.isConnected) return;
      const same = !error && kept.list && JSON.stringify(kept.list) === JSON.stringify(list) && !$('[data-si-loading], [data-si-error]', host);
      if (!error) kept.list = list;
      if (!same) draw(error ? null : list, error);
      if (had && !host.contains(document.activeElement)) { const to = [...focus, () => $('[data-si-here] .btn', host)].map((find) => find()).find(Boolean) || host; to.focus({ preventScroll: to === host }); }
    };
    host.tabIndex = -1;
    draw(kept.list || null, null);
    load();
  };
  // The last list this log in was shown (another log in in the same tab starts without one)
  const keptSignIns = () => { if (!state.mySignIns || state.mySignIns.by !== state.token) state.mySignIns = { by: state.token, list: null }; return state.mySignIns; };
  // The apps the owner gave a password of their own (GET /v1/node/app-passwords), drawn into `host`:
  // each with when it was made and last used, and Remove (asked first); then a name field and
  // "Make a password", whose password shows once, with Copy, until the page is drawn again.
  // Loading, none yet, and an error with Retry.
  const mountAppPasswords = (host) => {
    if (!host) return;
    const row = (name, note, ctl, attr = '') => `<div class="set-row" ${attr}><div class="set-label"><div class="set-name">${name}</div>${note ? `<p class="set-help">${note}</p>` : ''}</div>${ctl ? `<div class="set-ctl">${ctl}</div>` : ''}</div>`;
    const note = (a) => [
      a.created_at ? `made ${new Date(a.created_at * 1000).toLocaleDateString()}` : '',
      a.last_used == null ? 'not used yet' : a.last_used >= Math.floor(Date.now() / 86400000) * 86400 ? 'last used today' : `last used ${new Date(a.last_used * 1000).toLocaleDateString(undefined, { timeZone: 'UTC' })}`,
    ].filter(Boolean).join(' · ');
    const form = '<div class="set-row set-block" data-ap-new><div class="set-label"><label class="set-name" for="apname">Give an app its own password</label><p class="set-help">Name the app, then copy its password into it.</p><div class="toolbar"><input class="input" id="apname" maxlength="60" placeholder="The app’s name, e.g. Home Assistant" autocomplete="off"><button class="btn" id="apgo">Make a password</button></div><p class="field-err small" id="apnameerr" role="alert" hidden></p><div id="apout" aria-live="polite"></div></div></div>';
    let shown = '';
    const draw = (list, error) => {
      const typed = $('#apname', host) ? $('#apname', host).value : '';
      host.innerHTML = (error
        ? `<div class="set-row" data-ap-error><div class="set-label"><p class="set-help pa-err" role="alert">Couldn’t read the apps: ${esc(error)}</p></div><div class="set-ctl"><button class="btn sm" data-ap-retry>Retry</button></div></div>`
        : !list ? '<div class="set-row" data-ap-loading aria-busy="true"><div class="skeleton pa-skel"></div></div>'
          : !list.length ? '<div class="set-row" data-ap-none><div class="set-label"><p class="set-help">No app has a password of its own yet.</p></div></div>'
            : list.map((a) => row(esc(a.name), esc(note(a)), `<button class="btn sm danger" data-ap-del="${esc(a.id)}" aria-label="Remove the password of ${esc(a.name)}">Remove…</button>`, `data-ap="${esc(a.id)}"`)).join(''))
        + form;
      $('#apname', host).value = typed;
      $('#apout', host).innerHTML = shown;
      $$('[data-ap-del]', host).forEach((b) => b.onclick = async () => {
        const a = (list || []).find((x) => x.id === b.dataset.apDel); const name = a ? a.name : 'this app';
        const go = await choiceDialog({ title: `Remove the password of ${name}?`, text: `${name} stops working with this node at once. Your other apps go on.`, choices: [{ label: 'Remove', value: true, primary: true, danger: true }] });
        if (!go) { b.focus(); return; }
        try { await api('DELETE', `/node/app-passwords/${encodeURIComponent(b.dataset.apDel)}`); toast(`Removed the password of ${name}`); } catch (e) { toast(e.message, 'error'); }
        await load();
        const next = $('[data-ap-del]', host) || $('#apname', host); if (next) next.focus();
      });
      const err = $('#apnameerr', host);
      $('#apname', host).oninput = () => { err.hidden = true; };
      $('#apgo', host).onclick = async () => {
        const name = $('#apname', host).value.trim();
        if (!name) { err.textContent = 'Name the app first.'; err.hidden = false; $('#apname', host).focus(); return; }
        let made;
        try { made = await api('POST', '/node/app-passwords', { name }); } catch (e) { toast(e.message, 'error'); return; }
        shown = `<p class="small pa-newpass">The password of ${esc(made.name)}, shown only now. Copy it into the app (a Subsonic app takes it as its password).</p><div class="toolbar"><code class="pa-code" data-ap-made>${esc(made.password)}</code><button class="btn sm primary" data-copylink="${esc(made.password)}">Copy</button></div>`;
        $('#apname', host).value = '';
        await load();
        const copy = $('#apout [data-copylink]', host); if (copy) copy.focus();
      };
      const again = $('[data-ap-retry]', host); if (again) again.onclick = () => load();
    };
    const load = async () => {
      let list = null; let error = null;
      try { list = await api('GET', '/node/app-passwords'); if (!Array.isArray(list)) throw new Error('The node’s answer was not a list.'); } catch (e) { list = null; error = e.message; }
      if (!host.isConnected) return;
      draw(list, error);
    };
    host.tabIndex = -1;
    draw(null, null);
    return load();
  };
  // Offline (the app's local server, pm-offline): only what this device keeps, played here
  const isOffline = () => !!(state.info && state.info.offline);
  const OFFLINE_VIEWS = new Set(['downloads', 'queue', 'settings']);
  // Phone mode (the app's own backend, no node): /v1/info lists what it can do. A node lists no
  // capabilities and can do everything; a name the phone doesn't list (zones, home, pubky…) is off
  const phoneMode = () => !!(state.info && state.info.mode === 'phone');
  // What this backend offers (/v1/info capabilities: a node has none listed and offers all; the
  // phone's own backend lists what it does). Not `can`, which is what this person may do (state.me)
  const offers = (cap) => !state.info || !Array.isArray(state.info.capabilities) || state.info.capabilities.includes(cap);
  const PHONE_CAPS = ['search', 'items', 'sources', 'library', 'songs', 'bookmarks', 'collections', 'listens', 'reviews', 'tags', 'add-songs', 'homeserver-files'];
  const PHONE_VIEWS = { search: ['search'], items: ['item', 'releases', 'source-item', 'source-artist'], sources: ['sources'], library: ['albums', 'artists', 'tracks', 'genres', 'genre'], songs: ['downloads'], bookmarks: ['favourites'], collections: ['playlists', 'playlist'] };
  const phoneView = (v) => v === 'queue' || v === 'settings' || Object.entries(PHONE_VIEWS).some(([c, vs]) => offers(c) && vs.includes(v));
  const phoneHome = () => (offers('search') ? 'search' : offers('library') ? 'tracks' : 'settings');
  const LISTENER_VIEWS_OFF = new Set(['home', 'radio', 'sources', 'source-item', 'identify', 'smart', 'smart-edit', 'dashboard', 'add-music']);
  // "No zone" as a node before that choice keeps it: a list of one zone without a name
  const noZoneList = (l) => Array.isArray(l) && l.length > 0 && l.every((id) => id === '');
  const zoneOpen = (z) => !isListener() || !state.me.zones || z.id === BROWSER || state.me.zones.includes(z.id);
  // The phone's own zone (the node's, on this app's output) while the app plays its music itself:
  // one "This phone" in the phone's picker; listed again while it is the zone in use or plays
  const ownZoneHidden = (z) => nativeOk() && z.id !== state.zone && z.transport !== 'playing' && ((window.pmDesktop && window.pmDesktop.outputs) || []).includes(z.output);
  // A zone's state in one word, the same in every zone list (the Zones page, Settings, Play to,
  // Playing on): playing or paused, "error" for one stopped by an error, and none for a quiet
  // stopped zone (its "Nothing playing" says so)
  const zoneStateWord = (z) => (z.transport === 'stopped' ? (z.error ? 'error' : '') : z.transport || '');
  const zoneStatePill = (z) => { const w = zoneStateWord(z); return w ? `<span class="pill ${w === 'playing' ? 'ok' : w === 'error' ? 'bad' : ''}">${esc(w)}</span>` : ''; };
  // The same word in a list row's small state slot (Play to, Playing on); an error is in the warning
  // colour, its reason in the tooltip and for a screen reader (the Zones card shows it)
  const zoneStateText = (z) => { const w = zoneStateWord(z); return w === 'playing' ? `${icon('play')} playing` : w === 'error' ? `<span class="warn-text" title="${esc(z.error)}">error<span class="sr-only">: ${esc(z.error)}</span></span>` : esc(w); };
  const zoneList = () => { const zs = Object.values(state.zones).filter((z) => (state.devOutputs || !devOutput(z.output) || z.id === state.zone) && zoneOpen(z) && !ownZoneHidden(z)); return appPlatform() && zs.length ? [...zs, browserState()] : [browserState(), ...zs]; };
  // The browser's queue survives a reload, every song with where it was (UX review #6): stream
  // links are rebuilt on load (they carry the token), live radio is not kept.
  let lastSavedQueue = '';
  const isLiveRadio = (it) => it.album === 'Internet radio' && /^https?:/.test(it.uri || '');
  // Every 2 s, and at once when the page is hidden or closed (a phone's app is usually ended in
  // the background, so a song queued just before would otherwise be gone after a reload)
  const saveBrowserQueue = () => {
    if (accountForgotten) return;
    try {
      const keep = state.browserQueue.filter((it) => !isLiveRadio(it)).map((it) => { const { stream, nativeNo, ...rest } = it; return rest; });
      const j = JSON.stringify(keep);
      if (j !== lastSavedQueue) { lastSavedQueue = j; localStorage.setItem('pm.browserQueue', j); }
      const cur = state.browserQueue[state.browserIndex];
      // (the song itself with its place: the queue can change before the place comes back)
      // (a song brought back that hasn't opened, its file not answering: still where it was left)
      const pos = browserTime();
      if (cur && !isLiveRadio(cur)) localStorage.setItem('pm.browserAt', JSON.stringify({ index: state.browserQueue.slice(0, state.browserIndex).filter((it) => !isLiveRadio(it)).length, id: cur.id, pos: Math.floor(pos || 0) }));
    } catch { /* storage unavailable */ }
  };
  setInterval(saveBrowserQueue, 2000);
  window.addEventListener('pagehide', saveBrowserQueue);
  document.addEventListener('visibilitychange', () => { if (document.visibilityState === 'hidden') saveBrowserQueue(); });
  // The saved song and its position, if it is still in the queue (whose stream links are rebuilt:
  // they carry the token). The song is found by its id, the saved index only a hint: Shuffle, Play
  // next or a Queue page edit before the place comes back (right after a reload) moves it, and the
  // song at the old index is another one. (A place saved before the id was kept: by its index.)
  const savedBrowserAt = () => {
    for (const it of state.browserQueue) if (!it.stream && it.track_id == null) { const u = streamUrlFor(it.uri); if (u) it.stream = u; }
    let at = null; try { at = JSON.parse(localStorage.getItem('pm.browserAt') || 'null'); } catch { /* none */ }
    if (!at || typeof at !== 'object') return null;
    const q = state.browserQueue; const hint = Number.isInteger(at.index) ? at.index : -1;
    const index = at.id == null ? hint : q[hint] && q[hint].id === at.id ? hint : q.findIndex((it) => it.id === at.id);
    return q[index] ? { index, pos: Number(at.pos) || 0 } : null;
  };
  // A song in the bar, paused at a place, whose file is opening (after a reload, or moved here
  // paused from a zone): the bar, the saver and a Move show that place until it opens, and Play on
  // a file that didn't open starts it there (browserResume)
  const holdPlace = (it, pos) => {
    state.restoredAt = { it, pos };
    // Once it opens, at its place (moved by a seek meanwhile), and from then on an ordinary song: a
    // later break picks up where it broke. A play that took over first (Play on a file that didn't
    // open, from its place or a seek's) is left where it is
    audio.addEventListener('loadedmetadata', () => {
      const r = state.restoredAt; if (!r || r.it !== it || state.browserQueue[state.browserIndex] !== it) return;
      state.restoredAt = null;
      if (r.pos > 0) { try { audio.currentTime = r.pos; } catch { /* not seekable */ } }
      renderBar();
    }, { once: true });
  };
  // Back where it was, paused: the song in the bar and its position
  const restoreBrowserQueue = () => {
    const at = savedBrowserAt();
    const it = at && state.browserQueue[at.index];
    if (!it) return;
    state.browserIndex = at.index;
    holdPlace(it, at.pos);
    audio.preload = 'metadata';
    browserSrc(it);
    renderBar();
    // (the Queue page opened by the reload was drawn before the place came back: its song is marked now)
    if (isBrowser()) queueShown();
  };
  // (a Queue page drawn for the zone gone is drawn again for this browser: its buttons were the gone zone's)
  const ensureZone = () => { if (!isBrowser() && !state.zones[state.zone]) { state.zone = BROWSER; queueShown(); } };
  // What the bar showed last (zone, song, artist), kept in this browser: until the node answers,
  // the bar shows it rather than "Zone" and "Nothing playing" (owner's recording: a second shell
  // that swapped to the real one). index.html draws it before this script runs.
  const barKept = () => { try { return JSON.parse(localStorage.getItem('pm.bar') || 'null') || {}; } catch { return {}; } };
  // (asked on every draw of the bar: the same words as last time read and write nothing)
  const barKeptLast = {};
  const barKeep = (part) => { const key = Object.keys(part).join(); const said = JSON.stringify(part); if (barKeptLast[key] === said) return; barKeptLast[key] = said; const was = barKept(); const next = { ...was, ...part }; if (JSON.stringify(next) !== JSON.stringify(was)) { try { localStorage.setItem('pm.bar', JSON.stringify(next)); } catch { /* private window */ } } };
  const renderZoneName = () => { const z = currentZone(); if (z && state.zonesSeen) barKeep({ zone: z.name }); const name = (!state.zonesSeen && !state.zonePicked && barKept().zone) || (z ? z.name : 'Zone'); $('#zonename').textContent = name; $('#zonebtn').setAttribute('aria-label', `Playing on ${name}: choose where to play`); $('#zonebtn').title = `Playing on ${name}`; syncVolumeSliders(z); if (!volPop().hidden && volPop().dataset.drawn !== volPopKey()) drawVolPop(); };
  // ---- One volume, one source (TASK-volume-one-source) ----
  // Every slider for a zone reads the same thing: the level the node read back from the output
  // (`z.volume`, dB or percent on the output's own scale, with its mute), This browser its own
  // audio element. `value` is null while the node hasn't read it: nothing is made up.
  // In the Android app the page's own player has no gain of its own (audio.volume stays 1): its
  // slider is Android's media volume, which the phone's volume keys move too. The app says it
  // (`window.pmDesktop.mediaVolume`, `pmdesktop:mediavolume`) and sets it
  // (`OdeionAndroid.setMediaVolume`), and learns which zone the page controls for the system's
  // media controls (`OdeionAndroid.controlling`).
  const appVolume = () => !!(window.pmDesktop && window.OdeionAndroid);
  const appMediaVolume = () => (appVolume() && typeof window.pmDesktop.mediaVolume === 'number' ? window.pmDesktop.mediaVolume : null);
  let appZoneSaid = null;
  const tellAppZone = (id) => { if (!window.OdeionAndroid || id === appZoneSaid) return; appZoneSaid = id; try { window.OdeionAndroid.controlling(id); } catch { /* the app's side is gone */ } };
  const zoneVolume = (z) => {
    if (!z) return null;
    if (z.id === BROWSER && appVolume() && state.natOn) return window.pmDesktop.phoneVolume || null;
    if (z.id === BROWSER && appVolume()) return { kind: 'percent', min: 0, max: 100, step: 1, value: appMediaVolume(), muted: !!audio.muted };
    if (z.id === BROWSER) return { kind: 'db', min: -60, max: 0, step: 1, value: Math.max(-60, Math.round(20 * Math.log10(audio.volume || 0.001))), muted: !!audio.muted };
    const out = (state.outputs || []).find((o) => o.id === z.output); const hv = out && out.caps && out.caps.hw_volume; const hk = typeof hv === 'string' ? hv : hv && hv.kind;
    const v = z.volume || null;
    // No level yet and no output list to say the scale (a listener never gets one): a plain 0–100
    // slider still lets them set a level; the owner's bar waits for the list instead
    const kind = v && v.percent != null ? 'percent' : v && v.db != null ? 'db' : hk === 'db' ? 'db' : hk === 'percent' ? 'percent' : (!state.outputs && !isOwner()) ? 'percent' : null;
    if (!kind) return null;
    if (kind === 'percent') return { kind, min: 0, max: 100, step: 1, value: v && v.percent != null ? v.percent : null, muted: !!(v && v.muted) };
    const r = hk === 'db' ? hv : { min: -60, max: 0, step: 1 };
    return { kind, min: r.min, max: r.max, step: r.step || 1, value: v && v.db != null ? Math.min(r.max, Math.max(r.min, v.db)) : null, muted: !!(v && v.muted) };
  };
  // A zone id inside an attribute selector (no CSS.escape: ui-smoke's jsdom has none)
  const cssStr = (v) => String(v).replace(/["\\]/g, '\\$&').replace(/[\n\r\f]/g, (c) => `\\${c.charCodeAt(0).toString(16)} `);
  const levelText = (r) => (!r ? '' : r.muted ? 'Muted' : r.value == null ? 'Level unknown' : r.kind === 'percent' ? `${Math.round(r.value)} %` : `${Math.round(r.value)} dB`);
  // A slider shows the zone's level; one being dragged is left alone until it is let go
  const applyVol = (el, r) => {
    if (!el || !r || el.dataset.dragging) return;
    el.min = r.min; el.max = r.max; el.step = r.step;
    el.classList.toggle('unknown', r.value == null); el.classList.toggle('muted', !!r.muted);
    el.value = r.value == null ? r.min : r.value;
    el.setAttribute('aria-valuetext', levelText(r)); setFill(el);
  };
  const volSliders = (z) => { const els = new Set(); if (z.id === state.zone) for (const el of [$('#vol'), $('#npvol'), $('#volpopvol')]) if (el) els.add(el); for (const el of $$(`[data-zvol="${cssStr(z.id)}"]`)) els.add(el); return [...els]; };
  const syncVolumeSliders = (z, except) => {
    if (!z) return;
    if (z.id === state.zone) tellAppZone(z.id);
    // The owner's output list says the scale (and whether there is a volume at all); asked once
    if (z.id !== BROWSER && isOwner() && !state.outputs && !state.outputsP) state.outputsP = api('GET', '/outputs').then((o) => { state.outputs = o; for (const cur of Object.values(state.zones || {})) syncVolumeSliders(cur); if (!volPop().hidden && volPop().dataset.drawn !== volPopKey()) drawVolPop(); }).catch(() => {}).finally(() => { state.outputsP = null; });
    const r = zoneVolume(z);
    if (z.id === state.zone) $('.player').classList.toggle('novol', !r);
    if (!r) return;
    for (const el of volSliders(z)) if (el !== except) applyVol(el, r);
    // (asked on every event of the zone: the same level and mute as shown write nothing)
    { const words = levelText(r); for (const t of $$(`[data-vollevel="${cssStr(z.id)}"]`)) if (t.textContent !== words) t.textContent = words; }
    for (const b of $$(`[data-volmute="${cssStr(z.id)}"]`)) { const m = String(!!r.muted); if (b.getAttribute('aria-pressed') !== m) { b.setAttribute('aria-pressed', m); b.title = r.muted ? 'Unmute' : 'Mute'; } }
  };
  const setZoneVol = async (z, value, from) => {
    const r = zoneVolume(z); if (!r) return;
    const num = Number(value);
    if (z.id === BROWSER && appVolume() && state.natOn) { try { window.OdeionAndroid.setMediaVolume(r.kind === 'percent' ? Math.round(num) : num, r.kind); } catch { /* the app's side is gone */ } return; }
    if (z.id === BROWSER && appVolume()) { const p = Math.round(num); audio.volume = 1; window.pmDesktop.mediaVolume = p; try { window.OdeionAndroid.setMediaVolume(p); } catch { /* the app's side is gone */ } syncVolumeSliders(z, from); return; }
    if (z.id === BROWSER) { audio.volume = Math.pow(10, num / 20); syncVolumeSliders(z, from); return; }
    const body = r.kind === 'percent' ? { percent: Math.round(num) } : { db: num };
    // Shown as set until the node's read-back arrives (its zone event syncs every slider); a
    // refused request puts the old value back
    const was = z.volume;
    z.volume = { ...(z.volume || {}), ...body };
    syncVolumeSliders(z, from);
    try { await api('POST', `/zones/${encodeURIComponent(z.id)}/volume`, { volume: body }); } catch (e) { z.volume = was; syncVolumeSliders(z); toast(e.message, 'error'); }
  };
  const setZoneMute = async (z, m) => {
    if (!z) return;
    if (z.id === BROWSER && appVolume() && state.natOn) { try { window.OdeionAndroid.setMediaVolume(m, 'mute'); } catch { /* the app's side is gone */ } return; }
    if (z.id === BROWSER) { audio.muted = m; syncVolumeSliders(z); return; }
    const was = z.volume;
    z.volume = { ...(z.volume || {}), muted: m };
    syncVolumeSliders(z);
    try { await api('POST', `/zones/${encodeURIComponent(z.id)}/volume`, { volume: { mute: m } }); } catch (e) { z.volume = was; syncVolumeSliders(z); toast(e.message, 'error'); }
  };
  // Android's media volume changed (a key, the panel, the slider): every slider of the page's own
  // player follows; it never keeps a gain of its own
  window.addEventListener('pmdesktop:mediavolume', () => { if (audio.volume !== 1) audio.volume = 1; syncVolumeSliders(browserState()); });
  const bindVolSlider = (el, zoneOf) => {
    if (!el) return;
    const done = () => { delete el.dataset.dragging; };
    el.addEventListener('pointerdown', () => { el.dataset.dragging = '1'; });
    // Let go: the slider catches up with the zone (after `change`, which follows the pointer)
    const letGo = () => { done(); setTimeout(() => syncVolumeSliders(zoneOf()), 0); };
    el.addEventListener('pointerup', letGo); el.addEventListener('pointercancel', letGo);
    el.addEventListener('input', () => { setFill(el); const z = zoneOf(); const r = zoneVolume(z); if (!r) return; const now = levelText({ ...r, value: Number(el.value), muted: false }); el.classList.remove('unknown'); el.setAttribute('aria-valuetext', now); for (const t of $$(`[data-vollevel="${cssStr(z.id)}"]`)) t.textContent = now; });
    el.addEventListener('change', () => { done(); const z = zoneOf(); if (z) setZoneVol(z, el.value, el); });
  };
  const volPop = () => $('#volpop');
  const closeVolPop = (fromBack) => {
    const vp = volPop(); if (!vp || vp.hidden) return;
    const had = focusWasIn(vp);
    vp.hidden = true; $('#scrim').hidden = true; $('#volbtn').setAttribute('aria-expanded', 'false');
    const k = transients.indexOf(volTr); if (k >= 0) transients.splice(k, 1);
    if (fromBack !== true) popTransient();
    focusBack(had, $('#volbtn'));
  };
  const volTr = { open: () => !volPop().hidden, close: closeVolPop };
  // Where the bar has no room for the slider: the volume button opens this, the zone's one slider
  // What the open popover shows: its zone and scale (a redraw would drop the slider being moved)
  const volPopKey = () => { const z = currentZone(); const r = zoneVolume(z); return `${z ? `${z.id}|${z.name}` : ''}|${r ? r.kind : ''}`; };
  const drawVolPop = () => {
    const vp = volPop(); const z = currentZone(); const r = zoneVolume(z); vp.dataset.drawn = volPopKey();
    vp.innerHTML = `<div class="pop-title">Volume · ${esc(z ? z.name : 'Zone')}</div>` + (r && z
      ? `<div class="volpop-row"><button class="icon-btn" data-volmute="${esc(z.id)}" aria-pressed="${!!r.muted}" title="${r.muted ? 'Unmute' : 'Mute'}" aria-label="Mute">${icon('volume')}</button><input type="range" id="volpopvol" aria-label="Volume of ${esc(z.name)}"><span class="vol-level" data-vollevel="${esc(z.id)}">${esc(levelText(r))}</span></div>`
      : '<p class="qp-note">This zone sets its volume on the device.</p>');
    const el = $('#volpopvol', vp); if (el) { applyVol(el, r); bindVolSlider(el, currentZone); }
    const m = $('[data-volmute]', vp); if (m) m.onclick = () => setZoneMute(currentZone(), m.getAttribute('aria-pressed') !== 'true');
  };
  const openVolPop = () => {
    closePop(); closeJobPop(); closeQPanel(); drawVolPop();
    volPop().hidden = false; $('#scrim').hidden = false; $('#volbtn').setAttribute('aria-expanded', 'true');
    transients.push(volTr); pushTransient();
    const el = $('#volpopvol'); if (el) el.focus();
  };
  bindVolSlider($('#vol'), currentZone);
  // A queue URI the browser can play directly: TIDAL tracks go through the node's proxy.
  const streamUrlFor = (uri) => { const local = /^track:(\d+)$/.exec(uri || ''); if (local) return `/v1/library/tracks/${local[1]}/file${tokenQ()}`; if (ytOf(uri)) return null; const m = /^(tidal|qobuz|soundcloud|bandcamp|deezer|jamendo|subsonic|jellyfin):track:([A-Za-z0-9._-]+)$/.exec(uri || ''); if (m) return `/v1/sources/${m[1]}/stream/${m[2]}${tokenQ()}`; return /^https?:/.test(uri || '') ? uri : null; };
  // Source tracks (TIDAL, archive…) as queue items for the current zone.
  // `o.shuffle` (a page's Play or Shuffle): the queue is loaded with that mode (see playTracks)
  const playSourceItems = async (qitems, replace = true, at = 'end', o = {}) => {
    if (!qitems.length) return;
    // YouTube's player lives in this page: it can't send audio to the node's zones (unless yt-dlp is on)
    const ytZones = (state.sources || []).some((x) => x.id === 'youtube' && x.enabled !== false && /zones/.test(x.status));
    if (!onBrowser(o) && !ytZones && qitems.some((q) => ytOf(q.uri))) {
      const rest = qitems.filter((q) => !ytOf(q.uri));
      if (!rest.length) {
        showMenu(document.activeElement && document.activeElement !== document.body ? document.activeElement : $('#zonebtn'), [{ title: `YouTube plays only ${hereOnly()}` }, { label: `Play ${hereOnly()}`, icon: 'zone', run: () => { state.zone = BROWSER; try { localStorage.setItem('pm.zone', BROWSER); } catch { /* */ } renderZoneName(); playSourceItems(qitems, replace, at, { ...o, turn: undefined, mode: undefined }); } }]);
        return;
      }
      toast(`${countOf(qitems.length - rest.length, 'YouTube items')} left out: YouTube plays only ${hereOnly()}`);
      qitems = rest;
    }
    if (onBrowser(o)) {
      const bitems = qitems.map((q, k) => ({ id: Date.now() + k, uri: q.uri, track_id: q.track_id ?? trackIdOf(q), artists: q.artists, stream: streamUrlFor(q.uri) || q.uri, title: q.title, artist: q.artist, album: q.album, duration_ms: q.duration_ms, image: q.image || null }));
      if (at === 'now' && state.browserQueue.length) { state.browserQueue.splice(state.browserIndex + 1, 0, ...bitems); browserPlay(state.browserIndex + 1, 0, true); return; }
      if (replace || at === 'now') browserLoad(bitems, o.shuffle); else if (at === 'next') state.browserQueue.splice(state.browserIndex + 1, 0, ...bitems); else browserAppend(bitems);
      // (as on a zone and for library songs: one song played next says so; one played now says nothing, it plays)
      if (!replace && at !== 'now') toast(bitems.length === 1 ? (at === 'next' ? 'Playing next' : 'Added to queue') : `${countOf(bitems.length, 'songs')} added`);
      return;
    }
    // (every change of the zone's queue takes its turn: an add can't be lost to a replace still waiting)
    const zid = zoneOf(o);
    if (at === 'now') { try { await zoneOp(zid, async (t) => { await api('POST', `/zones/${encodeURIComponent(zid)}/queue`, { items: qitems, at: 'next', replace: false, play: false }, { signal: t.signal }); const ev0 = evOf(zid); const st = await api('POST', `/zones/${encodeURIComponent(zid)}/transport`, { action: 'next' }, { repeat: true, signal: t.signal }); applyAnswer(st, ev0); }, o.turn); renderBar(); refreshQueue(); } catch (e) { toast(e.message, 'error'); } return; }
    if (replace && o.shuffle !== undefined) return loadZoneQueue(qitems, o.shuffle, o);
    const wait = replace ? startWait('play', zid, qitems[0].title || '', true, true) : null;
    try { await zoneOp(zid, (t) => { if (replace) startSending(wait, zid); return api('POST', `/zones/${encodeURIComponent(zid)}/queue`, { items: qitems, at, replace, play: replace }, { signal: t.signal }); }, o.turn); if (replace) startSent(wait); if (!replace) toast(qitems.length === 1 ? (at === 'next' ? 'Playing next' : 'Added to queue') : `${countOf(qitems.length, 'songs')} added`); refreshQueue(); } catch (e) { if (replace) startFail(e.message, false, wait); else toast(e.message, 'error'); }
  };

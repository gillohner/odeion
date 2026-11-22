  // ---- first run (steward next-3, item 2) ------------------------------------
  // Home's "Get started" card: only the steps that matter on this node and to this person
  // (GET /v1/me/first-steps): add music, play something, the contact Discover needs, speakers and
  // Pubky sign-in. It goes once every step that isn't optional is done, or when hidden: by the
  // owner for everyone (kept on the node; Settings › Home for everyone shows it again), by a
  // listener in their browser.
  const setupHiddenHere = () => { try { return localStorage.getItem('pm.setupHidden') === '1'; } catch { return false; } };
  const renderSetup = async () => {
    const box = $('#h-setup'); if (!box) return;
    const owner = isOwner();
    const [info, fs, user, bundled] = await Promise.all([
      api('GET', '/info'), api('GET', '/me/first-steps'), owner ? pubkyUser() : null,
      owner ? api('GET', '/homeserver/bundled').catch(() => null) : null,
    ]).catch(() => [null]);
    if (!info || !fs || state.view !== 'home' || !box.isConnected) return;
    state.info = info;
    // (the node didn't say in 15 s whether it holds a Pubky session: the Pubky step is left out
    // rather than offering a sign-in as if signed out)
    if (owner && state.pubkyUser === undefined) fs.steps = fs.steps.filter((x) => x.id !== 'pubky' || x.done);
    // Hidden in this browser before the node kept it: the owner's choice moves to the node
    if (owner && setupHiddenHere() && !fs.hidden) { try { localStorage.removeItem('pm.setupHidden'); } catch { /* no storage */ } api('PUT', '/node/settings', { first_steps_hidden: true }).catch(() => {}); fs.show = false; }
    state.setupShown = fs.show && (owner || !setupHiddenHere());
    if (!state.setupShown) { box.innerHTML = ''; return; }
    // A folder added from the card makes its step needless; it stays, ticked, until the page reloads
    if (fs.steps.some((x) => x.id === 'music')) state.setupMusic = true;
    else if (state.setupMusic && owner) fs.steps.unshift({ id: 'music', done: info.library.tracks > 0, optional: false });
    const hsHere = !!(bundled && bundled.configured && bundled.reachable);
    const tracks = info.library.tracks > 0;
    const has = Object.fromEntries(fs.steps.map((x) => [x.id, x.done]));
    const musicTodo = fs.steps.some((x) => x.id === 'music' && !x.done);
    let suggestions = [];
    if (!state.sources) { try { state.sources = await api('GET', '/sources'); } catch { state.sources = []; } }
    if (musicTodo && !info.scanning) suggestions = await api('GET', '/library/folders/suggest').catch(() => []);
    if (state.view !== 'home' || !box.isConnected) return;
    const n = (x) => Number(x).toLocaleString();
    const musicBody = info.scanning
      ? `<p class="small" role="status">Reading your files… ${countOf(n(info.library.tracks), 'songs')} so far.</p>`
      : `${suggestions.length ? `<div class="setup-suggest">${suggestions.map((f) => `<button class="btn sm" data-addfolder="${esc(f.path)}">${icon('plus')} ${esc(f.path)} <span class="muted">${f.more ? 'over ' : ''}${n(f.audio_files)} audio files</span></button>`).join('')}</div>` : ''}
        <div class="toolbar"><input class="input" id="setupfolder" placeholder="${suggestions.length ? 'Or another folder' : 'The folder with your music, e.g. /home/you/Music'}" aria-label="Music folder on this computer"><button class="btn primary" id="setupadd">Add folder</button></div>
        <p class="muted small">No music files on this computer? <a class="lnk" href="/sources" data-setup-sources>Play from ${(state.sources || []).some((x) => x.id === 'jamendo' && x.status === 'ready') ? 'Internet Archive, Audius or Jamendo' : 'Internet Archive or Audius'}</a>, no account needed.</p>`;
    const text = {
      token_config: () => ['Change the password for apps', 'The password for apps (the API token) was in a saved queue’s links, where everyone who could use that zone could read it. It is set in the node’s config (<code>[api] token</code> in node.toml, <code>PM_TOKEN</code> in Docker): change it there and restart the node. Every app then needs the new one.', `<a class="btn sm primary" href="${TOKEN_GUIDE}" target="_blank" rel="noopener">${icon('external')} How to change it</a>`],
      token: () => ['Make a new password for apps', 'The password for apps (the API token) was in a saved queue’s links, where everyone who could use that zone could read it. Make a new password in Settings › People & access › Password for apps: every app then needs the new one, and your other browsers log in again.', '<a class="btn sm primary" href="/settings/people" data-setup-settings>Open People &amp; access</a>'],
      music: () => ['Add your music', has.music ? `${countOf(n(info.library.tracks), 'songs')}${info.library.albums ? ` · ${countOf(n(info.library.albums), 'albums')}` : ''}. More folders: Your library › Node library.` : 'Pick the folder with your music files. The node reads it and keeps watching it for new files; your files stay as they are.', musicBody],
      play: () => ['Play something', tracks ? `It plays ${hereIn()}. To play on speakers or another device, ${shellMode() === 'phone' ? 'pick one in Zones, or its name at the top of Now playing' : 'use the zone button at the bottom right'}.` : musicTodo ? 'Once your music is in.' : `Search for a song, or browse your services on Sources. It plays ${hereIn()}, or on a speaker from the zone button.`, tracks ? `<button class="btn sm primary" id="setupplay">${icon('play')} Shuffle my music</button>` : musicTodo ? '' : '<button class="btn sm" data-setup-sources>Open Sources</button>'],
      contact: () => ['See what people on Pubky play', has.contact ? 'Trending songs, tags and friends\' picks show on Home.' : 'The music index (Pinakes), MusicBrainz and Wikipedia ask each app for a way to reach whoever runs it: an email or a website. It goes only to them.', '<div class="toolbar"><input class="input" id="setupcontact" placeholder="you@example.org or https://your.site" aria-label="Your email or website"><button class="btn" id="setupcontactsave">Save</button></div>'],
      speakers: () => ['Play on your speakers (optional)', 'No speaker found yet. Speakers and streamers on your network show up by themselves; Snapcast and MPD need a line in the node’s config.', `<div class="toolbar"><button class="btn sm" id="setupspeakers">Look for speakers</button><a class="lnk small" href="https://github.com/gillohner/pubky-music-node/blob/main/docs/guides/speakers.md" target="_blank" rel="noopener">Which speakers work</a></div>`],
      pubky: () => (hsHere && !has.pubky
        ? ['Create your Pubky account here (optional)', 'This node has its own Pubky homeserver. Create your account on it with the Pubky Ring app: your collections, bookmarks, tags and files stay on this machine. Or sign in to a homeserver you already use.', '<div class="toolbar"><button class="btn sm primary" id="setupcreate">Create account here</button><button class="btn sm" id="setupsignin">Use an existing homeserver</button></div>']
        : ['Keep your collections on Pubky (optional)', has.pubky || user ? 'Signed in: collections, bookmarks and tags are kept on your homeserver.' : 'Sign in with the Pubky Ring app: your collections, bookmarks and tags are kept in your own Pubky storage and follow you to another node.', '<button class="btn sm" id="setupsignin">Sign in with Pubky Ring</button>']),
    };
    const step = (x, no) => { const [title, sub, body] = text[x.id](); return `<li class="setup-step ${x.done ? 'done' : ''}" data-step="${x.id}"><span class="stepmark" aria-hidden="true">${x.done ? icon('check') : no}</span><div class="grow"><div class="name">${title}${x.done ? '<span class="sr-only"> (done)</span>' : ''}</div><div class="sub">${sub}</div>${x.done ? '' : body}</div></li>`; };
    const done = fs.steps.filter((x) => x.done).length;
    box.innerHTML = `<section class="section fadein" aria-labelledby="setuph"><div class="card setup-card"><div class="section-head"><h3 id="setuph">Get started</h3><span class="muted small">${done} of ${fs.steps.length} done</span><button class="link" id="setuphide">Hide these steps</button></div>${musicTodo ? '<p class="muted small">Moving from another node? <a class="lnk" href="/settings" id="setuprestore">Restore it from a backup</a> instead.</p>' : ''}<ol class="setup-steps">`
      + fs.steps.filter((x) => text[x.id]).map((x, i) => step(x, i + 1)).join('')
      + '</ol></div></section>';
    const again = () => setTimeout(() => { if (state.view === 'home') renderSetup(); }, 1500);
    const addFolder = async (path) => {
      if (!path) return;
      try { const r = await api('POST', '/library/folders', { path }); toast(r.scan_started ? 'Folder added: reading your files' : 'Folder added'); watchScan({ scanning: true }); renderSetup(); } catch (e) { toast(e.message, 'error'); }
    };
    $$('[data-addfolder]', box).forEach((b) => b.onclick = () => addFolder(b.dataset.addfolder));
    const fi = $('#setupfolder'); if (fi) { $('#setupadd').onclick = () => addFolder(fi.value.trim()); fi.onkeydown = (e) => { if (e.key === 'Enter') $('#setupadd').click(); }; }
    $$('[data-setup-sources]', box).forEach((so) => so.onclick = (e) => { e.preventDefault(); setView('sources'); });
    // While the files are read the card counts them; when done, Home shows the albums once
    if (info.scanning) { state.setupScanning = true; again(); } else if (state.setupScanning) { state.setupScanning = false; render(); }
    const pb = $('#setupplay'); if (pb) pb.onclick = async () => {
      try { await shuffleMyMusic(); again(); } catch (e) { toast(e.message, 'error'); }
    };
    const ci = $('#setupcontact'); if (ci) {
      const save = async () => { const c = ci.value.trim(); if (!c) return; try { await api('PUT', '/node/settings', { contact: c }); state.discoveryOff = false; homeCache.clear(); toast('Saved: Discover is on'); render(); } catch (e) { toast(e.message, 'error'); } };
      $('#setupcontactsave').onclick = save; ci.onkeydown = (e) => { if (e.key === 'Enter') $('#setupcontactsave').click(); };
    }
    const sp = $('#setupspeakers'); if (sp) sp.onclick = () => setView('settings', 'playback');
    const sr = $('#setuprestore'); if (sr) sr.onclick = (e) => { e.preventDefault(); state.focusRestore = true; setView('settings', 'backup'); };
    const si = $('#setupsignin'); if (si) si.onclick = () => { state.autoResign = true; setView('settings', 'accounts'); };
    const sc = $('#setupcreate'); if (sc) sc.onclick = () => { state.autoCreate = true; setView('settings', 'accounts'); };
    $('#setuphide').onclick = async () => {
      if (owner) {
        try { await api('PUT', '/node/settings', { first_steps_hidden: true }); toast('Hidden for everyone on this node. Settings › Home for everyone shows them again.'); } catch (e) { toast(e.message, 'error'); return; }
      } else { try { localStorage.setItem('pm.setupHidden', '1'); } catch { /* still hidden for now */ } }
      state.setupShown = false; box.innerHTML = '';
      // Its contact step said why Pubky's rows are missing: now Home's line does
      const nc = $('#h-needcontact'); if (nc) nc.hidden = false;
    };
  };

  // ---- Navigation answers at once (owner, 2026-11-04) ----
  // setView() starts a navigation: the place clicked is marked at once (markTabs), a thin bar runs along
  // the top until the page is drawn, and a page that has drawn nothing of its own after 120 ms shows a
  // skeleton ("Still loading…" after 2 s). A newer navigation supersedes it: the reads the page's code
  // (renderPage's `api`, below) started before the page showed its content are aborted and never
  // answer, so the old page stops where it was and draws nothing (A→B ends on B). Reads started once it
  // shows its content (handlers, polls, slower extras) are never aborted and settle as usual. Changes
  // (anything but GET) are never held back.
  let navGen = 0; let navCtl = new AbortController(); let navStop = () => {}; let navRendering = Promise.resolve();
  const NEVER = () => new Promise(() => {}); // a fresh one each time: nothing piles up on a shared one
  const navApiFor = (gen, ctl, drawing) => (method, path, body, opts = {}) => {
    // Changes, and reads a press makes (a handler's GET before its PUT), are never held back
    if (method !== 'GET' || inHandler) return api(method, path, body, opts);
    // Once the page shows its content (or its code is done), its reads are as before: handlers, polls and
    // slower extras are never held back
    if (!drawing.on) return api(method, path, body, opts);
    if (gen !== navGen) return NEVER();
    return api(method, path, body, { ...opts, signal: opts.signal || ctl.signal, navRead: !!drawing.nav })
      .then((r) => (gen === navGen ? r : NEVER()), (e) => (gen === navGen ? Promise.reject(e) : NEVER()));
  };
  const NAV_SKELETON = `<div class="navskel" aria-busy="true"><div class="skeleton" style="height:28px;width:220px"></div><div class="skeleton sk-row" style="height:16px;width:140px;margin:10px 0 22px"></div>${skelRows(6)}${SLOW}</div>`;
  // Drawn: something of the page's own that isn't a placeholder (a page's own loading() is one)
  const pageDrawn = (before) => { const f = main.firstElementChild; return f !== before && !main.querySelector(':scope > .navskel, :scope > .skeleton, :scope > .sk-grid'); };
  const navigate = (fresh) => {
    const gen = ++navGen; navCtl.abort(); navCtl = new AbortController();
    navStop(); const superseded = new Promise((res) => { navStop = res; });
    let bar = document.getElementById('navprog');
    if (!bar) { bar = document.createElement('div'); bar.id = 'navprog'; bar.className = 'navprog'; bar.setAttribute('aria-hidden', 'true'); document.body.appendChild(bar); }
    // One status region outside the page, there before its words change, so "Loading…" is announced
    let said = document.getElementById('navstatus');
    if (!said) { said = document.createElement('p'); said.id = 'navstatus'; said.className = 'sr-only'; said.setAttribute('role', 'status'); document.body.appendChild(said); }
    bar.hidden = false;
    const before = main.firstElementChild;
    const skel = fresh ? setTimeout(() => { if (gen === navGen && main.firstElementChild === before) { main.innerHTML = NAV_SKELETON; said.textContent = 'Loading…'; slowNote(main, () => gen === navGen && !!main.querySelector(':scope > .navskel')); } }, 120) : 0;
    // Settles when the page has drawn its content (not when its slower extras arrive), when its code is
    // done, when a newer navigation takes over, or after 15 s at the most: the control that started it
    // (a sidebar place) and the bar never stay busy for good
    let mo = null; let cap = 0; let capped = false; let drawing = null;
    // A fresh page comes in with a short rise (and its first tiles and rows after one another); the
    // class goes after that, so what arrives later on the page doesn't move
    const pageIn = () => { if (!fresh) return; clearTimeout(main.viewInT); main.classList.remove('view-in'); void main.offsetWidth; main.classList.add('view-in'); main.viewInT = setTimeout(() => main.classList.remove('view-in'), 900); };
    const drawn = new Promise((res) => { mo = new MutationObserver(() => { if (gen === navGen && pageDrawn(before)) { if (drawing) drawing.on = false; pageIn(); res(); } }); /* a newer page's first draw isn't this one's */ mo.observe(main, { childList: true }); cap = setTimeout(() => { capped = true; res(); }, 15000); });
    const end = () => { clearTimeout(skel); clearTimeout(cap); mo.disconnect(); if (gen === navGen) { bar.hidden = true; said.textContent = ''; } };
    const t0 = performance.now(); const shown = state.view;
    fromNav = true; let rendering; try { rendering = render(); } finally { fromNav = false; } navRendering = rendering; drawing = lastDrawing;
    // Usage insights: how this page went (drawn, or its code failed or it never drew in 15 s); a
    // newer page first says nothing
    if (window.PM_USAGE) Promise.race([rendering.then(() => 'ok', () => 'failed'), superseded.then(() => null), drawn.then(() => (capped ? 'failed' : 'ok'))]).then((o) => { if (o) PM_USAGE.loaded(shown, o, performance.now() - t0); });
    return Promise.race([rendering, superseded, drawn]).finally(end);
  };

  // ---- render --------------------------------------------------------------
  // The page's code reads through its own `api`, tied to the navigation that draws it (navigate above)
  // (fromNav: this draw is a page being opened, not a redraw after a socket event or a timer;
  // usage insights count only the reads of a page opened or redrawn by a press)
  let lastDrawing = null; let fromNav = false;
  const render = () => {
    const drawing = { on: true, nav: fromNav || inHandler > 0 || pressing > 0 }; lastDrawing = drawing; // (or a press redraws it)
    // A page opened by a press is the page's own reading, not the press's
    const outer = inHandler; inHandler = 0; let p;
    try { p = renderPage(navApiFor(navGen, navCtl, drawing)); } finally { inHandler = outer; }
    const drawn = () => { drawing.on = false; markDownloads(); dropStaleSelect(); if (drawing === lastDrawing) pageTitle(); }; drawing.done = p.then(drawn, drawn);
    return p;
  };
  const renderPage = async (api) => {
    // The search box is a combobox over the results page only
    if (state.view !== 'search') { search.setAttribute('aria-expanded', 'false'); search.removeAttribute('aria-controls'); search.removeAttribute('aria-activedescendant'); $('#sprog').hidden = true; }
    const v = state.view;
    // The dashboard is a full screen of its own (a TV or tablet): no sidebar, bars or player
    document.body.classList.toggle('kiosk', v === 'dashboard');
    // (another page ends Select; this page drawn again keeps it where its list does, enableSelect)
    if (!selOn() || selNow.page !== selPage()) endSelect();
    const lists = {}; let listMine = 0; // (Albums, Artists: this draw's number, pageGen)
    let drawStale = null; // (a playlist: this draw is no longer the latest, plGen)
    try {
      switch (v) {

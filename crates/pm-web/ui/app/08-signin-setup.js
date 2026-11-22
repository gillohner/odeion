  // Play on a Spotify Connect device: the active one, else a menu of the devices online. Spotify streams it; the node only drives it.
  // Spotify can't share one app between nodes, so the owner makes their own: these steps, then only its client ID (PKCE through the return page, no secret)
  const spotSetup = (sp, hidden) => `<form class="stack" id="spotapp" ${hidden ? 'hidden' : ''}><ol class="muted small steps"><li>Open the <a href="https://developer.spotify.com/dashboard" target="_blank" rel="noopener">Spotify developer dashboard</a>, sign in with your Spotify account and choose <b>Create app</b> (any name and description).</li><li>Under <b>Redirect URIs</b> add this address, tick <b>Web API</b>, and save:<div class="toolbar"><code class="path">${esc(sp.redirect_uri)}</code><button type="button" class="btn ghost sm" id="spotcopy">Copy</button></div></li><li>Under <b>User Management</b>, add every Spotify account that will sign in here (up to 25).</li><li>Copy the <b>Client ID</b> from the app's Settings and paste it below. No secret is needed.</li></ol><div class="toolbar"><label class="sr-only" for="spotid">Spotify client ID</label><input id="spotid" name="id" class="input" autocomplete="off" spellcheck="false" placeholder="Client ID (32 letters and digits)" pattern="[0-9a-fA-F]{32}" required value="${esc(sp.client_id || '')}"><button class="btn primary">Save</button>${hidden ? '<button type="button" class="btn ghost" id="spotappcancel">Cancel</button>' : ''}</div></form>`;
  // The fallback for a sign-in whose last step could not reach this node (another network, an app allowing only
  // loopback redirects): paste the address the sign-in page landed on; the node finishes from its parameters.
  const pasteBack = (svc) => `<details class="small"><summary>Landed on a page that didn't load?</summary><form class="toolbar" id="${svc}paste"><input class="input" name="url" placeholder="Paste that page's address (…?code=…)" autocomplete="off" spellcheck="false" required aria-label="The address you landed on"><button class="btn sm" type="submit">Finish sign-in</button></form></details>`;
  const wirePasteBack = (svc, done) => { const f = $(`#${svc}paste`); if (f) f.onsubmit = async (e) => { e.preventDefault(); const b = f.querySelector('button'); b.disabled = true; try { await api('POST', `/sources/${svc}/callback`, { url: String(new FormData(f).get('url') || '').trim() }); toast(done); state.sources = null; render(); } catch (er) { toast(er.message, 'error'); b.disabled = false; } }; };
  const spotAppRow = (sp) => sp.app_from === 'settings' ? `<p class="muted small">Your Spotify app: <code class="path">${esc(sp.client_id || '')}</code> <button class="btn ghost sm" id="spotappchange">Change</button> <button class="btn ghost sm" id="spotappforget">Forget</button></p>` : sp.app_from === 'config' ? '<p class="muted small">This node’s Spotify app is set in its config.</p>' : '';
  const wireSpotSetup = () => {
    const f = $('#spotapp'); if (!f) return;
    const save = async (client_id, msg) => { try { await api('PUT', '/sources/spotify/app', { client_id }); toast(msg); render(); } catch (e) { toast(e.message, 'error'); } };
    f.onsubmit = (e) => { e.preventDefault(); return save(String(new FormData(f).get('id') || '').trim(), 'Spotify app saved. Now connect Spotify'); };
    const cp = $('#spotcopy'); if (cp) cp.onclick = () => { const t = f.querySelector('code').textContent; (navigator.clipboard ? navigator.clipboard.writeText(t) : Promise.reject()).then(() => toast('Redirect URI copied')).catch(() => toast(t)); };
    const ch = $('#spotappchange'); if (ch) ch.onclick = () => { f.hidden = false; $('#spotid').focus(); };
    const ca = $('#spotappcancel'); if (ca) ca.onclick = () => { f.hidden = true; };
    const fg = $('#spotappforget'); if (fg) fg.onclick = () => save('', 'Spotify app forgotten');
  };
  // One Sign in per source (owner, 2026-11-08: “make sure ui is simpler; remove all options that don't
  // work at the moment”): a first line saying what it gives and needs; only the ways that work here;
  // one of them is the action, the others a line each under one closed “Other ways to sign in”;
  // signed in, one row. A source's config gives `lead(si)` (that first line), `keep(w, si)` (drops a
  // way that is not a sign-in or not a working path), `actions(pick, si, h)` for its own ways' areas
  // (null = the shared import/paste area), `wire(pick, si, h, act)` for their handlers, `inUse(si)`
  // and `signOut(h)` where the sign-in shown is not the source's in-use one, `extra(si, pick)` for
  // rows under it, `after(si, h, el)` once rendered.
  const sgHelp = (h) => (h ? `<p class="set-help">${h}</p>` : '');
  // A long note's first sentence (or the clause before a colon), and the rest: settings show the
  // rest only where it is needed (UX sweep A25)
  const splitNote = (t) => { const x = String(t || ''); const m = /^(.{24,}?[.:])\s+(\S.{19,})$/s.exec(x); if (!m) return [x, '']; const colon = m[1].endsWith(':'); return [colon ? m[1].slice(0, -1) + '.' : m[1], colon ? m[2].charAt(0).toUpperCase() + m[2].slice(1) : m[2]]; };
  const sgRow = (label, h, ctl) => `<div class="set-row"><div class="set-label"><div class="set-name">${label}</div>${sgHelp(h)}</div>${ctl ? `<div class="set-ctl">${ctl}</div>` : ''}</div>`;
  const sgBlock = (label, h, body) => `<div class="set-row set-block">${label || h ? `<div class="set-label">${label ? `<div class="set-name">${label}</div>` : ''}${sgHelp(h)}</div>` : ''}<div class="set-body">${body}</div></div>`;
  const sgGroup = (title, body) => `<div class="set-group">${title ? `<h3 class="set-h">${title}</h3>` : ''}<div class="set-rows">${body}</div></div>`;
  // ---- Guided setup pieces (Sources › Qobuz and TIDAL full tracks, TASK-qobuz-tidal-setup-ux) ----
  const GUIDE = (file) => `https://github.com/gillohner/pubky-music-node/blob/main/docs/guides/${file}`;
  const guideLink = (file) => `<a href="${GUIDE(file)}" target="_blank" rel="noopener">Guide</a>`;
  // A numbered step card, a check mark once done. `sub` and `body` are HTML
  // What the node's owner sets up (node apps, device clients), in a listener's own sign-ins
  const ownerSets = (what) => `<p class="muted small">${what} is set up by whoever runs this node (Settings › This node); ask them. Your own account goes here once it is.</p>`;
  const stepCard = (no, title, done, sub, body) => `<div class="setup-step card-step ${done ? 'done' : ''}" data-step="${no}"><span class="stepmark" aria-hidden="true">${done ? icon('check') : no}</span><div class="grow"><div class="name">${no}. ${title}${done ? '<span class="sr-only"> (done)</span>' : ''}</div><div class="sub">${sub}</div>${body}</div></div>`;
  // A secret field with Show / Hide: a password until shown
  const secretInput = (name, label, placeholder, autocomplete = 'off', disabled = false) => `<span class="secretwrap"><input class="input" name="${name}" type="password" placeholder="${esc(placeholder)}" autocomplete="${autocomplete}" aria-label="${esc(label)}" spellcheck="false" required ${disabled ? 'disabled' : ''}><button class="icon-btn showpw" type="button" aria-label="Show ${esc(label)}" aria-pressed="false" title="Show">${icon('eye')}</button></span>`;
  // One name ("Show …") and the pressed state say it all; only the picture and the tooltip change
  const wireSecretToggles = (root) => $$('.showpw', root).forEach((b) => b.onclick = () => { const i = b.previousElementSibling; const show = i.type === 'password'; i.type = show ? 'text' : 'password'; b.setAttribute('aria-pressed', String(show)); b.title = show ? 'Hide' : 'Show'; b.innerHTML = icon(show ? 'eye-off' : 'eye'); });
  // A pasted block (`client_id=… client_secret=…`, a config section, JSON) split into its two values
  const splitAppPaste = (text) => {
    const t = String(text || ''); if (!/[=:]/.test(t)) return null;
    const idRe = /client_?id\b["']?\s*[=:]\s*["']?([A-Za-z0-9._-]{4,})/i;
    const secRe = /client_?secret\b["']?\s*[=:]\s*["']?([A-Za-z0-9+/=._-]{8,})/i;
    const id = idRe.exec(t); const sec = secRe.exec(t);
    return id || sec ? { id: id ? id[1] : '', secret: sec ? sec[1] : '' } : null;
  };
  const wireAppPaste = (form, kind) => {
    if (!form) return; const idF = form.querySelector('input[name=id]'); const secF = form.querySelector('input[name=secret]');
    [idF, secF].forEach((f) => f && f.addEventListener('paste', (e) => {
      const got = splitAppPaste(e.clipboardData && e.clipboardData.getData('text')); if (!got) return;
      e.preventDefault(); if (got.id) idF.value = got.id; if (got.secret) secF.value = got.secret;
      (got.id && got.secret ? form.querySelector('.btn[type=button]') || form.querySelector('button[type=submit]') : got.id ? secF : idF).focus();
      toast('Split into ID and secret');
    }));
  };
  // What a Check answered, in place: OK in green, the problem in red
  const showCheck = (out, r) => { if (!out) return; out.textContent = r.detail; out.classList.toggle('ok', !!r.ok); out.classList.toggle('bad', !r.ok); };
  const runCheck = async (btn, out, path, body) => { btn.disabled = true; out.textContent = 'Checking…'; out.className = 'check-out small'; try { showCheck(out, await api('POST', path, body)); } catch (e) { showCheck(out, { ok: false, detail: e.message }); } btn.disabled = false; };
  const signinChooser = async (cfg, si) => {
    const el = typeof cfg.el === 'string' ? $(cfg.el) : cfg.el; if (!el || !el.isConnected) return;
    if (!si) { try { si = await api('GET', `/sources/${cfg.source}/signin`); } catch (e) { if (el.isConnected) { el.innerHTML = `<div class="notice warn" role="alert">${esc(e.message)} <button class="btn sm" data-sg="retry">Try again</button></div>`; el.querySelector('[data-sg="retry"]').onclick = () => signinChooser(cfg); } return; } }
    if (!el.isConnected) return;
    const mem = state.signin = state.signin || {}; const m = mem[cfg.source] = mem[cfg.source] || {};
    const wayOf = (id) => si.ways.find((w) => w.id === id) || {};
    const inUse = cfg.inUse ? cfg.inUse(si) : si.in_use;
    // Only the ways that work here (owner, 2026-11-08): no greyed rows; the everyday ones first
    const usable = si.ways.filter((w) => w.available && (!cfg.keep || cfg.keep(w, si))).sort((a, b) => (a.advanced ? 1 : 0) - (b.advanced ? 1 : 0));
    const changing = !!inUse && !!m.change && usable.some((w) => w.id !== inUse.way);
    const choosing = !inUse || changing;
    const offered = changing ? usable.filter((w) => w.id !== inUse.way) : usable;
    const main = offered.find((w) => w.recommended) || offered[0];
    // The way opened here, else one with a sign-in under way (a code waiting), else the recommended one
    const pick = [m.way, cfg.prefer && cfg.prefer(si)].find((id) => id && offered.some((w) => w.id === id)) || (main || {}).id;
    const others = offered.filter((w) => w.id !== pick);
    const rows = [];
    const lead = cfg.lead ? cfg.lead(si) : '';
    if (lead) rows.push(`<p class="sg-lead" data-sg="lead">${lead}</p>`);
    if (cfg.before) rows.push(...[].concat(cfg.before(si) || []));
    // Signed in: one row, who and what it gives; Change only when another way works here
    if (inUse) {
      const who = inUse.user && !/^\d+$/.test(inUse.user) ? inUse.user : '';
      const gives = inUse.gives ? String(inUse.gives).charAt(0).toUpperCase() + String(inUse.gives).slice(1) : '';
      rows.push(`<div class="set-row sg-in" data-sg="signed-in"><div class="set-label"><div class="set-name">${who ? `Signed in as ${esc(who)}` : 'Signed in'}</div>${sgHelp([esc(inUse.label), esc(gives)].filter(Boolean).join(' · '))}</div><div class="set-ctl">${usable.some((w) => w.id !== inUse.way) ? `<button class="btn ghost sm" data-sg="change" aria-expanded="${changing}">${changing ? 'Cancel' : 'Change'}</button>` : ''}<button class="btn ghost sm" data-sg="out">Sign out</button></div></div>`);
    }
    if (choosing && !offered.length && !inUse) rows.push(`<p class="muted small" data-sg="none">${esc(cfg.none ? cfg.none(si) : ((si.ways.find((w) => w.reason) || {}).reason || `${cfg.name} can’t be signed in to on this node.`))}</p>`);
    if (choosing && offered.length) {
      // One way: just its button or form. Several: the picked one, the rest a line each under one closed list
      const pw = wayOf(pick);
      rows.push(`<div class="set-row set-block sg-main">${offered.length > 1 ? `<div class="set-label"><div class="set-name">${esc(pw.label)}</div>${sgHelp(esc(splitNote(pw.note)[0]))}</div>` : ''}<div class="set-body"><div class="way-action" data-sg="action"></div></div></div>`);
      if (others.length) rows.push(`<details class="sg-other" data-sg="other"><summary>Other ways to sign in</summary><div class="list">${others.map((w) => `<div class="list-row"><div class="grow"><div class="name">${esc(w.label)}</div><div class="sub">${esc(splitNote(w.note)[0])}</div></div><button class="btn ghost sm" data-sg-way="${esc(w.id)}" aria-label="Use ${esc(w.label)}">Use this</button></div>`).join('')}</div></details>`);
      rows.push(`<div class="notice warn" role="alert" data-sg="error" hidden><span data-sg="error-text"></span>${others.length ? '<div class="actions"><button class="btn sm" data-sg="another">Try another way</button></div>' : ''}</div>`);
    }
    if (cfg.extra) rows.push(...[].concat(cfg.extra(si, pick) || []));
    el.innerHTML = sgGroup(cfg.title === undefined ? 'Sign in' : cfg.title, rows.join(''));
    const h = {
      si, el, m, wayOf,
      pick: () => pick,
      choose: async (way) => { m.way = way; try { await api('PUT', `/sources/${cfg.source}/signin`, { way }); return true; } catch (e) { toast(e.message, 'error'); return false; } },
      refill: (now) => signinChooser(cfg, now),
      done: () => { m.change = false; m.way = null; state.sources = null; render(); },
      useImport: async (body, what) => { try { const r = await api('POST', `/sources/${cfg.source}/signin/import`, body); toast(`Saved ${r.in_use ? r.in_use.label : what}`); m.change = false; m.way = null; state.sources = null; render(); } catch (e) { const notice = el.querySelector('[data-sg="error"]'); if (notice) { notice.querySelector('[data-sg="error-text"]').textContent = e.message; notice.hidden = false; } else toast(e.message, 'error'); } },
      useButton: (w, who) => `<div class="toolbar"><button class="btn primary" data-sg="use">Use ${esc(w.label)}</button><span class="muted small">Already signed in with it${who || ''}.</span></div>`,
      // What other apps on this machine hold (read-only), the file itself, where the node looked
      importArea: (o = {}) => {
        const found = si.imports || []; const places = si.places || [];
        const when = (f) => f.usable ? (f.renews ? 'the node renews it' : f.expires_at ? 'until ' + new Date(f.expires_at * 1000).toLocaleDateString() : '') : f.reason;
        const list = found.length ? `<div class="list">${found.map((f) => `<div class="list-row"><div class="grow"><div class="name">${esc(f.app)}</div><div class="sub">${esc([f.user ? 'user ' + f.user : '', when(f), f.place].filter(Boolean).join(' · '))}</div></div>${f.usable ? `<button class="btn primary sm" data-import="${esc(f.id)}">Use it</button>` : '<span class="pill">not usable</span>'}</div>`).join('')}</div>` : '';
        const intro = o.intro || `${found.length ? 'Or the file itself' : 'The file itself'} (${esc(places.filter((p) => !/not read|not looked for/.test(p)).map((p) => p.split(':')[0]).join(', ') || o.what || 'the file')}); it is only read.`;
        const file = `<form class="stack" data-sg="file"><p class="muted small">${intro}</p><div class="toolbar"><input class="input" type="file" name="file" aria-label="${esc(o.fileLabel || 'The file')}" accept="${esc(o.accept || '')}"><button class="btn primary sm" type="submit">Use this file</button></div></form>`;
        const paste = o.withPaste ? h.pasteArea(o) : '';
        return `${list}${file}${paste}${places.length ? `<details class="small"><summary>Where the node looked</summary><ul class="muted small steps">${places.map((p) => `<li>${esc(p)}</li>`).join('')}</ul></details>` : ''}`;
      },
      pasteArea: (o = {}) => `<form class="stack" data-sg="paste"><textarea class="input" name="text" rows="${o.rows || 3}" aria-label="${esc(o.label || 'What to paste')}" placeholder="${esc(o.placeholder || '')}" autocomplete="off" spellcheck="false" required></textarea><div class="toolbar"><button class="btn primary sm" type="submit">Use it</button><span class="muted small">${esc(o.note || 'Kept like the node’s own sign-in; never shown again.')}</span></div></form>`,
    };
    const act = el.querySelector('[data-sg="action"]');
    if (act) {
      const w = wayOf(pick);
      let html = cfg.actions ? cfg.actions(pick, si, h) : null;
      if (html == null) html = w.action === 'import' ? h.importArea() : w.action === 'paste' ? h.pasteArea() : '';
      act.innerHTML = html;
      const useIt = act.querySelector('[data-sg="use"]'); if (useIt) useIt.onclick = async () => { if (await h.choose(pick)) { toast(`Using ${w.label || 'this way'}`); h.done(); } };
      act.querySelectorAll('[data-import]').forEach((b) => b.onclick = async () => { b.disabled = true; await h.useImport({ id: b.dataset.import }, b.dataset.import); b.disabled = false; });
      const ff = act.querySelector('[data-sg="file"]'); if (ff) ff.onsubmit = async (e) => { e.preventDefault(); const file = ff.querySelector('input[type=file]').files[0]; if (!file) { toast('Pick the file first', 'error'); return; } await h.useImport({ text: await file.text() }, 'the file'); };
      const pf = act.querySelector('[data-sg="paste"]'); if (pf) pf.onsubmit = (e) => { e.preventDefault(); return h.useImport({ text: pf.querySelector('textarea').value }, 'the pasted sign-in'); };
      if (cfg.wire) cfg.wire(pick, si, h, act);
    }
    // Another way swaps in as the action; the focus goes to it
    el.querySelectorAll('[data-sg-way]').forEach((b) => b.onclick = () => { m.way = b.dataset.sgWay; signinChooser(cfg, si).then(() => { const a = el.querySelector('[data-sg="action"]'); const f = a && a.querySelector('button, input, textarea, select, a[href]'); if (f) f.focus(); }); });
    const another = el.querySelector('[data-sg="another"]'); if (another) another.onclick = () => { const d = el.querySelector('[data-sg="other"]'); if (d) { d.open = true; const b = d.querySelector('[data-sg-way]'); if (b) b.focus(); } };
    const ch = el.querySelector('[data-sg="change"]'); if (ch) ch.onclick = () => { m.change = !m.change; m.way = null; signinChooser(cfg, si).then(() => { const a = m.change && el.querySelector('[data-sg="action"]'); const f = (a && a.querySelector('button, input, textarea, select, a[href]')) || el.querySelector('[data-sg="change"]'); if (f) f.focus(); }); };
    const out = el.querySelector('[data-sg="out"]'); if (out) out.onclick = async () => { try { if (cfg.signOut) { if ((await cfg.signOut(h)) === false) return; } else await api('DELETE', `/sources/${cfg.source}/signin`); toast(`Signed out of ${cfg.name}`); m.change = false; m.way = null; state.sources = null; render(); } catch (e) { toast(e.message, 'error'); } };
    if (cfg.after) cfg.after(si, h, el);
  };
  // TIDAL (owner, 2026-11-04 and 11-08): Sign in with TIDAL (the browser sign-in: full tracks,
  // lossless and hi-res) where this build can, else the code sign-in; the official app and a
  // pasted token under Other ways. Your own device client only once one is set here: TIDAL hands
  // none out. Quality and health under the full-tracks sign-in.
  const tidalBrowserSignIn = async (btn) => { if (btn) btn.disabled = true; try { const r = await api('GET', '/sources/tidal/login'); location.href = r.url; let tries = 0; const t = setInterval(async () => { tries++; try { const st = await api('GET', '/sources/tidal/status'); if (st.signed_in) { clearInterval(t); toast('TIDAL signed in'); render(); } } catch { /* keep polling */ } if (tries > 60) clearInterval(t); }, 3000); } catch (e) { toast(e.message, 'error'); if (btn) btn.disabled = false; } };
  // Sign in with TIDAL: TIDAL's sign-in in a new tab (opened at the click, so no popup blocker
  // stops it), then the page here asks for the address TIDAL lands on. In the desktop and Android
  // apps (window.pmDesktop) another origin's window.open goes to the system browser and a blank
  // tab would take the app's own window: TIDAL's address is opened there once it is known.
  const tidalPkceStart = async (h, btn) => {
    if (btn) btn.disabled = true;
    const inApp = !!window.pmDesktop;
    const tab = inApp ? null : window.open('about:blank', '_blank');
    try {
      if (!(await h.choose('browser'))) { if (tab) tab.close(); if (btn) btn.disabled = false; return; }
      const r = await api('POST', '/sources/tidal/unofficial/browser');
      h.m.browserUrl = r.url; h.m.way = 'browser';
      if (inApp) window.open(r.url, '_blank');
      else if (tab) { tab.opener = null; tab.location.href = r.url; } else toast(`${appPlatform() ? 'The app' : 'Your browser'} kept the tab from opening: use Open TIDAL’s sign-in`, 'error');
    } catch (e) { if (tab) tab.close(); toast(e.message, 'error'); }
    // The chooser is drawn again: whoever comes back from TIDAL's tab lands on the address field
    await h.refill();
    const field = $('#tbfinish input[name=text]'); if (field) field.focus();
  };
  const tidalPkceSteps = (u, h) => {
    const waiting = !!u.browser_pending;
    const open = h.m.browserUrl ? `<a class="btn ghost sm" href="${esc(h.m.browserUrl)}" target="_blank" rel="noopener">${icon('external')} Open TIDAL’s sign-in again</a>` : '';
    return stepCard(1, 'Sign in at TIDAL', waiting, 'Opens TIDAL’s sign-in in a new tab. Sign in there; TIDAL then shows a page that may say “Oops” or look empty. That is the one: keep it open.',
      `<div class="toolbar"><button class="btn primary" id="tbin">${icon('external')} ${waiting ? 'Sign in with TIDAL again' : 'Sign in with TIDAL'}</button>${open}</div>`)
      + (waiting ? stepCard(2, 'Paste its address', false, 'Copy the whole address of that TIDAL page from the address bar and paste it here.',
        `<form class="stack" id="tbfinish" autocomplete="off"><div class="toolbar"><input class="input" name="text" type="text" inputmode="url" placeholder="https://tidal.com/android/login/auth?code=…" aria-label="Address of the TIDAL page" autocomplete="off" spellcheck="false" required><button class="btn primary" type="submit">Done</button></div><p class="check-out small bad" id="tbfinisherr" role="alert" hidden></p></form>`) : '');
  };
  const tidalSigninCfg = (el) => ({
    source: 'tidal', name: 'TIDAL', el,
    lead: () => `Full tracks, lossless and hi-res, and your TIDAL collection and playlists. Needs a paid TIDAL plan. Unofficial: can stop working. ${guideLink('tidal-full-tracks.md')}`,
    keep: (w, si) => w.id !== 'own-device' || (!isListener() && (si.details.unofficial || {}).client_from === 'settings'),
    prefer: (si) => { const u = si.details.unofficial || {}; return u.pending ? si.chosen : u.browser_pending ? 'browser' : null; },
    actions: (pick, si, h) => {
      const u = si.details.unofficial || {}; const st = si.details.status || {}; const w = h.wayOf(pick);
      const own = !u.from && u.signed_in;
      if (pick === 'browser') return own && u.session_client === 'pkce' ? h.useButton(w, u.user ? ' as user ' + esc(u.user) : '') : tidalPkceSteps(u, h);
      if (pick === 'app') return st.signed_in ? h.useButton(w, st.user_id ? ' as user ' + esc(st.user_id) : '') : `<div class="toolbar"><button class="btn primary" id="tidalin">${icon('zone')} Sign in at TIDAL</button><span class="muted small">Opens TIDAL’s sign-in in ${appPlatform() ? 'your browser' : 'this browser'} and comes back here.</span></div>${st.redirect_uri && !isListener() && !/^Odeion/.test(w.label || '') ? `<details class="small"><summary>Your app’s settings</summary><p class="muted small">It must list <code>${esc(st.redirect_uri)}</code> as a redirect URI and have these scopes enabled (<code>[tidal] scopes</code> in the node config):</p><div class="toolbar"><code class="path">${esc(st.requested_scopes || '')}</code></div></details>` : ''}${pasteBack('tidal')}`;
      if (pick === 'device') {
        if (u.pending) return tidalCodeBox(u.pending);
        if (own && u.session_client === 'device') return h.useButton(w, u.user ? ' as user ' + esc(u.user) : '');
        return `<div class="toolbar"><button class="btn primary" id="tfin">${h.m.codeExpired ? 'New code' : 'Sign in with a code'}</button><span class="muted small">${h.m.codeExpired ? 'The code expired before it was entered. ' : ''}A code to enter at link.tidal.com on any device; no password goes through this node.</span></div>`;
      }
      if (pick === 'own-device') {
        // A device client replaces the built-in one for everyone on the node: the owner's to set
        if (isListener()) return ownerSets('A device client of the node’s own');
        const now = u.client_from === 'settings' ? '<p class="muted small">Your device client is set here and used for the code sign-in. <button class="btn ghost sm" id="tfclientforget">Forget it</button></p>'
          : u.client_from === 'config' ? '<p class="muted small">A device client is set in the node config; one entered here is used instead.</p>' : '';
        return now + stepCard(1, 'Device client ID and secret', u.client_from === 'settings', `For the code sign-in, for everyone here; the secret is never shown again. ${guideLink('tidal-full-tracks.md')}`,
          `<form class="stack" id="tfclient" autocomplete="off"><div class="toolbar"><input class="input" name="id" placeholder="Device client ID" autocomplete="off" spellcheck="false" aria-label="Device client ID" required>${secretInput('secret', 'Device client secret', 'Device client secret')}</div><div class="toolbar"><button class="btn" type="button" id="tfcheck">Check</button><button class="btn primary" type="submit">Save and sign in with a code</button><span class="check-out small" id="tfcheckout" role="status"></span></div><p class="muted small">Paste a whole <code>client_id=… client_secret=…</code> block or JSON into either field: it is split.</p></form>`);
      }
      if (pick === 'import') return h.importArea({ fileLabel: 'Token file', accept: '.json,.toml,.txt,application/json' });
      if (pick === 'paste') return h.importArea({ withPaste: true, fileLabel: 'Token file', accept: '.json,.toml,.txt,application/json', label: 'Token file contents or access token', placeholder: 'Token file contents, or an access token (a refresh token on the next line)' });
      return null;
    },
    wire: (pick, si, h, act) => {
      const a = $('#tidalin'); if (a) a.onclick = async () => { if (await h.choose('app')) tidalBrowserSignIn(a); };
      wirePasteBack('tidal', 'TIDAL signed in');
      const tb = $('#tbin'); if (tb) tb.onclick = () => tidalPkceStart(h, tb);
      const tf = $('#tbfinish'); if (tf) tf.onsubmit = async (e) => {
        e.preventDefault(); const btn = tf.querySelector('button[type=submit]'); const err = $('#tbfinisherr'); btn.disabled = true; err.hidden = true;
        try { await api('POST', '/sources/tidal/unofficial/browser/finish', { text: new FormData(tf).get('text') }); h.m.browserUrl = null; toast('TIDAL signed in: full tracks, lossless'); h.done(); }
        catch (er) { err.textContent = er.message; err.hidden = false; btn.disabled = false; }
      };
      const d = $('#tfin'); if (d) d.onclick = async () => { d.disabled = true; h.m.codeExpired = false; if (await h.choose('device')) { try { await api('POST', '/sources/tidal/unofficial/login'); } catch (e) { toast(e.message, 'error'); } } h.refill(); };
      const cf = $('#tfclient'); if (cf) {
        wireSecretToggles(cf); wireAppPaste(cf, 'tidal');
        const tc = $('#tfcheck'); if (tc) tc.onclick = () => { const f = new FormData(cf); runCheck(tc, $('#tfcheckout'), '/sources/tidal/unofficial/client/check', { client_id: f.get('id'), client_secret: f.get('secret') }); };
        cf.onsubmit = async (e) => { e.preventDefault(); const f = new FormData(cf); cf.querySelector('button[type=submit]').disabled = true; try { await api('PUT', '/sources/tidal/unofficial', { client_id: f.get('id'), client_secret: f.get('secret') }); toast('Device client saved'); h.m.way = 'device'; if (await h.choose('device')) await api('POST', '/sources/tidal/unofficial/login'); } catch (er) { toast(er.message, 'error'); } h.refill(); };
      }
      const cfg = $('#tfclientforget'); if (cfg) cfg.onclick = async () => { try { await api('PUT', '/sources/tidal/unofficial', { client_id: '' }); toast('Device client forgotten'); } catch (e) { toast(e.message, 'error'); } h.refill(); };
    },
    // The full-tracks status, only when it says more than the signed-in row: Off, Problem, AAC
    // only, Waiting, or no way to full tracks in this build
    before: (si) => {
      const u = si.details.unofficial || {}; const broke = u.health && !u.health.ok; const lossy = !!(u.health && u.health.ok && u.health.lossy);
      // A session first (also one imported or pasted, which needs no device client here), then
      // a code waiting, then what is missing
      const st = u.signed_in
        ? (broke ? { cls: 'bad', label: 'Problem', text: `Switched itself off: ${u.health.detail}.` }
          : !u.enabled ? { cls: '', label: 'Off', text: 'Signed in, switched off here.' }
          // AAC only is a code sign-in's (TIDAL sends its device client AAC); a browser
          // session's AAC track is that track (the Health row below says so)
          : lossy && u.session_client !== 'pkce' ? { cls: 'warn', label: 'AAC only', text: 'Full tracks, but TIDAL sends them as AAC (lossy).' }
          : { cls: 'ok', label: 'Working', text: `Full tracks ${u.quality === 'high' ? 'up to 16-bit/44.1 kHz' : 'up to 24-bit/192 kHz'}.` })
        : u.pending ? { cls: 'warn', label: 'Waiting', text: 'Waiting for the code to be entered at link.tidal.com.' }
        : u.browser_pending ? { cls: 'warn', label: 'Waiting', text: 'Waiting for the address of the TIDAL page you landed on (step 2 below).' }
        : !u.client_from && !u.browser_available && !si.ways.some((w) => (w.id === 'browser' || w.id === 'device') && w.available) ? { cls: '', label: 'Not in this build', text: 'This build can’t sign in for full tracks itself; a token from another app (tidal-dl-ng and others) still works.' }
        : null;
      if (!st || st.cls === 'ok') return [];
      return [sgRow(`Full tracks <span class="pill ${st.cls}">${esc(st.label)}</span>`, `${esc(st.text)} ${guideLink('tidal-full-tracks.md')}`, (u.signed_in && !u.enabled && !broke ? '<button class="btn sm" id="tfon">Turn on</button>' : '') + (u.signed_in ? '<button class="btn ghost sm" id="tfout">Sign out</button>' : ''))];
    },
    extra: (si) => {
      const u = si.details.unofficial || {}; const st = si.details.status || {}; const rows = [];
      const deviceOn = !!(si.in_use && si.in_use.way !== 'app');
      // A code sign-in where the browser one is possible: one line, with the button
      const codeSession = u.signed_in && u.enabled && !u.from && u.session_client === 'device' && u.browser_available;
      const again = '<button class="btn primary sm" id="tbagain">Sign in with TIDAL</button>';
      if (u.health && !u.health.ok) rows.push(`<div class="notice warn" role="status">Full tracks switched themselves off: ${esc(u.health.detail)}. TIDAL plays 30-second previews until it works again.${u.from ? ' That sign-in came from ' + esc(u.from === 'pasted' ? 'a pasted token' : u.from) + '; pick another way below, or a newer file.' : ''}<div class="actions"><button class="btn sm" id="tfretry">Try again</button></div></div>`);
      if (deviceOn) {
        rows.push(sgRow('Quality', 'Max plays up to 24-bit/192 kHz where TIDAL has it.', `<select class="input" id="tfq" aria-label="TIDAL full track quality"><option value="max" ${u.quality === 'max' ? 'selected' : ''}>Max (up to 24-bit/192 kHz)</option><option value="high" ${u.quality === 'high' ? 'selected' : ''}>High (16-bit/44.1 kHz)</option></select>`));
        if (u.health && u.health.ok && u.health.lossy) rows.push(sgRow('Health', `${esc(u.health.detail)}.`, `<span class="pill warn">AAC</span>${codeSession ? again : ''}`));
        else {
          if (codeSession) rows.push(sgRow('Lossless', 'Sign in with TIDAL again for lossless.', again));
          if (u.health && u.health.ok) rows.push(sgRow('Health', `Working: ${esc(u.health.detail)}.`, '<span class="pill ok">OK</span>'));
        }
        if (!st.signed_in && st.configured && si.ways.some((w) => w.id === 'app' && w.available)) rows.push(sgRow('Your collection', `Importing your collection and playlists needs ${esc(((si.ways.find((w) => w.id === 'app') || {}).label || 'the official app').replace(/^Your /, 'your '))} too.`, '<button class="btn sm" id="tsiapp2">Sign in with it too</button>'));
      } else if (si.in_use && st.playback_limit) rows.push(sgRow('Playback', `30-second previews with this sign-in.${si.ways.some((w) => (w.id === 'browser' || w.id === 'device') && w.available) ? ' For full tracks: Change.' : ''}`, '<span class="pill warn">previews</span>'));
      // The page has no general Status row (the full-tracks status leads): a problem with the
      // sign-in in use still shows, as a phone has no source list beside it
      const src = (state.sources || []).find((x) => x.id === 'tidal'); const ss = src && srcState({ ...src, enabled: true });
      if (ss && ss.cls === 'bad') rows.push(sgRow(`Status <span class="pill bad">${esc(ss.label)}</span>`, esc(ss.long), ''));
      return rows;
    },
    after: (si, h) => {
      const u = si.details.unofficial || {};
      const putU = async (body, ok) => { try { await api('PUT', '/sources/tidal/unofficial', body); if (ok) toast(ok); } catch (e) { toast(e.message, 'error'); } h.refill(); };
      // Select the retained full-tracks session as well as enabling it. If its original sign-in
      // method is no longer available, paste selects the same session without replacing it.
      const sessionWay = u.from === 'pasted' ? 'paste' : u.from ? 'import' : u.session_client === 'pkce' ? 'browser' : 'device';
      const resumeWay = [sessionWay, 'paste'].find((id) => si.ways.some((w) => w.id === id && w.available));
      const resume = async (ok) => { if (resumeWay && await h.choose(resumeWay)) { toast(ok); h.done(); } };
      const tr = $('#tfretry'); if (tr) tr.onclick = () => u.signed_in ? resume('Trying full tracks again') : putU({ enabled: true }, 'Trying full tracks again');
      const ton = $('#tfon'); if (ton) ton.onclick = () => resume('TIDAL full tracks are on');
      const tq = $('#tfq'); if (tq) tq.onchange = () => putU({ quality: tq.value }, 'Quality saved');
      const a2 = $('#tsiapp2'); if (a2) a2.onclick = () => tidalBrowserSignIn(a2);
      // From a code sign-in to the browser one: the chooser opens on it and TIDAL's tab opens
      const ag = $('#tbagain'); if (ag) ag.onclick = () => { h.m.change = true; h.m.way = 'browser'; tidalPkceStart(h, ag); };
      const out = $('#tfout'); if (out) out.onclick = async () => { try { await api('DELETE', '/sources/tidal/unofficial/login'); toast('Signed out of TIDAL full tracks'); h.m.change = false; h.m.codeExpired = false; h.m.browserUrl = null; state.sources = null; render(); } catch (e) { toast(e.message, 'error'); } };
      clearTimeout(state.tidalFullPoll); clearInterval(state.tidalCountdown);
      if (u.pending) {
        // The time left, each second; gone, the page says so and offers a new code
        const count = $('#tfcount'); const ends = Number(u.pending.expires_at) * 1000;
        const tick = () => { if (!count || !count.isConnected) { clearInterval(state.tidalCountdown); return; } const left = Math.max(0, Math.round((ends - Date.now()) / 1000)); count.textContent = left > 0 ? `The code is good for ${Math.floor(left / 60)}:${String(left % 60).padStart(2, '0')}` : 'The code expired'; if (left <= 0) { clearInterval(state.tidalCountdown); h.m.codeExpired = true; } };
        tick(); state.tidalCountdown = setInterval(tick, 1000);
        // The node notices the approval; this page asks every few seconds and continues by itself
        state.tidalFullPoll = setTimeout(async () => { let now; try { now = await api('GET', '/sources/tidal/signin'); } catch { return; } const nu = now.details.unofficial || {}; if (now.in_use && now.in_use.way === 'device' && !(si.in_use && si.in_use.way === 'device')) { h.m.codeExpired = false; toast('TIDAL signed in: full tracks'); h.done(); } else { if (!nu.pending && !nu.signed_in) h.m.codeExpired = true; h.refill(now); } }, 3000);
      }
    },
  });
  // The device code, big: Open link.tidal.com fills it in (TIDAL's complete verification address);
  // Copy for a device where that doesn't work; the time left under it (wired in `after`)
  const tidalCodeBox = (p) => `<div class="codebox" id="tfpending"><p class="small">Open link.tidal.com, sign in to TIDAL and enter this code. The button opens it with the code filled in. This page continues by itself once TIDAL says yes.</p><div class="toolbar"><code class="tf-code big" id="tfcode">${esc(p.user_code)}</code><button class="btn ghost sm" type="button" data-copylink="${esc(p.user_code)}" aria-label="Copy the code">${icon('copy')} Copy</button></div><div class="toolbar"><a class="btn primary" id="tfopen" href="${esc(webUrl(p.uri))}" target="_blank" rel="noopener">${icon('external')} Open link.tidal.com</a><span class="muted small" id="tfcount" role="status"></span></div></div>`;
  // ---- Qobuz: one account card; web-player app discovery happens on the node ----
  const qobuzSetup = async (el) => {
    if (!el || !el.isConnected) return;
    let u;
    try { u = await api('GET', '/sources/qobuz/unofficial'); } catch (e) { if (el.isConnected) el.innerHTML = `<div class="notice warn" role="alert">${esc(e.message)}</div>`; return; }
    if (!el.isConnected) return;
    const broke = !!(u.health && !u.health.ok);
    const quality = u.quality === 'mp3' ? 'MP3 320 kbps' : u.quality === 'cd' || !u.hires ? 'up to 16-bit/44.1 kHz' : 'up to 24-bit/192 kHz';
    const st = !u.signed_in ? { cls: '', label: 'Needs sign-in', text: 'Sign in with your Qobuz account below.' }
      : broke ? { cls: 'bad', label: 'Problem', text: `Switched itself off: ${u.health.detail}.` }
      : !u.enabled ? { cls: '', label: 'Off', text: 'Signed in, switched off here.' }
      : { cls: 'ok', label: 'Working', text: `Full tracks ${quality}.` };
    const lead = `<p class="sg-lead" data-sg="lead">Full tracks in CD or hi-res quality. Needs a paid Qobuz plan. Unofficial: can stop working. ${guideLink('qobuz.md')}</p>`;
    const account = u.signed_in
      ? `<div class="set-row sg-in" data-sg="signed-in"><div class="set-label"><div class="set-name">Signed in${u.user ? ' as ' + esc(u.user) : ''} <span class="pill ${st.cls}">${esc(st.label)}</span></div>${sgHelp(esc([u.plan, st.text].filter(Boolean).join(' · ')))}</div><div class="set-ctl"><input type="checkbox" class="switch" role="switch" id="qbon" ${u.enabled ? 'checked' : ''} aria-label="Qobuz full tracks"><button class="btn ghost sm" id="qbout">Sign out</button></div></div>`
        + (broke ? `<div class="notice warn" role="status">${esc(u.health.detail)}. Qobuz links still open on qobuz.com.<div class="actions"><button class="btn sm" id="qbretry">Try again</button></div></div>` : '')
      : sgRow(`Full tracks <span class="pill">Needs sign-in</span>`, 'Your email and password go to Qobuz once; the node keeps only the session.', '')
        + `<form class="stack" id="qbin"><div class="toolbar"><input class="input" name="email" type="email" placeholder="Qobuz email" autocomplete="username" aria-label="Qobuz email" required>${secretInput('password', 'Qobuz password', 'Password', 'current-password')}<button class="btn primary" type="submit">Sign in</button></div><div class="notice warn" role="alert" id="qbinerr" hidden></div></form>`;
    const playback = u.signed_in ? sgGroup('Playback', sgRow('Quality', 'Max plays up to 24-bit/192 kHz where Qobuz has it and your plan allows.', `<select class="input" id="qbq" aria-label="Qobuz quality"><option value="max" ${u.quality === 'max' ? 'selected' : ''}>Max (up to 24-bit/192 kHz)</option><option value="cd" ${u.quality === 'cd' ? 'selected' : ''}>CD (16-bit/44.1 kHz)</option><option value="mp3" ${u.quality === 'mp3' ? 'selected' : ''}>MP3 (320 kbps)</option></select>`)
      + (u.health && u.health.ok ? sgRow('Health', `Working: ${esc(u.health.detail)}.`, '<span class="pill ok">OK</span>') : '')) : '';
    el.innerHTML = sgGroup('Sign in with your Qobuz account', lead + account) + playback;
    const refill = () => qobuzSetup(el);
    const put = async (body, ok) => { try { await api('PUT', '/sources/qobuz/unofficial', body); if (ok) toast(ok); } catch (e) { toast(e.message, 'error'); } state.sources = null; refill(); };
    const on = $('#qbon'); if (on) on.onchange = (e) => put({ enabled: e.target.checked }, e.target.checked ? 'Qobuz full tracks are on' : 'Qobuz full tracks are off');
    const retry = $('#qbretry'); if (retry) retry.onclick = () => put({ enabled: true }, 'Trying Qobuz again');
    const signOut = async () => { try { await api('DELETE', '/sources/qobuz/unofficial/login'); toast('Signed out of Qobuz'); } catch (e) { toast(e.message, 'error'); } state.sources = null; render(); };
    const so = $('#qbout'); if (so) so.onclick = signOut;
    const qi = $('#qbin'); if (qi) {
      wireSecretToggles(qi);
      qi.onsubmit = async (e) => {
        e.preventDefault(); const f = new FormData(qi); const b = qi.querySelector('button[type=submit]'); const err = $('#qbinerr'); b.disabled = true; err.hidden = true;
        try { await api('POST', '/sources/qobuz/unofficial/login', { email: f.get('email'), password: f.get('password') }); toast('Signed in to Qobuz'); state.sources = null; render(); } catch (er) { err.textContent = er.message; err.hidden = false; b.disabled = false; }
      };
    }
    const qq = $('#qbq'); if (qq) qq.onchange = () => put({ quality: qq.value }, 'Quality saved');
  };
  // Spotify (owner, 2026-11-08): one sign-in, your own free developer app then Connect, in two
  // numbered steps. librespot and Soloist are ways to play, not to sign in: they sit under Other
  // ways to play on the Spotify tab, and only when they can run on this machine.
  const spotifyAppInUse = (si) => {
    if (si.in_use && si.in_use.way === 'app') return si.in_use;
    const st = si.details.status || {};
    return st.signed_in ? { way: 'app', label: 'Your own Spotify app', user: st.display_name || '', gives: st.product === 'premium' ? 'Connect playback on your devices, your library' : '' } : null;
  };
  const spotifySigninCfg = (el) => ({
    source: 'spotify', name: 'Spotify', el,
    // (a listener: the node's app is the owner's to set up, so no developer app to make)
    lead: (si) => `Control Spotify on your devices and import your library. Needs Spotify Premium${isListener() || (si.ways.find((w) => w.id === 'app') || {}).available === false || (si.details.status || {}).configured ? '' : ' and a free Spotify developer app (2 minutes)'}.`,
    keep: (w) => w.id === 'app',
    inUse: spotifyAppInUse,
    none: (si) => (si.ways.find((w) => w.id === 'app') || {}).reason || 'Spotify isn’t set up on this node yet.',
    // Sign out of the app, also while librespot is the way picked last
    signOut: async () => { await api('DELETE', '/sources/spotify/signin?way=app'); return true; },
    actions: (pick, si) => {
      const st = si.details.status || {};
      if (pick !== 'app') return null;
      // A listener signs in with the node's app: the owner sets it up (Settings › This node)
      if (isListener()) return `<div class="setup-steps">${stepCard(1, 'The node’s Spotify app', !!st.configured, st.configured ? 'Set up by whoever runs this node.' : '', st.configured ? '' : ownerSets('The node’s Spotify app'))}`
        + stepCard(2, 'Connect Spotify', false, st.configured ? 'Opens Spotify’s sign-in and comes back here.' : 'After step 1.', st.configured ? `<div class="toolbar"><button class="btn primary" id="spotin">${icon('zone')} Connect Spotify</button></div>${pasteBack('spotify')}` : '') + '</div>';
      const app = st.app_from === 'config' ? '<p class="muted small">Set up on this node.</p>' : spotAppRow(st);
      return `<div class="setup-steps">${stepCard(1, 'Your free developer app', !!st.configured, st.configured ? 'Done.' : 'At Spotify, once:', st.configured ? app + (st.app_from === 'settings' ? spotSetup(st, true) : '') : spotSetup(st, false))}`
        + stepCard(2, 'Connect Spotify', false, st.configured ? 'Opens Spotify’s sign-in and comes back here.' : 'After step 1.', st.configured ? `<div class="toolbar"><button class="btn primary" id="spotin">${icon('zone')} Connect Spotify</button></div>${pasteBack('spotify')}` : '') + '</div>';
    },
    wire: (pick, si, h) => {
      const sp = $('#spotin'); if (sp) sp.onclick = async () => { if (await h.choose('app')) { try { const r = await api('GET', '/sources/spotify/login'); location.href = r.url; } catch (e) { toast(e.message, 'error'); } } };
      wireSpotSetup();
      wirePasteBack('spotify', 'Spotify signed in');
    },
  });
  // YouTube: your Data API key for search; the cookies file for yt-dlp only once YouTube plays on
  // your speakers (`speakers`), as it does nothing before.
  const youtubeSigninCfg = (el, speakers) => ({
    source: 'youtube', name: 'YouTube', el,
    lead: () => `Search YouTube here and play it ${hereIn()}. Search needs a free YouTube Data API key.`,
    keep: (w) => w.id !== 'cookies' || !!speakers,
    actions: (pick, si, h) => {
      const key = (si.details.key || {}).set;
      if (pick === 'key') return key ? `<div class="toolbar"><span class="pill ok account">Search on · your API key</span><button class="btn ghost sm" data-yt="forget">Forget the key</button></div><p class="muted small">Each search spends 100 of the key’s 10,000 free daily units, so YouTube is searched only when you pick it, never in the everyday search.</p>` : `<form class="stack" data-yt="key"><p class="muted small">In the <a href="https://console.cloud.google.com/apis/library/youtube.googleapis.com" target="_blank" rel="noopener">Google Cloud console</a>, turn on the YouTube Data API v3, create an API key under Credentials and paste it here.</p><div class="toolbar"><label class="sr-only" for="ytkeyin">YouTube Data API key</label><input id="ytkeyin" class="input" name="key" type="password" autocomplete="off" spellcheck="false" placeholder="YouTube Data API key" required><button class="btn primary" type="submit">Save</button></div></form>`;
      if (pick === 'cookies') return h.importArea({ fileLabel: 'Cookies file', accept: '.txt,text/plain', what: 'the cookies file', intro: 'The Netscape cookies file you export for yt-dlp; the copy is kept readable by the node only, and yt-dlp on your speakers plays as your account.', withPaste: true, label: 'Cookies file contents', placeholder: '# Netscape HTTP Cookie File …', rows: 4 });
      return null;
    },
    wire: (pick, si, h, act) => {
      const kf = act.querySelector('[data-yt="key"]'); if (kf) kf.onsubmit = async (e) => { e.preventDefault(); const b = kf.querySelector('button'); b.disabled = true; try { await api('PUT', '/sources/youtube/key', { key: String(new FormData(kf).get('key') || '').trim() }); toast('YouTube key saved'); if (await h.choose('key')) h.done(); } catch (er) { toast(er.message, 'error'); b.disabled = false; } };
      const kg = act.querySelector('[data-yt="forget"]'); if (kg) kg.onclick = async () => { try { await api('PUT', '/sources/youtube/key', { key: '' }); toast('YouTube key forgotten'); h.done(); } catch (e) { toast(e.message, 'error'); } };
    },
  });
  // Last.fm (Settings › Accounts): your own API account then Connect, in two steps; a sign-in
  // found on this machine (rescrobbled) only when there is one, or the three values pasted.
  // Scrobbling on/off and the waiting plays stay as rows.
  const lastfmSigninCfg = (el) => ({
    source: 'lastfm', name: 'Last.fm', el, title: '',
    lead: () => 'Sends what you play to your Last.fm profile. Needs a free Last.fm API account (2 minutes).',
    keep: (w, si) => w.id !== 'import' || (si.imports || []).length > 0,
    actions: (pick, si, h) => {
      const lf = si.details.lastfm || {};
      if (pick === 'own') {
        const appForm = `<form class="stack" data-lf="app"><p class="muted small"><a href="https://www.last.fm/api/account/create" target="_blank" rel="noopener">Create an API account</a> (any name, no callback), then paste its API key and shared secret.</p><div class="toolbar"><input class="input" name="key" placeholder="API key" autocomplete="off" spellcheck="false" pattern="[0-9a-fA-F]{32}" aria-label="Last.fm API key" required><input class="input" name="secret" type="password" placeholder="Shared secret" autocomplete="off" aria-label="Last.fm shared secret" required><button class="btn" type="submit">Save</button></div></form>`;
        const appRow = lf.app_from === 'config' ? '<p class="muted small">Set up on this node.</p>' : `<p class="muted small">Your Last.fm app: <code class="path">${esc(lf.api_key || '')}</code> <button class="btn ghost sm" data-lf="appforget">Forget</button></p>`;
        return `<div class="setup-steps">${stepCard(1, 'Your free Last.fm API account', !!lf.configured, lf.configured ? 'Done.' : 'At Last.fm, once:', lf.configured ? appRow : appForm)}`
          + stepCard(2, 'Connect Last.fm', false, lf.configured ? 'Opens Last.fm’s permission page and comes back here.' : 'After step 1.', lf.configured ? '<div class="toolbar"><button class="btn primary" data-lf="in">Connect Last.fm</button></div>' : '') + '</div>';
      }
      if (pick === 'import') return h.importArea({ fileLabel: 'Last.fm key, secret and session key', accept: '.toml,.txt', intro: 'From another machine, use a text file with three lines: the API key, shared secret and session key. rescrobbled’s config alone does not contain the session.', withPaste: true, label: 'API key, shared secret and session key', placeholder: 'API key\nshared secret\nsession key' });
      if (pick === 'paste') return h.pasteArea({ label: 'API key, shared secret and session key', placeholder: 'API key\nshared secret\nsession key', rows: 3 });
      return null;
    },
    wire: (pick, si, h, act) => {
      const af = act.querySelector('[data-lf="app"]'); if (af) af.onsubmit = async (e) => { e.preventDefault(); const f = new FormData(af); try { await api('PUT', '/lastfm', { api_key: String(f.get('key') || '').trim(), api_secret: String(f.get('secret') || '').trim() }); toast('Last.fm app saved'); h.refill(); } catch (er) { toast(er.message, 'error'); } };
      const ag = act.querySelector('[data-lf="appforget"]'); if (ag) ag.onclick = async () => { try { await api('PUT', '/lastfm', { api_key: '' }); toast('Last.fm app forgotten'); h.refill(); } catch (er) { toast(er.message, 'error'); } };
      const li = act.querySelector('[data-lf="in"]'); if (li) li.onclick = async () => { if (await h.choose('own')) { try { const r = await api('GET', '/lastfm/login'); location.href = r.url; } catch (er) { toast(er.message, 'error'); } } };
    },
    extra: (si) => {
      const lf = si.details.lastfm || {}; if (!lf.connected) return [];
      return [
        lf.problem ? `<div class="notice warn" role="status">${esc(lf.problem)}<div class="actions"><button class="btn sm" data-lf="again">Sign in again</button></div></div>` : '',
        sgRow('Send what I play', lf.pending ? `${lf.pending} plays wait for Last.fm; they go with the next play.` : 'Plays count after half the song or four minutes; now-playing at the start.', `<input type="checkbox" class="switch" role="switch" data-lf="submit" ${lf.submit ? 'checked' : ''} aria-label="Send what I play to Last.fm">`),
      ];
    },
    after: (si, h, el) => {
      const sw = el.querySelector('[data-lf="submit"]'); if (sw) sw.onchange = async (e) => { try { await api('PUT', '/lastfm', { submit: e.target.checked }); toast(e.target.checked ? 'Plays go to Last.fm' : 'Plays stay on this node'); } catch (er) { toast(er.message, 'error'); } };
      const ag = el.querySelector('[data-lf="again"]'); if (ag) ag.onclick = async () => { try { const r = await api('GET', '/lastfm/login'); location.href = r.url; } catch (er) { toast(er.message, 'error'); } };
    },
  });
  const spotifyPlay = async (uris, anchor) => {
    try {
      const devs = await api('GET', '/sources/spotify/devices');
      const go = async (device_id) => { try { await api('POST', '/sources/spotify/play', { uris, device_id }); toast('Playing on Spotify'); } catch (e) { toast(e.message, 'error'); } };
      if (!devs.length) return toast('No Spotify device online: open Spotify on a device first', 'error');
      const active = devs.find((d) => d.is_active);
      if (devs.length === 1 || (active && !anchor)) return go((active || devs[0]).id);
      showMenu(anchor || document.body, [{ title: 'Play on Spotify device' }, ...devs.map((d) => ({ label: `${d.name} (${d.type}${d.is_active ? ', active' : ''})`, icon: 'zone', run: () => go(d.id) }))]);
    } catch (e) { toast(e.message, 'error'); }
  };
  // TIDAL's official embed player in a panel: full-length playback for subscribers, inside this UI, no tier needed.
  const tidalEmbed = (kind, id, title) => { const p = $('#embedpanel'); $('#embedtitle').textContent = title || 'TIDAL player'; $('#embedframe').src = `https://embed.tidal.com/${kind}/${encodeURIComponent(id)}?layout=gridify`; p.hidden = false; };
  $('#embedclose').onclick = () => { const p = $('#embedpanel'); p.hidden = true; $('#embedframe').src = 'about:blank'; };
  // Back from a cloud drive's sign-in (drives.rs callback: /settings/library?drive=ok|error). The
  // answer waits for Node library's first drawing (driveReturn), then goes to that drive's row: a
  // drive with no folder yet opens its folder picker; a failure says why on the row while the app
  // stays open, until the drive shows signed in. (The provider is the one whose Sign in was pressed.)
  let driveBack = null; let driveFailed = null;
  { const sp = new URLSearchParams(location.search); const d = sp.get('drive'); if (d === 'ok' || d === 'error') { let provider = null; try { provider = sessionStorage.getItem('pm.driveLogin'); sessionStorage.removeItem('pm.driveLogin'); } catch { /* storage off */ } const msg = sp.get('message'); driveBack = d === 'ok' ? { provider, text: 'Drive signed in: add a folder' } : { provider, kind: 'error', why: msg || 'The drive gave no reason', text: 'Drive: ' + (msg || 'sign-in failed') }; history.replaceState(null, '', location.pathname); } }
  const driveFailedHtml = (why) => `<span class="warn-text" data-drv-failed>Last sign-in failed: ${esc(why)}</span>`;
  const driveReturn = (drv) => {
    if (!driveBack) return; const b = driveBack; driveBack = null;
    const ps = drv.providers.filter((p) => p.configured); const want = !b.why;
    // (the drive whose Sign in was pressed, if its state agrees with the answer; else, e.g. storage
    // off or a Sign in left half-way earlier, the only drive in the state the answer says)
    const fits = ps.filter((x) => !!x.connected === want);
    const p = fits.find((x) => x.id === b.provider) || (fits.length === 1 ? fits[0] : null);
    const group = [...$$('.set-group', main)].find((g) => /Cloud drives/.test(($('.set-h', g) || {}).textContent || ''));
    const into = (el) => { if (el) el.scrollIntoView({ block: 'center', behavior: 'instant' }); }; // (at once, as a page opened: .main glides)
    if (b.why) {
      const btn = p && !p.connected ? $(`[data-dr-login="${p.id}"]`, main) : null;
      if (btn) {
        driveFailed = { provider: p.id, why: b.why };
        const row = btn.closest('.set-row'); const help = $('.set-help', row);
        if (help && !$('[data-drv-failed]', help)) help.insertAdjacentHTML('beforeend', '<br>' + driveFailedHtml(b.why));
        into(row);
      } else into(group);
      if (btn) btn.focus({ preventScroll: true });
      // (the node's reasons mostly start with the drive's name already: "Google Drive refused the sign-in: …")
      toast(p && !b.why.toLowerCase().startsWith(p.name.toLowerCase()) ? `${p.name}: ${b.why}` : b.why, 'error');
      return;
    }
    const add = p && p.connected ? $(`[data-dr-add="${p.id}"]`, main) : null;
    if (!add) { into(group); toast(b.text); return; }
    into(add.closest('.set-row')); add.focus({ preventScroll: true }); // (a closed picker gives the focus back here)
    if (drv.roots.some((r) => r.provider === p.id)) { toast(`${p.name} signed in`); return; }
    add.click(); // its folder picker: the next step after a first sign-in
  };

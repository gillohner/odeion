  // ---- Log in / Log out (TASK-login-ux) -------------------------------------
  // Signed out, the page is one Log in screen instead of the app: Pubky Ring for the owner and
  // listeners alike (the node tells them apart), the node's API token as the small alternative.
  // Phones and tablets get "Open in Pubky Ring" first, desktops the QR code. The node's answer
  // names who approved when it refuses, so a wrong identity is said plainly.
  const LOGIN_WHY = { ended: 'Your session ended. Log in again.', out: 'You logged out.' };
  // This browser's own state that belongs to the account, not the device (theme and layout stay)
  const ACCOUNT_KEYS = ['pm.token', 'pm.zone', 'pm.browserQueue', 'pm.browserAt', 'pm.setupHidden', 'pm.bar'];
  // The auth middleware's answer to a missing or wrong token (lib.rs `auth`)
  const NODE_REFUSED = 'missing or wrong bearer token';
  // Set once the page forgot the account (log out, session ended): the page is about to reload,
  // and its last queue save must not write the old account's queue back
  let accountForgotten = false;
  const forgetAccount = () => {
    const mine = state.token; state.token = ''; accountForgotten = true;
    if (window.PM_FEEDBACK) PM_FEEDBACK.reset(); // a draft or open report goes with the sign-in
    if (window.PM_USAGE) PM_USAGE.end(); // and anything usage insights still had (never sent after)
    try {
      // Another tab may have logged in again meanwhile: its token stays
      if (localStorage.getItem('pm.token') !== mine) return;
      ACCOUNT_KEYS.forEach((k) => localStorage.removeItem(k));
      Object.keys(localStorage).filter((k) => k.startsWith('pm.up.') || k.startsWith('pm.srcmine.')).forEach((k) => localStorage.removeItem(k));
    } catch { /* no storage */ }
  };
  const ringFirst = () => matchMedia('(pointer: coarse)').matches || /Android|iPhone|iPad|iPod|Mobile/i.test(navigator.userAgent);
  const loginFace = (f, cls = '') => (f && f.avatar && (/^data:image\//.test(f.avatar) || f.avatar === '/v1/auth/pubky/owner/avatar')
    ? `<img class="face ${cls}" src="${esc(f.avatar)}" alt="">`
    : `<span class="face mono ${cls}" style="background:hsl(${hue(f && f.id)} 35% 34%)">${esc(initials((f && f.name) || '?'))}</span>`);
  // One poll at a time: `timer` is a poll waiting to run, `asking` one under way
  // `renewal`: the owner logged out while the node needs new folders, with a code from the node
  // (good until a renewal), so this log in asks for them too, also after Try again or an expired
  // QR code. Only then: anyone scanning a plain QR code is asked to read, nothing more
  const lg = { flow: null, timer: null, asking: false, showQr: false, gen: 0, renewal: null };
  const loginStop = () => { clearTimeout(lg.timer); lg.timer = null; lg.gen += 1; };
  const drawLoginFlow = () => {
    const f = lg.flow; if (!f) return;
    const owner = f.owner_face || { id: f.owner, name: '' };
    $('#loginface').innerHTML = loginFace(owner);
    $('#loginh').textContent = f.node_name ? `Log in to ${f.node_name}` : 'Log in';
    const qr = f.qr_svg ? `<div class="loginqr" role="img" aria-label="QR code to scan with Pubky Ring">${f.qr_svg}</div>` : '';
    const open = (cls) => `<a class="btn ${cls}" id="loginring" href="${esc(ringUrl(f.url))}">Open in Pubky Ring</a>`;
    const wait = '<p class="loginwait" id="loginwait"><span class="spin" aria-hidden="true"></span>Waiting for Pubky Ring… this page continues by itself.</p>';
    const renew = f.renews ? `<p class="muted small">Pubky Ring also asks you to approve the new folders this node needs.</p>` : '';
    $('#loginbody').innerHTML = ringFirst()
      ? `${open('primary loginmain')}${lg.showQr ? qr : ''}<button class="btn ghost sm" type="button" id="loginqrbtn" aria-expanded="${lg.showQr}">${lg.showQr ? 'Hide QR code' : 'Show QR code'}</button>${wait}${renew}`
      : `${qr}<p class="muted">Scan with Pubky Ring on your phone.</p>${open('ghost sm')}${wait}${renew}`;
    const qb = $('#loginqrbtn'); if (qb) qb.onclick = () => { lg.showQr = !lg.showQr; drawLoginFlow(); };
  };
  const drawRefused = (p) => {
    const owner = p.owner_face || (lg.flow && lg.flow.owner_face) || { name: 'its owner' };
    const them = p.approver || { id: p.user, name: (p.user || '').slice(0, 12) + '…' };
    const msg = p.reason === 'no_access'
      ? `${esc(them.name)} doesn’t have access to the music here. Ask ${esc(owner.name)} to give you access.`
      : `This node belongs to ${esc(owner.name)}. You approved as ${esc(them.name)}. Pick ${esc(owner.name)} in Pubky Ring and try again.`;
    $('#loginbody').innerHTML = `<div class="loginwho"><figure>${loginFace(owner)}<figcaption>${esc(owner.name)}<br><span class="muted small">this node’s owner</span></figcaption></figure><figure>${loginFace(them, 'wrong')}<figcaption>${esc(them.name)}<br><span class="muted small">you approved as</span></figcaption></figure></div>`
      + `<p class="loginmsg" id="loginrefused" role="alert">${msg}</p>`
      + (p.reason === 'no_access' ? '' : `<p class="muted small">To listen here as ${esc(them.name)}, ask ${esc(owner.name)} to give you access.</p>`)
      + '<button class="btn primary loginmain" type="button" id="loginagain">Try again</button>';
    $('#loginagain').onclick = () => { $('#loginwhy').hidden = true; loginStart(); };
    $('#loginagain').focus();
  };
  const loginPoll = (gen, wait = 2000) => {
    lg.timer = setTimeout(async () => {
      lg.timer = null;
      if (gen !== lg.gen) return;
      let p; lg.asking = true;
      try { const r = await fetch(`/v1/auth/pubky/poll?id=${encodeURIComponent(lg.flow.id)}`); p = await r.json(); if (!r.ok) throw new Error(p.error || `HTTP ${r.status}`); } catch { lg.asking = false; if (gen === lg.gen) loginPoll(gen); return; } finally { lg.asking = false; }
      if (gen !== lg.gen) return;
      if (p.status === 'pending') return loginPoll(gen);
      if (p.status === 'ok' && p.token) {
        loginStop(); lg.renewal = null; state.token = p.token; try { localStorage.setItem('pm.token', p.token); } catch { /* this tab keeps it */ }
        hideLogin(); connect(); return;
      }
      if (p.status === 'denied') { loginStop(); drawRefused(p); return; }
      loginStart(); // the sign-in expired (ten minutes): a fresh one, quietly
    }, wait);
  };
  const loginStart = async () => {
    loginStop(); const gen = lg.gen;
    $('#loginbody').innerHTML = '<p class="loginwait"><span class="spin" aria-hidden="true"></span>Starting…</p>';
    try {
      const r = await fetch('/v1/auth/pubky/start', lg.renewal ? { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ renewal: lg.renewal }) } : { method: 'POST' }); const j = await r.json();
      if (gen !== lg.gen) return;
      if (r.status === 412) {
        $('#loginbody').innerHTML = '<p class="muted">Pubky Ring can’t log you in here right now: this node isn’t linked to a Pubky identity, or couldn’t reach it when it started. Use its API token below.</p>';
        $('#logintoken').open = true; return;
      }
      if (!r.ok) throw new Error(j.error || `HTTP ${r.status}`);
      lg.flow = j; drawLoginFlow(); loginPoll(gen);
    } catch (e) {
      if (gen !== lg.gen) return;
      $('#loginbody').innerHTML = `<p class="muted">Can’t start a log in: ${esc(e.message)}</p><button class="btn primary" type="button" id="loginretry">Try again</button>`;
      $('#loginretry').onclick = () => loginStart();
    }
  };
  // Back from Pubky Ring: ask at once instead of waiting for the next tick (phones pause timers)
  document.addEventListener('visibilitychange', () => { if (!document.hidden && lg.flow && lg.timer && !lg.asking) { clearTimeout(lg.timer); loginPoll(lg.gen, 0); } });
  const showLogin = (why) => {
    if (signedOut()) return;
    dropSocket(); clearTimeout(link.retry); link.retry = null;
    document.body.classList.add('signedout');
    for (const d of document.querySelectorAll('dialog[open]')) d.close();
    $('#loginpage').hidden = false;
    const w = $('#loginwhy'); w.hidden = !LOGIN_WHY[why]; w.textContent = LOGIN_WHY[why] || '';
    $('#tok').value = ''; $('#logintoken').open = false;
    try { lg.renewal = sessionStorage.getItem('pm.loginRenew'); sessionStorage.removeItem('pm.loginRenew'); } catch { /* a plain log in; Settings asks again */ }
    // The app behind is hidden: the screen's heading takes the focus, so it is announced
    $('#loginh').focus();
    loginStart();
  };
  const hideLogin = () => { loginStop(); document.body.classList.remove('signedout'); $('#loginpage').hidden = true; };
  // The node refused this browser's token (401): the session ended (or there was none). Start
  // over from a fresh page, so nothing of the old session lingers; it says why.
  const signedOut = () => document.body.classList.contains('signedout');
  const sessionEnded = (sent = state.token) => {
    if (sent !== state.token) return; // an answer to an older token: this one may be fine
    if (!state.token) { let why; try { why = sessionStorage.getItem('pm.loginWhy'); sessionStorage.removeItem('pm.loginWhy'); } catch { /* no reason */ } showLogin(why); return; }
    forgetAccount();
    try { sessionStorage.setItem('pm.loginWhy', 'ended'); } catch { /* the screen just doesn't say why */ }
    location.reload();
  };
  // Log out: the node ends this browser's session (an owner's or a listener's; a pasted API
  // token stays the node's), and the page forgets everything of the account, back to Log in.
  // Only once the node said so (or the session had already ended): else the session would go on
  // working while the page claims it is over, so it says why, still logged in, with Try again.
  // While the node needs new folders, the owner's next log in from here asks for them (only the
  // owner's page asks: anyone scanning a plain QR code is asked to read, nothing more).
  // A node that never answers would keep Log out busy for good: 15 s in all, of them at most 5
  // for the renewal code (then skipped); the session end that gets no answer asks Try again
  const LOGOUT_MS = 15000; const RENEWAL_MS = 5000;
  const timedFetch = async (url, opts, ms) => {
    const c = new AbortController(); const t = setTimeout(() => c.abort(), Math.max(0, ms));
    try { const r = await fetch(url, { ...opts, signal: c.signal }); return { r, body: await r.json().catch(() => null) }; } finally { clearTimeout(t); }
  };
  // One Log out at a time: a second press while one waits (the account menu's again, or Settings'
  // while the menu's runs) joins it instead of asking the node a second time
  let loggingOut = null;
  const logOut = () => loggingOut || (loggingOut = logOutNow().finally(() => { loggingOut = null; }));
  const logOutNow = async () => {
    const until = Date.now() + LOGOUT_MS;
    // Asked while still logged in: the node gives a code only to the owner, and only when needed
    let renewal = null;
    if (state.token && !isListener()) {
      const a = await timedFetch('/v1/auth/pubky/renewal', { method: 'POST', headers: { authorization: 'Bearer ' + state.token } }, RENEWAL_MS).catch(() => null);
      renewal = (a && a.r.ok && a.body && a.body.renewal) || null;
    }
    if (state.token) {
      let why = '';
      try {
        const { r, body } = await timedFetch('/v1/me/token', { method: 'DELETE', headers: { authorization: 'Bearer ' + state.token } }, until - Date.now());
        if (!r.ok && r.status !== 401) why = (body && body.error) || `HTTP ${r.status}`;
      } catch { why = 'the node didn’t answer'; }
      if (why) {
        popTransient(); if (popPending) await popPending;
        // The page's own dialog, as every other question in the app (no browser box)
        // (the node's own reason may already be a sentence: one full stop, and none after its
        // own ?, ! or …)
        const said = String(why).replace(/[.\s]+$/, '') || 'The node refused';
        const end = /[?!…]$/.test(said) ? '' : '.';
        const again = await choiceDialog({ title: 'Couldn’t log out', text: `${said[0].toUpperCase()}${said.slice(1)}${end} You are still logged in.`, choices: [{ label: 'Try again', value: true, primary: true }] });
        if (again) return logOutNow();
        return;
      }
    }
    try { if (renewal) sessionStorage.setItem('pm.loginRenew', renewal); } catch { /* a plain log in */ }
    forgetAccount();
    try { sessionStorage.setItem('pm.loginWhy', 'out'); } catch { /* no reason shown */ }
    // From a menu: its history entry goes first, or popping it later would cancel this navigation
    popTransient(); if (popPending) await popPending;
    location.replace('/');
  };
  // A pasted token is tried first: a wrong one is said here, not taken for an ended session
  onBusy($('#tokform'), 'submit', async (e) => {
    e.preventDefault();
    const t = $('#tok').value.trim(); if (!t) return;
    const out = $('#tokerr'); out.textContent = '';
    let r; try { r = await fetch('/v1/me', { headers: { authorization: 'Bearer ' + t } }); } catch { out.textContent = 'Can’t reach the node. Try again.'; return; }
    if (r.status === 401) { out.textContent = 'The node doesn’t take that token.'; $('#tok').select(); return; }
    state.token = t; try { localStorage.setItem('pm.token', t); } catch { /* this tab keeps it */ }
    hideLogin(); connect();
  });


// Install Odeion as an app (docs/guides/install-web-app.md). Separate from app.js: a button in
// the top bar that shows the browser's own install prompt when the browser offers one, and the
// steps for this browser otherwise; nothing in the installed app's window or inside the desktop
// or Android app. It never prompts by itself, never reloads the page and keeps nothing but its
// own "hidden" choice. The service worker (sw.js) is registered here too.
(function () {
  'use strict';
  // The desktop and Android apps (window.pmDesktop, their own pages): already an app. An Android
  // WebView whose early script didn't run still says "; wv)" in its user agent.
  if (window.pmDesktop || !/^https?:$/.test(location.protocol) || location.hostname === 'tauri.localhost' || /; wv\)/.test(navigator.userAgent || '')) return;

  // /#install opens the help (docs/guides/install-web-app.md), on arrival or typed on the open page.
  // Dropped from the address (path and query kept, no reload), so the app sees a plain path; on
  // arrival it is read before app.js routes the address (this script loads first, in the head).
  const takeHash = () => {
    if (location.hash !== '#install') return false;
    try { history.replaceState(history.state, '', location.pathname + location.search); } catch { /* sandboxed */ }
    return true;
  };
  const askedHelp = takeHash();

  const HIDDEN = 'pm.installHidden';
  const MODES = ['standalone', 'fullscreen', 'minimal-ui', 'window-controls-overlay'];
  const media = (m) => (window.matchMedia ? window.matchMedia(`(display-mode: ${m})`) : null);
  const installed = () => navigator.standalone === true || MODES.some((m) => { const q = media(m); return !!(q && q.matches); });
  const stored = (k) => { try { return localStorage.getItem(k); } catch { return null; } };
  const store = (k, v) => { try { if (v === null) localStorage.removeItem(k); else localStorage.setItem(k, v); } catch { /* private mode: this visit only */ } };

  let deferred = null; // the browser's BeforeInstallPromptEvent, usable once
  let done = false; // installed during this visit
  let btn = null; // in the top bar (signed in)
  let loginBtn = null; // on the login page (signed out: the top bar is hidden then)
  let dlg = null;
  // The help was asked for (#install) and not yet closed by the person: when the app's sign-in screen
  // closes every dialog as it starts (signed out), it opens again, once
  let wanted = false;

  const show = () => { const off = done || installed() || stored(HIDDEN) === '1'; for (const b of [btn, loginBtn]) if (b) b.hidden = off; };

  // Chromium: keep its prompt for our button instead of its own mini-infobar
  window.addEventListener('beforeinstallprompt', (e) => { e.preventDefault(); deferred = e; show(); });
  // Installed by any route (our prompt, the address bar, the browser menu)
  window.addEventListener('appinstalled', () => { deferred = null; done = true; show(); if (dlg && dlg.open) dlg.close(); });
  for (const m of MODES) { const q = media(m); if (q && q.addEventListener) q.addEventListener('change', show); }

  // Which steps fit this browser. iPadOS Safari says "Macintosh": it has touch points.
  function browser() {
    const ua = navigator.userAgent || '';
    const ios = /iPhone|iPad|iPod/.test(ua) || (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1);
    const android = /Android/.test(ua);
    if (!window.isSecureContext) return 'insecure';
    if (ios) return 'ios';
    if (/SamsungBrowser/.test(ua)) return 'samsung';
    if (/Firefox\//.test(ua)) return android ? 'firefox-android' : /Windows/.test(ua) ? 'firefox-windows' : 'firefox';
    if (/Edg(e|A)?\//.test(ua)) return android ? 'android' : 'edge';
    if (/Chrome\/|Chromium\//.test(ua)) return android ? 'android' : 'chrome';
    if (/Safari\//.test(ua) && /Macintosh/.test(ua)) return 'safari-mac';
    return 'other';
  }

  const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
  const STEPS = {
    chrome: '<ol><li>Click the install icon at the right end of the address bar,</li><li>or open the browser menu and choose <b>Install page as app</b> (in some versions under <b>Cast, save and share</b>).</li></ol>',
    edge: '<ol><li>Open <b>Settings and more</b> (the menu at the top right).</li><li>Choose <b>Apps</b>, then <b>Install this site as an app</b>.</li></ol>',
    android: '<ol><li>Open the browser menu.</li><li>Choose <b>Install app</b> or <b>Add to Home screen</b>, then <b>Install</b>.</li></ol>',
    samsung: '<ol><li>Open the menu.</li><li>Choose <b>Add page to</b>, then <b>Home screen</b>.</li></ol>',
    ios: '<ol><li>Tap <b>Share</b> (the square with an arrow).</li><li>Choose <b>Add to Home Screen</b>, then <b>Add</b>.</li><li>Open Odeion from your Home Screen.</li></ol><p class="muted small">iOS 16.4 and later: from Safari, Chrome, Edge or Firefox. Earlier iOS: from Safari only.</p>',
    'safari-mac': '<ol><li>In the menu bar choose <b>File</b>, then <b>Add to Dock</b>.</li></ol><p class="muted small">Safari 17 (macOS Sonoma) and later.</p>',
    'firefox-windows': '<ol><li>Click the web apps icon in the address bar.</li><li>Choose <b>Add to taskbar</b>.</li></ol><p class="muted small">Firefox 143 and later on Windows.</p>',
    'firefox-android': '<ol><li>Open the menu and choose <b>Install</b>.</li><li>In <b>Add to Home Screen</b>, choose to add it automatically (or place the icon yourself).</li></ol><p class="muted small">If the menu has no Install, choose <b>Add to Home screen</b> instead: that adds a shortcut that opens in Firefox.</p>',
    firefox: '<p>Firefox on this system can’t install web apps. Open this page in Chrome or Edge (or Safari on a Mac) and install it from there.</p>',
    other: '<p>Look in your browser’s menu for <b>Install</b>, <b>Add to Home screen</b> or <b>Add to Dock</b>. If there’s none, this browser can’t install web apps: try Chrome, Edge or Safari.</p>',
  };
  const insecure = () => `<p>This page comes from <b>${esc(location.host)}</b> over plain HTTP. Browsers install apps only from an HTTPS address (or from this computer’s own <code>localhost</code>), so there is no install here.</p>`
    + '<p>To install Odeion, open your node through an HTTPS address (for example a reverse proxy with a certificate) and install from there, or use the desktop or Android app for your node.</p>';

  function helpHtml() {
    const kind = browser();
    return '<h2 id="installh">Install Odeion</h2>'
      + (kind === 'insecure' ? insecure() : '<p>Odeion opens in its own window, like any app on this device.</p>' + STEPS[kind])
      + '<p class="muted small">The app is this node’s page in its own window: it needs your node to be reachable and doesn’t play music offline.</p>'
      + '<div class="dialog-actions"><button class="btn ghost" type="button" data-install="hide">Hide this button</button><button class="btn primary" type="button" data-install="close">Done</button></div>';
  }

  function openHelp() {
    if (!dlg) {
      dlg = document.createElement('dialog'); dlg.className = 'dialog install-help'; dlg.id = 'installhelp'; dlg.setAttribute('aria-labelledby', 'installh');
      dlg.addEventListener('cancel', () => { wanted = false; }); // Escape
      dlg.addEventListener('close', () => {
        const again = wanted && document.body.classList.contains('signedout');
        wanted = false;
        if (again) setTimeout(openHelp, 0);
      });
      dlg.addEventListener('click', (e) => {
        const b = e.target.closest && e.target.closest('[data-install]');
        if (!b) return;
        wanted = false;
        if (b.dataset.install === 'hide') { store(HIDDEN, '1'); show(); }
        dlg.close();
      });
      document.body.appendChild(dlg);
    }
    dlg.innerHTML = helpHtml();
    if (!dlg.open) dlg.showModal();
  }

  async function install() {
    const e = deferred;
    if (!e) { openHelp(); return; }
    deferred = null; // one prompt per event, whatever the answer
    let outcome = 'dismissed';
    try { const r = await e.prompt(); outcome = (r && r.outcome) || (e.userChoice ? (await e.userChoice).outcome : outcome); } catch { openHelp(); return; }
    if (outcome === 'accepted') { done = true; show(); }
  }

  // Opened from the worker's kept copy (sw.js): the node didn't answer in time. The app may wait for
  // it without a limit ("Connecting…"), so this says so at once, looks every few seconds whether the
  // node answers (a public file, past the copy) and then offers to open the page afresh. Nothing
  // reloads by itself. Once the app shows its own connection state, this note gives way to it.
  let note = null;
  function unreachable() {
    const conn = document.getElementById('conn');
    const appSays = () => !!(conn && conn.dataset.state);
    if (note || appSays()) return;
    note = document.createElement('div');
    note.id = 'pwanotice'; note.className = 'connbar bad pwa-notice'; note.setAttribute('role', 'status'); note.setAttribute('aria-live', 'polite');
    note.innerHTML = '<span class="dot"></span><span class="connbar-text">Can’t reach your node. This is the copy of its page kept on this device; it keeps trying.</span>';
    document.body.appendChild(note);
    if (conn && window.MutationObserver) new MutationObserver(() => { if (appSays() && note) { note.remove(); note = null; } }).observe(conn, { attributes: true, attributeFilter: ['data-state'] });
    const probe = async () => {
      if (!note) return;
      const ac = new AbortController();
      const t = setTimeout(() => ac.abort(), 4000);
      let up = false;
      // The node's own manifest, not a proxy's sign-in or maintenance page (no redirects followed)
      try {
        const r = await fetch('/manifest.webmanifest?reach=' + Date.now(), { cache: 'no-store', redirect: 'error', signal: ac.signal });
        up = r.ok && !r.redirected && /^application\/manifest\+json/i.test(r.headers.get('content-type') || '') && (await r.json()).id === '/';
      } catch { /* still away */ }
      clearTimeout(t);
      if (!note) return;
      if (!up) { setTimeout(probe, 5000); return; }
      note.className = 'connbar wait pwa-notice';
      note.innerHTML = '<span class="dot"></span><span class="connbar-text">Your node answers again.</span><button type="button" class="connbar-retry" id="pwareconnect">Reconnect</button>';
      note.querySelector('#pwareconnect').addEventListener('click', () => location.reload());
    };
    probe();
  }
  function askWorker() {
    const sw = navigator.serviceWorker;
    if (!sw || !sw.controller) return;
    sw.addEventListener('message', (e) => { if (e.data && e.data.pwa === 'opened-from-copy' && e.data.value) unreachable(); });
    sw.controller.postMessage('pwa:opened-from-copy?');
  }

  let started = false;
  function start() {
    if (started) return;
    started = true;
    const me = document.getElementById('me');
    if (me && me.parentNode) {
      btn = document.createElement('button');
      btn.className = 'icon-btn install-btn'; btn.id = 'installbtn'; btn.type = 'button';
      btn.setAttribute('aria-label', 'Install app'); btn.title = 'Install Odeion as an app on this device';
      btn.innerHTML = '<svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><rect x="4" y="4" width="16" height="16" rx="3" fill="none" stroke="currentColor" stroke-width="1.8"/><path d="M12 8v7m-3-3 3 3 3-3" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>';
      btn.addEventListener('click', install);
      me.parentNode.insertBefore(btn, me);
    }
    // A first visit, before anyone signs in, gets the same action under the login
    const card = document.querySelector('#loginpage .logincard');
    if (card) {
      loginBtn = document.createElement('button');
      loginBtn.className = 'btn ghost install-login'; loginBtn.id = 'installlogin'; loginBtn.type = 'button';
      loginBtn.title = 'Install Odeion as an app on this device';
      loginBtn.innerHTML = '<svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><rect x="4" y="4" width="16" height="16" rx="3" fill="none" stroke="currentColor" stroke-width="1.8"/><path d="M12 8v7m-3-3 3 3 3-3" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>Install app';
      loginBtn.addEventListener('click', install);
      card.appendChild(loginBtn);
    }
    const askHelp = () => { store(HIDDEN, null); show(); wanted = true; openHelp(); };
    if (askedHelp) askHelp();
    window.addEventListener('hashchange', () => { if (takeHash()) askHelp(); });
    show();
    // The worker keeps the public page for a launch while the node is away (sw.js); HTTPS or localhost only.
    if (window.isSecureContext && navigator.serviceWorker) navigator.serviceWorker.register('/sw.js', { scope: '/', updateViaCache: 'none' }).catch(() => { /* the page works without it */ });
    if (window.isSecureContext) askWorker();
  }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', start); else start();
}());

  // One place for the toast on a phone (owner's recording, 2026-11-08: it floated mid-screen): 8 px
  // above the highest bar at the bottom (the mini player, the tabs) or the keyboard, whichever is
  // higher; 16 px above the screen's bottom when none shows (Now playing). Wider screens: the CSS.
  const placeToast = (t) => {
    if ((document.documentElement.dataset.shell || 'desktop') !== 'phone') { t.style.bottom = ''; return; }
    const bars = ['.player', '#bottomnav'].map((q) => document.querySelector(q)).filter((el) => el && el.getClientRects().length && getComputedStyle(el).visibility !== 'hidden');
    const top = Math.min(window.innerHeight, ...bars.map((el) => el.getBoundingClientRect().top));
    const kb = parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--kb')) || 0;
    t.style.bottom = `${Math.max(window.innerHeight - top + 8, kb + 8, 16)}px`;
  };
  // The soft keyboard (Android WebView, iOS): how much of the screen's bottom it covers, for the
  // phone's Now playing sheet and the toast; a field focused in the sheet scrolls into view above it
  if (window.visualViewport) {
    const vv = window.visualViewport;
    let kbWas = 0;
    const kbMove = () => {
      const h = Math.max(0, Math.round(window.innerHeight - vv.height - vv.offsetTop));
      document.documentElement.style.setProperty('--kb', `${h}px`);
      // (the keyboard's height is the viewport's lost height; a pan moves offsetTop only)
      const kbH = Math.round(window.innerHeight - vv.height);
      const a = document.activeElement;
      if (kbH !== kbWas && a && a.matches && a.matches('input, textarea') && a.closest('.np-bsheet')) requestAnimationFrame(() => a.scrollIntoView({ block: 'center' }));
      kbWas = kbH;
      const t = document.getElementById('toast'); if (t && t.classList.contains('show')) placeToast(t);
    };
    vv.addEventListener('resize', kbMove); vv.addEventListener('scroll', kbMove);
  }
  document.addEventListener('focusin', (e) => { const a = e.target; if (a && a.matches && a.matches('input, textarea') && a.closest('.np-bsheet')) setTimeout(() => { if (document.activeElement === a) a.scrollIntoView({ block: 'center' }); }, 300); });
  // The toast's element: in the page, or in the modal dialog that is open (a modal dialog and its
  // backdrop are drawn over everything else on the page, so a message raised from one was under the
  // backdrop). It comes back to the page when the dialog closes, also when the dialog is taken away
  // in the same step.
  const toastBox = document.getElementById('toast');
  const toastEl = () => {
    // (a browser that does not know :modal keeps the toast in the page, as before)
    const open = [...document.querySelectorAll('dialog[open]')].filter((d) => { try { return d.matches(':modal'); } catch { return false; } }).pop() || null;
    const host = open || document.body;
    // (closed: to the dialog still open under it, else the page)
    if (toastBox.parentElement !== host) { host.appendChild(toastBox); if (open) open.addEventListener('close', () => { if (toastBox.parentElement === open || !toastBox.isConnected) toastEl(); }, { once: true }); }
    return toastBox;
  };
  // `action` ({ label, run }): a button in the toast (Undo); the toast stays longer then. Returns that button
  // An error stays readable and an Undo within reach: a plain message that comes in the first 2.5 s
  // of an error or of a toast with a button waits for that time to be up, then shows (the newest
  // one if several came). Before, it took their place at once: an error was gone milliseconds
  // after it appeared when the page said something else next. One rule for every message, wherever
  // it comes from; an error, a toast with a button or a progress message ('wait': the answer to a
  // press, "Moving to …") is shown at once, as before.
  const toast = (msg, kind = '', action = null) => {
    const t = toastEl();
    const until = toast.keep && t.classList.contains('show') ? toast.keep : 0;
    if (!kind && !action && performance.now() < until) {
      toast.next = msg; clearTimeout(toast.nextT);
      toast.nextT = setTimeout(() => { if (toast.next === msg) { toast.next = null; toast.keep = 0; toast(msg); } }, until - performance.now());
      return null;
    }
    toast.next = null; clearTimeout(toast.nextT);
    toast.keep = kind === 'error' || action ? performance.now() + 2500 : 0;
    // (an error interrupts a screen reader, as the errors written into the page do; the rest wait their turn)
    t.setAttribute('role', kind === 'error' ? 'alert' : 'status'); t.setAttribute('aria-live', kind === 'error' ? 'assertive' : 'polite');
    t.textContent = msg;
    let b = null;
    // (gone: a plain message that waited for it shows now)
    const hide = () => { t.classList.remove('show'); toast.keep = 0; toast.h = setTimeout(() => { t.textContent = ''; }, 400); const n = toast.next; if (n) { toast.next = null; clearTimeout(toast.nextT); toast(n); } };
    // (listeners, not on… properties: focusin and focusout have none; the next toast drops them)
    const unhook = () => { if (toast.ac) toast.ac.abort(); toast.ac = null; };
    if (action) { b = document.createElement('button'); b.className = 'btn sm toast-act'; b.textContent = action.label; b.onclick = () => { clearTimeout(toast.h); unhook(); b.remove(); hide(); Promise.resolve(action.run()).catch((e) => toast(e.message, 'error')); }; t.appendChild(b); }
    t.className = `toast show ${kind}`;
    placeToast(t);
    clearTimeout(toast.h);
    // (a toast with an action waits while the pointer or the focus is on it: it goes 4 s after both left)
    // (a 'wait' toast stays until the next toast replaces it: the answer's, or its error)
    const stay = kind === 'error' ? 6000 : action ? 8000 : 3800;
    const arm = (ms) => { clearTimeout(toast.h); toast.h = setTimeout(hide, ms); };
    if (kind !== 'wait') arm(stay);
    unhook();
    if (action) {
      const ac = new AbortController(); toast.ac = ac; const on = (type, fn) => t.addEventListener(type, fn, { signal: ac.signal });
      let over = false; let focused = false; const settle = () => { if (!over && !focused) arm(4000); };
      on('mouseenter', () => { over = true; clearTimeout(toast.h); }); on('focusin', () => { focused = true; clearTimeout(toast.h); });
      on('mouseleave', () => { over = false; settle(); }); on('focusout', () => { focused = false; settle(); });
    }
    return b;
  };
  // A progress message ("Looking up…") goes as soon as its answer comes, unless another message
  // replaced it meanwhile (before, it stayed its full 3.8 s: behind the next dialog, and on the page
  // the answer opened, over the phone's soft keyboard)
  const toastDone = (msg) => {
    const t = toastBox;
    if (toast.next === msg) { toast.next = null; clearTimeout(toast.nextT); return; } // (its answer came while it still waited)
    if (!t.classList.contains('show') || t.textContent !== msg) return;
    clearTimeout(toast.h); t.classList.remove('show'); toast.keep = 0; toast.h = setTimeout(() => { t.textContent = ''; }, 400);
    const n = toast.next; if (n) { toast.next = null; clearTimeout(toast.nextT); toast(n); }
  };
  // A press whose button a redraw removed leaves the focus on the page: it goes to the toast's Undo
  // (keyboard and screen-reader users reach it there; the toast waits while it has the focus)
  const focusUndo = (b) => { const a = document.activeElement; if (b && b.isConnected && (!a || a === document.body)) b.focus(); };
  // A node without a route answers 404, or 405 when the path matches another method's route
  const noRoute = (e) => (e && (e.status === 404 || e.status === 405) ? new Error('This node doesn’t offer that yet: update it first') : e);
  // An Undo pressed says so at once (bringing back thousands of songs takes the node a moment)
  // (it stays until the answer's own toast replaces it). A node that stops answering doesn't leave it
  // up for good: the request is given up and the toast says so. (While a scan writes, the node itself
  // waits up to 10 minutes, identify.rs BUSY_PATIENCE, then answers why: the cut comes after that.)
  // The node may still bring them back: the Identify card counts what is still skipped
  const UNSKIP_MS = 11 * 60000;
  const UNSKIP_LATE = 'The node stopped answering, so the songs may not be back. Your library › Node library shows how many are still skipped.';
  const unskipSongs = (body) => {
    toast('Bringing the songs back…', 'wait');
    const ac = new AbortController(); ac.signal.why = new Error(UNSKIP_LATE);
    const cut = setTimeout(() => ac.abort(ac.signal.why), UNSKIP_MS);
    return api('POST', '/library/suggestions/unskip', body, { signal: ac.signal }).finally(() => clearTimeout(cut));
  };
  // After an Undo the review page is drawn again: the focus goes to its heading (not Skip all, which
  // an Enter meant for the Undo would press), and only when it was left on the page's body or the
  // toast: a user who moved it meanwhile keeps it where they put it
  const focusLeft = () => { const a = document.activeElement; return !a || a === document.body || !!a.closest('#toast'); };
  const focusReview = async () => {
    for (let k = 0; k < 300; k++) {
      if (state.view !== 'identify') return;
      const a = document.activeElement;
      if (a && a !== document.body && !a.closest('#toast') && !a.closest('#main')) return;
      const f = !$('#main .idloading') && $('#main .page-head h2');
      if (f) { if (a && a !== document.body && !a.closest('#toast')) return; f.tabIndex = -1; f.focus(); return; }
      await new Promise((r) => setTimeout(r, 50));
    }
  };
  // The Undo of a skip says how many songs came back (the node's unskip can bring back earlier skips too)
  const undoLabel = (r) => (r && r.counts && r.counts.skipped > r.skipped ? `Undo (brings back all ${r.counts.skipped.toLocaleString()} skipped)` : 'Undo');
  // ---- Every click shows progress and fires once (owner, 2026-11-04) ----
  // busy(el, fn) runs fn. When fn returns a promise, el is busy until it settles: aria-busy and
  // aria-disabled at once (.is-busy; not `disabled`, so keyboard focus stays on it), a spinner
  // after 150 ms (.is-spinning), and further clicks on it are ignored. Then it is restored; a failure
  // nobody caught shows its message. A plain return value passes through (`return false` still cancels).
  // BUSY maps a control to the event that made it busy: only a busy mark made by a click drops later
  // clicks (and says aria-disabled), and the same click bubbling on to a row's own handler is not a
  // second click. A mark made without an event (a menu's action on its ⋯) only shows the progress.
  const BUSY = new WeakMap();
  let inHandler = 0; // a press's handler is running (its reads are its own, not a page's: navApiFor)
  let pressing = 0; // a press's handler that returned a promise has not settled yet
  const busy = (el, fn, ev = null) => {
    let out;
    const dialogsBefore = $$('dialog[open]'); // (open before the press: not one it opened, see the spinner below)
    inHandler++;
    try { out = fn(); } catch (e) { console.error(e); toast((e && e.message) || 'That didn’t work; try again.', 'error'); return undefined; } finally { inHandler--; }
    if (!out || typeof out.then !== 'function') return out;
    // (until it settles, a redraw it makes is the press's: usage insights count its reads)
    // (at most 10 s, or while its dialog is open: a press that never settles doesn't make every later
    // redraw and request its own, and a confirmed dialog's change still counts)
    pressing++; { let left = false; const settled = () => { if (!left) { left = true; pressing--; } }; out.then(settled, settled); const cap = () => { if (left) return; if (document.querySelector('dialog[open]')) setTimeout(cap, 10000); else settled(); }; setTimeout(cap, 10000); }
    const failed = (e) => { if (!e || e.name !== 'AbortError') toast((e && e.message) || 'That didn’t work; try again.', 'error'); };
    if (!el) { out.catch(failed); return out; }
    // Several actions on one control at once (the same click reaching its row's handler, or changes in a
    // row): it stays busy until the last one has settled
    let mark = BUSY.get(el);
    if (!mark) {
      mark = { ev, n: 0, was: [el.getAttribute('aria-busy'), el.getAttribute('aria-disabled')] };
      el.setAttribute('aria-busy', 'true'); if (ev) el.setAttribute('aria-disabled', 'true'); el.classList.add('is-busy');
      // (no spinner while a dialog the press opened waits for you, Add to collection… or Edit tags…
      // from a ⋯: it is waiting on your choice, not the node; it turns again if work goes on after the
      // dialog. A dialog it sits in, or one open before the press, doesn't count. Taken off the page,
      // it is no longer looked at.)
      const spin = () => { if (!el.isConnected) return; el.classList.toggle('is-spinning', !$$('dialog[open]').some((d) => !d.contains(el) && !dialogsBefore.includes(d))); mark.spin = setTimeout(spin, 150); };
      mark.spin = setTimeout(spin, 150);
      BUSY.set(el, mark);
    }
    mark.n++;
    const done = () => {
      if (--mark.n > 0) return;
      clearTimeout(mark.spin); if (BUSY.get(el) === mark) BUSY.delete(el);
      el.classList.remove('is-busy', 'is-spinning');
      ['aria-busy', 'aria-disabled'].forEach((a, i) => { if (mark.was[i] == null) el.removeAttribute(a); else el.setAttribute(a, mark.was[i]); });
    };
    out.then(done, (e) => { done(); failed(e); });
    return out;
  };
  // The element that shows the progress: the handler's own control, the control clicked inside a
  // container that handles its rows' clicks, or a form's submit button.
  const BUSY_CONTROL = 'button, a[href], [role="button"], [role="menuitem"], [role="tab"], [role="option"], .chip, input[type="checkbox"], input[type="radio"], select';
  const busyTarget = (host, e) => {
    if (e && e.type === 'submit') { const f = e.target; return e.submitter || (f instanceof HTMLFormElement ? f.querySelector('button[type="submit"], button:not([type])') || f : null); }
    if (host.matches(BUSY_CONTROL)) return host;
    const t = e && e.target instanceof Element ? e.target.closest(BUSY_CONTROL) : null;
    return t && host.contains(t) ? t : null;
  };
  // An event handler run through busy(): a click or submit on a control that is still busy is dropped
  // (one request per action); a change still runs (it carries a new value). Controls meant to be pressed
  // quickly in a row (volume steps) opt out with data-repeat.
  const viaBusy = (host, fn, self, e) => {
    const t = busyTarget(host, e);
    if (!t) return fn.call(self, e);
    if (t.closest('[data-repeat]')) return busy(t, () => fn.call(self, e)); // progress only: every press runs
    const was = BUSY.get(t);
    if (was && was.ev && was.ev !== e && e.type !== 'change') { e.preventDefault(); return false; }
    return busy(t, () => fn.call(self, e), e.type === 'change' ? null : e);
  };
  // For a container that handles its controls' events (addEventListener on the container)
  const onBusy = (host, type, fn) => host.addEventListener(type, (e) => { viaBusy(host, fn, host, e); });
  // Every onclick, onsubmit and onchange handler goes through it, wherever it is assigned: the setters
  // are wrapped once here, so no handler can miss it.
  for (const prop of ['onclick', 'onsubmit', 'onchange']) {
    const d = Object.getOwnPropertyDescriptor(HTMLElement.prototype, prop);
    if (!d || !d.set) continue;
    Object.defineProperty(HTMLElement.prototype, prop, {
      configurable: true, enumerable: d.enumerable,
      get() { const f = d.get.call(this); return (f && f.pmHandler) || f; },
      set(fn) {
        if (typeof fn !== 'function') { d.set.call(this, fn); return; }
        const host = this;
        const wrapped = function (e) { return viaBusy(host, fn, this, e); };
        wrapped.pmHandler = fn;
        d.set.call(this, wrapped);
      },
    });
  }
  const setFill = (el) => { const min = +el.min || 0; const max = +el.max || 1; el.style.setProperty('--fill', `${((el.value - min) / (max - min)) * 100}%`); };

  // Errors in people's words: what happened and what to do, in one line. The node's own wording
  // passes through; raw service answers ("…answered 403 Forbidden: …") are translated, and the raw
  // text stays on the error as `raw` (UX review: wording leaks internals).
  const SVC_WORDS = { tidal: 'TIDAL', qobuz: 'Qobuz', soundcloud: 'SoundCloud', bandcamp: 'Bandcamp', spotify: 'Spotify', deezer: 'Deezer', jamendo: 'Jamendo', subsonic: 'Subsonic', jellyfin: 'Jellyfin', archive: 'The Internet Archive', audius: 'Audius', radio: 'The radio directory', pubky: 'Your homeserver', pinakes: 'Pinakes', listenbrainz: 'ListenBrainz' };
  const humanError = (raw, path = '') => {
    const text = String(raw || '');
    const svcKey = (/^\/sources\/([a-z]+)/.exec(path) || /^([a-z]+): /.exec(text) || [])[1];
    const who = SVC_WORDS[svcKey] || (svcKey ? svcKey[0].toUpperCase() + svcKey.slice(1) : 'The service');
    const where = svcKey && SVC_WORDS[svcKey] && svcKey !== 'pubky' && svcKey !== 'pinakes' ? ' · Sources' : '';
    if (/deserializ|missing field|invalid type|unknown variant|at line \d+ column/i.test(text)) return "The node didn't understand that request. Reload the page and try again.";
    if (/premium/i.test(text)) return `${who} needs a Premium plan for this${where}.`;
    if (/answered 401|unauthori[sz]ed|invalid[_ ]token|token (has )?expired/i.test(text)) return `${who} wants you to sign in again${where}.`;
    if (/answered 403|forbidden/i.test(text)) return `${who} refused this request (your plan or region may not include it)${where}.`;
    if (/answered 404|not found/i.test(text) && /answered/.test(text)) return `${who} doesn't have this any more.`;
    if (/answered 429|rate.?limit|too many requests/i.test(text)) return `${who} is busy; try again in a minute.`;
    if (/answered 5\d\d|bad gateway|service unavailable/i.test(text)) return `${who} isn't answering right now; try again later.`;
    if (/timed? ?out|deadline/i.test(text)) return `${who} took too long to answer; try again.`;
    if (/error sending request|connection refused|dns error|could not resolve/i.test(text)) return `Couldn't reach ${who === 'The service' ? 'the service' : who}; check the node's internet connection.`;
    return text;
  };
  // A change already on its way is not sent twice: the same request again (method, path and body) while
  // it is the latest one to that resource and still in flight gets its answer. Any other change to the
  // resource in between is sent, and so is a repeat after it (A, B, A sends all three, one after another).
  // Reads are left alone.
  const inFlight = new Map();
  // A listing of someone's shared playlists that may miss some (x-partial)
  const SHARED_PARTIAL = "Some of their posts couldn't be read, so this list may miss some.";
  let libWrote = () => {}; // (the search's, below: forgetLibrarySearches)
  const api = (method, path, body, opts = {}) => {
    // Usage insights (usage.js) count only requests a press or a page the person opened made
    if (opts.usage === undefined) opts = { ...opts, usage: inHandler > 0 || pressing > 0 || !!opts.navRead };
    if (method === 'GET') return apiSend(method, path, body, opts);
    // A tag written anywhere: an item page's kept Pubky tags are out of date (again once it lands: an
    // answer asked for while it was on its way may not have it yet)
    const tagWrite = path.split('?')[0] === '/tags'; if (tagWrite) forgetHeroPub();
    // A playlist written (made, changed, deleted) or music imported: the kept searches holding the
    // library's answers are out of date once it is answered (libWrote, forgetLibrarySearches)
    const libWrite = (/^\/playlists(\/|$)/.test(path) && !/\/play(\?|$)/.test(path)) || /\/imports?(\?|$)/.test(path) || /^\/shared\/[^/]+\/playlists\/[^/]+\/copy$/.test(path);
    // The body as it is now (a queued change sends what was asked, even if the object changes meanwhile)
    let sent; let snap; try { sent = body === undefined ? '' : JSON.stringify(body); snap = sent ? JSON.parse(sent) : body; } catch { return apiSend(method, path, body, opts); }
    // One line per resource (its path without the query: PUT /favourites and DELETE /favourites?… too)
    const line = path.split('?')[0]; const was = inFlight.get(line); const ask = `${method} ${path} ${sent}`;
    // opts.repeat: each one is meant (Next pressed twice skips two), so it is never shared
    if (!opts.repeat && was && was.ask === ask) return was.p;
    // After the one before it on the same line, so the node applies them, and their answers land, in the
    // order they were made (a late answer to the first Next can't undo the second)
    const send = () => apiSend(method, path, snap, opts);
    // (one cut while it waits behind another on its line stops waiting)
    const after = (p) => (opts.signal ? Promise.race([p, new Promise((_, rej) => { const fail = () => rej(opts.signal.why || new Error('aborted')); if (opts.signal.aborted) fail(); else opts.signal.addEventListener('abort', fail, { once: true }); })]) : p);
    // (`settled`: this one and every earlier one on the line are done, even one cut while it waited:
    // the next on the line still goes after the earlier ones)
    const prev = was ? was.settled : Promise.resolve();
    const entry = { ask: opts.repeat ? null : ask, p: was ? after(prev).then(send) : send() };
    entry.settled = prev.then(() => entry.p.catch(() => {}));
    inFlight.set(line, entry);
    // (kept on its line until it and every earlier one there have settled, even when it was cut while
    // waiting: a later one still goes after the earlier ones; a failed one isn't handed to a new ask)
    const done = () => { entry.ask = null; if (tagWrite) forgetHeroPub(); if (libWrite) libWrote(); }; entry.p.then(done, done);
    entry.settled.then(() => { if (inFlight.get(line) === entry) inFlight.delete(line); });
    return entry.p;
  };
  const apiSend = async (method, path, body, opts = {}) => {
    // A change made while the connection is down waits a few seconds for it, then is refused (nothing is sent).
    if (method !== 'GET') await whenConnected();
    // Switching a service on or off, signing in or out, or a new play order changes the ranking
    if (method !== 'GET' && path.startsWith('/me/prefs')) { state.prefsCache = null; state.prefsGen = (state.prefsGen || 0) + 1; }
    if (method !== 'GET' && /^\/(sources|play-order)\b/.test(path)) { state.svcRank = null; state.svcRankLoading = null; state.svcRankGen = (state.svcRankGen || 0) + 1; }
    let r; const sent = state.token;
    const t0 = performance.now(); const told = (status) => { if (opts.usage && window.PM_USAGE) PM_USAGE.apiDone(method, path, status, performance.now() - t0); };
    try {
      r = await fetch('/v1' + path, {
        method,
        headers: { 'content-type': 'application/json', ...(sent ? { authorization: 'Bearer ' + sent } : {}) },
        body: body === undefined ? undefined : JSON.stringify(body),
        signal: opts.signal,
      });
    } catch (e) { if (opts.signal && opts.signal.aborted) throw opts.signal.why || e; told(0); linkHint(); throw new Error("Can't reach the node."); }
    told(r.status);
    // opts.onHeaders: a caller that reads a header of the answer (x-partial on a listing that may miss some)
    if (opts.onHeaders) opts.onHeaders(r.headers);
    // and again once the change has landed: a read that started meanwhile isn't kept either
    if (method !== 'GET' && path.startsWith('/me/prefs')) { state.prefsCache = null; state.prefsGen = (state.prefsGen || 0) + 1; }
    const text = await r.text();
    let json = null;
    try { json = text ? JSON.parse(text) : null; } catch { json = { error: text }; }
    // The node refused this browser's token (its own answer, not a service refusing a sign-in typed here)
    if (r.status === 401 && json && json.error === NODE_REFUSED) { sessionEnded(sent); throw new Error('Log in to use the node.'); }
    // A listener's collections are on their own homeserver: connect it (or approve it again), then the change goes
    // again (sent as it is: this one is still the change on its way, so api() would hand back this very answer)
    if (r.status === 409 && json && json.needs_connect && /^\/(playlists|collections)\b/.test(path) && await homeserverDialog(!!json.lost)) return apiSend(method, path, body, opts);
    if (!r.ok) { const raw = json && json.error ? json.error : `HTTP ${r.status}`; if (json && json.detail) console.warn(`${method} ${path}: ${json.detail}`); else if (raw !== humanError(raw, path)) console.warn(`${method} ${path}: ${raw}`); const err = new Error(humanError(raw, path)); err.raw = raw; err.status = r.status; err.body = json; throw err; }
    return json;
  };



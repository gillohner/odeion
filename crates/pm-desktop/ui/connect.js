// Drawn once the window has its size: a first frame laid out before it put the page at the top,
// then it jumped to the middle (owner's recording, 2026-11-06). The window's colour shows meanwhile.
// First in the file, so nothing below can keep the page hidden.
(function ready(n) { if (innerHeight > 200 || n > 30) document.body.classList.add('ready'); else requestAnimationFrame(() => ready(n + 1)); })(0);
// The connect page: reconnect to the remembered node, or connect with
// Pubky Ring or by address. Commands live in src/lib.rs.
(() => {
  'use strict';
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;
  const $ = (id) => document.getElementById(id);
  // The saved node's message; a change of state is also said in the live region (the retry
  // countdown's seconds are not: a screen reader would read every one)
  const announce = (text) => { const l = $('saved-live'); if (l) l.textContent = text; };
  const say = (text) => { $('saved-msg').textContent = text; announce(text); };
  let ringUrl = '';

  const show = (id, on = true) => { $(id).hidden = !on; };
  const fail = (e) => { $('err').textContent = String(e); show('err'); $('err').scrollIntoView({ block: 'nearest' }); };
  const clearErr = () => show('err', false);

  function connectView() {
    show('saved', false);
    show('connect');
    // Phone mode (the app without a node) where this build has it
    invoke('phone_available').then((on) => show('phone', !!on)).catch(() => show('phone', false));
    searchLan();
  }

  // Which sign-in is waiting for Ring: the node's (Sign in with Pubky Ring) or phone mode's
  let ringFor = 'node';
  const ringButton = () => $(ringFor === 'phone' ? 'phone-go' : 'ring');

  // Nodes on the LAN (mDNS and SSDP). A node without a token connects at once; one with a token
  // fills the address form and asks for it.
  let searching = false;
  async function searchLan() {
    if (searching) return;
    searching = true;
    show('lan-again', false);
    $('lan-status').textContent = 'Looking for nodes…';
    show('lan-status');
    let found = [];
    try { found = await invoke('discover_nodes'); } catch (e) { found = []; }
    searching = false;
    show('lan-again');
    const list = $('lan-list');
    list.textContent = '';
    if (!found.length) {
      $('lan-status').textContent = 'None found. The node must be on the same network, with its API listening beyond localhost (for example 0.0.0.0:7788).';
      return;
    }
    show('lan-status', false);
    for (const n of found) {
      const li = document.createElement('li');
      const b = document.createElement('button');
      b.type = 'button';
      const name = document.createElement('span');
      name.textContent = n.name || n.url;
      const addr = document.createElement('span');
      addr.className = 'addr';
      addr.textContent = n.url.replace(/^https?:\/\//, '') + (n.token_required ? ' · needs a token' : '');
      b.append(name, addr);
      b.onclick = () => pickNode(n, b);
      li.append(b);
      list.append(li);
    }
  }

  async function pickNode(n, btn) {
    clearErr();
    if (n.token_required) {
      $('url').value = n.url;
      $('manual-box').open = true;
      show('tok-hint');
      $('tok').focus();
      $('manual-box').scrollIntoView({ block: 'nearest' });
      return;
    }
    btn.disabled = true;
    try {
      await invoke('connect_manual', { nodeUrl: n.url, token: '' });
    } catch (e) {
      // A node without a token answers only at 127.0.0.1: the node on this computer, else it
      // needs a token (its config, [api] token) to be reached from here
      if (/no API token/.test(String(e))) {
        // Only the node that was found: one with another id (or none) at 127.0.0.1 is not taken
        if (n.id) {
          const here = new URL(n.url);
          here.hostname = '127.0.0.1';
          try {
            await invoke('connect_manual', { nodeUrl: here.toString().replace(/\/$/, ''), token: '', expectId: n.id });
            return;
          } catch (e2) { /* not on this computer, or another node is */ }
        }
        $('url').value = n.url;
        $('manual-box').open = true;
        show('tok-hint');
        $('tok').focus();
        $('manual-box').scrollIntoView({ block: 'nearest' });
      }
      fail(e);
    } finally { btn.disabled = false; }
  }
  $('lan-again').onclick = () => searchLan();

  // The remembered node is tried again for about a minute before the page gives up: a phone
  // opening the app as Wi-Fi comes back, or a node still starting, answers a few seconds later.
  const RETRIES = [2, 3, 5, 5, 10, 10, 15, 15];
  let attempt = 0; // bumped by every new reconnect, so an older retry loop stops

  // A node on the network claims the remembered node's id from another address. Anyone can claim
  // an id, so the shell sends nothing there until the person says yes here (follow_moved).
  const MOVED = 'Moved: ';
  function askMoved(e, oldUrl) {
    const m = String(e);
    if (!m.startsWith(MOVED)) return false;
    const url = m.slice(MOVED.length);
    attempt++;
    show('connect', false);
    show('saved');
    show('saved-spin', false);
    clearErr();
    say((oldUrl ? 'Your node didn’t answer at ' + oldUrl + ', and a' : 'A')
      + ' node with its id answers at ' + url + '. If you moved the node or its address changed, use the new address. '
      + 'Anyone on this network can claim to be your node, so your token goes there only if you say so.');
    const yes = $('moved-yes');
    yes.textContent = 'Use ' + url;
    yes.disabled = false;
    show('moved-yes');
    show('retry');
    show('other');
    yes.onclick = async () => {
      clearErr();
      yes.disabled = true;
      try { await invoke('follow_moved', { nodeUrl: url }); } catch (err) { fail(err); show('moved-yes', false); } finally { yes.disabled = false; }
    };
    return true;
  }

  // A node that hasn't answered after this long: the page says so and offers the ways on, while
  // it keeps trying (owner's recording: ~6 s of "Connecting…" with nothing else)
  const SLOW_MS = 5000;
  async function reconnect(nodeUrl) {
    const mine = ++attempt;
    show('connect', false);
    show('saved');
    show('moved-yes', false);
    show('retry', false);
    show('other', false);
    show('saved-spin');
    clearErr();
    say('Connecting to ' + nodeUrl + '…');
    for (let i = 0; ; i++) {
      // Each attempt that hangs says so after SLOW_MS (a node that stops answering mid-retry too)
      const slow = setTimeout(() => {
        if (mine !== attempt) return;
        say('Your node at ' + nodeUrl + ' hasn’t answered yet. Still trying…');
        show('retry');
        show('other');
      }, SLOW_MS);
      try {
        await invoke('open_saved');
        clearTimeout(slow);
        return;
      } catch (e) {
        clearTimeout(slow);
        if (mine !== attempt) return;
        if (askMoved(e, nodeUrl)) return;
        // Away from the node with music on this device: play it now; the app comes back to the
        // node by itself when it answers (open_offline)
        if (i === 0 && String(e).startsWith("Can't reach") && await invoke('has_downloads').catch(() => false)) {
          say(String(e) + '. Playing what this device keeps meanwhile…');
          try { await invoke('open_offline'); return; } catch (err) { /* fall through to the retries */ }
        }
        show('retry');
        show('other');
        // Only an unreachable node is worth waiting for; a refused token is not
        if (i >= RETRIES.length || !String(e).startsWith("Can't reach")) {
          say(String(e) + '.');
          show('saved-spin', false);
          return;
        }
        for (let left = RETRIES[i]; left > 0; left--) {
          if (left === RETRIES[i]) announce(String(e) + '. Trying again shortly.');
          $('saved-msg').textContent = String(e) + '. Trying again in ' + left + ' s…';
          await new Promise((r) => setTimeout(r, 1000));
          if (mine !== attempt) return;
        }
        say('Connecting to ' + nodeUrl + '…');
      }
    }
  }

  $('retry').onclick = async () => {
    const s = await invoke('saved');
    if (s) reconnect(s.node_url); else connectView();
  };
  $('other').onclick = async () => { attempt++; await invoke('forget'); connectView(); };

  listen('ring-url', (ev) => {
    // Also while reconnecting: a node that refused this device's sign-in asks for a new one
    show('saved', false);
    show('connect');
    ringUrl = ev.payload.url;
    $('qr').innerHTML = ev.payload.qr_svg;
    ringButton().textContent = 'Waiting for Pubky Ring…';
    $('ring-cancel').disabled = false;
    $('ring-cancel').textContent = 'Cancel';
    show('ring-out');
  });

  // While the sign-in waits for the app to be on screen and online (ring_wait.rs).
  // Only while a sign-in runs (the button is disabled then): a late event must not relabel it.
  listen('ring-state', (ev) => { const b = ringButton(); if (b.disabled) b.textContent = String(ev.payload); });

  $('ring').onclick = async () => {
    clearErr();
    ringFor = 'node';
    $('ring').disabled = true;
    $('phone-go').disabled = true;
    $('ring').textContent = 'Starting…';
    try {
      await invoke('ring_connect');
    } catch (e) {
      // Cancel (ring_wait::CANCELLED) is not an error.
      if (String(e) !== 'Sign-in cancelled' && !askMoved(e)) fail(e);
      show('ring-out', false);
    } finally {
      $('ring').disabled = false;
      $('phone-go').disabled = false;
      $('ring').textContent = 'Sign in with Pubky Ring';
    }
  };

  // Phone mode started with sign-ins it couldn't read: they were set aside (the file kept), and
  // the person is told before its page opens; Continue opens it (open_phone, src/phone.rs)
  function showAside(text) {
    show('connect', false);
    show('saved', false);
    show('ring-out', false);
    $('aside-msg').textContent = String(text);
    show('aside');
    announce(String(text));
    $('aside-go').focus();
  }
  $('aside-go').onclick = async () => {
    clearErr();
    $('aside-go').disabled = true;
    try {
      const more = await invoke('open_phone', { seen: true });
      if (more) showAside(more);
    } catch (e) {
      show('aside', false);
      connectView();
      fail(e);
    } finally {
      $('aside-go').disabled = false;
    }
  };

  // Use without a node: Pubky Ring first (it keeps phone mode's settings, sign-ins and data),
  // then the app opens on this phone (phone_ring, src/phone.rs)
  $('phone-go').onclick = async () => {
    clearErr();
    ringFor = 'phone';
    $('phone-go').disabled = true;
    $('ring').disabled = true;
    $('phone-go').textContent = 'Starting…';
    try {
      const notice = await invoke('phone_ring');
      if (notice) {
        $('phone-go').disabled = false;
        $('phone-go').textContent = 'Use without a node';
        showAside(notice);
      }
    } catch (e) {
      if (String(e) !== 'Sign-in cancelled') fail(e);
      show('ring-out', false);
      $('phone-go').disabled = false;
      $('phone-go').textContent = 'Use without a node';
    } finally {
      $('ring').disabled = false;
    }
  };
  $('ring-open').onclick = () => { if (ringUrl) invoke('open_ring', { url: ringUrl }).catch(fail); };
  // Stops the running sign-in; ring_connect then ends with 'Sign-in cancelled'.
  // An approval already being finished is not cut off: the button says so until the sign-in ends.
  $('ring-cancel').onclick = () => {
    $('ring-cancel').disabled = true;
    $('ring-cancel').textContent = 'Cancelling…';
    invoke('ring_cancel').catch(fail);
  };

  $('manual').onsubmit = async (ev) => {
    ev.preventDefault();
    clearErr();
    const btn = ev.submitter || $('manual').querySelector('button');
    btn.disabled = true;
    try {
      await invoke('connect_manual', { nodeUrl: $('url').value, token: $('tok').value });
    } catch (e) {
      fail(e);
    } finally {
      btn.disabled = false;
    }
  };

  // Phone mode chosen before (and no node since): open it again
  async function openPhone() {
    show('connect', false);
    show('saved');
    show('retry', false);
    show('other', false);
    show('saved-spin');
    say('Opening Odeion on this phone…');
    try {
      const notice = await invoke('open_phone');
      if (notice) {
        show('saved-spin', false);
        showAside(notice);
      }
    } catch (e) {
      show('saved-spin', false);
      connectView();
      fail(e);
    }
  }

  const start = () => invoke('saved')
    .then(async (s) => {
      if (s) return reconnect(s.node_url);
      const phone = await invoke('phone_saved').catch(() => null);
      return phone ? openPhone() : connectView();
    })
    .catch(fail);

  // A crash last time: show its report first, and reconnect only after Continue, so a crash on
  // reconnecting cannot loop (src/crash.rs).
  function showCrash(text) {
    $('crash-text').textContent = text;
    // The newest entry's title, in words above the details ("Java exception on thread …")
    const titles = text.split('\n').filter((l) => l.startsWith('== '));
    $('crash-first').textContent = (titles[titles.length - 1] || '').replace(/^== .*?[+-]\d{4}: /, '').replace(/^== /, '');
    show('crash');
    const copy = async () => {
      try { await navigator.clipboard.writeText(text); $('crash-copy').textContent = 'Copied'; return; } catch (e) { /* older WebViews */ }
      const r = document.createRange(); r.selectNodeContents($('crash-text'));
      const sel = getSelection(); sel.removeAllRanges(); sel.addRange(r);
      try { document.execCommand('copy'); $('crash-copy').textContent = 'Copied'; } catch (e) { $('crash-copy').textContent = 'Selected: copy it'; }
    };
    $('crash-copy').onclick = copy;
    $('crash-share').onclick = () => invoke('crash_share', { text }).catch(copy);
    $('crash-go').onclick = async () => { try { await invoke('crash_dismiss'); } catch (e) { /* carry on */ } show('crash', false); start(); };
  }

  invoke('crash_report').then((t) => (t ? showCrash(t) : start())).catch(start);
})();

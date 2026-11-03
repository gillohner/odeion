// Help and problem reports (TASK-trusted-user-feedback; crates/pm-api/src/feedback.rs).
// A "?" button opens Help: "Report a problem" and "My reports". A report is what the person
// writes, plus the technical details they tick (shown exactly before sending); nothing is
// collected on its own. It is saved privately on the node; "received" and "reviewed" show only
// once the node's operator has recorded them.
//
// The page mounts it once: PM_FEEDBACK.mount(hostElement, { token, view, shell, version }), each
// a function returning the current value (the bearer token, the page's view name, phone, tablet
// or desktop, the app version).
(() => {
  'use strict';
  const KINDS = [['problem', 'Problem'], ['idea', 'Idea'], ['question', 'Question']];
  const STATES = { saved: 'Saved on the node', received: 'Received for review', reviewed: 'Reviewed' };
  const MAX = 4000;
  let opts = null;
  let draft = null; // { kind, text, details, client_id }: kept until it is saved
  let helpBtn = null;
  let closeMenu = null; // the open Help menu's close

  const h = (tag, attrs, ...kids) => {
    const el = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs || {})) {
      if (v === false || v == null) continue;
      if (k === 'class') el.className = v;
      else if (k === 'text') el.textContent = v;
      else if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
      else el.setAttribute(k, v === true ? '' : v);
    }
    for (const kid of kids.flat()) if (kid != null) el.append(kid);
    return el;
  };
  const uuid = () => (crypto.randomUUID ? crypto.randomUUID()
    : '10000000-1000-4000-8000-100000000000'.replace(/[018]/g, (c) => (c ^ (crypto.getRandomValues(new Uint8Array(1))[0] & (15 >> (c / 4)))).toString(16)));

  // The browser, as one of four names and its major version
  const browser = () => {
    const ua = navigator.userAgent || '';
    let m;
    if ((m = ua.match(/Firefox\/(\d+)/))) return ['firefox', +m[1]];
    if ((m = ua.match(/(?:Chrome|Chromium|CriOS|Edg)\/(\d+)/))) return ['chromium', +m[1]];
    if ((m = ua.match(/Version\/(\d+).*Safari/))) return ['safari', +m[1]];
    return ['other', null];
  };
  const details = () => {
    const [name, major] = browser();
    const d = {};
    const v = opts.version && opts.version();
    if (v && /^[0-9A-Za-z.+-]{1,32}$/.test(v)) d.version = v;
    const view = opts.view && opts.view();
    if (view && /^[a-z-]{1,32}$/.test(view)) d.view = view;
    const shell = opts.shell && opts.shell();
    if (['phone', 'tablet', 'desktop'].includes(shell)) d.shell = shell;
    d.browser = name;
    if (Number.isInteger(major) && major >= 0 && major <= 65535) d.browser_major = major;
    return d;
  };

  const request = async (method, path, body) => {
    const headers = { 'content-type': 'application/json' };
    const t = opts.token && opts.token();
    if (t) headers.authorization = `Bearer ${t}`;
    const r = await fetch(`/v1${path}`, { method, headers, body: body ? JSON.stringify(body) : undefined });
    let json = null;
    try { json = await r.json(); } catch (e) { /* no body */ }
    return { status: r.status, json, retryAfter: r.headers.get('retry-after') };
  };

  const openDialog = (d, onClose) => {
    document.body.appendChild(d);
    const close = () => { if (d.open) { if (d.close) d.close(); else d.removeAttribute('open'); } d.remove(); if (onClose) onClose(); if (helpBtn) helpBtn.focus(); };
    d.addEventListener('cancel', (e) => { e.preventDefault(); close(); });
    if (d.showModal) d.showModal(); else d.setAttribute('open', '');
    return close;
  };

  // "?": a small menu of the two things Help does
  const menu = () => {
    const items = [
      h('button', { class: 'btn fb-item', type: 'button', role: 'menuitem', text: 'Report a problem…', onclick: () => { if (closeMenu) closeMenu(); report(); } }),
      h('button', { class: 'btn fb-item', type: 'button', role: 'menuitem', text: 'My reports', onclick: () => { if (closeMenu) closeMenu(); mine(); } }),
    ];
    const m = h('div', { class: 'fb-menu', role: 'menu', 'aria-label': 'Help' }, items);
    closeMenu = () => { m.remove(); helpBtn.setAttribute('aria-expanded', 'false'); document.removeEventListener('pointerdown', outside, true); closeMenu = null; };
    const outside = (e) => { if (!m.contains(e.target) && e.target !== helpBtn) closeMenu(); };
    m.addEventListener('keydown', (e) => {
      const i = items.indexOf(document.activeElement);
      if (e.key === 'Escape') { e.preventDefault(); if (closeMenu) closeMenu(); helpBtn.focus(); }
      else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') { e.preventDefault(); items[(i + (e.key === 'ArrowDown' ? 1 : items.length - 1)) % items.length].focus(); }
      else if (e.key === 'Tab' && closeMenu) closeMenu();
    });
    helpBtn.after(m);
    helpBtn.setAttribute('aria-expanded', 'true');
    document.addEventListener('pointerdown', outside, true);
    items[0].focus();
  };

  const report = () => {
    if (!draft) draft = { kind: 'problem', text: '', withDetails: false, client_id: uuid() };
    const status = h('p', { class: 'fb-status', role: 'status', 'aria-live': 'polite' });
    const error = h('p', { class: 'field-err fb-error', role: 'alert' });
    const text = h('textarea', { class: 'input fb-text', id: 'fb-text', rows: '6', maxlength: String(MAX), required: true,
      placeholder: 'What happened, and what did you expect?' });
    text.value = draft.text;
    const count = h('span', { class: 'muted fb-count', 'aria-live': 'off' });
    const counted = () => { count.textContent = `${text.value.length} / ${MAX}`; };
    counted();
    text.addEventListener('input', () => { draft.text = text.value; counted(); error.textContent = ''; });
    const kinds = h('div', { class: 'fb-kinds', role: 'radiogroup', 'aria-label': 'Kind of report' },
      KINDS.map(([k, label]) => h('label', { class: 'fb-kind' },
        h('input', { type: 'radio', name: 'fb-kind', value: k, checked: draft.kind === k, onchange: () => { draft.kind = k; } }), ` ${label}`)));
    const preview = h('pre', { class: 'fb-preview', id: 'fb-preview', hidden: !draft.withDetails });
    const showDetails = () => { preview.textContent = JSON.stringify(details(), null, 1); preview.hidden = !draft.withDetails; };
    showDetails();
    const tick = h('input', { type: 'checkbox', id: 'fb-details', checked: draft.withDetails, 'aria-controls': 'fb-preview',
      onchange: (e) => { draft.withDetails = e.target.checked; showDetails(); error.textContent = ''; } });
    const send = h('button', { class: 'btn primary', type: 'button', 'data-send': true, text: 'Send' });
    const cancel = h('button', { class: 'btn', type: 'button', text: 'Cancel' });
    const d = h('dialog', { class: 'dialog fb-dialog', 'aria-labelledby': 'fb-title' },
      h('h2', { id: 'fb-title', text: 'Report a problem' }),
      h('p', { class: 'muted fb-share', id: 'fb-share',
        text: 'Saved privately on this node for its owner. Not public. Only what you write here is sent, and the details below if you tick them: no logs, screenshots or library.' }),
      kinds,
      h('label', { class: 'field fb-field', for: 'fb-text' }, h('span', { text: 'Description' }), text),
      count,
      h('label', { class: 'fb-tick' }, tick, ' Include technical details'),
      preview,
      error, status,
      h('div', { class: 'dialog-actions' }, cancel, send));
    text.setAttribute('aria-describedby', 'fb-share');
    const close = openDialog(d);
    cancel.addEventListener('click', close);
    send.addEventListener('click', async () => {
      error.textContent = '';
      if (!text.value.trim()) { error.textContent = 'Describe the problem first.'; text.focus(); return; }
      const body = { kind: draft.kind, text: text.value, client_id: draft.client_id };
      if (draft.withDetails) body.details = details();
      send.disabled = true; send.textContent = 'Sending…'; status.textContent = 'Sending…';
      let r;
      try { r = await request('POST', '/feedback', body); } catch (e) { r = null; }
      send.disabled = false; send.textContent = 'Send'; status.textContent = '';
      if (r && (r.status === 201 || r.status === 200)) {
        draft = null;
        d.replaceChildren(
          h('h2', { id: 'fb-title', text: 'Report saved' }),
          h('p', { class: 'fb-done', role: 'status', text: 'Saved privately on this node. "My reports" shows when it has been received and reviewed.' }),
          h('div', { class: 'dialog-actions' },
            h('button', { class: 'btn', type: 'button', text: 'My reports', onclick: () => { close(); mine(); } }),
            h('button', { class: 'btn primary', type: 'button', 'data-done': true, text: 'Done', onclick: close })));
        d.querySelector('[data-done]').focus();
        return;
      }
      if (!r) error.textContent = "Couldn't reach the node. Your report is kept here: try again.";
      else if (r.status === 429) {
        const min = Math.max(1, Math.ceil((+r.retryAfter || 60) / 60));
        error.textContent = `That's a lot of reports at once. Your report is kept here: try again in ${min} minute${min === 1 ? '' : 's'}.`;
      } else if (r.status === 401) error.textContent = 'Your sign-in has ended. Sign in again, then send: your report is kept here until you close the page.';
      else error.textContent = (r.json && r.json.error) || "The node couldn't save the report. Try again.";
    });
    text.focus();
  };

  const mine = async () => {
    const list = h('ul', { class: 'fb-list', 'aria-label': 'My reports' });
    const note = h('p', { class: 'muted', role: 'status', text: 'Loading…' });
    const done = h('button', { class: 'btn primary', type: 'button', text: 'Close' });
    const d = h('dialog', { class: 'dialog fb-dialog', 'aria-labelledby': 'fb-mine-title' },
      h('h2', { id: 'fb-mine-title', text: 'My reports' }), note, list, h('div', { class: 'dialog-actions' }, done));
    const close = openDialog(d);
    done.addEventListener('click', close);
    done.focus();
    let r;
    try { r = await request('GET', '/feedback/mine'); } catch (e) { r = null; }
    if (!r || r.status !== 200 || !Array.isArray(r.json)) { note.textContent = (r && r.json && r.json.error) || "Couldn't reach the node. Try again."; return; }
    note.textContent = r.json.length ? '' : "You haven't sent a report yet.";
    for (const it of r.json) {
      const when = new Date(it.sent_at * 1000).toLocaleString();
      const kind = (KINDS.find(([k]) => k === it.kind) || [, 'Report'])[1];
      list.append(h('li', { class: 'fb-row' },
        h('span', { class: 'fb-row-head' }, h('strong', { text: kind }), h('span', { class: 'muted', text: ` · ${when}` })),
        h('span', { class: `fb-state fb-${STATES[it.state] ? it.state : 'saved'}`, text: STATES[it.state] || STATES.saved }),
        h('span', { class: 'fb-preview-line', text: it.preview || '' })));
    }
  };

  window.PM_FEEDBACK = {
    mount(host, o) {
      opts = o || {};
      if (!document.querySelector('link[data-fb-css]')) document.head.append(h('link', { rel: 'stylesheet', href: '/feedback.css', 'data-fb-css': true }));
      helpBtn = h('button', { class: 'btn fb-help', type: 'button', 'aria-label': 'Help and problem reports', 'aria-haspopup': 'menu', 'aria-expanded': 'false', text: '?',
        onclick: () => (closeMenu ? closeMenu() : menu()) });
      host.append(h('span', { class: 'fb-anchor' }, helpBtn));
      return helpBtn;
    },
    report: () => report(),
    mine: () => mine(),
  };
})();

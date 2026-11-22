  // ---- Copy to your homeserver (TASK-copy-library-folders; the job is mn-infra's) ----
  // One dialog: what to copy (everything, an artist, an album, or folders from a lazy tree with
  // boxes that can be partly ticked), the selection's totals from a dry run, then a background
  // job: the dialog closes at once and the job chip shows the progress anywhere. Slow answers show
  // a skeleton, "Still loading…" after 2 s, and their error in place with Try again.
  const fmtBytes = (b) => (b >= 1e9 ? `${(b / 1e9).toFixed(1)} GB` : `${Math.max(1, Math.round(b / 1e6))} MB`);
  const fmtRate = (bps) => (bps >= 1e6 ? `${(bps / 1e6).toFixed(1)} MB/s` : `${Math.max(1, Math.round(bps / 1e3))} kB/s`);
  const fmtLeft = (s) => (s >= 3600 ? `${Math.floor(s / 3600)} h ${Math.round((s % 3600) / 60)} min left` : s >= 60 ? `${Math.round(s / 60)} min left` : 'less than a minute left');
  // A call whose answer is read whatever its status (a 507's numbers, a node without the route)
  const call = async (method, path, body) => {
    // A change made while the connection is down waits for it, as with api(); when the wait gives
    // up, the answer is a failed one (the callers show it), never a throw
    if (method !== 'GET') { try { await whenConnected(); } catch (e) { return { status: 0, ok: false, json: { error: (e && e.message) || 'The node isn’t connected. Try again in a moment.' } }; } }
    const sent = state.token;
    let r; try { r = await fetch('/v1' + path, { method, headers: { 'content-type': 'application/json', ...(sent ? { authorization: 'Bearer ' + sent } : {}) }, body: body === undefined ? undefined : JSON.stringify(body) }); } catch { return { status: 0, ok: false, json: { error: 'Can’t reach the node.' } }; }
    let json = null; try { json = await r.json(); } catch { json = null; }
    if (r.status === 401 && json && json.error === NODE_REFUSED) sessionEnded(sent);
    return { status: r.status, ok: r.ok, json: json || {} };
  };
  const skelRows = (n) => `<div class="skeleton sk-row" style="height:${n * 36}px" aria-hidden="true"></div>`;
  // "Still loading…" under a skeleton after 2 s, as long as `still()` says it is the same load
  const slowNote = (el, still) => setTimeout(() => { if (still() && el.isConnected) { const n = el.querySelector('.slow-note'); if (n) n.hidden = false; } }, 2000);
  const SLOW = '<p class="muted small slow-note" role="status" hidden>Still loading…</p>';
  // (the page's own first loader, index.html #bootskel, says so too while the node's first answers take long)
  slowNote(document.getElementById('main'), () => !!document.getElementById('bootskel'));
  // (and "Loading…" for a screen reader: in the status region navigate() uses, put in after the page
  // has opened, since words a status region already has when the page loads are not read out; not
  // behind Log in. The first page's draw empties it, as after any navigation)
  { const said = document.createElement('p'); said.id = 'navstatus'; said.className = 'sr-only'; said.setAttribute('role', 'status'); document.body.appendChild(said);
    setTimeout(() => { if (document.getElementById('bootskel') && !document.body.classList.contains('signedout')) said.textContent = 'Loading…'; }, 300); }
  const ROOT = '';
  const copyTree = () => ({ nodes: new Map([[ROOT, { path: ROOT, parent: null, kids: null, next: null, open: true }]]), sel: new Map(), focus: null, missing: false });
  // `body`: a preset (an album's or songs' ⋯ Copy); none opens on the folders
  const startUpload = (body) => copyDialog(body || null);
  const copyDialog = (preset) => new Promise((resolve) => {
    const t = copyTree();
    const pick = { mode: preset ? 'preset' : 'folders', artist: null, album: null };
    const d = document.createElement('dialog'); d.className = 'dialog copydlg'; d.setAttribute('aria-labelledby', 'copyh');
    const presetLabel = preset ? (preset.album_id != null ? 'This album' : preset.track_ids ? (preset.track_ids.length === 1 ? 'This song' : `These ${preset.track_ids.length} songs`) : 'Everything') : '';
    d.innerHTML = `<h2 id="copyh">Copy to your homeserver</h2><p class="muted small">Into your homeserver’s private storage (never public). Files already there are skipped. It runs on the node: you can close this page.</p>`
      + `<div class="chips copypick" role="radiogroup" aria-label="What to copy">${[...(preset ? [['preset', presetLabel]] : []), ['folders', 'Folders'], ['all', 'Everything'], ['artist', 'An artist'], ['album', 'An album']].map(([k, l]) => `<button class="chip" type="button" role="radio" data-cpick="${k}">${esc(l)}</button>`).join('')}</div>`
      + '<div class="copypanel" id="copypanel"></div>'
      + '<p class="small copytotals" id="copytotals" role="status" aria-live="polite"></p>'
      + '<div class="dialog-actions"><button class="btn ghost" type="button" data-cclose>Cancel</button><button class="btn primary" type="button" id="copygo" disabled>Copy</button></div>';
    document.body.appendChild(d);
    const tr = { open: () => d.open, close: (fromBack) => done(null, fromBack) };
    const done = (v, fromBack) => { const k = transients.indexOf(tr); if (k >= 0) transients.splice(k, 1); clearTimeout(tot.timer); if (d.open) { if (d.close) d.close(); else d.removeAttribute('open'); } d.remove(); if (fromBack !== true) popTransient(); resolve(v); };
    transients.push(tr); pushTransient();
    d.addEventListener('cancel', (e) => { e.preventDefault(); done(null); });
    $('[data-cclose]', d).onclick = () => done(null);
    const panel = $('#copypanel', d); const totals = $('#copytotals', d); const go = $('#copygo', d);
    // ---- the folder tree ----
    const node = (p) => t.nodes.get(p);
    const under = (q, p) => { for (let x = node(q) && node(q).parent; x != null; x = node(x) ? node(x).parent : null) if (x === p) return true; return false; };
    const effOn = (p) => { for (let q = p; q != null && q !== ROOT; q = node(q).parent) if (t.sel.has(q)) return t.sel.get(q); return false; };
    const mixed = (p) => { const on = effOn(p); for (const [q, v] of t.sel) if (v !== on && under(q, p)) return true; return false; };
    const boxState = (p) => (mixed(p) ? 'mixed' : effOn(p) ? 'true' : 'false');
    const level = (p) => { let n = 0; for (let q = p; q != null && q !== ROOT; q = node(q).parent) n++; return n; };
    // One request per level at a time: a second caller (Show more and an untick's loadAll at once)
    // waits for the one under way instead of returning at once, so loadAll never spins
    const loadLevel = (p, more) => { const n = node(p); if (!n.req) n.req = fetchLevel(p, more).finally(() => { n.req = null; }); return n.req; };
    const fetchLevel = async (p, more) => {
      const n = node(p);
      const qs = new URLSearchParams({ limit: '200' }); if (p !== ROOT) qs.set('path', p); if (more && n.next) qs.set('after', n.next);
      n.loading = true; n.error = null; drawTree();
      const r = await call('GET', `/library/tree?${qs}`);
      n.loading = false;
      if (r.status === 404 || r.status === 405) { t.missing = true; drawPanel(); return; }
      if (!r.ok) { n.error = r.json.error || `HTTP ${r.status}`; drawTree(); return; }
      const kids = Array.isArray(r.json) ? r.json : r.json.children || [];
      if (!more) n.kids = [];
      for (const k of kids) { if (!t.nodes.has(k.path)) t.nodes.set(k.path, { ...k, parent: p, kids: null, next: null, open: false }); n.kids.push(k.path); }
      n.next = (!Array.isArray(r.json) && r.json.next) || null;
      // A ticked folder's new children are ticked with it (they inherit; nothing to store)
      drawTree();
    };
    const row = (p) => {
      const n = node(p); const lvl = level(p);
      const there = n.present_pending ? '' : n.present ? ` · ${n.present} already there` : '';
      return `<div class="ctrow" role="treeitem" aria-level="${lvl}" ${n.leaf ? '' : `aria-expanded="${!!n.open}"`} aria-checked="${boxState(p)}" tabindex="${t.focus === p ? 0 : -1}" data-cpath="${esc(p)}" style="--lvl:${lvl - 1}">`
        + `<span class="ctcaret" data-ccaret aria-hidden="true">${n.leaf ? '' : icon('chevron')}</span><span class="ctbox" aria-hidden="true"></span>`
        + `<span class="grow ctname">${esc(n.name || p)}</span><span class="muted small ctmeta">${countOf(n.files || 0, 'files')} · ${fileSize(n.bytes || 0)}${there}</span></div>`;
    };
    const rowsOf = (p) => {
      const n = node(p); let h = '';
      if (n.kids) for (const k of n.kids) { h += row(k); const c = node(k); if (c.open) h += `<div role="group">${rowsOf(k)}</div>`; }
      if (n.loading) h += `<div class="ctwait" style="--lvl:${level(p)}">${skelRows(n.kids && n.kids.length ? 1 : 3)}${SLOW}</div>`;
      else if (n.error) h += `<div class="ctwait notice warn small" style="--lvl:${level(p)}">${esc(n.error)} <button class="btn sm" type="button" data-cretry="${esc(p)}">Try again</button></div>`;
      else if (n.next) h += `<div class="ctwait" style="--lvl:${level(p)}"><button class="btn ghost sm" type="button" data-cmore="${esc(p)}">Show more</button></div>`;
      return h;
    };
    const drawTree = () => {
      const box = $('#ctree', d); if (!box) return;
      const had = document.activeElement && document.activeElement.closest && document.activeElement.closest('#ctree') ? document.activeElement.dataset.cpath : null;
      if (t.focus == null || !t.nodes.has(t.focus)) t.focus = (node(ROOT).kids || [])[0] ?? null;
      box.innerHTML = rowsOf(ROOT) || (node(ROOT).kids ? '<p class="muted small">No music folders on this node yet.</p>' : '');
      const n = node(ROOT); if (n.loading) slowNote(box, () => n.loading);
      $$('.ctwait', box).forEach((w) => { const p = w.parentElement; if (p) slowNote(w, () => w.isConnected); });
      if (had != null) { const r = box.querySelector(`[data-cpath="${cssStr(had)}"]`); if (r) r.focus(); }
    };
    // Only the boxes change on a tick: no redraw of the whole tree
    const refreshBoxes = () => $$('#ctree .ctrow', d).forEach((r) => r.setAttribute('aria-checked', boxState(r.dataset.cpath)));
    // Every child of a level (for an untick inside a ticked folder); false when one page failed
    const loadAll = async (p) => { while (node(p).next && !node(p).error && !t.missing) await loadLevel(p, true); return !node(p).next && !node(p).error; };
    const tick = async (p) => {
      if (t.busy) return;
      t.busy = true; go.disabled = true;
      try { await tickNow(p); } finally { t.busy = false; totalsSoon(); }
    };
    const tickNow = async (p) => {
      const want = boxState(p) !== 'true';
      if (!want) {
        // Unticking under a ticked folder: that folder is unticked and the others beside the way
        // down ticked, so what is sent stays a plain list of folders
        let top = null; for (let q = node(p).parent; q != null && q !== ROOT; q = node(q).parent) if (t.sel.get(q) === true) { top = q; break; }
        if (top != null) {
          const chain = []; for (let q = p; q !== top; q = node(q).parent) chain.unshift(q);
          let at = top;
          // All of each level first: a child that didn't load would silently be left out
          for (let q = top, k = 0; k < chain.length; q = chain[k], k++) if (!(await loadAll(q))) { toast('Couldn’t load every folder there; nothing was unticked. Try again.', 'error'); return; }
          for (const step of chain) { t.sel.set(at, false); for (const k of node(at).kids || []) if (k !== step) t.sel.set(k, true); at = step; }
        }
      }
      for (const q of [...t.sel.keys()]) if (under(q, p)) t.sel.delete(q);
      t.sel.set(p, want);
      // A choice its parent already makes is not stored
      const par = node(p).parent; if (par != null && par !== ROOT && effOn(par) === want) t.sel.delete(p); else if ((par == null || par === ROOT) && !want) t.sel.delete(p);
      refreshBoxes();
    };
    const toggleOpen = async (p) => { const n = node(p); if (n.leaf) return; n.open = !n.open; if (n.open && !n.kids) { await loadLevel(p); return; } drawTree(); };
    const visibleRows = () => $$('#ctree .ctrow', d);
    const moveFocus = (r) => { if (!r) return; t.focus = r.dataset.cpath; visibleRows().forEach((x) => x.tabIndex = -1); r.tabIndex = 0; r.focus(); };
    const treeKeys = (e) => {
      const r = e.target.closest && e.target.closest('.ctrow'); if (!r) return;
      const rows = visibleRows(); const i = rows.indexOf(r); const p = r.dataset.cpath; const n = node(p);
      if (e.key === 'ArrowDown') { e.preventDefault(); moveFocus(rows[i + 1]); }
      else if (e.key === 'ArrowUp') { e.preventDefault(); moveFocus(rows[i - 1]); }
      else if (e.key === 'Home') { e.preventDefault(); moveFocus(rows[0]); }
      else if (e.key === 'End') { e.preventDefault(); moveFocus(rows[rows.length - 1]); }
      else if (e.key === 'ArrowRight') { e.preventDefault(); if (!n.leaf && !n.open) toggleOpen(p); else if (n.open) moveFocus(rows[i + 1]); }
      else if (e.key === 'ArrowLeft') { e.preventDefault(); if (n.open) toggleOpen(p); else if (n.parent && n.parent !== ROOT) moveFocus(rows.find((x) => x.dataset.cpath === n.parent)); }
      else if ((e.key === ' ' || e.key === 'Enter') && !t.busy) { e.preventDefault(); tick(p); }
    };
    // ---- artists and albums: a search over the library's lists ----
    // `lists` holds only whole lists; `partial` the one paging in now, drawn as it grows
    const lists = { artists: null, albums: null }; const partial = { artists: null, albums: null };
    const pickList = (kind) => {
      const all = lists[kind] || partial[kind]; const box = $('#cplist', d); if (!box || !all) return;
      const q = ($('#cpq', d).value || '').trim().toLowerCase();
      const hit = (x) => !q || String(kind === 'artists' ? x.name : `${x.title} ${x.album_artist || x.artist || ''}`).toLowerCase().includes(q);
      const rows = []; for (const x of all) { if (hit(x)) rows.push(x); if (rows.length >= 30) break; }
      const chosen = kind === 'artists' ? pick.artist : pick.album;
      box.innerHTML = rows.length ? rows.map((x) => `<button class="list-row cpitem${chosen && chosen.id === x.id ? ' on' : ''}" type="button" role="radio" aria-checked="${!!(chosen && chosen.id === x.id)}" data-cpid="${x.id}"><span class="grow">${esc(kind === 'artists' ? x.name : x.title)}</span><span class="muted small">${esc(kind === 'artists' ? countOf(x.track_count ?? x.tracks ?? 0, 'songs') : x.album_artist || x.artist || '')}</span></button>`).join('') : '<p class="muted small">Nothing matches.</p>';
      $$('[data-cpid]', box).forEach((b) => b.onclick = () => { const x = all.find((y) => String(y.id) === b.dataset.cpid); if (kind === 'artists') pick.artist = x; else pick.album = x; pickList(kind); totalsSoon(); });
    };
    const loadList = async (kind) => {
      const box = $('#cplist', d); box.innerHTML = skelRows(4) + SLOW; const gen = ++loadGen; slowNote(box, () => gen === loadGen);
      try {
        if (kind === 'artists') lists.artists = await api('GET', '/library/artists');
        else {
          // The node gives at most 1 000 albums an answer: page on with `after`, the list growing.
          // Only the newest load may draw, and only a whole list is kept: one left part way
          // (another choice, a failed page) loads whole next time
          let after = null; let got = []; partial.albums = null;
          for (;;) {
            const page = await api('GET', `/library/albums?limit=1000${after != null ? `&after=${after}` : ''}`);
            if (gen !== loadGen) return;
            got = got.concat(page); partial.albums = got; pickList(kind);
            if (page.length < 1000 || !page.length) break;
            after = page[page.length - 1].id;
          }
          lists.albums = got; partial.albums = null;
        }
      } catch (e) {
        // Only the newest load answers: the part it had goes (a search would draw it over the
        // error), and its Try again is the one bound
        if (gen !== loadGen) return;
        partial[kind] = null;
        if (box.isConnected) { box.innerHTML = `<div class="notice warn small">${esc(e.message)} <button class="btn sm" type="button" id="cplretry">Try again</button></div>`; $('#cplretry', d).onclick = () => loadList(kind); }
        return;
      }
      if (gen === loadGen) pickList(kind);
    };
    let loadGen = 0;
    // ---- the panel for each choice ----
    const drawPanel = () => {
      loadGen++; // a list still loading for the last choice draws nothing more
      // A node without the folder list: Folders goes, and the choice (and the focus) moves on
      const fell = pick.mode === 'folders' && t.missing;
      if (fell) pick.mode = preset ? 'preset' : 'all';
      $$('[data-cpick]', d).forEach((b) => { const on = b.dataset.cpick === pick.mode; b.classList.toggle('on', on); b.setAttribute('aria-checked', String(on)); b.tabIndex = on ? 0 : -1; b.hidden = b.dataset.cpick === 'folders' && t.missing; });
      if (fell && (!d.contains(document.activeElement) || document.activeElement === document.body)) { const on = $('[data-cpick].on', d); if (on) on.focus(); }
      if (pick.mode === 'folders') {
        panel.innerHTML = '<p class="muted small">Tick folders: a folder copies everything inside it. Open one to pick inside it.</p><div class="ctree" id="ctree" role="tree" aria-label="Music folders" aria-multiselectable="true"></div>';
        const box = $('#ctree', d);
        box.addEventListener('click', (e) => {
          const more = e.target.closest('[data-cmore]'); if (more) { more.disabled = true; loadLevel(more.dataset.cmore, true); return; }
          const retry = e.target.closest('[data-cretry]'); if (retry) { loadLevel(retry.dataset.cretry); return; }
          const r = e.target.closest('.ctrow'); if (!r || t.busy) return; moveFocus(r);
          if (e.target.closest('[data-ccaret]')) toggleOpen(r.dataset.cpath); else tick(r.dataset.cpath);
        });
        box.addEventListener('keydown', treeKeys);
        if (!node(ROOT).kids && !node(ROOT).loading) loadLevel(ROOT); else drawTree();
      } else if (pick.mode === 'artist' || pick.mode === 'album') {
        const kind = pick.mode === 'artist' ? 'artists' : 'albums';
        panel.innerHTML = `<label class="sr-only" for="cpq">${pick.mode === 'artist' ? 'Find an artist' : 'Find an album'}</label><input class="input" id="cpq" placeholder="${pick.mode === 'artist' ? 'Find an artist' : 'Find an album'}" autocomplete="off"><div class="list compact cplist" id="cplist" role="radiogroup" aria-label="${pick.mode === 'artist' ? 'Artists' : 'Albums'}"></div>`;
        $('#cpq', d).oninput = () => pickList(kind);
        if (lists[kind]) pickList(kind); else loadList(kind);
      } else panel.innerHTML = pick.mode === 'all' ? '<p class="muted small">Every song in this node’s library.</p>' : '';
      totalsSoon();
    };
    $$('[data-cpick]', d).forEach((b) => b.onclick = () => { pick.mode = b.dataset.cpick; drawPanel(); });
    $('.copypick', d).addEventListener('keydown', (e) => { if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft') return; const bs = $$('[data-cpick]:not([hidden])', d); const i = bs.indexOf(e.target); const nx = bs[(i + (e.key === 'ArrowRight' ? 1 : bs.length - 1)) % bs.length]; if (nx) { e.preventDefault(); nx.click(); nx.focus(); } });
    // ---- what the selection is, and its totals ----
    const selection = async () => {
      if (pick.mode === 'preset') return preset;
      if (pick.mode === 'all') return { all: true };
      if (pick.mode === 'album') return pick.album ? { album_id: pick.album.id } : null;
      // The artist chosen when asked: a slow answer belongs to it, whatever is chosen meanwhile
      if (pick.mode === 'artist') { const a = pick.artist; if (!a) return null; if (!a.track_ids) { const r = await api('GET', `/library/artists/${a.id}/tracks`); a.track_ids = (r.tracks || []).map((x) => x.id); } return a.track_ids.length ? { track_ids: a.track_ids } : null; }
      const paths = [...t.sel].filter(([, v]) => v).map(([q]) => q);
      return paths.length ? { paths } : null;
    };
    const tot = { timer: null, gen: 0, body: null };
    // A new selection: an answer still on its way for the old one is dropped (it can't turn Copy on)
    const totalsSoon = () => { clearTimeout(tot.timer); tot.gen++; tot.body = null; go.disabled = true; tot.timer = setTimeout(showTotals, 400); };
    const showTotals = async () => {
      const gen = ++tot.gen; let body;
      try { body = await selection(); } catch (e) { totals.textContent = e.message; return; }
      if (gen !== tot.gen) return;
      tot.body = body;
      if (!body) { totals.innerHTML = '<span class="muted">Pick what to copy.</span>'; go.disabled = true; return; }
      totals.innerHTML = '<span class="muted"><span class="spin" aria-hidden="true"></span> Counting…</span>';
      const r = await call('POST', '/pubky/files/upload', { ...body, dry_run: true });
      if (gen !== tot.gen) return;
      if (r.status === 0) { totals.innerHTML = `<span class="warn-text">${esc(r.json.error)}</span> <button class="btn sm" type="button" id="copyretot">Try again</button>`; $('#copyretot', d).onclick = () => { clearTimeout(tot.timer); tot.body = null; showTotals(); }; go.disabled = true; return; }
      const j = r.json;
      if (!r.ok && r.status !== 507) { totals.innerHTML = `<span class="warn-text">${esc(j.error || `HTTP ${r.status}`)}</span>`; go.disabled = true; return; }
      const files = j.total ?? 0; const bytes = j.bytes_total ?? 0; const skip = j.skipped ?? 0;
      const need = j.needed_bytes; const free = j.free_bytes;
      const room = need != null && free != null ? ` · needs ${fmtBytes(need)}, ${fmtBytes(free)} free` : '';
      const full = r.status === 507 || (need != null && free != null && need > free);
      totals.innerHTML = `${countOf(files, 'files')} · ${fmtBytes(bytes)}${skip ? ` · ${skip} already on your homeserver` : ''}${room}`
        + (full ? `<br><span class="warn-text">${esc(j.error || `Not enough room on your homeserver: ${fmtBytes(need)} needed, ${fmtBytes(free)} free.`)}</span>` : '');
      go.disabled = full || !(files - skip > 0 || files > 0);
    };
    go.onclick = async () => {
      if (!tot.body) return; go.disabled = true;
      const r = await call('POST', '/pubky/files/upload', tot.body);
      if (!r.ok) { totals.innerHTML = `<span class="warn-text">${esc(r.json.error || `HTTP ${r.status}`)}</span>`; go.disabled = false; return; }
      done(true);
      toast(r.json.state === 'queued' ? 'Queued: it starts when the copy before it is done' : 'Copying in the background: the chip in the player bar shows how far it is');
      watchJob(r.json.id);
    };
    if (d.showModal) d.showModal(); else d.setAttribute('open', '');
    drawPanel();
    const first = $('[data-cpick].on', d); if (first) first.focus();
  });
  // The homeserver's private files in Settings: sizes that read right for small files, and the
  // broken ones (empty, or a second copy under a doubled name) marked with Remove.
  const fileSize = (b) => (b == null ? '' : b >= 1e9 ? `${(b / 1e9).toFixed(1)} GB` : b >= 1e6 ? `${(b / 1e6).toFixed(1)} MB` : b >= 1e3 ? `${Math.round(b / 1e3)} KB` : `${b} bytes`);
  const brokenWhy = (f, names) => {
    if (f.size === 0) return 'Empty: copy the library again to send it again';
    const n = f.path.split('/').pop(); const m = /^([0-9a-f]{16})-(.+)$/.exec(n);
    if (m && m[2].startsWith(`${m[1]}-`) && names.has(m[2])) return 'A second copy (doubled name)';
    return '';
  };
  const drawHomeFiles = (box, files) => {
    if (!files.length) { box.innerHTML = '<span>No private files on the homeserver yet. Copy the library, or an album from its ⋯ menu; audio only ever goes under /priv/.</span>'; return; }
    const names = new Set(files.map((f) => f.path.split('/').pop()));
    const rows = files.map((f) => ({ f, why: brokenWhy(f, names) })).sort((a, b) => (b.why ? 1 : 0) - (a.why ? 1 : 0));
    const broken = rows.filter((r) => r.why);
    const total = files.reduce((n, f) => n + (f.size || 0), 0);
    box.innerHTML = `<div class="eyebrow">Private files on the homeserver · ${fmtCount(files.length)} · ${fileSize(total)}</div>`
      + (broken.length ? `<div class="set-warn small">${countOf(broken.length, 'files')} ${broken.length === 1 ? 'is' : 'are'} broken or a second copy. <button class="btn ghost sm" data-rmbroken>Remove ${broken.length === 1 ? 'it' : 'them'}</button></div>` : '')
      + `<div class="list compact">${rows.slice(0, 50).map(({ f, why }) => `<div class="list-row${why ? ' broken' : ''}"><div class="grow"><div class="name">${esc(f.path.split('/').pop())}</div>${why ? `<div class="muted small">${esc(why)}</div>` : ''}</div><span class="muted small">${fileSize(f.size)}</span>${why ? `<button class="btn ghost sm" data-rmfile="${esc(f.path)}">Remove</button>` : ''}</div>`).join('')}${files.length > 50 ? `<div class="muted small">… and ${fmtCount(files.length - 50)} more</div>` : ''}</div>`;
    const removeFiles = async (paths, btn) => { btn.disabled = true; try { drawHomeFiles(box, await api('DELETE', '/pubky/files', { paths })); toast(`Removed ${countOf(paths.length, 'files')} from your homeserver`); } catch (e) { toast(e.message, 'error'); btn.disabled = false; } };
    const all = $('[data-rmbroken]', box); if (all) all.onclick = () => removeFiles(broken.map((r) => r.f.path), all);
    $$('[data-rmfile]', box).forEach((b) => b.onclick = () => removeFiles([b.dataset.rmfile], b));
  };
  // My music's files on the homeserver (TASK-copy-library-folders): drawn after the page with a
  // skeleton meanwhile (never "No music" before the answer), 200 at a time with Show more and a
  // search. A node that sends every file at once (before paging) is paged here.
  const MINE_PAGE = 200;
  const mineList = (box, ctx) => {
    if (!box) return;
    // Kept over a re-render of the page (the save status redraws it every 3 s while saving)
    const kept = state.mineSt && Date.now() - state.mineSt.at < 15000 ? state.mineSt : null;
    const st = kept || { q: '', files: [], count: 0, bytes: 0, gen: 0, local: null, at: 0, loaded: false };
    state.mineSt = st;
    box.innerHTML = `<div class="toolbar mmsearch" hidden><label class="sr-only" for="mmq">Find in your files</label><input class="input" id="mmq" type="search" placeholder="Find in your files" autocomplete="off"></div><div id="mmlist"></div>`;
    const list = $('#mmlist', box);
    const match = (f) => { const q = st.q.toLowerCase(); return [f.name, f.song && f.song.title, f.song && f.song.artist, f.song && f.song.album].some((x) => x && String(x).toLowerCase().includes(q)); };
    const cover = (f) => (f.song.album_id ? `<img class="thumb" src="${esc(imgSrc(`/v1/library/albums/${f.song.album_id}/cover`))}" alt="" loading="lazy" data-err="hide">` : '<span class="thumb"></span>');
    const songRow = (f, k) => `<div class="list-row mrow" data-mrow="${k}">${cover(f)}<button class="icon-btn" data-mplay="${f.song.id}" aria-label="Play ${esc(f.song.title)}" title="Play">${icon('play')}</button><div class="grow"><div class="name">${esc(f.song.title)}${f.song.edited ? ' <span class="pill" title="Tags edited on this node">edited</span>' : ''}</div><div class="sub">${esc([f.song.artist, f.song.album].filter(Boolean).join(' · '))}</div></div><span class="muted small">${f.song.duration_ms ? fmt(f.song.duration_ms) : ''}</span><span class="muted small msize">${fileSize(f.size)}</span><button class="icon-btn" data-mmore="${k}" aria-label="More for ${esc(f.song.title)}" title="More">${icon('more')}</button></div>`;
    const looseRow = (f, k) => `<div class="list-row mrow"><div class="grow"><div class="name">${esc(f.name)}</div><div class="sub">${f.removed ? 'Taken out of the library: Node library brings it back.' : 'Not in the library yet: it is read with the next scan.'}</div></div><span class="muted small msize">${fileSize(f.size)}</span><button class="btn sm ghost" data-mdel="${k}" aria-label="Delete ${esc(f.name)} from your homeserver">Delete</button></div>`;
    const delFile = async (f) => {
      const ok = await choiceDialog({ title: 'Delete from your homeserver?', text: `“${f.song ? f.song.title : f.name}” is deleted from your homeserver${ctx.bundled ? ' and leaves the library' : ''}. This can’t be undone.`, choices: [{ label: 'Delete', value: true, primary: true }] });
      if (!ok) return;
      try { await api('DELETE', '/pubky/files', { paths: [f.path] }); toast('Deleted'); st.local = null; load(); } catch (er) { toast(er.message, 'error'); }
    };
    const draw = () => {
      st.at = Date.now(); st.loaded = true;
      box.removeAttribute('aria-busy');
      const used = $('#mmused'); if (used) used.textContent = st.q ? '' : ` · ${countOf(st.count, 'files')} · ${fileSize(st.bytes)}${ctx.room || ''} used`;
      $('.mmsearch', box).hidden = !(st.q || st.count > 20);
      const files = st.files; const songs = files.filter((f) => f.song); const loose = files.filter((f) => !f.song);
      if (!st.count && !st.q) { list.innerHTML = `<div class="card mmempty"><p>No music on your homeserver yet.</p><div class="row-btns"><button class="btn primary" data-libup>${icon('upload')} Upload files</button><button class="btn" data-mmcopy>${icon('upload')} Copy from this node…</button></div></div>`; }
      else if (!files.length) list.innerHTML = '<p class="muted">Nothing matches.</p>';
      else list.innerHTML = (songs.length ? `<section class="section"><div class="section-head"><h3>On your homeserver</h3><span class="muted small">${st.q ? `${countOf(st.count, 'matches')}` : countOf(st.count, 'files')}</span></div><div class="list">${songs.map((f) => songRow(f, files.indexOf(f))).join('')}</div></section>` : '')
        + (loose.length ? `<section class="section"><div class="section-head"><h3>${loose.every((f) => f.removed) ? 'Not in the library' : 'Being added'}</h3></div><div class="list">${loose.map((f) => looseRow(f, files.indexOf(f))).join('')}</div></section>` : '')
        + (files.length < st.count ? `<div class="row-btns"><button class="btn ghost" data-mmmore>Show more <span class="muted small">(${fmtCount(st.count - files.length)} more)</span></button></div>` : '');
      $$('[data-mplay]', list).forEach((b) => b.onclick = async () => { b.classList.add('starting'); try { await playTracks([await api('GET', `/library/tracks/${b.dataset.mplay}`)], true); } catch (er) { toast(er.message, 'error'); } finally { b.classList.remove('starting'); } });
      $$('[data-mdel]', list).forEach((b) => b.onclick = () => delFile(files[Number(b.dataset.mdel)]));
      $$('[data-mmore]', list).forEach((b) => b.onclick = () => { const f = files[Number(b.dataset.mmore)]; showMenu(b, [{ title: f.song.title }, { label: 'Edit tags…', icon: 'notes', run: () => editTrackTags(f.song.id) }, { label: 'Download', icon: 'download', run: () => { location.href = `/v1/library/tracks/${f.song.id}/download${tokenQ()}`; } }, ...(f.song.album_id ? [{ label: 'Go to album', icon: 'disc', run: () => setView('item', { kind: 'album', id: `lib-${f.song.album_id}` }) }] : []), { label: 'Delete from my homeserver', icon: 'trash', run: () => delFile(f) }]); });
      const mm = $('[data-mmmore]', list); if (mm) mm.onclick = () => { mm.disabled = true; load(true); };
      $$('[data-libup]', list).forEach((b) => b.onclick = () => { const s2 = $('#libupload'); if (s2) { s2.hidden = false; s2.scrollIntoView({ block: 'start' }); } });
      $$('[data-mmcopy]', list).forEach((b) => b.onclick = () => startUpload());
    };
    const load = async (more) => {
      const gen = ++st.gen;
      // A node before paging sent every file: page and search here, no new request
      if (st.local) { const all = st.q ? st.local.filter(match) : st.local; st.count = all.length; st.bytes = all.reduce((n, f) => n + (f.size || 0), 0); st.files = all.slice(0, more ? st.files.length + MINE_PAGE : MINE_PAGE); draw(); return; }
      if (!more) { box.setAttribute('aria-busy', 'true'); list.innerHTML = skelRows(5) + SLOW; slowNote(list, () => gen === st.gen); }
      const qs = new URLSearchParams({ offset: String(more ? st.files.length : 0), limit: String(MINE_PAGE) }); if (st.q) qs.set('q', st.q);
      let r; try { r = await api('GET', `/me/music?${qs}`); } catch (e) {
        if (gen !== st.gen || !box.isConnected) return;
        box.removeAttribute('aria-busy'); list.innerHTML = `<div class="notice warn">${esc(e.message)} <button class="btn sm" data-mmretry>Try again</button></div>`; $('[data-mmretry]', list).onclick = () => load(more); return;
      }
      if (gen !== st.gen || !box.isConnected) return;
      // Every file came (a node before paging, or one page holds them all): search and page here
      if (!more && !st.q && r.files.length >= (r.count ?? r.files.length)) { st.local = r.files; load(); return; }
      st.files = more ? st.files.concat(r.files) : r.files; st.count = r.count ?? st.files.length; st.bytes = r.bytes ?? st.files.reduce((n, f) => n + (f.size || 0), 0);
      draw();
    };
    let qt = null;
    $('#mmq', box).value = st.q;
    $('#mmq', box).oninput = (e) => { clearTimeout(qt); qt = setTimeout(() => { st.q = e.target.value.trim(); load(); }, 300); };
    if (kept && st.loaded) draw(); else load();
  };
  // "Bring files here": how much first (and what is here already), where to, then a job.
  const bringFiles = async () => {
    let plan; try { plan = await api('POST', '/pubky/files/mirror', { dry_run: true }); } catch (e) { return toast(e.message, 'error'); }
    const broken = plan.broken ? ` ${countOf(plan.broken, 'files')} on the homeserver are empty or a second copy; they are left out (remove them below).` : '';
    const unchecked = plan.unchecked ? ` ${countOf(plan.unchecked, 'files')} couldn't be checked (the homeserver didn't answer in time); they are left out this time, try again later.` : '';
    if (!plan.total) return toast(`Nothing to bring: ${countOf(plan.skipped, 'files')} already on this node.${broken}${unchecked}`);
    const go = await choiceDialog({
      title: 'Bring files here',
      text: `${countOf(plan.total, 'files')} (${fmtBytes(plan.bytes_total)}) to bring${plan.skipped ? `, ${plan.skipped} already here` : ''}. They go into this folder, then into your library.${broken}${unchecked}`,
      field: { label: 'Folder', value: plan.dest },
      choices: [{ label: 'Bring them', note: 'Runs on the node at your copy speed; you can leave this page.', value: true, primary: true }],
    });
    if (!go) return;
    let job; try { job = await api('POST', '/pubky/files/mirror', go.text && go.text !== plan.dest ? { dest: go.text } : {}); } catch (e) { return toast(e.message, 'error'); }
    if (job.finished) return toast(`Nothing to bring: ${countOf(job.skipped, 'files')} already here.`);
    jobs.bringing.add(job.id); watchJob(job.id);
  };
  // A running files job (copy or bring) is a compact chip in the player bar; tapping it shows the
  // details with Pause, Cancel and the errors. It follows the node's `jobs` events, and asks
  // `/pubky/files/jobs` every 2 s while a job runs and no event came (a node before the events).
  // After a reload or a restart the chip finds the job, or the last one's summary, by itself.
  const jobs = { cur: null, queued: [], last: null, eventAt: 0, timer: null, showErrors: false, watch: null, hideAt: null, lookFails: 0, bringing: new Set() };
  const fjob = { get p() { return jobs.cur || jobs.last; } };
  const jobDone = (p) => !!(p && (p.finished || ['finished', 'cancelled', 'failed'].includes(p.state)));
  // A job that ended before its last file: cancelled, or stopped by the node (`stopped_reason`:
  // the space it keeps free running out, failures in a row), never shown as done
  const jobCancelled = (p) => !!(p.cancelled || p.state === 'cancelled');
  const jobStopped = (p) => jobDone(p) && (jobCancelled(p) || p.state === 'failed' || !!p.stopped_reason || !!p.full);
  const jobStopWhy = (p) => p.stopped_reason || (p.state === 'failed' && p.error) || (p.full ? 'Your homeserver is full.' : jobCancelled(p) ? 'Cancelled.' : p.state === 'failed' ? 'It stopped on an error.' : '');
  const jobStopShort = (p) => (jobCancelled(p) && !p.stopped_reason ? 'Cancelled' : /space|full|reserve|room/i.test(jobStopWhy(p)) ? 'Stopped: full' : /in a row/i.test(jobStopWhy(p)) ? 'Stopped: failures' : 'Stopped');
  const jobTitle = (p) => (p.kind === 'import' ? (jobDone(p) ? (jobStopped(p) ? 'Copy stopped' : 'Copied into your library') : 'Copying into your library') : p.kind === 'bring'
    ? (jobDone(p) ? (jobStopped(p) ? 'Stopped bringing files' : 'Brought files here') : 'Bringing files here')
    : p.state === 'queued' ? 'Waiting to copy' : p.state === 'paused' ? 'Copy paused'
      : (jobDone(p) ? (jobStopped(p) ? 'Copy stopped' : 'Copied to your homeserver') : 'Copying to your homeserver'));
  const jobPct = (p) => (p.bytes_total ? Math.min(100, Math.round((100 * p.bytes_done) / p.bytes_total)) : (jobDone(p) ? 100 : 0));
  // A panel closed with the keyboard (the zone picker, the Sound panel, the volume, the copy's popup)
  // gives the focus back to the button that opened it when the focus was in it, or on nothing: not lost
  // to the page's top (then Tab started over, and Help, opened with ?, gave it back to nothing). Read
  // before it hides; focus elsewhere stays. After a mouse it stays as it was, as a ⋯ menu's does, so
  // Space still plays or pauses (on the button it would open the panel again). (A button no longer
  // shown takes none: the copy's chip, gone with its copy)
  const focusWasIn = (panel) => { const a = document.activeElement; return !!panel && keyedLast && (panel.contains(a) || !a || a === document.body); };
  const focusBack = (had, btn) => { if (had && btn && btn.isConnected && btn.getClientRects().length) btn.focus({ preventScroll: true }); };
  const jobPop = () => $('#jobpop');
  const closeJobPop = (fromBack) => { const jp = jobPop(); if (!jp || jp.hidden) return; const had = focusWasIn(jp); jp.hidden = true; $('#scrim').hidden = true; if (fromBack !== true) popTransient(); focusBack(had, $('#jobchip')); };
  const hideJobChip = () => { const c = $('#jobchip'); if (c) c.hidden = true; };
  // An error line, as the node sends it: text, or {path, reason}
  const jobErr = (e) => (typeof e === 'string' ? esc(e) : `<span class="jeprob">${esc(e.reason || '')}</span> <span class="muted">${esc(e.path || '')}</span>`);
  // The card in the popover and on Downloads
  const jobCard = (p) => {
    const done = jobDone(p); const files = p.done + (p.failed || 0);
    const errs = p.errors || []; const nErr = p.errors_count ?? errs.length;
    const facts = done
      ? [`${countOf(p.done, 'files')} copied`, p.skipped ? `${p.skipped} already ${p.kind === 'bring' ? 'here' : 'there'}` : '', p.not_audio ? `${p.not_audio} not audio` : '', p.failed ? `${p.failed} failed` : ''].filter(Boolean).join(' · ')
      : `${files} of ${countOf(p.total, 'files')} · ${fmtBytes(p.bytes_done)} of ${fmtBytes(p.bytes_total)}${p.skipped ? ` · ${p.skipped} already ${p.kind === 'bring' ? 'here' : 'there'}` : ''}${p.failed ? ` · ${p.failed} failed` : ''}`;
    const pace = !done && p.state !== 'paused' && p.state !== 'queued' && p.speed_bps ? `${fmtRate(p.speed_bps)}${p.eta_s ? ` · ${fmtLeft(p.eta_s)}` : ''}` : '';
    const why = done && jobStopped(p) ? jobStopWhy(p) : p.stopped_reason || '';
    const canAct = p.kind !== 'bring' && p.kind !== 'import' && p.state;
    const btns = done
      ? `${(p.failed_retryable ?? p.failed) && canAct ? '<button class="btn" data-jact="retry">Retry failed</button>' : ''}${canAct ? '<button class="btn ghost" data-jact="dismiss">Dismiss</button>' : ''}<button class="btn ghost" data-jobclose>Close</button>`
      : `${canAct ? (p.state === 'paused' ? '<button class="btn" data-jact="resume">Resume</button>' : p.state === 'queued' ? '' : '<button class="btn" data-jact="pause">Pause</button>') : ''}<button class="btn ghost" data-jact="cancel">Cancel</button><button class="btn ghost" data-jobclose>Hide</button>`;
    return `<div class="pop-title">${esc(jobTitle(p))}</div><div class="jobpop-body">`
      + `<div class="jobbar"><span style="width:${jobPct(p)}%"></span></div>`
      + `<p class="small">${facts}</p>`
      + (pace ? `<p class="muted small jobpace">${pace}</p>` : '')
      + (p.kind === 'bring' && p.dest ? `<p class="muted small">Into ${esc(p.dest)}, then added to your library.</p>` : '')
      + (!done && (p.current_folder || p.current) ? `<p class="muted small jobcur">${esc(p.current_folder || p.current)}</p>` : '')
      + (why ? `<p class="small warn-text jobwhy" role="status">${done && p.total > files + (p.skipped || 0) + (p.not_audio || 0) ? `Stopped after ${fmtCount(files)} of ${countOf(p.total, 'files')}: ` : ''}${esc(why)}</p>` : '')
      + (nErr ? `<button class="lnk small" type="button" data-jerrs aria-expanded="${jobs.showErrors}">${countOf(nErr, 'errors')}</button>${jobs.showErrors ? `<ul class="small joberr">${errs.map((e) => `<li>${jobErr(e)}</li>`).join('')}</ul>` : ''}` : '')
      + (jobs.queued.length ? `<p class="muted small">Then: ${jobs.queued.map((q) => esc(q.label || countOf(q.total || 0, 'files'))).join(', ')}</p>` : '')
      + `<div class="row-btns">${btns}</div></div>`;
  };
  const jobAct = async (act, btn) => {
    const p = fjob.p; if (!p) return; if (btn) btn.disabled = true;
    try {
      if (act === 'dismiss') { await api('DELETE', '/pubky/files/jobs/last'); jobs.last = null; }
      else if (act === 'cancel') await api('POST', `/pubky/files/jobs/${p.id}/cancel`);
      else await api('POST', `/pubky/files/upload/${p.id}/${act}`);
    } catch (e) { toast(e.message, 'error'); if (btn) btn.disabled = false; }
    refreshJobs();
  };
  const bindJobCard = (root) => {
    $$('[data-jact]', root).forEach((b) => b.onclick = () => jobAct(b.dataset.jact, b));
    const cl = $('[data-jobclose]', root); if (cl) cl.onclick = () => { closeJobPop(); if (jobDone(fjob.p) && !jobs.cur) hideJobChip(); };
    const er = $('[data-jerrs]', root); if (er) er.onclick = () => { jobs.showErrors = !jobs.showErrors; drawJobs(); };
  };
  const drawJobPop = () => { const jp = jobPop(); const p = fjob.p; if (!jp || !p) return; jp.innerHTML = jobCard(p); bindJobCard(jp); };
  const drawJobChip = () => {
    const c = $('#jobchip'); const p = fjob.p; if (!c) return;
    if (!p) { hideJobChip(); return; }
    c.hidden = false;
    const done = jobDone(p);
    const stopped = jobStopped(p);
    c.classList.toggle('done', done && !stopped); c.classList.toggle('stopped', stopped);
    c.innerHTML = `${icon(p.kind === 'bring' ? 'download' : 'upload')}<span class="jc-text">${stopped ? jobStopShort(p) : done ? (p.failed ? `${p.failed} failed` : 'Done') : p.state === 'paused' ? 'Paused' : p.state === 'queued' ? 'Waiting' : `${p.done + (p.failed || 0)}/${p.total}`}</span><span class="jc-bar"><span style="width:${jobPct(p)}%"></span></span>`;
    const tl = `${jobTitle(p)}: ${fmtCount(p.done + (p.failed || 0))} of ${countOf(p.total, 'files')}, ${fmtBytes(p.bytes_done)} of ${fmtBytes(p.bytes_total)}${stopped ? `. ${jobStopWhy(p)}` : ''}`;
    c.title = tl; c.setAttribute('aria-label', tl);
  };
  const drawJobs = () => {
    drawJobChip();
    // Dismissed, or the job is gone: the popover has nothing left to show
    if (!jobPop().hidden) { if (fjob.p) drawJobPop(); else closeJobPop(); }
    // On Downloads the card is part of the page: no Hide or Close (they belong to the popover)
    const dj = $('#dljob'); if (dj) { const p = fjob.p; dj.innerHTML = p ? `<div class="card jobcard">${jobCard(p)}</div>` : ''; $$('[data-jobclose]', dj).forEach((b) => b.remove()); bindJobCard(dj); }
  };
  // What `/jobs` (or an event) says, in either shape: `{current, queued, last}`, or one job
  const onJobs = async (j) => {
    if (j && ('current' in j || 'queued' in j || 'last' in j)) { jobs.cur = j.current || null; jobs.queued = j.queued || []; if ('last' in j) jobs.last = j.last || null; }
    else if (j && j.id) { if (jobDone(j)) { jobs.last = j; jobs.cur = null; } else jobs.cur = j; }
    else jobs.cur = null;
    if (jobs.cur) jobs.watch = jobs.cur.id;
    // The job this page followed is gone from the list (a node that lists running jobs only, or
    // one that finished before the first look): its own answer is the result
    if (!jobs.cur && jobs.watch && !(jobs.last && jobs.last.id === jobs.watch)) {
      const id = jobs.watch; jobs.watch = null;
      // A failed look keeps following it (a few times); only a node that no longer knows the job ends it
      try { const p = await api('GET', `/pubky/files/jobs/${id}`); jobs.lookFails = 0; if (p && p.id) jobs.last = p; }
      catch (e) { if (e.status !== 404 && (jobs.lookFails = (jobs.lookFails || 0) + 1) <= 5) { jobs.watch = id; clearTimeout(jobs.timer); jobs.timer = setTimeout(refreshJobs, 4000); drawJobs(); return; } jobs.lookFails = 0; }
    }
    if (jobs.cur == null && jobs.last && jobs.last.id === jobs.watch) jobs.watch = null;
    // A bring this page saw running says how it ended, once, whichever way the answer came
    if (jobs.cur && jobs.cur.kind === 'bring' && !jobDone(jobs.cur)) jobs.bringing.add(jobs.cur.id);
    const ended = jobs.last && jobs.last.kind === 'bring' && jobDone(jobs.last) && jobs.bringing.delete(jobs.last.id) ? jobs.last : null;
    if (ended && jobStopped(ended)) toast(`Stopped bringing files: ${jobStopWhy(ended)}`, 'error'); else if (ended && ended.done) toast(`Brought ${countOf(ended.done, 'files')} here; they are being added to your library.`);
    drawJobs(); pollJobs();
    // A node without a kept summary (old jobs have no `state`): the chip goes after 8 s, as before
    clearTimeout(jobs.hideAt);
    if (!jobs.cur && jobs.last && !jobs.last.state) jobs.hideAt = setTimeout(() => { if (jobPop().hidden && !jobs.cur) { jobs.last = null; drawJobs(); } }, 8000);
  };
  const pollJobs = () => {
    clearTimeout(jobs.timer);
    // While one runs: every 2 s, unless the node's events came lately
    if (jobs.cur && !jobDone(jobs.cur)) jobs.timer = setTimeout(() => { if (Date.now() - jobs.eventAt > 5000) refreshJobs(); else pollJobs(); }, 2000);
  };
  // A failed look while a job runs is tried again (the chip must not freeze on one bad answer)
  const refreshJobs = async () => { try { onJobs(await api('GET', '/pubky/files/jobs')); } catch { if ((jobs.cur && !jobDone(jobs.cur)) || jobs.watch) { clearTimeout(jobs.timer); jobs.timer = setTimeout(refreshJobs, 4000); } } };
  const watchJob = (id) => { if (id) jobs.watch = id; refreshJobs(); };
  // A job already running (another tab started it, or the node carried on after a restart)
  const findJob = refreshJobs;
  // External resources that do not answer (Pinakes, the homeserver, a service), or that need a
  // change (the API token): one quiet line under
  // the node status, only when something is wrong; the library keeps playing without them.
  // Help "?" in the top bar: Report a problem and My reports (feedback.js), for the owner and
  // listeners, mounted once the node answers. The version comes from the node, once.
  let helpMounted = false;
  const mountHelp = () => {
    if (helpMounted || !window.PM_FEEDBACK || !state.token) return;
    helpMounted = true;
    let version = '';
    api('GET', '/info').then((i) => { version = (i && i.version) || ''; }).catch(() => {});
    PM_FEEDBACK.mount($('#help'), { token: () => state.token, view: () => state.view, shell: shellMode, version: () => version });
  };
  let healthTimer = null;
  const checkHealth = async () => {
    clearTimeout(healthTimer); healthTimer = setTimeout(checkHealth, 60000);
    let parts; try { parts = await api('GET', '/health'); } catch { return; }
    const el = $('#upstreams'); if (!el) return;
    // The owner's chores (sign in again, change the API token) show to whoever can change the node's
    // settings: anyone else has nowhere to do them
    const chores = can('settings'); const chore = (p) => p.state === 'needs_sign_in' || p.state === 'change';
    const bad = parts.filter((p) => p.state !== 'ok' && (chores || !chore(p)));
    el.hidden = !bad.length; if (!bad.length) return;
    const first = bad[0];
    // Each state in its own words: "not answering" only for one that doesn't answer
    $('.up-text', el).textContent = (first.state === 'needs_sign_in' ? (first.id === 'homeserver' ? `${first.name}: log out and in to approve new folders` : `${first.name}: sign in again`) : first.state === 'change' ? (first.id === 'api-token' ? 'Change the API token' : `${first.name} needs a change`) : `${first.name} not answering`) + (bad.length > 1 ? ` · ${bad.length - 1} more` : '');
    el.title = bad.map((p) => `${p.name}: ${p.detail || p.state}`).join('\n') + '\nYour library keeps playing.';
    el.onclick = () => showMenu(el, [{ title: 'External resources' }, ...parts.map((p) => ({ label: p.name, note: p.state === 'ok' ? `answering${p.ms != null ? ` · ${p.ms} ms` : ''}` : p.detail || p.state, icon: p.state === 'ok' ? 'check' : 'close', disabled: !chores || !chore(p), run: () => { if (p.state === 'change') { setView('settings', 'people'); return; } state.autoResign = true; setView('settings', 'accounts'); } })), { label: 'Check again', icon: 'shuffle', run: () => checkHealth() }]);
  };
  // Clickable rows that are not buttons or links are made reachable for the keyboard, also when
  // they arrive after a page first draws (Home's sections, search results).
  const UNTAGGED = '.clickable:not(button):not(a):not([tabindex])';
  const keyable = (el) => { el.tabIndex = 0; if (!el.getAttribute('role')) el.setAttribute('role', 'button'); };
  {
    let queued = false;
    const tag = () => { queued = false; document.querySelectorAll(UNTAGGED).forEach(keyable); };
    // (what was just added is made reachable in the same step, before a key press or anything else
    // can come: a frame later, Tab pressed in that frame went past a row that had just been drawn.
    // The whole page is still looked over once a frame, for a row that became clickable in place)
    const tagIn = (n) => { if (n.nodeType !== 1) return; if (n.matches(UNTAGGED)) keyable(n); n.querySelectorAll(UNTAGGED).forEach(keyable); };
    new MutationObserver((recs) => { for (const r of recs) r.addedNodes.forEach(tagIn); if (!queued) { queued = true; requestAnimationFrame(tag); } }).observe(document.body, { childList: true, subtree: true });
  }

  // ---- Library › Add music (STEWARD-BUNDLED-HOMESERVER.md §7): upload files, copy a folder, or use a folder in place ----
  // Uploads go one file at a time in 4 MiB chunks; a file's upload id is kept in this browser so a
  // reload or a dropped connection carries on where it stopped.
  const AUDIO_EXT = /\.(flac|mp3|m4a|mp4|aac|alac|ogg|oga|opus|wav|aiff?|wv|ape|dsf|dff|mka|wma)$/i;
  const up = { files: [], running: false };
  const upKey = (f) => `pm.up.${f.name}|${f.size}|${f.lastModified}`;
  const upGet = (k) => { try { return localStorage.getItem(k); } catch { return null; } };
  const upSet = (k, v) => { try { if (v == null) localStorage.removeItem(k); else localStorage.setItem(k, v); } catch { /* no storage: no resume */ } };
  const upTotals = () => { const t = { n: up.files.length, done: 0, failed: 0, bytes: 0, sent: 0 }; for (const u of up.files) { t.bytes += u.file.size; t.sent += u.done; if (u.state === 'stored' || u.state === 'already') t.done += 1; if (u.state === 'failed') t.failed += 1; } return t; };
  const drawUploads = () => {
    const box = $('#upbox'); if (!box || !up.files.length) { if (box) box.innerHTML = ''; return; }
    const t = upTotals(); const pct = t.bytes ? Math.round((100 * t.sent) / t.bytes) : 100;
    const word = { waiting: 'waiting', sending: 'sending', stored: 'added', already: 'already there', failed: 'failed' };
    const recent = up.files.slice(-30).reverse();
    box.innerHTML = `<div class="jobbar"><span style="width:${pct}%"></span></div><p class="small" role="status">${up.running ? 'Uploading' : 'Uploaded'} ${fmtCount(t.done)} of ${countOf(t.n, 'files')} · ${fmtBytes(t.sent)} of ${fmtBytes(t.bytes)}${t.failed ? ` · ${fmtCount(t.failed)} failed` : ''}</p>`
      + `<ul class="uplist">${recent.map((u) => `<li><span class="grow">${esc(u.rel || u.file.name)}</span><span class="muted small ${u.state === 'failed' ? 'joberr' : ''}" title="${esc(u.error || '')}">${u.state === 'sending' ? `${Math.round((100 * u.done) / (u.file.size || 1))}%` : word[u.state]}</span></li>`).join('')}</ul>`
      + (up.files.length > 30 ? `<p class="muted small">and ${countOf(up.files.length - 30, 'more files')}</p>` : '')
      + (!up.running ? '<button class="btn ghost sm" id="upclear">Clear the list</button>' : '');
    const c = $('#upclear', box); if (c) c.onclick = () => { up.files = []; drawUploads(); };
  };
  const sendOne = async (u) => {
    const f = u.file; const key = upKey(f);
    let id = upGet(key); let offset = 0; let chunk = 4 * 1024 * 1024;
    if (id) { try { const st = await api('GET', `/library/uploads/${id}`); if (st.state === 'stored') { u.done = f.size; u.state = 'stored'; upSet(key, null); return; } if (st.state === 'receiving') offset = st.offset || 0; else id = null; } catch { id = null; } }
    if (!id) {
      const r = await api('POST', '/library/uploads', { name: f.name, size: f.size, relative_path: u.rel || undefined });
      if (r.already) { u.done = f.size; u.state = 'already'; return; }
      id = r.id; offset = r.offset || 0; chunk = r.chunk_size || chunk; upSet(key, id);
    }
    for (;;) {
      const res = await fetch(`/v1/library/uploads/${encodeURIComponent(id)}?offset=${offset}`, { method: 'PUT', headers: { 'content-type': 'application/octet-stream', ...(state.token ? { authorization: 'Bearer ' + state.token } : {}) }, body: f.slice(offset, offset + chunk) });
      const j = await res.json().catch(() => ({}));
      if (res.status === 409 && j.offset != null) { offset = j.offset; continue; }
      if (!res.ok) throw new Error(humanError(j.error || `the node answered ${res.status}`, '/library/uploads'));
      offset = j.offset; u.done = Math.min(offset, f.size); drawUploads();
      if (j.state === 'failed') throw new Error(j.error || 'the homeserver did not take it');
      if (j.state === 'stored' || j.state === 'storing' || offset >= f.size) break;
    }
    u.state = 'stored'; upSet(key, null);
  };
  const runUploads = async () => {
    if (up.running) return; up.running = true; drawUploads();
    for (;;) {
      const u = up.files.find((x) => x.state === 'waiting'); if (!u) break;
      u.state = 'sending'; drawUploads();
      try { await sendOne(u); } catch (e) {
        u.state = 'failed'; u.error = e.message;
        // Not enough space on the machine: the rest waits for the person, not for space to free up
        if (e.status === 507) { up.files.filter((x) => x.state === 'waiting').forEach((x) => { x.state = 'failed'; x.error = e.message; }); }
      }
      drawUploads();
    }
    up.running = false; drawUploads();
    if (phoneMode() && state.view === 'downloads') render();
    const t = upTotals(); toast(t.failed ? `${countOf(t.done, 'files')} added, ${fmtCount(t.failed)} failed` : `${countOf(t.done, 'files')} added to your library`, t.failed ? 'error' : undefined);
  };
  const addUploads = (files) => {
    const list = [...files].filter((f) => f.size > 0 && AUDIO_EXT.test(f.name));
    const skipped = files.length - list.length;
    for (const f of list) if (!up.files.some((x) => x.file.name === f.name && x.file.size === f.size && x.rel === (f.webkitRelativePath || f.relPath || '') && x.state !== 'failed')) up.files.push({ file: f, rel: f.webkitRelativePath || f.relPath || '', done: 0, state: 'waiting', error: null });
    if (skipped) toast(`${countOf(skipped, 'files')} left out: only music files are added`);
    if (list.length) runUploads(); else drawUploads();
  };
  // Dropped folders: walk them (webkitGetAsEntry), keeping each file's path inside the folder
  const filesFromDrop = async (dt) => {
    const entries = [...(dt.items || [])].map((it) => it.webkitGetAsEntry && it.webkitGetAsEntry()).filter(Boolean);
    if (!entries.length) return [...(dt.files || [])];
    const out = [];
    const walk = async (e, path) => {
      if (e.isFile) { const f = await new Promise((res, rej) => e.file(res, rej)).catch(() => null); if (f) { f.relPath = path + f.name; out.push(f); } return; }
      if (!e.isDirectory) return;
      const rd = e.createReader(); let batch;
      do { batch = await new Promise((res) => rd.readEntries(res, () => res([]))); for (const c of batch) await walk(c, `${path}${e.name}/`); } while (batch.length);
    };
    for (const e of entries) await walk(e, '');
    return out;
  };
  window.addEventListener('beforeunload', (e) => { if (up.running) { e.preventDefault(); e.returnValue = ''; } });

  // ---- Your library (owner, 2026-10-29; TASK-pinakes-social-ux G) ----
  // One place for the music: My music (what is on your homeserver, as songs, with saving and
  // tagging) and, for the node's owner, Node library (the folders, shares and drives the node plays
  // in place, scans, edits). One "Add music" button. Settings keeps settings only.
  // Settings-style rows, the same look as Settings
  const sHelp = (h) => (h ? `<p class="set-help">${h}</p>` : '');
  const sRow = (label, h, ctl, cls = '') => `<div class="set-row ${cls}"><div class="set-label"><div class="set-name">${label}</div>${sHelp(h)}</div>${ctl ? `<div class="set-ctl">${ctl}</div>` : ''}</div>`;
  const sBlock = (label, h, body) => `<div class="set-row set-block">${label || h ? `<div class="set-label">${label ? `<div class="set-name">${label}</div>` : ''}${sHelp(h)}</div>` : ''}<div class="set-body">${body}</div></div>`;
  const sGroup = (title, body) => `<div class="set-group">${title ? `<h3 class="set-h">${title}</h3>` : ''}<div class="set-rows">${body}</div></div>`;
  // The folders, shares and drives the node plays in place (was Settings › Library folders)
  const nodeLibraryHtml = ({ info, folders, ns, shares, drv }) => {
    if (driveFailed && drv.providers.some((p) => p.id === driveFailed.provider && p.connected)) driveFailed = null; // (signed in since: the old failure is over)
    const row = sRow; const block = sBlock; const group = sGroup;
      const scanErrs = info.last_scan && info.last_scan.error_count ? `<details class="scanerrs small" id="scanerrs"><summary>${countOf(Number(info.last_scan.error_count).toLocaleString(), 'files')} could not be read in the last scan</summary><ul>${info.last_scan.errors.map(([fp, why]) => `<li><code>${esc(fp.split('/').slice(-2).join('/'))}</code>: ${esc(why.replace(fp, '').replace(/^[: ]+/, '') || 'unreadable')}</li>`).join('')}</ul>${info.last_scan.errors.length < info.last_scan.error_count ? `<p class="scanmore">${info.last_scan.errors.length.toLocaleString()} of ${Number(info.last_scan.error_count).toLocaleString()} are listed here; the node’s log has the rest.</p>` : ''}</details>` : ''; // (the node keeps 20 files, and every folder it couldn't read)
      const library = group('Music folders',
        (folders.length ? folders.map(([fp, present]) => { const fromConfig = (ns.config_folders || []).includes(fp); return row(`<code class="path">${esc(fp)}</code>`, [present ? '' : 'Not found on this computer.', fromConfig ? 'From the node’s config (<code>node.toml</code>, or <code>PM_FOLDERS</code> in Docker), read at every start. To remove it, take it out there and restart the node; then Remove here forgets its songs.' : ''].filter(Boolean).join(' '), fromConfig ? '<span class="pill">From config</span>' : `<button class="btn sm ghost" data-folder-del="${esc(fp)}">Remove</button>`); }).join('') : row('No folders yet', 'Add the folder with your music below.', ''))
        + block('Add a folder', 'A folder on the node. It is read now and watched for new files; your files stay as they are.', `<div class="toolbar"><input class="input" id="folderadd" placeholder="/path/to/your/Music" aria-label="Folder to add"><button class="btn primary" id="folderbtn">${icon('plus')} Add folder</button></div>${scanErrs}`)
        + row('Check for changes', 'Reads every folder again; unchanged files are skipped.', '<button class="btn" id="scan">Scan now</button>'))
        + group('WebDAV shares', shares.map((sh) => row(esc(sh.name), `${esc(sh.url)} · ${countOf(sh.tracks, 'songs')}`, `<button class="btn sm" data-wd-scan="${esc(sh.id)}">Rescan</button><button class="btn sm ghost" data-wd-del="${esc(sh.id)}">Remove</button>`)).join('')
          + row('Add a WebDAV share', 'A music folder on Nextcloud, ownCloud or a NAS, played where it is (it is read again every 6 hours).', `<button class="btn" id="wdopen">${icon('plus')} Add share</button>`))
        + group('Cloud drives', drv.roots.map((r) => row(esc(r.name), `${r.provider === 'google' ? 'Google Drive' : 'OneDrive'} · ${countOf(r.tracks, 'songs')}`, `<button class="btn sm" data-dr-scan="${esc(r.id)}">Rescan</button><button class="btn sm ghost" data-dr-del="${esc(r.id)}">Remove</button>`)).join('')
          + drv.providers.map((p) => row(esc(p.name), !p.configured ? 'Not set up on this node: add its app in Settings › This node › Service apps' : p.connected ? `Signed in${p.user ? ` as ${esc(p.user)}` : ''}` : 'Sign in, then pick a music folder; it is read again every 6 hours.' + (driveFailed && driveFailed.provider === p.id ? '<br>' + driveFailedHtml(driveFailed.why) : ''), !p.configured ? '' : p.connected ? `<button class="btn" data-dr-add="${esc(p.id)}" data-dr-name="${esc(p.name)}">${icon('plus')} Add a folder</button><button class="btn sm ghost" data-dr-out="${esc(p.id)}">Sign out</button>` : `<button class="btn" data-dr-login="${esc(p.id)}">Sign in</button>`)).join(''))
        + group('Fix my files\u2019 tags', block('', 'Write what the node knows into your files, so other players see it too: each artist on their own (not one artist called \u201cSefa, Dr. Peacock\u201d), and the MusicBrainz match of each song. You see every change first; each file\u2019s tags are saved before it changes, and the music itself is never touched.', '<div class="actions"><button class="btn" id="fixcheck">Check my files</button></div><div id="fixout" aria-live="polite"></div>'))
        + group('Identify', block('Match songs to MusicBrainz', 'Gives every file an identity that collections, tags and other apps understand: IDs in the tags first, then the folder as an album (one MusicBrainz lookup for a whole album), then how it sounds (AcoustID) for the rest. Unsure matches, and songs where two ways disagree, wait for you.', '<div id="idcard"><div id="idbox" class="muted small">Loading…</div></div>'));
    return library;
  };
  const bindNodeLibrary = (idSt) => {
      // Fix my files' tags: the list first, then the write
      const loadFixes = async () => {
        const out = $('#fixout'); if (!out) return;
        out.innerHTML = '<div class="skeleton" style="height:60px;margin-top:10px"></div>';
        let r; try { r = await api('GET', '/library/tag-fixes?limit=50'); } catch (e) { out.innerHTML = `<p class="small warn-text">${esc(e.message)}</p>`; return; }
        const label = (k) => ({ ARTISTS: 'Artists', 'MUSICBRAINZ_TRACKID': 'MusicBrainz song', 'MusicBrainz Track Id': 'MusicBrainz song', 'MUSICBRAINZ_ALBUMID': 'MusicBrainz album', 'MusicBrainz Album Id': 'MusicBrainz album', 'MUSICBRAINZ_ARTISTID': 'MusicBrainz artists', 'MusicBrainz Artist Id': 'MusicBrainz artists', 'ACOUSTID_ID': 'AcoustID', 'Acoustid Id': 'AcoustID' }[k] || k);
        if (!r.total) { out.innerHTML = `<p class="small muted" style="margin-top:10px">Your files already say everything the node knows.${r.cannot ? ` (${countOf(r.cannot, 'files')} in a format the writer leaves alone.)` : ''}</p>`; return; }
        out.innerHTML = `<p class="small" style="margin-top:10px">${countOf(r.total, 'files')} would change${r.total > r.files.length ? ` (the first ${r.files.length} below)` : ''}${r.cannot ? `; ${countOf(r.cannot, 'files')} in a format the writer leaves alone` : ''}.</p>
          <div class="set-list fixlist">${r.files.map((f) => `<div class="set-item"><div class="grow"><div class="name">${esc(f.title)}</div><div class="sub">${f.changes.map((c) => `${esc(label(c.key))}: ${c.now.length ? `<s>${esc(c.now.join(' · '))}</s> → ` : ''}${esc(c.then.join(' · '))}`).join('<br>')}</div></div></div>`).join('')}</div>
          <div class="actions" style="margin-top:10px"><button class="btn primary" id="fixgo">Write into ${countOf(r.total, 'files')}</button></div>`;
        $('#fixgo').onclick = async () => { const b = $('#fixgo'); b.disabled = true; b.textContent = 'Writing…'; try { const w = await api('POST', '/library/tag-fixes', {}); toast(`${countOf(w.written, 'files')} fixed${w.skipped.length ? `; ${fmtCount(w.skipped.length)} left as they were` : ''}. Backups: ${w.backup_dir}`); loadFixes(); } catch (e) { toast(e.message, 'error'); b.disabled = false; } };
      };
      const fc = $('#fixcheck'); if (fc) fc.onclick = loadFixes;
      $('#scan').onclick = async () => { try { await api('POST', '/library/scan', {}); toast('Scan started'); watchScan({ scanning: true }); } catch (e) { toast(e.message, 'error'); } };
      const addFolder = async () => { const p = $('#folderadd').value.trim(); if (!p) return; try { const r = await api('POST', '/library/folders', { path: p }); toast(r.scan_started ? 'Folder added; scanning' : 'Folder added; scanned after the current scan'); watchScan({ scanning: true }); render(); } catch (e) { toast(e.message, 'error'); } };
      $$('[data-dr-login]').forEach((b) => b.onclick = async () => { try { const r = await api('POST', `/library/drives/${b.dataset.drLogin}/login`); try { sessionStorage.setItem('pm.driveLogin', b.dataset.drLogin); } catch { /* storage off */ } location.href = r.url; } catch (e) { toast(e.message, 'error'); } });
      $$('[data-dr-add]').forEach((b) => b.onclick = async () => { const r = await driveFolderDialog(b.dataset.drAdd, b.dataset.drName); if (r) { toast(`${r.name} added; scanning`); render(); } });
      $$('[data-dr-out]').forEach((b) => b.onclick = async () => { try { await api('DELETE', `/library/drives/${b.dataset.drOut}/account`); render(); } catch (e) { toast(e.message, 'error'); } });
      $$('[data-dr-scan]').forEach((b) => b.onclick = async () => { try { await api('POST', `/library/drives/roots/${b.dataset.drScan}/scan`); toast('Reading the folder again'); } catch (e) { toast(e.message, 'error'); } });
      // A folder's, share's or drive folder's Remove asks as every removal does: the page's dialog,
      // naming it by its row, with Remove drawn as deleting (no browser confirm, no armed button).
      // Cancelled after the page drew again (every 3 s while scanning), the keyboard goes back to
      // the same Remove in the new rows.
      const askRemove = async (b, kind, stays) => {
        const n = (b.closest('.set-row') || b).querySelector('.set-name');
        const name = n ? n.textContent.trim() : '';
        const v = await choiceDialog({ title: `Remove this ${kind}?`, text: `${name ? `“${name}” and its` : 'Its'} songs leave the library. ${stays}`, choices: [{ label: 'Remove', value: true, primary: true, danger: true }] });
        if (v !== true && !b.isConnected && (!document.activeElement || document.activeElement === document.body)) {
          const a = [...b.attributes].find((x) => /^data-[a-z]+-del$/.test(x.name));
          const again = a && $(`[${a.name}="${CSS.escape(a.value)}"]`, main);
          if (again) again.focus();
        }
        return v === true;
      };
      $$('[data-dr-del]').forEach((b) => b.onclick = async () => { if (!(await askRemove(b, 'drive folder', 'The files on the drive stay.'))) return; try { const r = await api('DELETE', `/library/drives/roots/${b.dataset.drDel}`); toast(`Removed, with ${countOf(r.tracks, 'songs')}`); render(); } catch (e) { toast(e.message, 'error'); } });
      $('#wdopen').onclick = async () => { const sh = await webdavDialog(); if (sh) { toast(`${sh.name} added; scanning`); render(); } };
      $$('[data-wd-scan]').forEach((b) => b.onclick = async () => { try { await api('POST', `/library/webdav/${b.dataset.wdScan}/scan`); toast('Reading the share again'); } catch (e) { toast(e.message, 'error'); } });
      $$('[data-wd-del]').forEach((b) => b.onclick = async () => { if (!(await askRemove(b, 'share', 'The files on the share stay.'))) return; try { const r = await api('DELETE', `/library/webdav/${b.dataset.wdDel}`); toast(`Removed, with ${countOf(r.tracks, 'songs')}`); render(); } catch (e) { toast(e.message, 'error'); } });
      $('#folderbtn').onclick = addFolder; $('#folderadd').onkeydown = (e) => { if (e.key === 'Enter') $('#folderbtn').click(); };
      $$('[data-folder-del]', main).forEach((b) => b.onclick = async () => { if (!(await askRemove(b, 'folder', 'The files in it stay where they are.'))) return; try { const r = await api('DELETE', '/library/folders?path=' + encodeURIComponent(b.dataset.folderDel)); toast(`Folder removed · ${countOf(r.removed_tracks, 'songs')} gone`); render(); } catch (e) { toast(e.message, 'error'); } });
    renderIdentify(idSt);
  };
  // Choose a playlist file (Your library › Import) and Playlists' Import: one file at a time. While it
  // goes up, the button says Importing… (aria-busy and aria-disabled, so the keyboard stays on it; a
  // page drawn again meanwhile shows it too) and opens no picker; then the new playlist opens, with
  // what matched in a toast. Someone who went to another page, or opened a menu, Now playing or a
  // dialog, is typing (or left a name or link in Playlists' boxes), has the keyboard on another control
  // of the page, or started something else there meanwhile (a Move from Roon check or import, Create,
  // Open) stays where they are (the toast has Open playlist). A failure is said under the button (Your
  // library › Import) or in a toast, and the button takes a file again, the same one too.
  let plImport = null; // the file going up: { name }
  // What else is started on these pages, per page: how many times, and how many are still under way
  const pageWork = { started: {}, busy: {} };
  const pageAct = (fn) => async (...a) => { const at = location.pathname; pageWork.started[at] = (pageWork.started[at] || 0) + 1; pageWork.busy[at] = (pageWork.busy[at] || 0) + 1; try { return await fn(...a); } finally { pageWork.busy[at]--; } };
  const PL_IMPORT_LIMIT = 180000; // ms
  const PL_FILES = ['libplimport', 'plimport'];
  const plImportLook = () => PL_FILES.forEach((id) => {
    const input = document.getElementById(id); const lab = input && input.closest('label'); if (!lab) return;
    const word = [...lab.childNodes].find((n) => n.nodeType === 3 && n.textContent.trim());
    const on = !!plImport;
    if (on && lab.dataset.word == null && word) { lab.dataset.word = word.textContent; word.textContent = ' Importing…'; }
    if (!on && lab.dataset.word != null) { if (word) word.textContent = lab.dataset.word; delete lab.dataset.word; }
    lab.classList.toggle('is-busy', on); lab.classList.toggle('is-spinning', on && !!plImport.spin);
    if (on) { lab.setAttribute('aria-busy', 'true'); input.setAttribute('aria-disabled', 'true'); } else { lab.removeAttribute('aria-busy'); input.removeAttribute('aria-disabled'); }
  });
  const importPlaylistFile = async (input, outId) => {
    const f = input.files[0]; if (!f || plImport) { input.value = ''; return; }
    const from = location.pathname; const focused = document.activeElement === input; const started = pageWork.started[from] || 0;
    const out = () => document.getElementById(outId);
    const mine = plImport = { name: f.name, spin: false };
    plImportLook(); setTimeout(() => { if (plImport === mine) { mine.spin = true; plImportLook(); } }, 150);
    { const o = out(); if (o) o.innerHTML = `<span class="sr-only">Importing ${esc(f.name)}…</span>`; }
    // (a node that never answers doesn't leave the buttons busy for good: cut after PL_IMPORT_LIMIT)
    const ac = new AbortController(); const cut = setTimeout(() => ac.abort(), PL_IMPORT_LIMIT);
    try {
      const j = await (async () => {
        const r = await fetch('/v1/playlists/import/file?file=' + encodeURIComponent(f.name), { method: 'POST', signal: ac.signal, headers: { 'content-type': 'application/octet-stream', ...(state.token ? { authorization: 'Bearer ' + state.token } : {}) }, body: await f.arrayBuffer() });
        const b = await r.json().catch(() => ({})); if (!r.ok || !b.id) throw new Error(b.error || `The node answered ${r.status}`);
        libWrote(); return b;
      })().catch((e) => { throw ac.signal.aborted ? new Error('The node stopped answering, so the collection may not be in. Collections shows whether it is.') : e; });
      const said = `Imported “${j.name}”: ${j.imported} of ${j.parsed} matched${j.kept ? ` · ${countOf(j.kept, 'kept as links')}` : ''}${(j.unresolved || []).length ? ` · not matched: ${j.unresolved.slice(0, 4).map((d) => d.title).join(', ')}` : ''}`;
      api('GET', '/playlists').then((l) => { state.playlists = l; }, () => {});
      { const o = out(); if (o) o.textContent = ''; }
      // Still here, doing nothing else? Asked again after a menu's history entry is popped, just before it opens.
      const here = () => {
        const a = document.activeElement;
        const typing = a && a.matches('input:not([type="file"], [type="checkbox"], [type="radio"], [type="range"], [type="button"], [type="submit"], [type="reset"], [type="color"]), textarea, [contenteditable="true"]');
        // (the import button itself, or its copy on a page drawn again, and the body don't count)
        const other = a && a !== document.body && a.id !== input.id && main.contains(a) && a.matches('input, select, textarea, button, a[href], summary, [tabindex]:not([tabindex="-1"])');
        const left = ['plname', 'plshared'].some((id) => { const e = document.getElementById(id); return !!e && !!e.value.trim(); });
        return location.pathname === from && !transients.some((t) => t.open()) && !document.querySelector('dialog[open]') && !typing && !other && !left
          && (pageWork.started[from] || 0) === started && !pageWork.busy[from];
      };
      let go = here(); if (go && popPending) { await popPending; go = here(); }
      if (go) { setView('playlist', j.id); toast(said); } else {
        toast(said, '', { label: 'Open collection', run: () => setView('playlist', j.id) });
        // (on Playlists, the new row is in the list at once; drawn where it stands, what is typed stays)
        if (state.view === 'playlists') render();
      }
    } catch (er) {
      const o = out(); if (o && !o.classList.contains('sr-only')) o.innerHTML = `<span class="warn-text">${esc(er.message)}</span>`; else { if (o) o.textContent = ''; toast(er.message, 'error'); }
    } finally {
      clearTimeout(cut); plImport = null; plImportLook(); input.value = '';
      // (the keyboard goes back to the button, the one drawn now if the page was drawn again)
      if (focused && location.pathname === from && (document.activeElement === input || document.activeElement === document.body)) { const now = document.getElementById(input.id); if (now) now.focus(); }
    }
  };
  // The two file inputs: no picker while a file goes up (a click or Enter on the button is dropped)
  const bindPlaylistFile = (input, outId) => {
    if (!input) return;
    input.addEventListener('click', (e) => { if (plImport) e.preventDefault(); });
    input.onchange = () => importPlaylistFile(input, outId);
    plImportLook();
  };
  // Playlists from files and moving from Roon (was Settings › Import)
  const importHtml = () => {
    const block = sBlock; const group = sGroup;
      const importTab = group('Move from Roon', block('', '', `<ol class="muted small steps"><li>In Roon, open Tracks (and Albums), select all, then ⋯ › Export › Excel. Export each playlist you want the same way.</li><li>Choose the files here: they are checked right away, nothing changes yet.</li><li>Import what matched.</li></ol><label class="btn file-btn roonpick">${icon('upload')} Choose Roon export files<input type="file" id="roonfiles" accept=".xlsx,.csv" multiple></label><label class="check small"><input type="checkbox" id="roonfav" checked> Roon favourites become Bookmarks</label><label class="check small"><input type="checkbox" id="roonpl" checked> Playlist exports become collections</label><label class="check small"><input type="checkbox" id="roontags"> Roon tags become tags on Pubky <span class="muted">(public)</span></label><div class="toolbar"><button class="btn primary" id="roongo" disabled>${icon('download')} Import</button></div><div id="roonout"></div><p id="roonsay" class="sr-only" role="status"></p>`));
    return group('Playlists from files', sBlock('', 'An M3U or M3U8 list, a CSV, or a Roon playlist export: each line is matched to a song (MusicBrainz, ISRC, then title and artist); what doesn’t match stays as its line.', `<label class="btn file-btn">${icon('upload')} Choose a playlist file<input type="file" id="libplimport" accept=".m3u,.m3u8,.csv,.xlsx"></label><div id="libplout" class="small" aria-live="polite"></div>`)
      + sRow('Playlists on a service', 'TIDAL, Spotify, Deezer and the others: open the playlist on Sources and choose Import.', '<button class="btn ghost" data-view-go="sources">Open Sources</button>'))
      + importTab;
  };
  const bindImport = () => {
    const pi = $('#libplimport');
    bindPlaylistFile(pi, 'libplout');
      // Move from Roon: a dry run per file as soon as files are chosen or an option changes, then the
      // import of what matched (roon_import.rs). Only the latest check draws; during the import the
      // files and options stay as they were. The chosen files are kept here and the input emptied, so
      // choosing the same files again checks them again; after an import, only new files offer Import.
      // Import sends only the files whose latest check went through (a file whose check failed is
      // never imported unseen); after it, Check again checks just the files left out.
      // A screen reader hears #roonsay only: what started, then one line with the outcome, not
      // every "Checking 2 of 3…" or the whole list of files.
      let roonSeq = 0; let roonFiles = []; let roonOk = []; let roonDone = false;
      // (Emptied first and filled a frame later, so the same words twice are read twice.)
      let roonSaid = 0;
      const roonSay = (s, t) => { if (!s || !s.isConnected) return; const n = ++roonSaid; s.textContent = ''; if (t) requestAnimationFrame(() => { if (n === roonSaid) s.textContent = t; }); };
      const roonCtl = () => ['#roonfiles', '#roonfav', '#roonpl', '#roontags'].map((q) => $(q)).filter(Boolean);
      const roonRun = pageAct(async (dry) => {
        const files = dry ? roonFiles : roonOk; const out = $('#roonout'); const go = $('#roongo');
        // (this run's own status line and controls: a run left behind by leaving Import says nothing
        // over a newer one and frees none of its controls)
        const sayEl = $('#roonsay'); const tell = (t) => { if (out.isConnected) roonSay(sayEl, t); }; const ctl = roonCtl();
        const seq = ++roonSeq; go.disabled = true;
        if (!files.length) { out.innerHTML = ''; tell(''); return; }
        const opts = `favourites=${$('#roonfav').checked}&playlist=${$('#roonpl').checked}&tags=${$('#roontags').checked}&dry_run=${dry}`;
        // What it is doing goes on the first line; the last results stay, dimmed, so the page keeps its height
        const say = (html) => { $$('.roonfile', out).forEach((e) => e.classList.add('stale')); const s = $('[data-roonstate]', out); if (s) s.outerHTML = html; else out.insertAdjacentHTML('afterbegin', html); };
        const parts = []; const ok = []; let matched = 0; let rows = 0; let failed = false;
        const many = files.length > 1 ? ` ${fmtCount(files.length)} files` : '';
        tell(dry ? `Checking${many}…` : `Importing${many}…`);
        if (!dry) ctl.forEach((e) => { e.disabled = true; });
        try {
          for (const [i, f] of files.entries()) {
            say(`<p class="muted small" data-roonstate><span class="spin"></span> ${dry ? 'Checking' : 'Importing'}${files.length > 1 ? ` ${i + 1} of ${files.length}` : ''}…</p>`);
            try {
              const res = await fetch(`/v1/import/roon?file=${encodeURIComponent(f.name)}&${opts}`, { method: 'POST', headers: { 'content-type': 'application/octet-stream', ...(state.token ? { authorization: 'Bearer ' + state.token } : {}) }, body: await f.arrayBuffer() });
              const r = await res.json(); if (!res.ok) throw new Error(r.error || res.status);
              if (!dry) libWrote();
              const kind = { tracks: 'Tracks', albums: 'Albums', playlist: `Playlist “${r.name || f.name}”` }[r.kind] || r.kind;
              const did = [r.favourites ? `${r.favourites} bookmark${r.favourites === 1 ? '' : 's'}${dry ? ' to add' : ' added'}` : '', r.tags_added ? `${r.tags_added} tag${r.tags_added === 1 ? '' : 's'}${dry ? ' to put on Pubky' : ' put on Pubky'}` : '', r.playlist ? `collection made with ${countOf(r.playlist.imported, 'songs')}${r.playlist.kept ? ` and ${countOf(r.playlist.kept, 'links')} to match by hand` : ''}` : '', r.not_identified ? `${r.not_identified} need identifying first` : ''].filter(Boolean).join(' · ');
              const row = `<div class="roonfile"><div class="name">${esc(f.name)} <span class="muted small">${esc(kind)}</span></div><p class="small">${fmtCount(r.matched)} of ${fmtCount(r.rows)} matched this library${did ? ' · ' + esc(did) : ''}</p>`
                + (r.tags_found.length ? `<p class="muted small">Roon tags: ${r.tags_found.slice(0, 12).map(([l, c]) => `${esc(l)} (${c})`).join(', ')}${r.tags_found.length > 12 ? '…' : ''}</p>` : '')
                + (r.misses.length ? `<details class="small"><summary>${r.unmatched + r.not_identified} not moved, and why</summary><ul class="misses">${r.misses.map((m) => `<li><strong>${esc(m.what)}</strong>: ${esc(m.reason)}</li>`).join('')}</ul></details>` : '') + '</div>';
              // (counted, and offered to Import, only once its answer could be read in full)
              matched += r.matched; rows += r.rows; ok.push(f); parts.push(row);
            } catch (e) { failed = true; parts.push(`<div class="roonfile"><div class="name">${esc(f.name)}</div><p class="small joberr">${esc(e.message)}</p></div>`); }
            if (dry && seq !== roonSeq) return; // (a newer check is on its way)
          }
        } finally { if (!dry) ctl.forEach((e) => { e.disabled = false; }); }
        // (files whose check failed, not sent by Import: they stay chosen, for Check again)
        const left = dry ? 0 : roonFiles.length - files.length;
        if (dry) roonOk = ok; else if (left) roonFiles = roonFiles.filter((f) => !files.includes(f)); else roonDone = true;
        const leftWords = left === 1 ? ' The file whose check failed was left out.' : left ? ` The ${fmtCount(left)} files whose check failed were left out.` : '';
        out.innerHTML = (dry ? `<p class="muted small" data-roonstate>${failed ? 'Not every file could be checked; nothing changed yet. <button class="lnk" data-roon-again>Check again</button>' : 'Checked: nothing changed yet.'}</p>`
          : `<p class="small" data-roonstate>${matched ? 'Imported.' : 'Nothing was imported.'}${leftWords}${left ? ` <button class="lnk" data-roon-again>Check ${left === 1 ? 'it' : 'them'} again</button>` : ''}</p>`) + parts.join('');
        const again = $('[data-roon-again]', out); if (again) again.onclick = () => roonRun(true);
        go.disabled = !dry || !matched || roonDone;
        const nothing = dry && !matched && !failed;
        if (nothing) out.insertAdjacentHTML('afterbegin', '<p class="small">Nothing in these files matched this library yet: scan your music folders first (Your library › Node library), then choose the files again.</p>');
        const tally = `${fmtCount(matched)} of ${fmtCount(rows)} matched this library`;
        tell(nothing ? 'Checked: nothing in these files matched this library yet. Scan your music folders first, then choose the files again.'
          : dry ? (failed ? `Not every file could be checked${matched ? `; in the others, ${tally}` : ''}. Nothing changed yet.${matched ? ' Import is ready for those.' : ''} Check again is above the results.`
            : `Checked: ${tally}. Nothing changed yet${roonDone ? '' : '; Import is ready'}.`)
            : (!matched ? (failed ? 'Nothing was imported. The results say why.' : 'Nothing was imported: nothing in these files matched this library.')
              : !failed ? `Imported: ${tally}.` : `Imported what matched (${tally}), but not every file could be imported. The results say which.`) + leftWords + (left ? ' Check again is above the results.' : ''));
        if (!dry) { state.playlists = await api('GET', '/playlists').catch(() => state.playlists); }
      });
      $('#roonfiles').onchange = (e) => { roonFiles = [...(e.target.files || [])]; e.target.value = ''; roonDone = false; roonRun(true); };
      ['#roonfav', '#roonpl', '#roontags'].forEach((q) => { $(q).onchange = () => { if (!roonDone) roonRun(true); }; });
      $('#roongo').onclick = () => roonRun(false);
  };
  // The chosen tab's content, as the tab panel the tabs control
  const libPanel = (tab, html) => `<div id="libpanel" role="tabpanel" aria-labelledby="libtab-${tab}">${html}</div>`;
  // "Saved · 2 min ago": where saving to the homeserver stands, in a few words
  const savedWords = (sv, pk) => (!sv ? ''
    : pk && pk.sync && pk.sync.running ? `<span class="spin"></span> ${esc(pk.sync.step || 'Saving')}…`
      : sv.state === 'saving' ? `<span class="spin"></span> Saving ${countOf(sv.pending, 'changes')}…`
        : sv.state === 'error' ? `<span class="warn-text">Not saved${sv.error ? `: ${esc(sv.error)}` : ''}</span>`
          : sv.pending ? `${countOf(sv.pending, 'changes')} to save`
            : `Saved${sv.last_saved_at ? ` · ${ago(sv.last_saved_at)}` : ''}`);

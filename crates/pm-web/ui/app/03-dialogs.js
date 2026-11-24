  // Wrapped phone dialog actions can move below the scrollport on resize or text reflow.
  // Reveal only the already-focused action; ordinary redraws and manual scrolling stay put.
  let dialogActionFrame = 0, dialogActionObserver = null;
  const revealDialogAction = () => {
    if (dialogActionFrame) return;
    dialogActionFrame = requestAnimationFrame(() => {
      dialogActionFrame = 0;
      const action = document.activeElement;
      if (shellMode() !== 'phone' || !action?.matches('dialog[open] .dialog-actions .btn')) return;
      const dialog = action.closest('dialog'), b = action.getBoundingClientRect(), d = dialog.getBoundingClientRect();
      // The opening animation scales the dialog; rects use that scale, scroll offsets do not.
      const scale = d.height / dialog.offsetHeight;
      // Include the four pixels occupied by the keyboard focus outline, rounding outwards.
      const top = Math.max(0, d.top + dialog.clientTop * scale) + 4 * scale;
      const bottom = Math.min(innerHeight, d.bottom - dialog.clientTop * scale) - 4 * scale;
      const delta = b.bottom > bottom ? b.bottom - bottom : b.top < top ? b.top - top : 0;
      if (delta) dialog.scrollBy({ top: delta > 0 ? Math.ceil(delta / scale) : Math.floor(delta / scale), behavior: 'instant' });
    });
  };
  window.addEventListener('resize', revealDialogAction);
  document.addEventListener('focusout', () => { if (dialogActionObserver) dialogActionObserver.disconnect(); });
  document.addEventListener('focusin', (e) => {
    if (dialogActionObserver) dialogActionObserver.disconnect();
    if (!window.ResizeObserver || !e.target.matches('dialog[open] .dialog-actions .btn')) return;
    const dialog = e.target.closest('dialog'), footer = e.target.closest('.dialog-actions');
    const size = () => `${dialog.clientWidth}:${dialog.clientHeight}:${footer.offsetWidth}:${footer.offsetHeight}`;
    let previous = size();
    dialogActionObserver = new ResizeObserver(() => {
      const next = size();
      if (next !== previous) { previous = next; revealDialogAction(); }
    });
    dialogActionObserver.observe(dialog); dialogActionObserver.observe(footer);
  });
  // Opens a dialog that has text fields, with its first focus. On a phone the heading takes it,
  // not `field`: a phone browser would raise its keyboard over the dialog, and the Android app's
  // WebView shows the field focused with no keyboard (as Search did). A tap on a field opens the
  // keyboard. Keys pressed on that heading stay in the dialog (Space, the arrows and N don't reach
  // the app's shortcuts behind it); a character typed or pasted there goes to `field`. Desktop and tablet
  // focus `field`. (The heading is focusable before showModal, so the field never has the focus.)
  const showFieldDialog = (d, field) => {
    const h = shellMode() === 'phone' && $('h2', d);
    if (h) {
      h.tabIndex = -1;
      h.onkeydown = (e) => { e.stopPropagation(); if ((e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) || ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'v')) field.focus(); };
    }
    nameDialog(d); if (d.showModal) d.showModal(); else d.setAttribute('open', '');
    (h || field).focus();
  };
  // Edit a song's tags on the node (the file is not touched)
  // `e`: GET /library/tracks/{id}/edit: the song's edit (in the fields) and what the file says
  // (shown as each field's placeholder); an empty field is the file's value
  const editTagsDialog = (e) => new Promise((resolve) => {
    const d = document.createElement('dialog'); d.className = 'dialog';
    const f = e.file || {}; const ed = e.edit || {};
    const field = (k, label) => `<label class="field"><span>${label}</span><input class="input" data-k="${k}" value="${esc(ed[k] || '')}" placeholder="${esc(f[k] || '')}"></label>`;
    d.innerHTML = `<h2>Edit tags</h2><p class="muted small">Kept on this node; the file stays as it is. An empty field shows what the file says.</p>
      ${field('title', 'Title')}${field('artist', 'Artist')}${field('album', 'Album')}${field('album_artist', 'Album artist')}
      <div class="dialog-actions"><button class="btn ghost" data-undo>Back to the file’s tags</button><button class="btn ghost" data-close>Cancel</button><button class="btn primary" data-save>Save</button></div>`;
    document.body.appendChild(d);
    const tr = { open: () => d.open, close: (fromBack) => done(null, fromBack) };
    const done = (v, fromBack) => { const k = transients.indexOf(tr); if (k >= 0) transients.splice(k, 1); if (d.open) { if (d.close) d.close(); else d.removeAttribute('open'); } d.remove(); if (fromBack !== true) popTransient(); resolve(v); };
    transients.push(tr); pushTransient();
    $('[data-close]', d).onclick = () => done(null);
    $('[data-undo]', d).onclick = () => done({ undo: true });
    $('[data-save]', d).onclick = () => done(Object.fromEntries($$('[data-k]', d).map((i) => [i.dataset.k, i.value.trim()])));
    d.addEventListener('cancel', (e) => { e.preventDefault(); done(null); });
    showFieldDialog(d, $('[data-k]', d));
  });
  const editTrackTags = async (id) => {
    let cur; try { cur = await api('GET', `/library/tracks/${id}/edit`); } catch (e) { return toast(e.message, 'error'); }
    const v = await editTagsDialog(cur); if (!v) return;
    try {
      if (v.undo) await api('DELETE', `/library/tracks/${id}/edit`); else await api('PUT', `/library/tracks/${id}/edit`, v);
      toast(v.undo ? 'Back to the file’s tags' : 'Saved on this node'); libraryChangedNow(); if (state.view === 'library') render();
    } catch (e) { toast(e.message, 'error'); }
  };
  // (anchor: the ⋯ it was chosen from; on a search result, the focus goes on to the next result's ⋯
  // once the song leaves the list, else to the search box, not to the page's top)
  const removeFromLibrary = async (id, title, anchor) => {
    // (its file, asked while the question is open: Bring back names the file, not the song, and the
    // node has no file for it once it is out)
    const file = api('GET', `/library/tracks/${id}`).then((t) => (t && t.file && t.file.path) || null, () => null);
    const ok = await choiceDialog({ title: 'Remove from the library?', text: `“${title || 'This song'}” leaves the library; the file stays where it is and scans skip it. Your library › Node library can bring it back.`, choices: [{ label: 'Remove', value: true, primary: true, danger: true }] });
    if (!ok) return;
    try {
      await api('DELETE', `/library/tracks/${id}`); toast('Removed from the library');
      // (the kept searches' answers lose the song now: the search on screen is drawn again from its
      // kept answer, not asked again, so it would list the song until started again; withoutTrack)
      const mark = { id: Number(id), path: null, back: false, since: restoredPaths.length }; removedTracks.push(mark);
      file.then((f) => { mark.path = f; if (f && restoredPaths.slice(mark.since).includes(f)) mark.back = true; });
      for (const r of searchRuns.values()) for (const [src, a] of Object.entries(r.answers)) r.answers[src] = withoutTrack(a, src, id);
      const at = anchor ? $$('#sresults [data-smore]').indexOf(anchor) : -1;
      const focused = at >= 0 && document.activeElement === anchor;
      libraryChangedNow();
      // (waited for only then: another page's redraw can take long, or never end when left meanwhile)
      const drawn = render();
      if (focused) { await drawn; if ((!document.activeElement || document.activeElement === document.body) && state.view === 'search') ($$('#sresults [data-smore]')[at] || search).focus(); }
    } catch (e) { toast(e.message, 'error'); }
  };
  const editAlbumTags = async (albumId, title, artist) => {
    const v = await choiceDialog({ title: 'Rename the album', text: 'Kept on this node; the files stay as they are.', field: { label: 'Album title', value: title || '' }, choices: [{ label: 'Save', value: 'save', primary: true }] });
    if (!v || !v.text) return;
    try { await api('PUT', `/library/albums/${albumId}/edit`, { title: v.text, ...(artist != null ? { album_artist: artist } : {}) }); toast('Album renamed on this node'); libraryChangedNow(); render(); } catch (e) { toast(e.message, 'error'); }
  };
  // "Add music": one button, three ways in
  const addMusicDialog = async () => {
    const owner = isOwner();
    const pick = await choiceDialog({ title: 'Add music', text: 'Wherever the files are, they’re one library: albums, search and playback don’t change.', choices: [
      { label: 'Upload files', note: 'From this device to your homeserver, kept privately; a cut connection carries on where it stopped.', value: 'upload', primary: true },
      ...(owner ? [{ label: 'Add a folder or network share', note: 'Played where it is: a folder on the node’s machine, a NAS (WebDAV) or a cloud drive. Checked before it is saved.', value: 'folder' }] : []),
      ...(owner ? [{ label: 'Import collections', note: 'M3U, CSV or a Roon export; service playlists from their page in Sources.', value: 'import' }] : []),
    ] });
    if (pick === 'upload') { state.libUpload = true; if (state.view === 'library' && (state.viewArg || 'mine') === 'mine') render(); else setView('library', null); }
    if (pick === 'folder') addPlaceDialog();
    if (pick === 'import') setView('library', 'import');
  };
  const addPlaceDialog = async () => {
    const how = await choiceDialog({ title: 'Add a folder or network share', text: 'The node reads it now and plays from it; nothing is copied.', choices: [
      { label: 'A folder on the node’s machine', note: 'A disk or a mounted share (USB, NAS, SMB mounted on the machine).', value: 'folder', primary: true },
      { label: 'A WebDAV share', note: 'Nextcloud, ownCloud or a NAS over the network, with its login.', value: 'webdav' },
      { label: 'A cloud drive', note: 'Google Drive or OneDrive: sign in on Node library.', value: 'drive' },
    ] });
    if (how === 'folder') {
      const v = await choiceDialog({ title: 'Add a folder', text: 'Its path on the node’s machine. The node checks it is there before saving it.', field: { label: 'Folder', value: '' }, choices: [{ label: 'Add folder', value: 'add', primary: true }] });
      if (!v || !v.text) return;
      try { const r = await api('POST', '/library/folders', { path: v.text }); toast(r.scan_started ? 'Folder added: reading your files' : 'Folder added'); watchScan({ scanning: true }); setView('library', 'node'); } catch (e) { toast(e.message, 'error'); }
    }
    if (how === 'webdav') { const sh = await webdavDialog(); if (sh) { toast(`${sh.name} added; scanning`); setView('library', 'node'); } }
    if (how === 'drive') setView('library', 'node');
  };

  // A small dialog with a few choices; resolves to the picked value, or null (Esc, Cancel).
  // With `field` ({label, value}) it shows a text field and resolves to {value, text}.
  // A Pubky key from what someone pasted or scanned: the bare key, `pubky<key>`, `pubky://<key>/…`
  // or `pk:<key>`; null when it isn't one.
  const keyFrom = (raw) => { const k = bareKey(String(raw || '').trim()).split(/[/?#]/)[0].toLowerCase(); return /^[ybndrfg8ejkmcpqxot1uwisza345h769]{52}$/.test(k) ? k : null; };
  // Scanning a key's QR code with the camera, where the browser reads QR codes itself (no library)
  const canScan = () => 'BarcodeDetector' in window && !!(navigator.mediaDevices && navigator.mediaDevices.getUserMedia);
  const scanKey = () => new Promise((resolve) => {
    const d = document.createElement('dialog'); d.className = 'dialog scan-dialog'; d.setAttribute('aria-labelledby', 'scantitle');
    d.innerHTML = '<h2 id="scantitle">Scan their key</h2><p class="muted small">Point the camera at the QR code of their Pubky key, as Pubky Ring shows it.</p><video class="scan-video" playsinline muted></video><p class="muted small" data-scanmsg role="status"></p><div class="dialog-actions"><button class="btn" data-scanclose>Cancel</button></div>';
    document.body.appendChild(d);
    const video = $('video', d); const msg = $('[data-scanmsg]', d);
    let stream = null; let timer = null; let over = false;
    const tr = { open: () => d.open, close: (fromBack) => done(null, fromBack) };
    const done = (v, fromBack) => {
      if (over) return; over = true; clearTimeout(timer);
      if (stream) stream.getTracks().forEach((t) => t.stop());
      const k = transients.indexOf(tr); if (k >= 0) transients.splice(k, 1);
      if (d.open) { if (d.close) d.close(); else d.removeAttribute('open'); } d.remove(); if (fromBack !== true) popTransient(); resolve(v);
    };
    transients.push(tr); pushTransient();
    $('[data-scanclose]', d).onclick = () => done(null);
    d.addEventListener('cancel', (e) => { e.preventDefault(); done(null); });
    nameDialog(d); if (d.showModal) d.showModal(); else d.setAttribute('open', '');
    (async () => {
      try { stream = await navigator.mediaDevices.getUserMedia({ video: { facingMode: 'environment' } }); } catch (e) { msg.textContent = `The camera isn’t available (${e.message || e.name}). Paste their key instead.`; return; }
      if (over) { stream.getTracks().forEach((t) => t.stop()); return; }
      video.srcObject = stream; try { await video.play(); } catch { /* shows the first frame anyway */ }
      // A browser can have the API without reading QR codes: the camera goes off and it says so
      let det; try { det = new window.BarcodeDetector({ formats: ['qr_code'] }); } catch (e) { stream.getTracks().forEach((t) => t.stop()); video.srcObject = null; msg.textContent = `${hereName()} can’t read QR codes (${e.message || e.name}). Paste their key instead.`; return; }
      const look = async () => {
        if (over) return;
        try { for (const c of await det.detect(video)) { const k = keyFrom(c.rawValue); if (k) return done(k); msg.textContent = 'That QR code isn’t a Pubky key.'; } } catch { /* the next frame */ }
        timer = setTimeout(look, 250);
      };
      look();
    })();
  });
  // `layer`: opened over Now playing, it gets a history entry of its own, so Back closes it alone
  const choiceDialog = ({ title, text, choices, field, layer }) => new Promise((resolve) => {
    const d = document.createElement('dialog'); d.className = 'dialog';
    d.innerHTML = `<h2>${esc(title)}</h2>${text ? `<p class="muted">${esc(text)}</p>` : ''}${field ? `<label class="dialog-field"><span class="muted small">${esc(field.label)}</span><input class="input" data-field value="${esc(field.value || '')}" aria-label="${esc(field.label)}"></label>` : ''}<div class="choices">${choices.map((c, i) => `<button class="btn choice ${c.primary ? 'primary' : ''}${c.danger ? ' danger' : ''}" data-ci="${i}"><span class="choice-label">${esc(c.label)}</span>${c.note ? `<span class="choice-note">${esc(c.note)}</span>` : ''}</button>`).join('')}</div><div class="dialog-actions"><button class="btn ghost" data-ci="-1">Cancel</button></div>`;
    document.body.appendChild(d);
    const tr = { layer: !!layer, open: () => d.open, close: (fromBack) => done(null, fromBack) };
    const done = (v, fromBack) => { const k = transients.indexOf(tr); if (k >= 0) transients.splice(k, 1); if (d.open) { if (d.close) d.close(); else d.removeAttribute('open'); } d.remove(); if (fromBack !== true) popTransient(!!layer && transients.some((t) => t.open())); resolve(v); };
    transients.push(tr); pushTransient(!!layer);
    $$('[data-ci]', d).forEach((b) => b.onclick = () => { const i = Number(b.dataset.ci); if (i < 0) return done(null); const v = choices[i].value; done(field ? { value: v, text: $('[data-field]', d).value.trim() } : v); });
    d.addEventListener('cancel', (e) => { e.preventDefault(); done(null); });
    nameDialog(d, true); if (d.showModal) d.showModal(); else d.setAttribute('open', ''); const first = $('.choice', d); if (first) first.focus();
  });
  // Comments (TASK-pinakes-social-ux.md B1): social v1 notes whose parent is the item's link, read
  // from Nexus by parent; reply to one, delete your own. One handler per section, by delegation.
  const commentRow = (c) => `<li class="comment" data-cpost="${esc(c.post)}"><div class="comment-head"><b>${esc(c.name || `${c.author.slice(0, 8)}…`)}</b> <span class="muted small">${c.at ? esc(ago(Math.floor(c.at / 1000))) : ''}</span></div><p class="comment-text">${esc(c.content)}</p><div class="toolbar">${c.replies ? `<button class="btn ghost sm" data-creplies="${esc(c.post)}">${countOf(c.replies, 'replies')}</button>` : ''}<button class="btn ghost sm" data-creply="${esc(c.post)}">Reply</button>${c.mine ? `<button class="btn ghost sm" data-cdel="${esc(c.post)}">Delete</button>` : ''}</div><div class="comment-sub"></div></li>`;
  const commentForm = (label) => `<form class="comment-form"><textarea class="input" name="content" rows="2" maxlength="2000" placeholder="${esc(label)}" aria-label="${esc(label)}" required></textarea><button class="btn primary sm" disabled>Post</button></form>`;
  // "Post about this" (B2): a public social v1 note with the item as its embed, so any Pubky app
  // (pubky.app among them) shows it with the item's card.
  const postAbout = async (uri, title) => {
    const r = await choiceDialog({ title: 'Post about this', text: `A public post on your homeserver with ${title ? `“${title}”` : 'this'} attached; Pubky apps show it with its card.`, field: { label: 'What you want to say', value: '' }, choices: [{ label: 'Post', value: true, primary: true }] });
    if (!r || !r.value) return;
    if (!r.text) { toast('Write something to post', 'error'); return; }
    try { await api('POST', '/posts', { uri, content: r.text }); toast('Posted'); } catch (e) { toast(e.message, 'error'); }
  };
  // What someone is typing survives the page's re-renders (an item still being read renders
  // again every few seconds): kept per item, and per comment for a reply
  const commentDrafts = new Map();
  const commentTyping = () => $$('.comment-form textarea, #htags input[data-htag-add]').some((t) => t.value.trim() || t === document.activeElement);
  const mountComments = (sec) => {
    const uri = sec.dataset.curi; const body = $('.comments-body', sec);
    const draftKey = (f) => `${uri}|${(f && f.dataset.replyTo) || ''}`;
    const fill = (f, text) => { if (!f || !text) return; f.elements.content.value = text; f.querySelector('button').disabled = false; };
    const openReply = (li, post) => {
      const sub = $('.comment-sub', li); if (!$(':scope > form', sub)) { sub.insertAdjacentHTML('afterbegin', commentForm('Reply')); $(':scope > form', sub).dataset.replyTo = post; }
      return $(':scope > form', sub);
    };
    const show = async () => {
      let t; try { t = await api('GET', `/comments?uri=${encodeURIComponent(uri)}`); } catch (e) { body.innerHTML = `<p class="muted small">${esc(e.message)}</p>`; return; }
      if (!sec.isConnected) return;
      body.innerHTML = `${commentForm('Add a comment')}${t.comments.length ? `<ul class="comments">${t.comments.map(commentRow).join('')}</ul>` : `<p class="muted small">No comments yet.</p>`}<p class="muted small">A comment is a public post on your homeserver, so every Pubky app shows it.</p>`;
      fill($('form.comment-form', body), commentDrafts.get(`${uri}|`));
      for (const [k, text] of commentDrafts) {
        const [u, post] = [k.slice(0, k.lastIndexOf('|')), k.slice(k.lastIndexOf('|') + 1)];
        const li = u === uri && post ? $$('li.comment', body).find((x) => x.dataset.cpost === post) : null;
        if (li) fill(openReply(li, post), text);
      }
    };
    const replies = async (li, post) => {
      let t; try { t = await api('GET', `/comments?post=${encodeURIComponent(post)}`); } catch (e) { toast(e.message, 'error'); return; }
      $('.comment-sub', li).innerHTML = t.comments.length ? `<ul class="comments">${t.comments.map(commentRow).join('')}</ul>` : '<p class="muted small">No replies yet.</p>';
    };
    // Post wakes up once there is something to post
    sec.addEventListener('input', (e) => {
      const f = e.target.closest('form.comment-form'); if (!f) return;
      const text = f.elements.content.value; f.querySelector('button').disabled = !text.trim();
      if (text.trim()) commentDrafts.set(draftKey(f), text); else commentDrafts.delete(draftKey(f));
    });
    onBusy(sec, 'submit', async (e) => {
      const f = e.target.closest('form.comment-form'); if (!f) return; e.preventDefault();
      const content = f.elements.content.value.trim(); if (!content) return;
      const b = f.querySelector('button'); b.disabled = true;
      try {
        await api('POST', '/comments', f.dataset.replyTo ? { reply_to: f.dataset.replyTo, content } : { uri, content });
        commentDrafts.delete(draftKey(f));
        toast(f.dataset.replyTo ? 'Reply posted' : 'Comment posted');
        if (f.dataset.replyTo) await replies(f.closest('li.comment'), f.dataset.replyTo); else await show();
      } catch (err) { b.disabled = false; toast(err.message, 'error'); }
    });
    onBusy(sec, 'click', async (e) => {
      const r = e.target.closest('[data-creply]');
      if (r) { const sub = $('.comment-sub', r.closest('li.comment')); if (!$(':scope > form', sub)) openReply(r.closest('li.comment'), r.dataset.creply).elements.content.focus(); return; }
      const rp = e.target.closest('[data-creplies]'); if (rp) { await replies(rp.closest('li.comment'), rp.dataset.creplies); return; }
      const d = e.target.closest('[data-cdel]');
      if (d) {
        const go = await choiceDialog({ title: 'Delete this comment?', text: 'It goes from your homeserver; apps that already showed it drop it once Nexus sees it gone.', choices: [{ label: 'Delete', value: true, primary: true, danger: true }] });
        if (!go) return;
        d.disabled = true;
        try { await api('DELETE', `/comments?post=${encodeURIComponent(d.dataset.cdel)}`); toast('Comment deleted'); d.closest('li.comment').remove(); } catch (err) { d.disabled = false; toast(err.message, 'error'); }
      }
    });
    show();
  };
  // Importing a provider playlist asks once: keep that provider as each song's source, or Auto.
  // Imports in flight, by service and playlist: a redrawn list keeps their buttons off (no second import)
  const importing = new Set();
  const importPlaylist = async (b, service, id) => {
    const key = `${service}:${id}`; if (importing.has(key)) return;
    const name = SERVICE_NAMES[service] || service;
    let order = 'your play order'; try { const o = await api('GET', '/play-order'); order = o.order.map(srcName).join(' › '); } catch { /* keep the words */ }
    const how = await choiceDialog({ title: 'Import this playlist', text: 'Where should its songs play from?', choices: [
      { label: `Keep ${name}`, note: `Each song is pinned to its ${name} track.`, value: 'keep', primary: true },
      { label: 'Auto', note: `Each song plays from the first source that has it (${order}).`, value: 'auto' },
    ] });
    if (!how || importing.has(key)) return;
    b.disabled = true; importing.add(key);
    try { const r = await api('POST', `/sources/${service}/playlists/${encodeURIComponent(id)}/import${how === 'keep' ? '?keep_source=true' : ''}`); state.playlists = await api('GET', '/playlists'); toast(`Imported “${r.playlist.summary.name}”: ${countOf(r.imported, 'songs')}${r.kept ? `, ${countOf(r.kept, 'kept as links')}` : ''}${r.skipped ? `, ${r.skipped} skipped` : ''}${how === 'keep' ? `, pinned to ${name}` : ''}`); }
    catch (e) { toast(e.message, 'error'); }
    importing.delete(key); b.disabled = false;
  };
  // Theme: System (no attribute), or Light/Dark kept in this browser (index.html applies it early).
  const themePref = () => { try { return localStorage.getItem('pm.theme') || 'system'; } catch { return 'system'; } };
  // Two layouts (docs/ux/responsive.md; owner, 2026-11-04): phone under 600 px (bottom tabs and a
  // mini player) and desktop from 600 px (the sidebar and the full player bar), tablets included.
  // Settings › Appearance › Layout can force either: Phone at any width, Desktop from 480 px (a
  // narrower window has no room for the sidebar and the bar). The old choices map across:
  // Compact (the phone arrangement at any width) becomes Phone; Comfortable (the tablet rail from
  // 600 px, a phone below) becomes Automatic, which is now the desktop layout from 600 px.
  const LAYOUT_WAS = { compact: 'phone', comfortable: 'auto' };
  const layoutPref = () => {
    let v = 'auto';
    try { v = localStorage.getItem('pm.layout') || 'auto'; if (LAYOUT_WAS[v]) { v = LAYOUT_WAS[v]; if (v === 'auto') localStorage.removeItem('pm.layout'); else localStorage.setItem('pm.layout', v); } } catch { /* no storage */ }
    return v;
  };
  // A touch screen (a tablet in the desktop layout too): the player bar is one target that opens
  // Now playing, as on a phone; with a mouse its title and artist are links of their own
  const barPlain = () => shellMode() !== 'desktop' || !!(window.matchMedia && window.matchMedia('(pointer: coarse)').matches);
  const shellFor = (w, pref) => (pref === 'phone' || w < (pref === 'desktop' ? 480 : 600) ? 'phone' : 'desktop');
  const shellMode = () => document.documentElement.dataset.shell || 'desktop';
  const densePref = () => { try { return localStorage.getItem('pm.dense') === '1'; } catch { return false; } };
  const setTheme = (v) => { try { if (v === 'system') localStorage.removeItem('pm.theme'); else localStorage.setItem('pm.theme', v); } catch { /* no storage */ } if (v === 'system') delete document.documentElement.dataset.theme; else document.documentElement.dataset.theme = v; };
  // A slow first load says so after 2 s (UX sweep 11-01, A7): `el` gets the line while `still()`
  // is true and `el` is on the page
  const stillLoading = (el, text, still = () => true) => setTimeout(() => { if (el && el.isConnected && still() && !el.querySelector('.still-loading')) el.insertAdjacentHTML('beforeend', `<p class="muted small still-loading" role="status">${esc(text)}</p>`); }, 2000);
  // A button that needs text in a field: disabled (with the reason) until there is some.
  const needsText = (input, btn) => { if (!input || !btn) return; const sync = () => { const empty = !input.value.trim(); btn.disabled = empty; btn.title = empty ? `Type in “${input.getAttribute('aria-label') || input.placeholder || 'the field'}” first` : ''; }; input.addEventListener('input', sync); sync(); };

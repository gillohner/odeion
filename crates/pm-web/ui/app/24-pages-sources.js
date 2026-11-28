        case 'sources': {
          // One tab per service with its on/off switch, laid out like Settings (STEWARD-UX-SOURCES.md).
          // Off hides a source everywhere; its tab stays (dimmed) so it can be switched on again.
          let listed = false; let listFailed = null; try { state.sources = await api('GET', '/sources'); applySourcesOff(); listed = true; } catch (e) { listFailed = e; state.sources = state.sources || []; }
          if (state.view !== 'sources') return;
          // (no list, and none from before: said above the page, with Try again; Your library is still
          // here, but the page is not a node with no services)
          const listNote = !listed && !state.sources.length ? failedHtml({ message: `Couldn’t read the services: ${listFailed.message}` }) : '';
          const srcs = state.sources;
          const phone = !!(window.matchMedia && matchMedia('(max-width: 720px)').matches); // the side tabs' CSS is by width until the Settings step
          const known = (k) => k === 'library' || srcs.some((x) => x.id === k);
          // A service this node doesn't have (an old link or bookmark): the address is plain /sources, as
          // the page shown is, so a reload or a copied link doesn't name it (kept when the list didn't load)
          if (listed && state.viewArg && state.viewArg.source && !known(state.viewArg.source)) { state.viewArg = undefined; try { history.replaceState({ ...(history.state || {}), arg: null }, '', '/sources'); } catch { /* sandboxed */ } }
          const asked = state.viewArg && known(state.viewArg.source) ? state.viewArg.source : null;
          const active = asked || (phone ? null : (known(state.sourceId) ? state.sourceId : 'library'));
          // Another source: its sign-in opens on the recommended way again, not one opened before
          if (active && active !== state.sourceId && state.signin) Object.values(state.signin).forEach((m) => { m.way = null; m.change = false; });
          if (active) state.sourceId = active;
          const cur = active === 'library' ? { id: 'library', name: 'Your library', description: '', status: 'ready', enabled: true } : srcs.find((x) => x.id === active) || null;
          const help = (h) => (h ? `<p class="set-help">${h}</p>` : '');
          const row = (label, h, ctl, cls = '') => `<div class="set-row ${cls}"><div class="set-label"><div class="set-name">${label}</div>${help(h)}</div>${ctl ? `<div class="set-ctl">${ctl}</div>` : ''}</div>`;
          const block = (label, h, body) => `<div class="set-row set-block">${label || h ? `<div class="set-label">${label ? `<div class="set-name">${label}</div>` : ''}${help(h)}</div>` : ''}<div class="set-body">${body}</div></div>`;
          const group = (title, body) => `<div class="set-group">${title ? `<h3 class="set-h">${title}</h3>` : ''}<div class="set-rows">${body}</div></div>`;
          const mark = (x) => `<span class="src-mark" style="background:hsl(${hue(x.id)} 45% 34%)" aria-hidden="true">${esc((x.name || x.id).replace(/^Your /, '').slice(0, 1).toUpperCase())}</span>`;
          const srcSwitch = (x, where) => `<input type="checkbox" class="switch" role="switch" data-src-on="${esc(x.id)}" ${x.enabled ? 'checked' : ''} aria-label="${esc(x.name)}${where}">`;
          const tabRow = (x) => { const st = srcState(x); return `<div class="src-tabrow ${x.enabled ? '' : 'off'}"><button class="set-tab ${active === x.id ? 'on' : ''}" id="srctab-${esc(x.id)}" ${active === x.id ? 'aria-current="page"' : ''} data-srctab="${esc(x.id)}">${mark(x)}<span class="src-tname"><span class="src-t1">${esc(x.name)}</span>${x.sign_in ? `<span class="src-tsub" title="${esc(x.sign_in)}">${esc(x.sign_in)}</span>` : ''}</span><span class="src-dot ${st.cls}" title="${esc(st.label)}"></span><span class="sr-only">${esc(st.label)}</span>${icon('chevron', 'ic chev')}</button>${x.id === 'library' ? '<span class="src-noswitch" aria-hidden="true"></span>' : srcSwitch(x, ' on')}</div>`; };
          const lib = { id: 'library', name: 'Your library', status: 'ready', enabled: true };
          // Working services first, in the owner's play order (STEWARD-UX-PLAYER-SOURCES §1)
          await loadSvcRank(); if (state.view !== 'sources') return;
          const on = rankSvc(srcs.filter((x) => x.enabled), (x) => x.id); const off = srcs.filter((x) => !x.enabled);
          const nav = `<nav class="set-nav src-nav" aria-label="Sources">${[lib, ...on].map(tabRow).join('')}${off.length ? (off.length > 2 ? `<details class="src-offgroup" ${off.some((x) => x.id === active) ? 'open' : ''}><summary>Turned off · ${off.length}</summary>${off.map(tabRow).join('')}</details>` : off.map(tabRow).join('')) : ''}</nav>`;
          const panelHead = (x) => `${phone ? `<button class="btn ghost set-back" data-src-back>${icon('back')} Sources</button>` : ''}<h2 class="set-title">${esc(x.name)}</h2>${x.description ? `<p class="muted small src-desc">${esc(x.description)}</p>` : ''}`;
          // TIDAL: one request carries the sign-in ways and both status views (STEWARD-CREDENTIALS-REUSE.md)
          let tidal = null; let tidalSi = null; if (active === 'tidal') { try { tidalSi = await api('GET', '/sources/tidal/signin'); tidal = tidalSi.details.status || null; } catch { tidalSi = null; tidal = null; } if (state.view !== 'sources') return; }
          let sub = null; let subPls = []; if (active === 'subsonic') { try { sub = await api('GET', '/sources/subsonic/server'); if (sub.connected) subPls = await api('GET', '/sources/subsonic/playlists').catch(() => []); } catch { sub = null; } if (state.view !== 'sources') return; }
          const subAccount = active !== 'subsonic' ? '' : sub && sub.connected
            ? row(`Signed in as ${esc(sub.user || '')}`, `${esc(sub.name || 'Your server')}${sub.version ? ' ' + esc(sub.version) : ''} · ${esc(sub.url || '')} · its music in search, its playlists below`, '<button class="btn ghost sm" id="subforget">Forget this server</button>', 'sg-in')
              + `<section class="section"><div class="section-head"><h3>Its playlists</h3><span class="muted small">import one as a private Pubky collection (songs need a MusicBrainz id or ISRC on the server)</span></div>${subPls.length ? `<div class="list">${subPls.map((p) => `<div class="list-row"><div class="grow"><div class="name" title="${esc(p.name)}">${esc(p.name)}</div><div class="sub">${p.songs} song${p.songs === 1 ? '' : 's'}</div></div><button class="btn sm" data-subpl="${esc(p.id)}" aria-label="Import ${esc(p.name)}">${icon('download')} Import</button></div>`).join('')}</div>` : '<p class="muted small">No playlists on the server.</p>'}</section>`
            : `<form class="card subform" id="subform"><p class="sg-lead" data-sg="lead">Play your own music server here: Navidrome, Airsonic, Gonic, Ampache, LMS or another Odeion node. Needs its address, a user and password.</p><label class="field"><span>Server address</span><input class="input" name="url" placeholder="https://music.example.com" autocomplete="url" required></label><label class="field"><span>User</span><input class="input" name="user" autocomplete="username" required></label><label class="field"><span>Password</span><input class="input" name="password" type="password" autocomplete="current-password" required></label><div class="toolbar"><button class="btn primary" id="subconnect" type="submit">${icon('zone')} Connect</button></div></form>`;
          let jf = null; let jfPls = []; if (active === 'jellyfin') { try { jf = await api('GET', '/sources/jellyfin/server'); if (jf.connected) jfPls = await api('GET', '/sources/jellyfin/playlists').catch(() => []); } catch { jf = null; } if (state.view !== 'sources') return; }
          const jfAccount = active !== 'jellyfin' ? '' : jf && jf.connected
            ? row(`Signed in as ${esc(jf.user || '')}`, `${esc(jf.server_name || 'Jellyfin')}${jf.version ? ' ' + esc(jf.version) : ''} · its music in search, its playlists below`, '<button class="btn ghost sm" id="jfout">Sign out</button>', 'sg-in')
              + `<section class="section"><div class="section-head"><h3>Its playlists</h3><span class="muted small">import one as a private Pubky collection (songs need a MusicBrainz recording id in Jellyfin)</span></div>${jfPls.length ? `<div class="list">${jfPls.map((p) => `<div class="list-row"><div class="grow"><div class="name" title="${esc(p.name)}">${esc(p.name)}</div><div class="sub">${p.songs} song${p.songs === 1 ? '' : 's'}</div></div><button class="btn sm" data-jfpl="${esc(p.id)}" aria-label="Import ${esc(p.name)}">${icon('download')} Import</button></div>`).join('')}</div>` : '<p class="muted small">No music playlists.</p>'}</section>`
            : `<form class="card subform" id="jfform"><p class="sg-lead" data-sg="lead">Play your Jellyfin or Emby music here and import its playlists. Needs its address, a user and password (used once; only Jellyfin’s token is kept).</p><label class="field"><span>Server address</span><input class="input" name="url" placeholder="https://jellyfin.example.com" autocomplete="url" required></label><label class="field"><span>User</span><input class="input" name="user" autocomplete="username" required></label><label class="field"><span>Password</span><input class="input" name="password" type="password" autocomplete="current-password"></label><div class="toolbar"><button class="btn primary" id="jfin" type="submit">${icon('zone')} Sign in</button></div></form>`;
          let jam = null; if (active === 'jamendo') { try { jam = await api('GET', '/sources/jamendo/status'); } catch { jam = null; } if (state.view !== 'sources') return; }
          // Jamendo: a sign-in only where one can be made here (no notice about apps when not)
          const jamAccount = active !== 'jamendo' || !jam || !jam.configured || (!jam.signed_in && !jam.can_sign_in) ? ''
            : jam.signed_in ? row(`Signed in${jam.name ? ' as ' + esc(jam.name) : ''}`, 'Your Jamendo likes can come to Bookmarks here.', `<button class="btn sm" id="jamimport">${icon('heart')} Add my likes to Bookmarks</button><button class="btn ghost sm" id="jamout">Sign out</button>`, 'sg-in')
            : `<p class="sg-lead" data-sg="lead">Optional: brings your Jamendo likes to Bookmarks here. Search and full tracks need no account.</p><div class="toolbar"><button class="btn" id="jamin">${icon('zone')} Sign in to Jamendo</button></div>`;
          let spot = null; if (active === 'spotify') { try { spot = await api('GET', '/sources/spotify/status'); } catch { spot = null; } if (state.view !== 'sources') return; }
          const account = active === 'jamendo' ? jamAccount : active === 'subsonic' ? subAccount : active === 'jellyfin' ? jfAccount : '';

          const body = !cur ? '' : cur.id === 'library'
            ? group('', row('Music folders, scans and matches', 'Your own files. Folders, scans, tags and MusicBrainz matches are in Your library.', '<button class="btn" data-view-go="library" data-view-arg="node">Open Your library › Node library</button>')
              + row('Play from', 'Which source plays a song first when several have it.', '<button class="btn ghost" data-goto-settings="playfrom">Open Settings › Play from</button>'))
            : group('', row(cur.enabled ? 'On' : 'Turned off', cur.enabled ? `Shown in search, on Home, on song and album pages and in the play order.` : `Hidden from search, Home, song and album pages and the play order. Its sign-in and settings are kept.`, srcSwitch(cur, ' on'))
                + (cur.id === 'tidal' || cur.id === 'qobuz' ? '' : (() => { const st = srcState({ ...cur, enabled: true }); return row('Status', esc(st.long), `<span class="pill ${st.cls === 'ok' ? 'ok' : st.cls === 'bad' ? 'bad' : ''}">${esc(st.label)}</span>`); })())
                + (cur.features ? row('What it does', (cur.features.length ? cur.features.map((f) => FEATURE_WORDS[f] || f).join(' · ') : 'Nothing yet') + (cur.stage === 'unofficial' ? '. Unofficial: it can stop working at any time.' : cur.stage === 'beta' ? '. New here; tell us what breaks.' : '.'), cur.stage && cur.stage !== 'stable' ? `<span class="pill ${cur.stage === 'unofficial' ? 'warn' : ''}">${esc(cur.stage === 'unofficial' ? 'Unofficial' : 'Beta')}</span>` : '') : ''))
              + (account ? group('Sign in', /^<div class="set-row/.test(account) ? account : block('', '', account)) : '')
              + (cur.id === 'tidal' ? '<div id="tidalsignin"></div>' : '')
              + (cur.id === 'qobuz' ? '<div id="signin-qobuz"></div>' : '')
              + (cur.id === 'spotify' ? '<div id="signin-spotify"></div>' : '')
              + (cur.id === 'youtube' ? '<div id="signin-youtube"></div>' : '')
              + (cur.enabled && (!cur.features || cur.features.includes('search')) && cur.status !== 'needs_config' && cur.status !== 'browser' && !/^not configured/.test(cur.status) && (cur.id !== 'qobuz' || cur.status === 'signed in') && (cur.id !== 'soundcloud' || /^(ready|signed in)/.test(cur.status)) && (cur.id !== 'bandcamp' || /^(ready|signed in)/.test(cur.status)) ? group('Use', row(`Search ${esc(cur.name)}`, `Opens the one search, scoped to ${esc(cur.name)}.`, `<button class="btn" id="ssearchall">${icon('search')} Search ${esc(cur.name)}</button>`)) : '')
              + (cur.id === 'youtube' ? '<div id="ytzones"></div>' : '')
              + (cur.id === 'bandcamp' ? '<div id="bcmode"></div>' : '')
              + (cur.id === 'spotify' ? '<div id="spotplay" hidden><h3 class="set-h sg-play-h">Other ways to play</h3><div id="splibrespot"></div><div id="spotsoloist"></div></div>' : '')
              + (cur.id === 'soundcloud' ? '<div id="scmode"></div>' : '')
              + '<div id="srcmine"></div>';
          // (the last sign-in here failed: why, above the way to try again)
          if (state.signinFailed && cur && state.signinFailed.svc === cur.id && /^signed in\b/.test(String(cur.status || ''))) state.signinFailed = null;
          const failed = state.signinFailed && cur && state.signinFailed.svc === cur.id ? `<p class="notice warn src-failed">Last sign-in failed: ${esc(state.signinFailed.why)}</p>` : '';
          main.innerHTML = (phone && active ? '' : pageHead('Sources')) + listNote + `<div class="settings ${active ? 'has-panel' : 'no-panel'}">${nav}<div class="set-body-wrap">${cur ? `<section class="set-panel" id="src-${esc(cur.id)}" aria-labelledby="srctab-${esc(cur.id)}">${panelHead(cur)}${failed}${body}</section>` : ''}</div></div>`;
          // A new try (a sign-in button, or Finish sign-in with a pasted address) puts the note away
          if (failed) $(`#src-${CSS.escape(cur.id)}`, main).addEventListener('click', (e) => { if (!e.target.closest('#tidalin, #tbin, #tbfinish, #tfin, #spotin, #jamin, form[id$="paste"] button[type=submit]')) return; state.signinFailed = null; $$('.src-failed', main).forEach((n) => n.remove()); }, true);
          $$('[data-srctab]', main).forEach((t) => t.onclick = () => setView('sources', { source: t.dataset.srctab }, phone ? 'push' : 'replace'));
          // "‹ Sources" opens the list: one step back when the list is the page before, else the list in
          // this panel's place (a service opened from an album's Connect TIDAL, so Back then returns there);
          // on the first page of the app's history (opened by a link or the address bar) the list is pushed,
          // like every Back's fallback, so the phone's Back from it returns to this panel, not out of the app
          $$('[data-src-back]', main).forEach((b) => b.onclick = () => { if (navDepth === 0 || String((history.state || {}).fromPath || '').split('?')[0] === '/sources') goBack('sources'); else setView('sources', undefined, 'replace'); });
          // The Sign in chooser of each source that has one (STEWARD-CREDENTIALS-REUSE.md)
          signinChooser(tidalSigninCfg($('#tidalsignin')), tidalSi);
          qobuzSetup($('#signin-qobuz'));
          signinChooser(spotifySigninCfg($('#signin-spotify')));
          if ($('#signin-youtube')) api('GET', '/sources/youtube/unofficial').catch(() => ({})).then((u) => signinChooser(youtubeSigninCfg($('#signin-youtube'), !!(u && u.enabled && (u.ytdlp || u.can_download)))));
          // YouTube › On your speakers (unofficial): yt-dlp as an external program (STEWARD-DUAL-MODE.md)
          const yz = $('#ytzones');
          const fillYtZones = async () => {
            if (!yz || !yz.isConnected) return;
            let u; try { u = await api('GET', '/sources/youtube/unofficial'); } catch (e) { yz.innerHTML = ''; return; }
            if (!yz.isConnected) return;
            const y = u.ytdlp;
            const usable = !!(y || u.can_download);
            if (!usable && !u.enabled) { yz.innerHTML = ''; return; }
            const rows = [row('On your speakers (unofficial)', usable ? esc(splitNote(u.warning)[0]) : 'Unavailable on this machine. You can turn it off.', `<input type="checkbox" class="switch" role="switch" id="yzon" ${u.enabled ? 'checked' : ''} aria-label="YouTube on your speakers (unofficial)">`)];
            if (usable && (u.enabled || y)) {
              rows.push(y
                ? row('yt-dlp', `${esc(y.version ? 'Version ' + y.version : 'Found')}: ${y.from === 'downloaded' ? 'downloaded by this node' : y.from === 'config' ? 'set in the node config' : 'installed on this machine'} (<code>${esc(y.path)}</code>).`, y.from === 'downloaded' ? '<button class="btn ghost sm" id="yzupdate">Update</button><button class="btn ghost sm" id="yzremove">Remove</button>' : '')
                : row('yt-dlp', u.can_download ? 'Needed to play YouTube on your speakers. The node downloads it from yt-dlp’s official GitHub releases and checks it against the release’s checksums.' : 'Needed to play YouTube on your speakers. Install yt-dlp on this machine, then reload this page.', u.can_download ? '<button class="btn primary sm" id="yzget">Download yt-dlp</button>' : ''));
              if (u.health) rows.push(row('Health', esc(u.health.ok ? `Working: ${u.health.detail}.` : `Last try: ${u.health.detail}`), `<span class="pill ${u.health.ok ? 'ok' : 'bad'}">${u.health.ok ? 'OK' : 'Problem'}</span>`));
            }
            yz.innerHTML = group('On your speakers', rows.join(''));
            $('#yzon').onchange = async (e) => { try { await api('PUT', '/sources/youtube/unofficial', { enabled: e.target.checked }); toast(e.target.checked ? 'YouTube can play on your speakers' : `YouTube plays only ${hereOnly()}`); } catch (er) { toast(er.message, 'error'); } render(); };
            const get = async (b, msg) => { b.disabled = true; b.textContent = 'Downloading…'; try { await api('POST', '/sources/youtube/unofficial/ytdlp'); toast(msg); } catch (er) { toast(er.message, 'error'); } render(); };
            const yg = $('#yzget'); if (yg) yg.onclick = () => get(yg, 'yt-dlp downloaded');
            const yu = $('#yzupdate'); if (yu) yu.onclick = () => get(yu, 'yt-dlp updated');
            const yr = $('#yzremove'); if (yr) yr.onclick = async () => { try { await api('DELETE', '/sources/youtube/unofficial/ytdlp'); toast('yt-dlp removed'); } catch (er) { toast(er.message, 'error'); } render(); };
          };
          fillYtZones();
          const bcm = $('#bcmode');
          const fillBandcamp = async () => {
            if (!bcm || !bcm.isConnected) return;
            let u; try { u = await api('GET', '/sources/bandcamp/unofficial'); } catch (e) { bcm.innerHTML = ''; return; }
            if (!bcm.isConnected) return;
            const FMT = { flac: 'FLAC', alac: 'ALAC', 'aiff-lossless': 'AIFF', wav: 'WAV', 'mp3-320': 'MP3 320', 'mp3-v0': 'MP3 V0', 'aac-hi': 'AAC', vorbis: 'Ogg Vorbis' };
            const rows = [row('Bandcamp (unofficial)', esc(u.warning), `<input type="checkbox" class="switch" role="switch" id="bcon" ${u.enabled ? 'checked' : ''} aria-label="Bandcamp (unofficial)">`)];
            if (u.health && !u.health.ok) rows.push(`<div class="notice warn" role="status">Switched itself off: ${esc(u.health.detail)}. Bandcamp links still open on bandcamp.com.<div class="actions"><button class="btn sm" id="bcretry">Try again</button></div></div>`);
            if (u.enabled) {
              rows.push(u.signed_in
                ? row('Signed in', `Your Bandcamp collection${u.user ? ' (' + esc(u.user) + ')' : ''}. The sign-in stays on the node and never comes back.`, '<button class="btn ghost sm" id="bcout">Sign out</button>')
                : `<form class="set-row set-block" id="bcin"><div class="set-label"><div class="set-name">Your purchases</div>${help('To download what you bought: sign in on bandcamp.com in your browser, open its cookies for bandcamp.com and copy the value of the one named identity. The node checks it once with Bandcamp and keeps it like a sign-in. Search and streams work without it.')}</div><div class="set-body"><div class="toolbar"><input class="input" name="identity" type="password" placeholder="identity cookie value" autocomplete="off" spellcheck="false" aria-label="Bandcamp identity cookie" required><button class="btn" type="submit">Save</button></div></div></form>`);
              rows.push(row('Download format', `Bought music downloads into <code class="path">${esc(u.folder)}</code>, a library folder. When a release lacks this format, the best one it has is used.`, `<select class="input" id="bcfmt" aria-label="Bandcamp download format">${u.formats.map((f) => `<option value="${esc(f)}" ${f === u.format ? 'selected' : ''}>${esc(FMT[f] || f)}</option>`).join('')}</select>`));
              rows.push(row('Streams', 'What Bandcamp’s own player plays: MP3 128 kbit/s, full tracks where the artist allows it.', ''));
              if (u.health && u.health.ok) rows.push(row('Health', `Working: ${esc(u.health.detail)}.`, '<span class="pill ok">OK</span>'));
            }
            bcm.innerHTML = group('Unofficial mode', rows.join(''));
            const put = async (body, ok) => { try { await api('PUT', '/sources/bandcamp/unofficial', body); if (ok) toast(ok); } catch (e) { toast(e.message, 'error'); } render(); };
            $('#bcon').onchange = (e) => put({ enabled: e.target.checked }, e.target.checked ? 'Bandcamp is on' : 'Bandcamp is off');
            const br = $('#bcretry'); if (br) br.onclick = () => put({ enabled: true }, 'Trying Bandcamp again');
            const bi = $('#bcin'); if (bi) bi.onsubmit = (e) => { e.preventDefault(); return put({ identity: String(new FormData(bi).get('identity') || '').trim() }, 'Signed in to Bandcamp'); };
            const bo = $('#bcout'); if (bo) bo.onclick = () => put({ identity: '' }, 'Signed out of Bandcamp');
            const bf = $('#bcfmt'); if (bf) bf.onchange = () => put({ format: bf.value }, 'Download format saved');
          };
          fillBandcamp();
          if (cur && cur.id === 'bandcamp' && cur.enabled && cur.status === 'signed in') {
            // Your purchases, each downloadable into the library. Each row ends in "album" or "song", as
            // the other services' lists on Sources do (Bandcamp's own word is "track")
            const box = document.createElement('div'); box.id = 'bcmine'; stillLoading(box, 'Still loading from Bandcamp… the first time can take a while.', () => !!box.querySelector('[data-loading]')); box.innerHTML = '<p class="muted small" data-loading>Loading your Bandcamp collection…</p>'; ($('#srcmine') || main).appendChild(box);
            const draw = async () => {
              try {
                const [items, jobs] = await Promise.all([api('GET', '/sources/bandcamp/collection'), api('GET', '/sources/bandcamp/downloads')]);
                if (state.view !== 'sources' || state.sourceId !== 'bandcamp' || !box.isConnected) return;
                const job = Object.fromEntries(jobs.map((j) => [j.id, j]));
                const act = (p) => { const j = job[p.id]; if (j && !['done', 'failed'].includes(j.state)) return `<span class="muted small">${esc(j.state === 'downloading' && j.total ? `${Math.round(100 * j.bytes / j.total)}%` : j.state)}</span>`; if (j && j.state === 'failed') return `<span class="pill bad" title="${esc(j.error || '')}">Failed</span><button class="btn ghost sm" data-bcdl="${esc(p.id)}">Retry</button>`; if (p.downloaded || (j && j.state === 'done')) return '<span class="pill ok">In your library</span>'; return p.downloadable ? `<button class="btn sm" data-bcdl="${esc(p.id)}">${icon('download')} Download</button>` : '<span class="muted small">no download</span>'; };
                box.innerHTML = `<section class="section"><div class="section-head"><h3>Your purchases</h3><span class="muted small">${countOf(items.length, 'items')}</span></div>${items.length ? `<div class="list">${items.map((p) => `<div class="list-row ${p.item ? 'clickable' : ''}" ${p.item ? `data-sitem="${esc(p.item)}"` : ''}>${p.image ? `<img class="thumb" src="${esc(p.image)}" alt="" loading="lazy">` : '<div class="thumb"></div>'}<div class="grow"><div class="name">${esc(p.title)}</div><div class="sub">${esc(p.artist || '')}</div></div><span class="muted small">${esc(homeKind(p))}</span>${act(p)}</div>`).join('')}</div>` : '<p class="muted">Nothing bought on Bandcamp yet.</p>'}</section>`;
                $$('[data-sitem]', box).forEach((b) => b.onclick = (e) => { if (e.target.closest('button')) return; setView('source-item', { source: 'bandcamp', id: b.dataset.sitem }); });
                $$('[data-bcdl]', box).forEach((b) => b.onclick = async (e) => { e.stopPropagation(); b.disabled = true; try { await api('POST', `/sources/bandcamp/collection/${encodeURIComponent(b.dataset.bcdl)}/download`); toast('Downloading into your library'); } catch (er) { toast(er.message, 'error'); } draw(); });
                if (jobs.some((j) => !['done', 'failed'].includes(j.state))) setTimeout(draw, 2000);
              } catch (e) { box.innerHTML = `<p class="muted small">Bandcamp collection: ${esc(e.message)}</p>`; }
            };
            draw();
          }
          // Spotify › Other ways to play: shown once one of them can run here (owner, 2026-11-08)
          // A group under the "Other ways to play" heading: its own title one level down
          const subGroup = (title, body) => `<div class="set-group"><h4 class="set-h">${title}</h4><div class="set-rows">${body}</div></div>`;
          const spotPlayShown = () => { const box = $('#spotplay'); if (box) box.hidden = !$$('#splibrespot > *, #spotsoloist > *', box).length; };
          const slo = $('#spotsoloist');
          const fillSoloist = async () => {
            if (!slo || !slo.isConnected) return;
            let u; try { u = await api('GET', '/sources/spotify/soloist'); } catch (e) { slo.innerHTML = ''; spotPlayShown(); return; }
            if (!slo.isConnected) return;
            const b = u.soloist;
            // Soloist plays on this machine's own PipeWire or PulseAudio output: none in a container
            // or a service without a desktop session, and nothing to get without a build for it
            if (!u.enabled && !u.running && !(u.supported && u.sound_output && (b || u.can_download))) { slo.innerHTML = ''; spotPlayShown(); return; }
            const rows = [row('Spotify Soloist', 'Spotify’s own player, on this machine’s speakers (not the node’s zones). Its key needs Spotify Premium.', `<input type="checkbox" class="switch" role="switch" id="slon" ${u.enabled ? 'checked' : ''} ${u.supported ? '' : 'disabled'} aria-label="Spotify Soloist">`)];
            // Left on where it can't play (no sound output any more): just the switch, to turn it off
            const noPlay = !u.running && !(u.supported && u.sound_output);
            if (noPlay) rows.push('<div class="notice" role="status">This machine has no sound output Soloist can play on.</div>');
            if (!noPlay) {
              if (u.health && !u.health.ok) rows.push(`<div class="notice warn" role="status">${esc(u.health.detail)}<div class="actions"><button class="btn sm" id="slretry">Try again</button></div></div>`);
              if (u.enabled && u.waiting_for) rows.push(`<div class="notice" role="status">Switched on, waiting for ${esc(u.waiting_for)}.</div>`);
              const stateText = !u.running ? (u.enabled ? 'Not running.' : 'Off.') : u.paired === false ? `Running as “${esc(u.device_name)}”. Nobody has picked it yet: open a Spotify app on this network, choose this device from the device picker and press play.` : u.paired ? `Running as “${esc(u.device_name)}”, paired with a Spotify account.` : `Starting as “${esc(u.device_name)}”…`;
              rows.push(row('Status', stateText, u.running ? '<span class="pill ok">Running</span><button class="btn ghost sm" id="slrestart">Restart</button>' : u.enabled ? '<span class="pill">Waiting</span>' : '<span class="pill">Off</span>'));
              if (u.running && u.now && u.now.title) {
                const n = u.now; const who = [n.artists.join(', '), n.album].filter(Boolean).join(' · ');
                rows.push(row(n.status === 'paused' ? 'Paused' : n.status === 'playing' ? 'Playing on this machine' : 'Loaded', `${esc(n.title)}${who ? ' · ' + esc(who) : ''}${n.volume != null ? ` · volume ${n.volume}` : ''}`, `<button class="btn ghost sm" id="slprev" aria-label="Previous">${icon('prev')}</button><button class="btn sm" id="sltoggle">${n.status === 'playing' ? 'Pause' : 'Play'}</button><button class="btn ghost sm" id="slnext" aria-label="Next">${icon('next')}</button>`));
              }
              rows.push(`<form class="set-row set-block" id="slkey"><div class="set-label"><div class="set-name">Your Soloist API key</div>${help('Each person makes their own key, with a Spotify Premium account, at developer.spotify.com › Dashboard › Spotify Soloist API Key (accept Spotify’s terms, then Generate). The node keeps it like a sign-in and never shows it again. Soloist takes the key only on its command line, so other users of this machine can see it while Soloist runs.')}</div><div class="set-body"><div class="toolbar">${u.key_set ? `<span class="pill ok">Saved</span><button type="button" class="btn ghost sm" id="slkeychange">Change</button><button type="button" class="btn ghost sm" id="slkeyforget">Forget</button>` : ''}<input class="input" name="key" type="password" placeholder="Soloist API key" autocomplete="off" spellcheck="false" aria-label="Spotify Soloist API key" ${u.key_set ? 'hidden' : 'required'}><button class="btn" type="submit" ${u.key_set ? 'hidden' : ''}>Save</button></div></div></form>`);
              rows.push(`<form class="set-row set-block" id="sldev"><div class="set-label"><div class="set-name">Device</div>${help('The name Spotify apps show for this machine, and, with PipeWire, which output Soloist uses (a node name or id; empty means the system default).')}</div><div class="set-body"><div class="toolbar"><input class="input" name="name" placeholder="Device name" maxlength="64" autocomplete="off" aria-label="Spotify Connect device name" value="${esc(u.device_name)}"><input class="input" name="pw" placeholder="PipeWire output (optional)" autocomplete="off" spellcheck="false" aria-label="PipeWire output" value="${esc(u.pipewire_device || '')}"><button class="btn" type="submit">Save</button></div></div></form>`);
              rows.push(b
                ? row('Soloist', `${esc(b.version ? b.version : 'Found')}: ${b.from === 'downloaded' ? `downloaded by this node${u.downloaded_at ? ' on ' + new Date(u.downloaded_at * 1000).toLocaleDateString() : ''}` : b.from === 'config' ? 'set in the node config' : 'installed on this machine'} (<code>${esc(b.path)}</code>). Spotify’s builds stop working 90 days after they were made.`, b.from === 'downloaded' ? '<button class="btn ghost sm" id="slupdate">Update</button><button class="btn ghost sm" id="slremove">Remove</button>' : '')
                : row('Soloist', u.can_download ? 'The node downloads Spotify’s build for this machine from Spotify’s own servers, when you ask. Spotify publishes no checksums; the node checks that the download runs. By using it you accept Spotify’s Terms and Conditions of Use.' : 'Spotify offers no build for this machine here. Install Soloist yourself and point <code>[spotify] soloist</code> in the node config at it, then reload this page.', u.can_download ? '<button class="btn primary sm" id="slget">Download Soloist</button>' : ''));
            }
            slo.innerHTML = subGroup('Spotify Soloist', rows.join('')); spotPlayShown();
            const put = async (body, ok) => { try { await api('PUT', '/sources/spotify/soloist', body); if (ok) toast(ok); } catch (e) { toast(e.message, 'error'); } render(); };
            $('#slon').onchange = (e) => put({ enabled: e.target.checked }, e.target.checked ? 'Spotify Soloist is on' : 'Spotify Soloist is off');
            const sr = $('#slretry'); if (sr) sr.onclick = () => put({ enabled: true }, 'Starting Spotify Soloist again');
            const srs = $('#slrestart'); if (srs) srs.onclick = () => put({ enabled: true }, 'Restarting Spotify Soloist');
            const kf = $('#slkey'); if (kf) kf.onsubmit = (e) => { e.preventDefault(); return put({ api_key: String(new FormData(kf).get('key') || '').trim() }, 'Soloist API key saved'); };
            const kc = $('#slkeychange'); if (kc) kc.onclick = () => { kf.querySelectorAll('[hidden]').forEach((x) => { x.hidden = false; }); kc.hidden = true; kf.querySelector('input').focus(); };
            const kg = $('#slkeyforget'); if (kg) kg.onclick = () => put({ api_key: '' }, 'Soloist API key forgotten');
            const df = $('#sldev'); if (df) df.onsubmit = (e) => { e.preventDefault(); const f = new FormData(df); return put({ device_name: String(f.get('name') || '').trim(), pipewire_device: String(f.get('pw') || '').trim() }, 'Device saved'); };
            const get = async (btn, msg) => { btn.disabled = true; btn.textContent = 'Downloading…'; try { await api('POST', '/sources/spotify/soloist/binary'); toast(msg); } catch (er) { toast(er.message, 'error'); } render(); };
            const sg = $('#slget'); if (sg) sg.onclick = () => get(sg, 'Soloist downloaded');
            const su = $('#slupdate'); if (su) su.onclick = () => get(su, 'Soloist updated');
            const sv = $('#slremove'); if (sv) sv.onclick = async () => { try { await api('DELETE', '/sources/spotify/soloist/binary'); toast('Soloist removed'); } catch (er) { toast(er.message, 'error'); } render(); };
            const ctl = async (command) => { try { await api('POST', '/sources/spotify/soloist/control', { command }); } catch (er) { toast(er.message, 'error'); } render(); };
            const st = $('#sltoggle'); if (st) st.onclick = () => ctl(u.now && u.now.status === 'playing' ? 'pause' : 'play');
            const sp = $('#slprev'); if (sp) sp.onclick = () => ctl('prev');
            const sn = $('#slnext'); if (sn) sn.onclick = () => ctl('next');
          };
          fillSoloist();
          const lso = $('#splibrespot');
          const fillLibrespot = async () => {
            if (!lso || !lso.isConnected) return;
            let u, si; try { [u, si] = await Promise.all([api('GET', '/sources/spotify/librespot'), api('GET', '/sources/spotify/signin')]); } catch (e) { lso.innerHTML = ''; spotPlayShown(); return; }
            if (!lso.isConnected) return;
            const b = u.librespot;
            const lsd = (si.details && si.details.librespot) || {};
            const lsIn = si.in_use && /^librespot/.test(si.in_use.way) ? si.in_use : lsd.signed_in ? { way: lsd.credentials_from ? 'librespot-file' : 'librespot', user: lsd.user, label: lsd.credentials_from ? 'librespot, from a credentials file' : 'librespot, picked in a Spotify app' } : null;
            // librespot publishes no builds: nothing here until it is installed on this machine (an
            // account still signed in without it: the switch and Sign out, to undo that)
            if ((!b || !u.supported) && !lsIn) { lso.innerHTML = ''; spotPlayShown(); return; }
            const gone = !b || !u.supported;
            const rows = [row('librespot', `Plays into one of the node’s zones: pick it in a Spotify app on this network. ${esc(splitNote(u.warning)[0])}`, `<input type="checkbox" class="switch" role="switch" id="lson" ${u.enabled ? 'checked' : ''} aria-label="librespot">`)];
            if (lsIn) rows.push(row(`Signed in as ${esc(lsIn.user || 'your Spotify account')}`, esc(lsIn.label), '<button class="btn ghost sm" id="lsout">Sign out</button>'));
            if (gone) rows.push('<div class="notice" role="status">librespot isn’t installed on this machine.</div>');
            if (!gone && u.health && !u.health.ok) rows.push(`<div class="notice warn" role="status">${esc(u.health.detail)}<div class="actions"><button class="btn sm" id="lsretry">Try again</button></div></div>`);
            if (!gone && u.enabled && u.waiting_for) rows.push(`<div class="notice" role="status">Switched on, waiting for ${esc(u.waiting_for)}.</div>`);
            if (!gone) {
              rows.push(row('Program', `${esc(b.version || 'Found')}: ${b.from === 'config' ? 'set in the node config' : b.from === 'data' ? 'in the node’s bin folder' : 'installed on this machine'} (<code>${esc(b.path)}</code>).`, ''));
              const zoneOpts = u.zones.map((z) => `<option value="zone:${esc(z.id)}" ${u.output === 'zone' && u.zone_id === z.id ? 'selected' : ''}>Zone: ${esc(z.name)}</option>`).join('');
              rows.push(`<form class="set-row set-block" id="lsdev"><div class="set-label"><div class="set-name">Device and output</div>${help('The name Spotify apps show, and where the sound goes: this machine’s own sound output, or one of the node’s zones (librespot then pipes its audio into the node, and the zone starts, pauses and stops with it).')}</div><div class="set-body"><div class="toolbar"><input class="input" name="name" maxlength="64" placeholder="Device name" autocomplete="off" aria-label="librespot device name" value="${esc(u.device_name)}"><select class="input" name="out" aria-label="librespot output"><option value="local" ${u.output === 'local' ? 'selected' : ''}>This machine’s sound</option>${zoneOpts}</select><button class="btn" type="submit">Save</button></div></div></form>`);
              rows.push(row('Status', !u.running ? (u.enabled ? 'Not running.' : 'Off.') : `Running as “${esc(u.device_name)}”${u.user ? `, picked by ${esc(u.user)}` : '. Pick it in a Spotify app on this network'}.${u.now && u.now.title ? ` ${u.now.status === 'paused' ? 'Paused' : 'Playing'}: ${esc(u.now.title)}${u.now.artists && u.now.artists.length ? ' · ' + esc(u.now.artists.join(', ')) : ''}.` : ''}`, u.running ? '<span class="pill ok">Running</span>' : `<span class="pill">${u.enabled ? 'Waiting' : 'Off'}</span>`));
            }
            // Signed in with another librespot (spotifyd, raspotify, go-librespot): its credentials.json
            if (!lsIn && !gone) rows.push(`<details class="small set-row set-block" id="lsfile"><summary>Signed in with another librespot? Use its credentials.json</summary><div class="stack"><form class="stack" id="lsfilepick"><div class="toolbar"><input class="input" type="file" name="file" aria-label="credentials.json" accept=".json,application/json"><button class="btn sm" type="submit">Use this file</button></div></form><form class="stack" id="lsfilepaste"><textarea class="input" name="text" rows="3" aria-label="credentials.json contents" placeholder='{"username": …, "auth_type": …, "auth_data": …}' autocomplete="off" spellcheck="false" required></textarea><div class="toolbar"><button class="btn sm" type="submit">Use it</button><span class="muted small">Kept readable by the node only.</span></div></form><div class="notice warn" role="alert" id="lsfileerr" hidden></div></div></details>`);
            lso.innerHTML = subGroup('librespot', rows.join('')); spotPlayShown();
            const put = async (body, ok) => { try { await api('PUT', '/sources/spotify/librespot', body); if (ok) toast(ok); } catch (e) { toast(e.message, 'error'); } render(); };
            const lsImport = async (text) => { const err = $('#lsfileerr'); err.hidden = true; try { await api('POST', '/sources/spotify/signin/import', { text }); toast('librespot signed in'); render(); } catch (e) { err.textContent = e.message; err.hidden = false; } };
            const lfp = $('#lsfilepick'); if (lfp) lfp.onsubmit = async (e) => { e.preventDefault(); const file = lfp.querySelector('input[type=file]').files[0]; if (!file) { toast('Pick the file first', 'error'); return; } return lsImport(await file.text()); };
            const lfs = $('#lsfilepaste'); if (lfs) lfs.onsubmit = (e) => { e.preventDefault(); return lsImport(lfs.querySelector('textarea').value); };
            // Sign out of librespot by its way, also while the app is the one in use
            const lo = $('#lsout'); if (lo) lo.onclick = async () => { try { await api('DELETE', `/sources/spotify/signin?way=${encodeURIComponent(lsIn.way)}`); toast('librespot signed out'); } catch (e) { toast(e.message, 'error'); } render(); };
            $('#lson').onchange = (e) => put({ enabled: e.target.checked }, e.target.checked ? 'librespot is on' : 'librespot is off');
            const lr = $('#lsretry'); if (lr) lr.onclick = () => put({ enabled: true }, 'Starting librespot again');
            const ld = $('#lsdev'); if (ld) ld.onsubmit = (e) => { e.preventDefault(); const f = new FormData(ld); const out = String(f.get('out') || 'local'); return put({ device_name: String(f.get('name') || '').trim(), output: out === 'local' ? 'local' : 'zone', zone_id: out.startsWith('zone:') ? out.slice(5) : '' }, 'librespot saved'); };
          };
          fillLibrespot();
          const scm = $('#scmode');
          const fillSoundCloud = async () => {
            if (!scm || !scm.isConnected) return;
            let u; try { u = await api('GET', '/sources/soundcloud/unofficial'); } catch (e) { scm.innerHTML = ''; return; }
            if (!scm.isConnected) return;
            const broke = u.health && !u.health.ok;
            const rows = [`<p class="sg-lead" data-sg="lead">Search and play SoundCloud here; your likes and playlists with your sign-in token. Needs a SoundCloud web client ID.</p>`, row('SoundCloud (unofficial)', esc(splitNote(u.warning)[0]), `<input type="checkbox" class="switch" role="switch" id="scon" ${u.enabled ? 'checked' : ''} aria-label="SoundCloud (unofficial)">`)];
            if (broke) rows.push(`<div class="notice warn" role="status">Switched itself off: ${esc(u.health.detail)}. SoundCloud links still open on soundcloud.com.<div class="actions"><button class="btn sm" id="scretry">Try again</button></div></div>`);
            if (u.client_from) rows.push(row('Client ID', `<code class="path">${esc(u.client_id || '')}</code> · ${u.client_from === 'config' ? 'set in the node config.' : 'set here.'}`, `${u.client_from === 'settings' ? '<button class="btn ghost sm" id="scclientchange">Change</button><button class="btn ghost sm" id="scclientforget">Forget</button>' : ''}`));
            rows.push(`<form class="set-row set-block" id="scclient" ${u.client_from ? 'hidden' : ''}><div class="set-label"><div class="set-name">Client ID</div>${help('A SoundCloud web client ID you are allowed to use; SoundCloud stays off until one is set.')}</div><div class="set-body"><div class="toolbar"><input class="input" name="id" placeholder="Client ID" autocomplete="off" spellcheck="false" aria-label="SoundCloud client ID" required><button class="btn" type="submit">Save</button></div></div></form>`);
            if (u.client_from) {
              rows.push(u.signed_in
                ? row(`Signed in as ${esc(u.user || 'your SoundCloud account')}`, 'Your likes and playlists, below.', '<button class="btn ghost sm" id="sctokenforget">Sign out</button>', 'sg-in')
                : `<form class="set-row set-block" id="sctoken"><div class="set-label"><div class="set-name">Your likes and playlists</div>${help('Optional: the sign-in token your browser uses on soundcloud.com. Checked once, never shown again.')}</div><div class="set-body"><div class="toolbar"><input class="input" name="token" type="password" placeholder="Sign-in token (optional)" autocomplete="off" spellcheck="false" aria-label="SoundCloud sign-in token" required><button class="btn" type="submit">Save</button></div></div></form>`);
              rows.push(row('Streams', 'MP3 as SoundCloud serves it, progressive or stitched from HLS. Tracks SoundCloud encrypts (Go+) are skipped; a track SoundCloud only snips plays its 30-second preview.', ''));
              if (u.health && u.health.ok) rows.push(row('Health', `Working: ${esc(u.health.detail)}.`, '<span class="pill ok">OK</span>'));
            }
            scm.innerHTML = group('Unofficial mode', rows.join(''));
            const put = async (body, ok) => { try { await api('PUT', '/sources/soundcloud/unofficial', body); if (ok) toast(ok); } catch (e) { toast(e.message, 'error'); } render(); };
            $('#scon').onchange = (e) => put({ enabled: e.target.checked }, e.target.checked ? 'SoundCloud is on' : 'SoundCloud is off');
            const sr = $('#scretry'); if (sr) sr.onclick = () => put({ enabled: true }, 'Trying SoundCloud again');
            const cf = $('#scclient'); if (cf) cf.onsubmit = (e) => { e.preventDefault(); return put({ client_id: String(new FormData(cf).get('id') || '').trim() }, 'SoundCloud client saved'); };
            const cc = $('#scclientchange'); if (cc) cc.onclick = () => { cf.hidden = false; cf.querySelector('input').focus(); };
            const cg = $('#scclientforget'); if (cg) cg.onclick = () => put({ client_id: '' }, 'SoundCloud client forgotten');
            const tf = $('#sctoken'); if (tf) tf.onsubmit = (e) => { e.preventDefault(); return put({ token: String(new FormData(tf).get('token') || '').trim() }, 'Signed in to SoundCloud'); };
            const tg = $('#sctokenforget'); if (tg) tg.onclick = () => put({ token: '' }, 'Signed out of SoundCloud');
          };
          fillSoundCloud();
          if (cur && cur.id === 'soundcloud' && cur.enabled && cur.status === 'signed in') {
            // Your likes and playlists, with imports into favourites and Pubky playlists.
            const box = document.createElement('div'); box.id = 'scmine'; stillLoading(box, 'Still loading from SoundCloud… the first time can take a while.', () => !!box.querySelector('[data-loading]')); box.innerHTML = '<p class="muted small" data-loading>Loading your SoundCloud likes…</p>'; ($('#srcmine') || main).appendChild(box);
            (async () => {
              try {
                const [likes, pls] = await Promise.all([api('GET', '/sources/soundcloud/likes'), api('GET', '/sources/soundcloud/playlists')]);
                if (state.view !== 'sources' || state.sourceId !== 'soundcloud' || !box.isConnected) return;
                const hit = (h) => `<div class="list-row clickable" data-sitem="${esc(h.id)}">${h.image ? `<img class="thumb" src="${esc(h.image)}" alt="" loading="lazy">` : '<div class="thumb"></div>'}<div class="grow"><div class="name">${esc(h.title)}</div><div class="sub">${esc([h.creator, h.year].filter(Boolean).join(' · '))}</div></div><span class="muted small">${esc(h.kind === 'item' ? 'set' : homeKind(h))}</span></div>`;
                box.innerHTML = `<section class="section"><div class="section-head"><h3>Your likes</h3><span class="muted small">${countOf(likes.tracks.length, 'songs')} · ${countOf(likes.playlists.length, 'sets')}</span><button class="btn sm" id="scfav" title="Bookmark them here too, by their SoundCloud link">${icon('heart')} Import as bookmarks</button></div>${likes.tracks.length || likes.playlists.length ? `<div class="list">${likes.playlists.slice(0, 20).map(hit).join('')}${likes.tracks.slice(0, 50).map(hit).join('')}</div>` : '<p class="muted">Nothing liked on SoundCloud yet.</p>'}</section>
                  <section class="section"><div class="section-head"><h3>Your playlists</h3><span class="muted small">import one as a private Pubky collection</span></div>${pls.length ? `<div class="list">${pls.map((p) => `<div class="list-row"><div class="grow"><div class="name">${esc(p.name)}</div><div class="sub">${countOf(p.items, 'songs')}${p.description ? ' · ' + esc(p.description) : ''}</div></div><button class="btn sm" data-scpl="${esc(p.id)}">${icon('download')} Import</button></div>`).join('')}</div>` : '<p class="muted">No sets of your own on SoundCloud.</p>'}</section>`;
                $$('[data-sitem]', box).forEach((b) => b.onclick = () => setView('source-item', { source: 'soundcloud', id: b.dataset.sitem }));
                $('#scfav').onclick = async () => { try { const r = await api('POST', '/sources/soundcloud/likes/import'); toast(`${r.imported} bookmarks imported${r.skipped ? `, ${r.skipped} skipped` : ''}`); loadFavourites().catch(() => {}); } catch (e) { toast(e.message, 'error'); } };
                $$('[data-scpl]', box).forEach((b) => b.onclick = () => importPlaylist(b, 'soundcloud', b.dataset.scpl));
              } catch (e) { box.innerHTML = `<p class="muted small">SoundCloud likes: ${esc(e.message)}</p>`; }
            })();
          }
          $$('[data-goto-settings]', main).forEach((b) => b.onclick = () => setView('settings', b.dataset.gotoSettings));
          $$('[data-view-go]', main).forEach((b) => b.onclick = () => setView(b.dataset.viewGo, b.dataset.viewArg || null));
          $$('[data-src-on]', main).forEach((sw) => sw.onchange = async () => {
            const id = sw.dataset.srcOn; const want = sw.checked;
            try { await api('PUT', `/sources/${encodeURIComponent(id)}/enabled`, { enabled: want }); applySourcesOff(); homeCache.clear(); toast(`${srcName(id)} ${want ? 'is on' : 'is off: hidden everywhere until you switch it on'}`); render(); } catch (e) { sw.checked = !want; toast(e.message, 'error'); }
          });
          const sf = $('#subform'); if (sf) sf.onsubmit = async (e) => { e.preventDefault(); const f = new FormData(sf); const b = $('#subconnect'); b.disabled = true; try { const v = await api('PUT', '/sources/subsonic/server', { url: f.get('url'), user: f.get('user'), password: f.get('password') }); state.sources = null; toast(`Connected to ${v.name || 'your server'}`); render(); } catch (er) { toast(er.message, 'error'); b.disabled = false; } };
          const jff = $('#jfform'); if (jff) jff.onsubmit = async (e) => { e.preventDefault(); const f = new FormData(jff); const b = $('#jfin'); b.disabled = true; try { const v = await api('PUT', '/sources/jellyfin/server', { url: f.get('url'), user: f.get('user'), password: f.get('password') || '' }); state.sources = null; toast(`Signed in to ${v.server_name || 'Jellyfin'}`); render(); } catch (er) { toast(er.message, 'error'); b.disabled = false; } };
          const jfo = $('#jfout'); if (jfo) jfo.onclick = async () => { try { await api('DELETE', '/sources/jellyfin/server'); state.sources = null; toast('Signed out of Jellyfin'); render(); } catch (e) { toast(e.message, 'error'); } };
          $$('[data-jfpl]', main).forEach((b) => b.onclick = async () => { b.disabled = true; try { const r = await api('POST', `/sources/jellyfin/playlists/${encodeURIComponent(b.dataset.jfpl)}/import`); state.playlists = await api('GET', '/playlists'); toast(`Imported “${r.playlist.summary.name}”: ${countOf(r.imported, 'songs')}${r.skipped ? `, ${r.skipped} without an id skipped` : ''}`); } catch (e) { toast(e.message, 'error'); } b.disabled = false; });
          const sfg = $('#subforget'); if (sfg) sfg.onclick = async () => { try { await api('DELETE', '/sources/subsonic/server'); state.sources = null; toast('Server forgotten'); render(); } catch (e) { toast(e.message, 'error'); } };
          $$('[data-subpl]', main).forEach((b) => b.onclick = async () => { b.disabled = true; try { const r = await api('POST', `/sources/subsonic/playlists/${encodeURIComponent(b.dataset.subpl)}/import`); state.playlists = await api('GET', '/playlists'); toast(`Imported “${r.playlist.summary.name}”: ${countOf(r.imported, 'songs')}${r.skipped ? `, ${r.skipped} without an id skipped` : ''}`); } catch (e) { toast(e.message, 'error'); } b.disabled = false; });
          const ji = $('#jamin'); if (ji) ji.onclick = async () => { try { const r = await api('POST', '/sources/jamendo/login'); location.href = r.url; } catch (e) { toast(e.message, 'error'); } };
          const jo = $('#jamout'); if (jo) jo.onclick = async () => { try { await api('POST', '/sources/jamendo/logout'); toast('Jamendo signed out'); render(); } catch (e) { toast(e.message, 'error'); } };
          const jm = $('#jamimport'); if (jm) jm.onclick = async () => { jm.disabled = true; try { const r = await api('POST', '/sources/jamendo/collection/import'); toast(`${r.imported} Jamendo song${r.imported === 1 ? '' : 's'} added to Bookmarks`); } catch (e) { toast(e.message, 'error'); } jm.disabled = false; };
          if (spot && spot.signed_in && cur.enabled) {
            const box = document.createElement('div'); stillLoading(box, 'Still loading from Spotify… the first time can take a while.', () => !!box.querySelector('[data-loading]')); box.innerHTML = '<p class="muted small" data-loading>Loading your Spotify library…</p>'; ($('#srcmine') || main).appendChild(box);
            (async () => {
              try {
                const [col, pls, devs] = await Promise.all([api('GET', '/sources/spotify/collection'), api('GET', '/sources/spotify/playlists'), api('GET', '/sources/spotify/devices').catch((e) => ({ failed: e }))]);
                if (state.view !== 'sources' || state.sourceId !== 'spotify' || !box.isConnected) return;
                const albums = col.items.filter((h) => h.kind === 'album'); const tracks = col.items.filter((h) => h.kind === 'track');
                const hit = (h) => `<div class="list-row clickable" data-sitem="${esc(h.id)}">${h.image ? `<img class="thumb" src="${esc(h.image)}" alt="" loading="lazy">` : '<div class="thumb"></div>'}<div class="grow"><div class="name">${esc(h.title)}${h.library_track_id || h.library_album_id ? ' <span class="pill ok">in library</span>' : ''}</div><div class="sub">${esc([h.creator, h.year].filter(Boolean).join(' · '))}</div></div><span class="muted small">${esc(homeKind(h))}</span><button class="icon-btn" data-splay="${esc(h.id)}" title="Play on Spotify" aria-label="Play ${h.kind === 'album' ? `the album ${esc(h.title || '')}`.trim() : esc(h.title || 'this')} on Spotify">${icon('play')}</button></div>`;
                box.innerHTML = `<section class="section"><div class="section-head"><h3>Your devices</h3><span class="muted small">Spotify Connect</span></div>${devs.length ? `<div class="chips">${devs.map((d) => `<span class="chip ${d.is_active ? 'on' : ''}">${esc(d.name)} <span class="muted">${esc(d.type)}${d.is_active ? ' · active' : ''}</span></span>`).join('')}</div>` : devs.failed ? failedHtml({ message: `Couldn’t read your Spotify devices: ${devs.failed.message}` }, 'spdevretry') : '<p class="muted small">No Spotify device online. Open Spotify on a phone, computer or speaker and it appears here.</p>'}</section>
                  <section class="section"><div class="section-head"><h3>Your library</h3><span class="muted small">${countOf(tracks.length, 'songs')} · ${countOf(albums.length, 'albums')}</span><button class="btn sm" id="spotfav" title="Bookmark them here too, by ISRC where Spotify has one">${icon('heart')} Import as bookmarks</button></div>${albums.length || tracks.length ? `<div class="list">${albums.slice(0, 20).map(hit).join('')}${tracks.slice(0, 50).map(hit).join('')}</div>` : '<p class="muted">Nothing saved on Spotify yet.</p>'}</section>
                  <section class="section"><div class="section-head"><h3>Your playlists</h3><span class="muted small">import one as a private Pubky collection</span></div>${pls.length ? `<div class="list">${pls.map((p) => `<div class="list-row">${p.image ? `<img class="thumb" src="${esc(p.image)}" alt="" loading="lazy">` : ''}<div class="grow"><div class="name">${esc(p.name)}</div><div class="sub">${countOf(p.items, 'songs')}${p.description ? ' · ' + esc(p.description) : ''}</div></div><button class="btn sm" data-spl="${esc(p.id)}">${icon('download')} Import</button></div>`).join('')}</div>` : '<p class="muted">No playlists on Spotify.</p>'}</section>`;
                $$('[data-sitem]', box).forEach((b) => b.onclick = () => setView('source-item', { source: 'spotify', id: b.dataset.sitem }));
                $$('[data-splay]', box).forEach((b) => b.onclick = (e) => { e.stopPropagation(); spotifyPlay([b.dataset.splay], b); });
                { const dr = $('#spdevretry', box); if (dr) dr.onclick = (ev) => busy(dr, () => render(), ev); }
                $('#spotfav').onclick = async () => { try { const r = await api('POST', '/sources/spotify/collection/import'); toast(`${r.imported} bookmarks imported${r.skipped ? `, ${r.skipped} skipped` : ''}`); loadFavourites().catch(() => {}); } catch (e) { toast(e.message, 'error'); } };
                $$('[data-spl]', box).forEach((b) => b.onclick = () => importPlaylist(b, 'spotify', b.dataset.spl));
              } catch (e) { box.innerHTML = `<p class="muted small">Spotify library: ${esc(e.message)}</p>`; }
            })();
          }
          if (tidal && tidal.signed_in && cur.enabled) {
            // Your collection and playlists, with imports into the node's favourites and Pubky playlists.
            // A cold node can take 20 s and more: the last good copy shows at once (this browser keeps
            // it), each part draws as it comes, and the top of the page says it is loading (UX A7)
            const box = document.createElement('div'); box.id = 'tidalmine'; box.innerHTML = '<div id="tidalcol"></div><div id="tidalpls"></div>'; ($('#srcmine') || main).appendChild(box);
            const memo = `pm.srcmine.tidal.${(state.me && state.me.key) || 'owner'}`;
            let kept = null; try { kept = JSON.parse(localStorage.getItem(memo) || 'null'); } catch { kept = null; }
            const hit = (h) => `<div class="list-row clickable" data-sitem="${esc(h.id)}">${h.image ? `<img class="thumb" src="${esc(h.image)}" alt="" loading="lazy">` : '<div class="thumb"></div>'}<div class="grow"><div class="name">${esc(h.title)}</div><div class="sub">${esc([h.creator, h.year].filter(Boolean).join(' · '))}</div></div><span class="muted small">${esc(homeKind(h))}</span></div>`;
            const failed = (what, err) => `<section class="section"><div class="section-head"><h3>${what}</h3></div><p class="muted small">Couldn’t load it: ${esc(err)} <button class="btn sm" type="button" data-tretry>Try again</button></p></section>`;
            const colHtml = (col) => view.colErr && !col ? failed('Your collection', view.colErr) : col ? `<section class="section"><div class="section-head"><h3>Your collection</h3><span class="muted small">${countOf(col.nTracks, 'songs')} · ${countOf(col.nAlbums, 'albums')}</span><button class="btn sm" id="tidalfav" title="Bookmark them here too: by ISRC where TIDAL has one, else by the TIDAL link">${icon('heart')} Import as bookmarks</button></div>${col.albums.length || col.tracks.length ? `<div class="list">${col.albums.map(hit).join('')}${col.tracks.map(hit).join('')}</div>` : '<p class="muted">Nothing in your TIDAL collection yet.</p>'}</section>`
              : '<section class="section"><div class="section-head"><h3>Your collection</h3></div><p class="muted small" data-loading>Loading your TIDAL collection…</p></section>';
            const plsHtml = (pls) => view.plsErr && !pls ? failed('Your playlists', view.plsErr) : pls ? `<section class="section"><div class="section-head"><h3>Your playlists</h3><span class="muted small">import one as a private Pubky collection</span></div>${pls.length ? `<div class="list">${pls.map((p) => `<div class="list-row"><div class="grow"><div class="name">${esc(p.name)}</div><div class="sub">${countOf(p.items, 'songs')}${p.description ? ' · ' + esc(p.description) : ''}</div></div><button class="btn sm" data-tpl="${esc(p.id)}" ${importing.has(`tidal:${p.id}`) ? 'disabled' : ''}>${icon('download')} Import</button></div>`).join('')}</div>` : '<p class="muted">No playlists of your own on TIDAL.</p>'}</section>`
              : '<section class="section"><div class="section-head"><h3>Your playlists</h3></div><p class="muted small" data-loading>Loading your TIDAL playlists…</p></section>';
            const view = { col: kept && kept.col, pls: kept && kept.pls, fresh: { col: false, pls: false }, colErr: null, plsErr: null, slow: false, topKey: null };
            const here = () => state.view === 'sources' && state.sourceId === 'tidal' && box.isConnected;
            // Under the page's title, where the eye is: loading, updating a kept copy, or the error
            const top = document.createElement('p'); top.className = 'muted small srcmine-status'; top.setAttribute('role', 'status');
            const desc = $('#src-tidal .src-desc') || $('#src-tidal .set-title'); if (desc) desc.after(top);
            // Rewritten only when what it says changes (a screen reader hears it once); an error stays,
            // with Try again, also over a kept copy
            const drawTop = () => {
              if (!top.isConnected) return;
              const done = view.fresh.col && view.fresh.pls; const err = view.colErr || view.plsErr;
              const k = done ? (err ? `e:${err}` : 'done') : `l:${view.slow}`; if (k === view.topKey) return; view.topKey = k;
              if (done && !err) { top.hidden = true; top.innerHTML = ''; return; }
              top.hidden = false;
              top.innerHTML = done
                ? `<span class="warn-text">Couldn’t ${kept ? 'update' : 'load'} your TIDAL collection: ${esc(err)}</span>${kept ? ' Showing the last copy.' : ''} <button class="btn sm" type="button" data-tretry>Try again</button>`
                : `<span class="spin" aria-hidden="true"></span> ${kept ? 'Updating your TIDAL collection' : 'Loading your TIDAL collection'}${view.slow ? '… the first time after a restart can take a while' : '…'} <a href="#tidalmine" data-jump>${kept ? 'Show the last copy' : 'Show'}</a>`;
              const j = $('[data-jump]', top); if (j) j.onclick = (e) => { e.preventDefault(); box.scrollIntoView({ behavior: glide(), block: 'start' }); };
              const t = $('[data-tretry]', top); if (t) t.onclick = () => render();
            };
            // Each part redraws on its own, so focus in the other one stays
            const drawCol = () => {
              const el = $('#tidalcol', box); if (!el || !here()) return;
              el.innerHTML = colHtml(view.col);
              $$('[data-sitem]', el).forEach((b) => b.onclick = () => setView('source-item', { source: 'tidal', id: b.dataset.sitem }));
              const fav = $('#tidalfav', el); if (fav) fav.onclick = async () => { try { const r = await api('POST', '/sources/tidal/collection/import'); toast(`${r.imported} bookmarks imported${r.skipped ? `, ${r.skipped} skipped` : ''}`); loadFavourites().catch(() => {}); } catch (e) { toast(e.message, 'error'); } };
              $$('[data-tretry]', el).forEach((b) => b.onclick = () => render());
            };
            const drawPls = () => {
              const el = $('#tidalpls', box); if (!el || !here()) return;
              el.innerHTML = plsHtml(view.pls);
              $$('[data-tpl]', el).forEach((b) => b.onclick = async () => { await importPlaylist(b, 'tidal', b.dataset.tpl); drawPls(); });
              $$('[data-tretry]', el).forEach((b) => b.onclick = () => render());
            };
            const settled = () => {
              drawTop();
              if (view.fresh.col && view.fresh.pls && !view.colErr && !view.plsErr) { try { localStorage.setItem(memo, JSON.stringify({ at: Date.now(), col: view.col, pls: view.pls })); } catch { /* a full or blocked store: shown fresh next time */ } }
            };
            drawCol(); drawPls(); drawTop();
            setTimeout(() => { if (!(view.fresh.col && view.fresh.pls)) { view.slow = true; drawTop(); } }, 2000);
            api('GET', '/sources/tidal/collection').then((col) => { view.col = { albums: col.albums.slice(0, 20), tracks: col.tracks.slice(0, 50), nAlbums: col.albums.length, nTracks: col.tracks.length }; view.fresh.col = true; drawCol(); settled(); })
              .catch((e) => { view.colErr = e.message; view.fresh.col = true; drawCol(); settled(); });
            api('GET', '/sources/tidal/playlists').then((pls) => { view.pls = pls; view.fresh.pls = true; drawPls(); settled(); })
              .catch((e) => { view.plsErr = e.message; view.fresh.pls = true; drawPls(); settled(); });
          }
          const ssa = $('#ssearchall'); if (ssa) ssa.onclick = () => { state.searchScope = cur.id; search.placeholder = `Search ${cur.name}…`; search.focus(); search.select(); if (search.value.trim()) runSearch(); else { const box = search.closest('.search') || search; box.classList.remove('nudge'); void box.offsetWidth; box.classList.add('nudge'); } };
          break;
        }
        case 'source-artist': {
          loading();
          const { source, id } = state.viewArg;
          const a = await api('GET', `/sources/${encodeURIComponent(source)}/artists/${encodeURIComponent(id)}`);
          if (state.view !== 'source-artist') return;
          main.innerHTML = backBtn('search', 'Back') + `<div class="artist-hero"><div class="portrait" style="background:hsl(${hue(a.name)} 35% 28%)"><span aria-hidden="true">${esc(initials(a.name))}</span>${a.image ? `<img src="${esc(a.image)}" alt="" data-err="remove">` : ''}</div><div class="grow"><div class="page-head"><div><div class="eyebrow">Artist · ${esc(source.toUpperCase())}</div><h1>${esc(a.name)}</h1><div class="muted">${countOf(a.albums.length, 'albums')} · <a href="${esc(webUrl(a.url))}" target="_blank" rel="noopener">open</a>${a.library_artist_id ? ` · <button class="link" data-artist="${a.library_artist_id}">in your library</button>` : ''}</div></div></div></div></div>` + (a.albums.length ? `<div class="grid">${a.albums.map((h) => `<button class="tile" data-sitem="${esc(h.id)}" title="${esc(h.title)}"><div class="art">${h.image ? `<img loading="lazy" alt="" src="${esc(h.image)}">` : ''}${coverBadge(source, true)}${h.library_album_id ? '<span class="src-badge lib">in library</span>' : ''}</div><div class="t">${esc(h.title)}</div><div class="a">${esc([h.year].filter(Boolean).join(' · '))}</div>${coverFrom(source)}</button>`).join('')}</div>` : '<p class="muted">No albums listed.</p>');
          $$('[data-sitem]', main).forEach((b) => b.onclick = () => setView('source-item', { source, id: b.dataset.sitem }));
          $$('[data-artist]', main).forEach((b) => b.onclick = () => setView('artist', Number(b.dataset.artist)));
          break;
        }
        case 'source-item': {
          loading();
          const { source, id } = state.viewArg;
          const srcName = ((state.sources || []).find((x) => x.id === source) || { name: source }).name;
          let it;
          try { it = await api('GET', `/sources/${encodeURIComponent(source)}/items/${encodeURIComponent(id)}`); }
          catch (e) {
            if (state.view !== 'source-item' || state.viewArg.id !== id || state.viewArg.source !== source) return;
            // An item the service won't play (the archive restricts it): the plain reason, nothing to play
            main.innerHTML = backBtn('sources', srcName) + `<div class="page-head"><div><div class="eyebrow">${esc(srcName)}</div><h1>${esc(id)}</h1></div></div><p class="notice warn" role="status">${esc(e.message)}</p>`;
            break;
          }
          if (state.view !== 'source-item' || state.viewArg.id !== id || state.viewArg.source !== source) return;
          const qitems = it.tracks.map((t) => ({ uri: t.url, title: t.title, artist: t.artist || it.creator || '', album: it.title, duration_ms: t.duration_ms, image: it.image || null }));
          const playFrom = (i, replace = true) => playSourceItems(qitems.slice(i), replace, 'end', replace ? { shuffle: false } : {}); // (Play: in order, shuffle off)
          const canHeart = !!(await (async () => { try { await api('GET', '/favourites?uri=' + encodeURIComponent(it.url)); return true; } catch { return false; } })());
          main.innerHTML = backBtn('sources', srcName) + `<div class="artist-hero"><div class="portrait" style="border-radius:12px;background:hsl(${hue(it.title)} 35% 28%)"><span aria-hidden="true">${esc(initials(it.title))}</span>${it.image ? `<img src="${esc(it.image)}" alt="" data-err="remove">` : ''}${coverBadge(source)}</div><div class="grow"><div class="page-head"><div><div class="eyebrow">${esc(srcName)}</div><h1>${esc(it.title)}</h1><div class="muted">${esc([it.creator, it.year, countOf(it.tracks.length, 'songs')].filter(Boolean).join(' · '))} · <a href="${esc(webUrl(it.url))}" target="_blank" rel="noopener">open</a>${it.licence ? (/^https?:/.test(it.licence) ? ` · <a href="${esc(webUrl(it.licence))}" target="_blank" rel="noopener">licence</a>` : ` · ${esc(it.licence)}`) : ''}</div></div><div class="actions"><button class="btn primary" id="aplay">${icon('play')} Play</button><button class="btn" id="aadd">${icon('plus')} Add to queue</button>${offers('collections') ? `<button class="btn" id="amore" title="Add to collection">${icon('playlist')} Collection</button>` : ''}${it.library_album_id ? `<button class="btn ghost" id="ainlib" title="You already have this album">${icon('disc')} In your library</button>` : ''}${canHeart ? `<button class="btn heart ${state.favUris.has(it.url) ? 'on' : ''}" id="aheart" aria-pressed="${state.favUris.has(it.url)}">${icon(state.favUris.has(it.url) ? 'heart-fill' : 'heart')} ${state.favUris.has(it.url) ? 'Bookmarked' : 'Bookmark'}</button>` : ''}</div></div>${it.description ? `<p class="bio">${esc(it.description.slice(0, 600))}${it.description.length > 600 ? '…' : ''}</p>` : ''}</div></div>`
            + `<div class="tracks src-tracks">${it.tracks.map((t, i) => { const on = !!t.url && t.url === npUri(); return `<div class="track compact${on ? ' cur' : ''}" data-i="${i}" data-np="${esc(t.url || '')}"${on ? ' aria-current="true"' : ''}><div class="no"><span class="num">${t.track_no ?? i + 1}</span><button class="rowplay" data-aplay="${i}" title="Play from here" aria-label="${esc(rowWords.play(t.title))}">${icon('play')}</button></div><div class="main-col"><div class="title">${esc(t.title)}</div><div class="by">${esc(t.artist || it.creator || '')}</div></div><div class="fmt-col"><span class="fmt">${esc(t.format)}</span></div><div class="dur">${fmt(t.duration_ms)}</div><div class="row-actions"><button class="icon-btn" data-aadd1="${i}" title="Add to queue" aria-label="${esc(rowWords.add(t.title))}">${icon('plus')}</button>${t.url ? `<button aria-haspopup="menu" aria-expanded="false" class="icon-btn" data-act="more" data-amore1="${i}" title="More" aria-label="${esc(rowWords.more(t.title))}">${icon('more')}</button>` : ''}</div></div>`; }).join('')}</div>`;
          $('#aplay').onclick = () => playFrom(0, true); $('#aadd').onclick = () => playFrom(0, false);
          // (Playlists only where they are kept: not in phone mode)
          if ($('#amore')) $('#amore').onclick = (e) => { e.stopPropagation(); uriMenu($('#amore'), it.tracks.map((t) => t.url), it.title); };
          const ah = $('#aheart'); if (ah) ah.onclick = async () => { const on = state.favUris.has(it.url); try { if (await setFavourite({ uri: it.url }, !on)) render(); } catch (er) { toast(er.message, 'error'); } };
          $$('[data-aplay]', main).forEach((b) => b.onclick = () => playFrom(Number(b.dataset.aplay), true));
          // A click on a song's row plays from there, as on a library album's or a playlist's rows (on a
          // touch screen the ▶ only shows on hover, so the row is the way to it). Its buttons keep their
          // own click; the second click of a double click, or a drag that selects its text, doesn't start it
          $$('.src-tracks > .track', main).forEach((r) => {
            r.onmousedown = (e) => { if (e.detail > 1 && !e.target.closest('a, button, input, select, textarea, label')) e.preventDefault(); };
            r.onclick = (e) => {
              if (e.target.closest('a, button')) return;
              const s = getSelection(); if ((e.detail > 1 && clickWas === srcSpot(r)) || (String(s) !== '' && (r.contains(s.anchorNode) || r.contains(s.focusNode)))) return;
              state.startAnchor = null; // (a tap on the row, not a press of a Play: no spinner on one pressed before)
              playFrom(Number(r.dataset.i), true);
            };
          });
          const ainlib = $('#ainlib'); if (ainlib) ainlib.onclick = () => setView('album', it.library_album_id);
          // (the + and the ⋯'s Add to queue: one song to the end, the same way as Play next)
          const add1 = (i) => playSourceItems([qitems[i]], false, 'end');
          $$('[data-aadd1]', main).forEach((b) => b.onclick = () => add1(Number(b.dataset.aadd1)));
          // A song's ⋯, as on every other song row: Play next, Add to queue, Add to playlist… (one song
          // by its address). Phones and touch screens then show only the ⋯ (the + is in it), and a
          // swipe or a long press on the row works as on a library row. Before, the rows had only +, so
          // a phone could neither play one song next nor put one in a playlist from here.
          // Add to playlist… only where playlists can be kept, as in the shared song menus: the Android app
          // without a node has no 'collections' (its /playlists answers 503), so there a left swipe picks
          // nothing instead of ending in "This needs an Odeion node".
          const keepsPlaylists = () => !state.info || !Array.isArray(state.info.capabilities) || state.info.capabilities.includes('collections');
          $$('[data-amore1]', main).forEach((b) => b.onclick = (e) => { e.stopPropagation(); const i = Number(b.dataset.amore1); const t = it.tracks[i]; showMenu(b, [{ title: t.title },
            { label: 'Play next', icon: 'playnext', run: () => playSourceItems([qitems[i]], false, 'next') },
            { label: 'Add to queue', icon: 'plus', run: () => add1(i) },
            ...(keepsPlaylists() ? [{ label: 'Add to collection…', icon: 'playlist', run: () => uriMenu(b, [t.url], t.title) }] : [])]); });
          break;
        }
        case 'radio': {
          const wantedR = state.viewArg && state.viewArg.q;
          if (wantedR && (!state.radioResults || state.radioResults.q !== wantedR)) {
            try { const r = await api('GET', '/radio/search?limit=40&q=' + encodeURIComponent(wantedR)); state.radioResults = { q: wantedR, stations: r.stations }; } catch (e) { toast(e.message, 'error'); state.radioResults = { q: wantedR, stations: [] }; }
            if (state.view !== 'radio') return;
          }
          const saved = (await api('GET', '/radio/stations')).stations;
          const row = (st, isSaved) => `<div class="list-row clickable" data-st-row="${esc(st.id)}"><button class="icon-btn" data-st-play="${esc(st.id)}" title="Play" aria-label="Play ${esc(st.name || 'this station')}">${icon('play')}</button><div class="avatar" style="background:hsl(${hue(st.name)} 35% 28%);overflow:hidden">${st.favicon ? `<img src="${esc(st.favicon)}" alt="" style="width:100%;height:100%;object-fit:cover" data-err="remove">` : esc(initials(st.name))}</div><div class="grow"><div class="name">${esc(st.name)}</div><div class="sub">${esc([st.country, st.codec ? st.codec + (st.bitrate ? ' ' + st.bitrate + ' kbps' : '') : '', (st.tags || []).slice(0, 4).join(', ')].filter(Boolean).join(' · '))}</div></div>${isSaved ? `<button class="btn sm ghost" data-st-del="${esc(st.id)}">Remove</button>` : saved.some((x) => x.url === st.url) ? '<span class="pill ok">saved</span>' : `<button class="btn sm" data-st-save="${esc(st.id)}">Save</button>`}${webUrl(st.homepage) ? `<a class="icon-btn" href="${esc(webUrl(st.homepage))}" target="_blank" rel="noopener" title="Homepage">${icon('external')}</a>` : ''}</div>`;
          const results = state.radioResults || { q: '', stations: [] };
          main.innerHTML = pageHead('Radio', 'internet radio via radio-browser.info')
            + `<div class="toolbar"><input class="input" id="rq" aria-label="Station search" placeholder="Search stations by name (jazz, FIP, BBC 6…)" value="${esc(results.q)}" style="width:360px"><button class="btn primary" id="rsearch">${icon('search')} Search</button><span class="muted small">Streams play on any zone; lossy codecs show as Lossy in the signal path.</span></div>`
            + (saved.length ? `<section class="section"><div class="section-head"><h3>Saved stations</h3></div><div class="list">${saved.map((st) => row(st, true)).join('')}</div></section>` : '')
            + (results.q ? `<section class="section"><div class="section-head"><h3>Results for “${esc(results.q)}”</h3></div>${results.stations.length ? `<div class="list">${results.stations.map((st) => row(st, false)).join('')}</div>` : '<p class="muted">No stations matched.</p>'}</section>` : (saved.length ? '' : emptyState('radio', 'No stations yet', 'Search the directory above and save the ones you like.')));
          const all = [...saved, ...results.stations];
          const byId = (id) => all.find((x) => x.id === id);
          const doSearch = async () => { const q = $('#rq').value.trim(); if (!q) return; $('#rsearch').disabled = true; try { const r = await api('GET', '/radio/search?limit=40&q=' + encodeURIComponent(q)); state.radioResults = { q, stations: r.stations }; render(); } catch (e) { toast(e.message, 'error'); $('#rsearch').disabled = false; } };
          $('#rsearch').onclick = doSearch; $('#rq').onkeydown = (e) => { if (e.key === 'Enter') $('#rsearch').click(); }; needsText($('#rq'), $('#rsearch'));
          $$('[data-st-play]', main).forEach((b) => b.onclick = () => playStation(byId(b.dataset.stPlay)));
          // The whole row plays the station, not only the small play button (fresh-eyes review)
          $$('[data-st-row]', main).forEach((r) => r.onclick = (e) => { if (e.target.closest('button, a, input')) return; state.startAnchor = null; playStation(byId(r.dataset.stRow)); });
          $$('[data-st-save]', main).forEach((b) => b.onclick = async () => { try { await api('PUT', '/radio/stations', byId(b.dataset.stSave)); toast('Station saved'); render(); } catch (e) { toast(e.message, 'error'); } });
          $$('[data-st-del]', main).forEach((b) => b.onclick = async () => { try { await api('DELETE', `/radio/stations/${encodeURIComponent(b.dataset.stDel)}`); render(); } catch (e) { toast(e.message, 'error'); } });
          break;
        }
        case 'releases': {
          // View all of one release section: 48 a page, newest first; paging replaces the history
          // entry, so Back goes straight to the artist (one press, whatever page)
          const gen = state.pageGen = (state.pageGen || 0) + 1;
          const { id, section } = state.viewArg;
          const live = () => gen === state.pageGen && state.view === 'releases' && state.viewArg.id === id && state.viewArg.section === section;
          const key = (REL_SECTIONS.find(([, , slug]) => slug === section) || [])[0] || 'other';
          const refresh = state.viewArg.refresh ? '?refresh=1' : ''; delete state.viewArg.refresh;
          // The next page's Back label, even when this one fails to load (drawn again in place, it
          // keeps the artist's name its last draw gave it while the new answer comes)
          state.relTitle = state.relTitle || (REL_SECTIONS.find(([k]) => k === key) || [, 'Releases'])[1];
          const pageKey = `releases:${id}:${section}`; const again = !refresh && redrawing(pageKey);
          if (!again) loading();
          let pg;
          try { pg = await api('GET', `/item/artist/${encodeURIComponent(id)}${refresh}`); }
          catch (e) { if (!live()) return; state.drawnPage = null; if (state.placeHold) state.placeHold(); main.innerHTML = `<div class="pagebar">${backBtn(`item:artist:${id}`, 'Artist')}</div><div class="notice">${esc(e.message)} <button class="btn sm" id="irelretry">Retry</button></div>`; $('[data-back]', main).onclick = () => goBack(`item:artist:${id}`); $('#irelretry', main).onclick = () => { state.viewArg.refresh = 1; render(); }; break; }
          if (!live()) return;
          pg.albums = pg.albums || [];
          const g = relGroups(pg.albums); const list = g[key];
          const name = pg.title || 'Artist'; const title = relTitle(key, g);
          // (the tab and the next page's Back say whose: "Albums by Sefa", not the library's Albums)
          state.relTitle = pg.title ? `${title} by ${pg.title}` : title; state.itemSection = 'artists';
          const pages = Math.max(1, Math.ceil(list.length / REL_PAGE));
          const page = Math.min(Math.max(1, Math.floor(Number(state.viewArg.page) || 1)), pages);
          const shown = list.slice((page - 1) * REL_PAGE, page * REL_PAGE);
          const stuck = (state.viewArg.tries || 0) >= 12;
          const place = again ? keepPlace() : null;
          main.innerHTML = `<div class="pagebar">${backBtn(`item:artist:${id}`, name)}</div>`
            + `<div class="page-head"><div><div class="eyebrow">${esc(name)}</div><h1>${esc(title)}</h1></div></div>`
            + `<div id="ialbums" class="relsecs">${relStatus(pg, stuck)}`
            + (shown.length ? itemTileGrid(shown.map((a) => relTile(a, key)), 'album') : pg.pending ? relSkeleton() : pg.catalogue && pg.catalogue.error ? '' : '<p class="muted small relempty">No releases here.</p>')
            + `</div>${list.length ? `<nav class="pager" aria-label="Pages of ${esc(title)}"><button class="btn sm" data-relpage="prev" ${page <= 1 ? 'disabled' : ''}>${icon('back')} Previous</button><span class="pageinfo muted small" aria-live="polite">Page ${page} of ${pages} · ${relCount(list.length, pg.catalogue)} releases</span><button class="btn sm" data-relpage="next" ${page >= pages ? 'disabled' : ''}>Next</button></nav>` : ''}`;
          $('[data-back]', main).onclick = () => goBack(`item:artist:${id}`);
          bindItemTiles($('#ialbums', main), shown, 'album');
          $$('[data-relpage]', main).forEach((b) => b.onclick = () => setView('releases', { id, section, page: page + (b.dataset.relpage === 'next' ? 1 : -1) }, 'replace'));
          { const rr = $('#irelretry', main); if (rr) rr.onclick = () => { state.viewArg.tries = 0; state.viewArg.refresh = 1; render(); }; }
          state.drawnPage = pageKey; if (place) place();
          if (pg.pending && !stuck) { const tries = (state.viewArg.tries || 0) + 1; setTimeout(() => { if (!live()) return; state.viewArg.tries = tries; render(); }, 3000); }
          break;
        }

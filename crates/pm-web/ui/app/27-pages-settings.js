        case 'settings': {
          // (Zones' "Group outputs…": read once, so a page left before it drew doesn't jump a later visit)
          const toGroup = state.focusGroup; state.focusGroup = false;
          if (isOffline()) {
            const help = (h) => (h ? `<p class="set-help">${h}</p>` : '');
            const row = (label, h, ctl) => `<div class="set-row"><div class="set-label"><div class="set-name">${label}</div>${help(h)}</div>${ctl ? `<div class="set-ctl">${ctl}</div>` : ''}</div>`;
            const group = (title, body) => `<div class="set-group"><h3 class="set-h">${title}</h3><div class="set-rows">${body}</div></div>`;
            let d = null; try { d = await api('GET', '/offline'); } catch { d = null; }
            if (state.view !== 'settings') return;
            main.innerHTML = pageHead('Settings', null)
              + group('This device', row('Offline', `Your node is out of reach, so this app plays what it keeps. It reconnects by itself.${d && d.synced_at ? ` Last synced ${esc(ago(d.synced_at))}.` : ''}`, '')
                + (d ? row('Kept here', `${countOf(d.tracks.length, 'songs')} · ${esc(fileSize(d.bytes))}${d.queued ? ` · ${countOf(d.queued, 'plays and hearts')} waiting to be sent` : ''}`, '<button class="btn" data-view-go="downloads">Downloads</button>') : ''))
              + group('Theme', row('Light or dark', 'Or follow this device.', `<div class="chips themeswitch" role="radiogroup" aria-label="Theme">${[['system', 'System'], ['light', 'Light'], ['dark', 'Dark']].map(([v, l]) => `<button class="chip scope ${themePref() === v ? 'on' : ''}" role="radio" aria-checked="${themePref() === v}" data-theme-pick="${v}">${l}</button>`).join('')}</div>`));
            $$('[data-theme-pick]', main).forEach((b) => b.onclick = () => { setTheme(b.dataset.themePick); $$('[data-theme-pick]', main).forEach((x) => { const on = x === b; x.classList.toggle('on', on); x.setAttribute('aria-checked', String(on)); }); });
            $$('[data-view-go]', main).forEach((b) => b.onclick = () => setView(b.dataset.viewGo));
            break;
          }
          if (phoneMode()) {
            // Phone mode: this phone, its Pubky account, its streaming accounts, the theme, and the way to a node
            const help = (h) => (h ? `<p class="set-help">${h}</p>` : '');
            const row = (label, h, ctl) => `<div class="set-row"><div class="set-label"><div class="set-name">${label}</div>${help(h)}</div>${ctl ? `<div class="set-ctl">${ctl}</div>` : ''}</div>`;
            const group = (title, body) => `<div class="set-group"><h3 class="set-h">${title}</h3><div class="set-rows">${body}</div></div>`;
            const pm = (state.me || {}).pubky; const pk = typeof pm === 'string' ? pm : pm && (pm.key || pm.public_key);
            const srcs = !offers('sources') ? [] : state.sources || await api('GET', '/sources').then((l) => (state.sources = l)).catch(() => []);
            if (state.view !== 'settings') return;
            const svcs = srcs.filter((x) => (x.id === 'tidal' || x.id === 'qobuz') && x.enabled !== false);
            main.innerHTML = pageHead('Settings', null)
              + group('This phone', row('Odeion is running on this phone without a node', 'It plays here, from your music services.', '')
                + (pk ? row('Pubky account', keyHtml(pk), '') : '')
                + (offers('homeserver-files') ? '<div id="phonestore"></div>' : '')
                + row('Your node', 'Connect to your Odeion node to play its music, on its zones too.', '<button class="btn primary" id="phoneleave">Connect to a node</button>'))
              + (svcs.length ? group('My streaming accounts', svcs.map((v) => `<details class="set-row svc-own" data-svc="${esc(v.id)}"><summary class="set-label"><span class="set-name">${esc(v.name)}</span><span class="set-help">${srcHealthy(v) ? 'Signed in.' : 'Not signed in. Sign in to play from it here.'}</span></summary><div class="svc-body"></div></details>`).join('')) : '')
              + group('Theme', row('Light or dark', 'Or follow this device.', `<div class="chips themeswitch" role="radiogroup" aria-label="Theme">${[['system', 'System'], ['light', 'Light'], ['dark', 'Dark']].map(([v, l]) => `<button class="chip scope ${themePref() === v ? 'on' : ''}" role="radio" aria-checked="${themePref() === v}" data-theme-pick="${v}">${icon(v === 'dark' ? 'moon' : v === 'light' ? 'sun' : 'settings')} ${l}</button>`).join('')}</div>`));
            $$('[data-theme-pick]', main).forEach((b) => b.onclick = () => { setTheme(b.dataset.themePick); $$('[data-theme-pick]', main).forEach((x) => { const on = x === b; x.classList.toggle('on', on); x.setAttribute('aria-checked', String(on)); }); });
            // A service opens its own sign-in ways (the Sources page's)
            $$('details[data-svc]', main).forEach((d) => d.addEventListener('toggle', () => {
              const body = $('.svc-body', d); if (!d.open || body.dataset.drawn) return; body.dataset.drawn = '1';
              if (d.dataset.svc === 'qobuz') qobuzSetup(body); else signinChooser(tidalSigninCfg(body));
            }));
            $('#phoneleave').onclick = (e) => phoneCmd(e.currentTarget, 'phone_leave', true);
            // Songs added here, also on the person's homeserver (on by default where it is their node's own)
            const ps = $('#phonestore', main); if (ps) {
              const draw = (st) => {
                const why = st.bundled ? 'On by default: your homeserver is the one installed with your Odeion node, which finds them there.' : 'Your Odeion node finds them there too.';
                const cant = st.cant || 0; const left = st.unreadable ? ` What this phone copied can’t be read here${st.on ? ', so no song is copied until it can' : ''}.` : (st.on && st.total > st.uploaded + cant ? ` ${countOf(st.total - st.uploaded - cant, 'songs')} still to copy.` : '') + (st.on && cant ? ` ${countOf(cant, 'songs')} can’t be copied: too big, or refused by your homeserver.` : '');
                ps.innerHTML = row('Keep added songs on my homeserver', `Copies the songs you add here to your Pubky homeserver (private). ${why}${left}`, `<input type="checkbox" class="switch" role="switch" id="psfiles" ${st.on ? 'checked' : ''} aria-label="Keep added songs on my homeserver">`);
                const box = $('#psfiles', ps); box.onchange = async () => { box.disabled = true; try { draw(await api('PUT', '/phone/storage', { on: box.checked })); } catch (e) { box.checked = !box.checked; box.disabled = false; toast(e.message, 'error'); } };
              };
              api('GET', '/phone/storage').then((st) => { if (ps.isConnected) draw(st); }).catch(() => { ps.remove(); });
            }
            break;
          }
          if (isListener()) {
            const me = state.me; const info = state.info || await api('GET', '/info');
            const [who] = await loadProfiles([me.key]);
            const ms = await api('GET', '/me/sources').catch(() => null); // whose music-service sign-ins they play with
            if (state.view !== 'settings') return; // they went elsewhere while the profile was looked up
            const help = (h) => (h ? `<p class="set-help">${h}</p>` : '');
            const row = (label, h, ctl) => `<div class="set-row"><div class="set-label"><div class="set-name">${label}</div>${help(h)}</div>${ctl ? `<div class="set-ctl">${ctl}</div>` : ''}</div>`;
            const group = (title, body) => `<div class="set-group"><h3 class="set-h">${title}</h3><div class="set-rows">${body}</div></div>`;
            // null: every zone; []: none (the owner opened none, or the node can't read its list); else the zones named
            // (a node before "no zone" was a choice says it as a list of one zone without a name)
            const noZones = Array.isArray(me.zones) && (!me.zones.length || noZoneList(me.zones));
            const zonesText = me.zones && me.zones.length ? `Use ${me.zones.length === 1 ? 'this zone' : 'these zones'}: ${listWords(me.zones.map((z) => (state.zones[z] ? state.zones[z].name : z)))}` : 'Use every zone of this node';
            // What they can do here, in the words the owner's People & access screen uses, from what
            // the node says they can (GET /v1/me's `can`); own sign-ins only where the node lets them.
            // No zone open to them is something they can't do, not a tick under "You can".
            const canDo = canLists({ has: (cap) => (cap === 'zone_use' && noZones ? false : cap === 'own_sign_in' && !Array.isArray(me.can) && ms ? ms.policy !== 'node' : can(cap)), their: 'your', owners: 'the owner’s', zones: zonesText, noZone: 'Play on a zone: none is open to you right now, ask the node’s owner', canTitle: 'You can', cantTitle: 'You can’t' });
            // Their own homeserver, for their bookmarks (personal.rs)
            const [hs, svcs] = await Promise.all([api('GET', '/me/homeserver').catch(() => null), api('GET', '/me/services').catch(() => null)]);
            const hsRow = !hs ? row('My homeserver', 'The node can’t say right now.', '')
              : hs.connected && !hs.lost ? row('My homeserver', `Connected: your bookmarks are kept there${hs.pending ? `, ${countOf(hs.pending, 'changes')} waiting on this node until it answers` : ''}. The node may write your bookmarks and collections folders, nothing else.`, '<button class="btn ghost" id="hsoff">Disconnect</button>')
                : row('My homeserver', hs.lost ? '<span class="warn-text">It no longer lets this node in.</span> Approve it again to keep your bookmarks there.' : 'Not connected. Your bookmarks are kept on your own homeserver once you approve this node in Pubky Ring (your bookmarks and collections folders only).', `<button class="btn primary" id="hson">${hs.lost ? 'Approve again' : 'Connect'}</button>`);
            const srcNote = !ms ? '' : ms.plays_with === 'node' ? 'TIDAL, Qobuz and Spotify play through this node’s sign-ins.' : 'TIDAL, Qobuz and Spotify play with your own sign-ins.';
            const srcCtl = ms && ms.may_choose ? `<div class="chips" role="radiogroup" aria-label="Whose sign-ins">${[['node', 'This node’s'], ['own', 'My own']].map(([v, l]) => `<button class="chip scope ${ms.use === v ? 'on' : ''}" role="radio" aria-checked="${ms.use === v}" data-mysrc="${v}">${l}</button>`).join('')}</div>` : '';
            main.innerHTML = pageHead('Settings', null)
              + group('You', row(esc((who && who.name) || 'Signed in with Pubky Ring'), keyHtml(me.key), `<button class="btn" id="lsignout">${icon('logout')} Log out</button>`)
                + hsRow
                + (ms ? row('Music services', srcNote + (ms.may_choose ? ' You choose; your own sign-ins stay kept either way.' : ''), srcCtl) : ''))
              + group('What you can do here', `<div class="set-row set-block" id="lcando">${canDo}</div>`)
              + group('Your sign-ins on this node', '<div id="mysignins" class="pa-rows"></div>')
              // Your own streaming accounts: nobody else on the node plays with them (node apps stay the owner's)
              + group('My streaming accounts', svcs ? svcs.map((v) => `<details class="set-row svc-own" data-svc="${esc(v.source)}"><summary class="set-label"><span class="set-name">${esc(v.name)}</span><span class="set-help">${v.connected ? `Signed in${v.user ? ' as ' + esc(v.user) : ''}.` : v.available ? 'Not signed in. Sign in with your own account to play from it here.' : esc(v.reason || 'Not set up on this node yet.')}</span></summary><div class="svc-body"></div></details>`).join('') : row('Streaming accounts', 'The node can’t say right now.', ''))
              + group('Play from', row('Your personal source order', 'Applies to every collection you play.', '<div id="gorder"></div>'))
              + group('Usage insights', '<div id="usageme"></div>')
              + group('Theme', row('Light or dark', 'Or follow this device.', `<div class="chips themeswitch" role="radiogroup" aria-label="Theme">${[['system', 'System'], ['light', 'Light'], ['dark', 'Dark']].map(([v, l]) => `<button class="chip scope ${themePref() === v ? 'on' : ''}" role="radio" aria-checked="${themePref() === v}" data-theme-pick="${v}">${icon(v === 'dark' ? 'moon' : v === 'light' ? 'sun' : 'settings')} ${l}</button>`).join('')}</div>`)) + layoutGroup(group, row) + colourGroup(group, row);
            $$('[data-theme-pick]', main).forEach((b) => b.onclick = () => { setTheme(b.dataset.themePick); $$('[data-theme-pick]', main).forEach((x) => { const on = x === b; x.classList.toggle('on', on); x.setAttribute('aria-checked', String(on)); }); }); bindLayoutPick();
            loadOrder();
            if (window.PM_USAGE) PM_USAGE.renderPersonal($('#usageme', main));
            $('#lsignout').onclick = logOut;
            mountSignIns($('#mysignins', main), { kept: keptSignIns() });
            // A service opens its own sign-in ways (the same as the owner's, without the node's setup)
            $$('details[data-svc]', main).forEach((d) => d.addEventListener('toggle', () => {
              const body = $('.svc-body', d); if (!d.open || body.dataset.drawn) return; body.dataset.drawn = '1';
              const src = d.dataset.svc;
              if (src === 'qobuz') qobuzSetup(body);
              else signinChooser(src === 'tidal' ? tidalSigninCfg(body) : spotifySigninCfg(body));
            }));
            const hon = $('#hson'); if (hon) hon.onclick = async () => { if (await homeserverDialog(!!(hs && hs.lost))) { loadFavourites().catch(() => {}); render(); } };
            const hoff = $('#hsoff'); if (hoff) hoff.onclick = async () => { hoff.disabled = true; try { await api('DELETE', '/me/homeserver'); toast('Disconnected: your homeserver keeps your bookmarks'); clearFavourites(); render(); } catch (e) { hoff.disabled = false; toast(e.message, 'error'); } };
            $$('[data-mysrc]', main).forEach((b) => b.onclick = async () => { try { await api('PUT', '/me/sources', { use: b.dataset.mysrc }); toast(b.dataset.mysrc === 'own' ? 'You play with your own sign-ins' : 'You play through this node’s sign-ins'); render(); } catch (e) { toast(e.message, 'error'); } });
            break;
          }
          const info = await api('GET', '/info'); state.info = info;
          const outs0 = await loadOutputs();
          // (`unread`: why they could not be read. People & access then says so instead of showing a choice nobody made)
          const ns = await api('GET', '/node/settings').catch((e) => ({ contact: '', discovery: false, unread: (e && e.message) || 'the node didn’t answer' }));
          // The homeserver bundled with the node (deploy/bundle) and its Cloudflare Tunnel, when there is one
          const bundled = await api('GET', '/homeserver/bundled').catch(() => null);
          const tunnel = await api('GET', '/remote/tunnel').catch(() => null);
          const hsHere = !!(bundled && bundled.configured && bundled.reachable);
          let pk = null; try { pk = await api('GET', '/pubky'); state.pubkyStatus = pk; state.pubkyUser = pk.user || null; pubkyHeard(); } catch { /* no pubky config */ }
          const fresh = (c) => c && Date.now() - c.at < 30000;
          const [nodeMap, prefs, apps, nodeHome] = await Promise.all([
            api('GET', '/node').catch(() => null),
            fresh(state.prefsCache) ? state.prefsCache.v : (() => { const gen = state.prefsGen || 0; return api('GET', '/me/prefs').then((v) => { if (gen === (state.prefsGen || 0)) state.prefsCache = { at: Date.now(), v }; return v; }).catch(() => null); })(), // an answer older than a change isn't kept
            api('GET', '/node/apps').catch(() => null),
            api('GET', '/home/layout').catch(() => null),
          ]);
          if (state.view !== 'settings') return;
          const saved = Object.fromEntries(((nodeMap && nodeMap.groups) || []).map((g) => [g.id, g.saved]));
          // One topic per tab, settings as rows (STEWARD-UX-SETTINGS.md). Every panel is in the page
          // (their controls stay wired); only the chosen one shows. On a phone: the list, then a tab.
          // The newest outputs, not the answer fetched before the awaits above (a change since wins)
          const outs = state.outputs || outs0; const myOut = appOutput(); const dv = myOut && myOut.device; state.dacDrawn = appDevice();
          const tabs = sideTabs({ view: 'settings', title: 'Settings', tabs: settingsTabs({ saved, bundled: !!(bundled && bundled.configured), tunnel: !!(tunnel && tunnel.available), dac: dv ? hereDevice() : null }), prefix: 'set', ids: { backup: 'backupcard' }, remembered: state.settingsTab });
          const { phone, active } = tabs;
          if (active) state.settingsTab = active;
          const help = (h) => (h ? `<p class="set-help">${h}</p>` : '');
          const row = (label, h, ctl, cls = '') => `<div class="set-row ${cls}"><div class="set-label"><div class="set-name">${label}</div>${help(h)}</div>${ctl ? `<div class="set-ctl">${ctl}</div>` : ''}</div>`;
          const block = (label, h, body, id = '') => `<div class="set-row set-block"${id ? ` id="${id}"` : ''}>${label || h ? `<div class="set-label">${label ? `<div class="set-name">${label}</div>` : ''}${help(h)}</div>` : ''}<div class="set-body">${body}</div></div>`;
          const group = (title, body) => `<div class="set-group">${title ? `<h3 class="set-h">${title}</h3>` : ''}<div class="set-rows">${body}</div></div>`;
          const panel = (k, html) => tabs.panel(k, html);
          state.devOutputs = !!ns.dev_outputs;
          const shownOuts = outs.filter((o) => state.devOutputs || !devOutput(o.id));
          const outList = shownOuts.length ? `<div class="set-list">${shownOuts.map((o) => `<div class="set-item"><div class="grow"><div class="name">${esc(o.name)}</div><div class="sub">${esc(o.id)}</div></div><span class="muted small caps">${o.caps.max_pcm_bits}-bit · ${o.caps.pcm_rates.map((r) => (r / 1000).toFixed(1).replace('.0', '') + 'k').join(' ')}${o.caps.dsd ? ' · DSD' : ''}</span></div>`).join('')}</div>` : '<p class="muted small">No outputs found.</p>';
          const zoneRows = zoneList().map((z) => row(esc(z.name), esc(z.id === BROWSER ? deviceWords(z) : z.output || ''), `${zoneStatePill(z)}${z.id !== BROWSER ? `<button class="btn sm ${state.dspOpen === z.id ? 'primary' : 'ghost'}" data-dsp="${esc(z.id)}">DSP</button>` : ''}${String(z.id).startsWith('group:') ? `<button class="btn sm ghost" data-zone-del="${esc(z.id)}">Remove</button>` : ''}`)).join('');
          const personalPlayFrom = group('Play from', block('Where songs play from', 'Your personal source order applies to every collection. A pinned song keeps its source.', '<div id="gordercard"><div id="gorder"><div class="skeleton" style="height:120px"></div></div></div>'));
          const playback = group('Playback', ''
            + row('Prefer full tracks', 'On a service\u2019s song page, Play plays that service, even a 30 s preview. With this on, a preview gives way to the full track from your Play from order.', `<input type="checkbox" class="switch" role="switch" id="preferfull" ${ns.prefer_full ? 'checked' : ''} aria-label="Prefer full tracks">`))
            + group('Speakers and outputs', block('', `What this node can play to, found on your network. <a class="lnk" id="spkguide" href="${SPEAKERS_GUIDE}" target="_blank" rel="noopener">Which speakers work</a>`, `${outList}<div class="toolbar" style="margin-top:10px"><button class="btn" id="rescan" title="Look again for speakers, streamers and devices on your network">Look for speakers again</button><button class="btn ghost" id="netscan" title="Ask every UPnP and Cast device on the network and explain what answers">${icon('search')} Explain what is on the network</button></div><div id="netscanout"></div>`)
              + block('Add a renderer by address', 'For a UPnP speaker or streamer the network scan doesn’t find: its IP address or its description URL. For a Linn: its IP address, MAC address and serial number, with spaces between.', `<div class="toolbar"><input class="input" id="rendaddr" placeholder="192.168.1.20" aria-label="Renderer address"><button class="btn" id="rendadd">${icon('plus')} Add renderer</button></div><div id="rendsaved" class="muted small"></div>`))
            + group('Zones', zoneRows + '<div id="dspbox"></div>'
              + block('Group outputs', 'Pick two or more of one kind to play as one zone, in sync; the first one checked leads.', shownOuts.filter(groupKind).length > 1 ? `<div id="groupouts" role="group" aria-label="Group outputs">${GROUP_KINDS.map(([k, label]) => { const os = shownOuts.filter((o) => groupKind(o) === k); return os.length ? `<div class="groupkind" role="group" aria-labelledby="groupkind-${k}"><div class="small muted" id="groupkind-${k}">${label}</div><div class="chips">${os.map((o) => `<label class="chip"><input type="checkbox" value="${esc(o.id)}" style="margin:0 6px 0 0"> ${esc(o.name)}</label>`).join('')}</div></div>` : ''; }).join('')}</div><p class="small muted" id="groupsync" role="status"></p><div class="toolbar" style="margin-top:10px"><input class="input" id="groupname" placeholder="Group name (Everywhere)" aria-label="Group name"><button class="btn" id="groupmake">${icon('plus')} Create group</button></div>` : '<p class="muted small">No two speakers of one kind yet. Sonos, HEOS and BluOS speakers group with their own kind, squeezelite players with each other, and so do the app\u2019s players on a DAC.</p>', 'groupcard'))
            + group('Subsonic apps', row('Jukebox zone', 'The zone Subsonic apps (Symfonium, Feishin…) control with their jukebox mode.', `<select class="input" id="jukeboxzone" aria-label="Subsonic jukebox zone"><option value="" ${ns.jukebox_zone ? '' : 'selected'}>First zone by name</option>${zoneList().filter((z) => z.id !== BROWSER).map((z) => `<option value="${esc(z.id)}" ${ns.jukebox_zone === z.id ? 'selected' : ''}>${esc(z.name)}</option>`).join('')}</select>`));
          const accounts = group('Pubky', pk ? (
              (!pk.user && hsHere ? row('Create your Pubky account on this node\u2019s homeserver', 'Your collections, bookmarks, tags and files stay on this machine. Pubky Ring creates the account here and signs this node in.', '<button class="btn primary" id="pkhscreate">Create account here</button>') : '')
              + row(pk.user ? 'Signed in' : hsHere ? 'Use an existing homeserver' : 'Not signed in', pk.user ? keyHtml(pk.user) : hsHere ? 'Already have a Pubky account somewhere else? Sign in with Pubky Ring instead.' : 'Sign in with the Pubky Ring app to keep your collections, bookmarks and tags in your own Pubky storage.', pk.user ? (pk.needs_resign && !state.token ? '<button class="btn primary" id="pklogin">Approve in Pubky Ring</button>' : '') : `<button class="btn ${hsHere ? 'ghost' : 'primary'}" id="pklogin">Sign in with Pubky</button>`)
              + (pk.needs_resign ? resignNotice(pk).replace('<div class="notice warn resign"', '<div id="pkresign" class="notice warn resign"') : '')
              + '<div id="pkurl"></div>'
              + (pk.user ? row('Your music', 'Your files on the homeserver, saving and tagging are in Your library.', '<button class="btn ghost" data-view-go="library">Open Your library</button>') : row('Saving', 'Starts once you sign in: then your collections, bookmarks and tags are kept on your homeserver.', ''))
            ) : row('Pubky', 'Pubky is not set up on this node.', ''))
            + group('ListenBrainz', block('', '', '<div id="lbcard"><div id="lbbody"><div class="skeleton" style="height:80px"></div></div></div>'))
            + group('Last.fm', block('', '', '<div id="lfcard"><div id="lfbody"><div class="skeleton" style="height:80px"></div></div></div>'))
            + group('Streaming services', row('TIDAL, Spotify, Deezer, Jamendo, your own servers', 'Sign in to them on Sources.', '<button class="btn ghost" data-goto-sources>Open Sources</button>'));
          const discover = group('Contact', block('Your email or website', 'The music index (Pinakes), MusicBrainz and Wikipedia ask each app for a way to reach whoever runs it. It goes only to them, and turns on what people on Pubky play.', `<div class="toolbar"><input class="input" id="contact" placeholder="you@example.org or https://your.site" value="${esc(ns.contact)}" aria-label="Your email or website"><button class="btn primary" id="contactsave">Save</button><span class="pill ${ns.discovery ? 'ok' : 'warn'}">${ns.discovery ? 'Discover on' : 'Discover off'}</span></div>`))
            + group('Concerts', block('Your city', 'For concerts near you on Home. Looked up once on OpenStreetMap; it stays on this node.', '<div class="toolbar"><input class="input" id="loccity" placeholder="City" aria-label="Your city"><button class="btn" id="locsave">Save</button><button class="btn ghost" id="locclear">Clear</button><span class="muted small" id="loccur"></span></div>'));
          const backup = group('Download a backup', block('', 'One file with everything needed to rebuild this node: matches and corrections, collections, bookmarks, tags, play history, queues and settings. Never your music files.', `<form id="bkform" class="stack"><label class="small" for="bkpass">Passphrase for your sign-ins (TIDAL, Spotify, ListenBrainz, Pubky…)</label><input class="input" id="bkpass" type="password" autocomplete="new-password" placeholder="Leave empty to leave sign-ins out"><input class="input" id="bkpass2" type="password" autocomplete="new-password" placeholder="The same passphrase again" aria-label="Repeat the passphrase" hidden><p class="muted small">Without it you can restore everything else and sign in to those services again. Nothing opens the sign-ins without it.</p><div class="actions"><button class="btn primary" id="bkdown" type="submit">${icon('download')} Download a backup</button></div></form>`))
            + group('Every day or week, to a folder', block('', '', '<div id="bksched"><div class="skeleton" style="height:60px"></div></div>'))
            + group('Restore', block('', 'Replaces this node’s library and settings with the backup’s. Your music files are not touched.', `<label class="btn file-btn">${icon('upload')} Choose a backup file<input type="file" id="rsfile" accept=".pmbackup,.zip"></label><div id="rsout"></div>`));
          const appearance = group('Theme', row('Light or dark', 'Or follow this device.', `<div class="chips themeswitch" role="radiogroup" aria-label="Theme">${[['system', 'System'], ['light', 'Light'], ['dark', 'Dark']].map(([v, l]) => `<button class="chip scope ${themePref() === v ? 'on' : ''}" role="radio" aria-checked="${themePref() === v}" data-theme-pick="${v}">${icon(v === 'dark' ? 'moon' : v === 'light' ? 'sun' : 'settings')} ${l}</button>`).join('')}</div>`)) + layoutGroup(group, row) + colourGroup(group, row);
          const about = group('This node', row('Name', '', esc(info.name)) + row('Version', '', esc(info.version)) + row('Library', '', `${countOf(info.library.tracks, 'songs')} · ${countOf(info.library.albums, 'albums')} · ${countOf(info.library.artists, 'artists')}`) + row('Files', '', `${info.library.files}${info.library.missing_files ? ` · <span class="warn-text">${info.library.missing_files} missing</span>` : ''}`))
            // (Log out, the browsers and apps signed in as you and the password for apps are on People & access)
            + group('Access', row('Log in, sign-ins and the password for apps', 'Who can use this node, the browsers and apps signed in as you, Log out and the password for apps are in one place.', '<button class="btn ghost" data-settab="people">Open People &amp; access</button>'))
            + group('For developers', row('Developer outputs', 'Show the Null output (it plays to nowhere, for testing) in the zone lists.', `<input type="checkbox" class="switch" role="switch" id="devouts" ${ns.dev_outputs ? 'checked' : ''} aria-label="Developer outputs">`))
            + group('Keyboard', block('', '', '<p class="muted small">Shortcuts: <kbd>/</kbd> search · <kbd>Space</kbd> play/pause · <kbd>←</kbd> <kbd>→</kbd> previous/next · <kbd>N</kbd> now playing · <kbd>Esc</kbd> close</p>'));
          // You › Home: whose layout applies; edit it on Home itself
          const applies = (a) => (a === 'yours' ? 'Yours' : 'The node’s default');
          const homeL = prefs && prefs.home_layout;
          const homeTab = group('Your Home', row('Rows on Home', `${homeL ? `${applies(homeL.applies)}. ` : ''}Which rows show and in what order; change them with Edit Home on Home.`, `<button class="btn ghost" id="sethomeedit">${icon('settings')} Edit Home</button>${homeL && homeL.applies === 'yours' ? ' <button class="btn ghost" id="sethomereset">Use the node’s default</button>' : ''}`)
            + row('Saved on', prefs ? (prefs.saved_on === 'homeserver' ? 'Your homeserver, so it follows you to every node and device.' : 'This node for now; it goes to your homeserver once it can be reached.') : '', ''));
          // This node › Home for everyone: the default, which Edit Home no longer writes (#123 follow-up).
          // Yours is what Home holds right now when it has loaded (a save may still be on its way), else the prefs answer.
          const mine = state.homeLayout || (homeL && homeL.value) || null;
          const shape = (l) => JSON.stringify([l.order || [], [...(l.hidden || [])].sort()]);
          const same = (x, y) => !!x && !!y && shape(x) === shape(y);
          const rowsOf = (l) => { if (!l) return 'Couldn’t read the node’s default'; const o = (l.order || []).length; const h = (l.hidden || []).length; if (!o && !h) return 'The built-in rows, in the built-in order'; return `${o ? `${countOf(o, 'rows')} arranged` : 'The built-in order'}${h ? `, ${countOf(h, 'rows')} hidden` : ''}`; };
          const canSetDefault = !!mine && (!homeL || homeL.applies === 'yours') && !same(mine, nodeHome); // only a layout of your own; a copy of the default may be old
          const whyDefault = !mine ? 'Your Home couldn’t be read.' : canSetDefault ? 'Copies your Home’s rows and order to the node; your own stays yours.' : homeL && homeL.applies !== 'yours' ? 'You haven’t arranged your own Home yet; use Edit Home on Home first.' : 'Your Home already is the default.';
          const nodeHomeTab = group('The default Home', row('Rows on Home for everyone', `${rowsOf(nodeHome)}. What Home shows to everyone who hasn’t arranged their own.`, '')
            + row('Make yours the default', `<span id="sethomedefault-why">${whyDefault}</span>`, `<button class="btn ghost" id="sethomedefault" aria-describedby="sethomedefault-why" ${canSetDefault ? '' : 'disabled'}>${icon('home')} Make my Home the default for everyone</button>`))
            + group('Get started', row('First steps on Home', ns.first_steps_hidden ? 'Hidden for everyone on this node.' : 'Shown on Home until its steps are done or you hide them.', `<input type="checkbox" class="switch" role="switch" id="firststeps" ${ns.first_steps_hidden ? '' : 'checked'} aria-label="First steps on Home">`));
          // You › Playback quality: your choice per service over the node's
          const qEntries = Object.entries((prefs && prefs.quality) || {});
          const qualityTab = group('Per service', qEntries.length
            ? qEntries.map(([src, q]) => row(esc(SERVICE_NAMES[src] || srcName(src)), applies(q.applies), `<span class="pill">${esc(String(q.value == null ? '—' : q.value))}</span>${q.applies === 'yours' ? ` <button class="btn ghost sm" data-qreset="${esc(src)}">Use the node’s</button>` : ''}`)).join('')
            : block('', 'Each service plays at the quality set on its page in Sources. Your own choice per service, over the node’s, comes here with the services’ quality choices.', '<button class="btn ghost" data-goto-sources>Open Sources</button>'));
          // This node › Service apps: the apps the admin sets once for everyone
          // Built in, or the admin's own app; the service's limits in plain words
          const appFrom = (a) => a.from === 'settings' ? 'Your own app' : a.from === 'builtin' ? 'Built in' : a.from === 'config' ? 'Your own app, set in node.toml' : a.from ? 'Set' : '';
          // Only what can be done here (owner, 2026-11-08): no row for an app with nothing to set and
          // none in use, and no own-app form where the service hands none out (Qobuz, TIDAL's device
          // clients); one saved here before keeps its Remove
          const NO_OWN_APP = new Set(['qobuz', 'tidal-full']);
          const ownAppForm = (a) => !!a.set && !NO_OWN_APP.has(a.id);
          const appShown = (a) => a.configured || !!a.page || ownAppForm(a);
          const appAdvanced = (a) => !ownAppForm(a) ? '' : `<details class="small app-adv"><summary>Use your own app</summary><form class="toolbar" data-appform="${esc(a.id)}">${(a.fields || []).map((f) => `<label class="sr-only" for="app-${esc(a.id)}-${esc(f.name)}">${esc(a.name)} ${esc(f.label)}</label><input id="app-${esc(a.id)}-${esc(f.name)}" name="${esc(f.name)}" class="input" type="${f.secret ? 'password' : 'text'}" autocomplete="off" spellcheck="false" placeholder="${esc(f.label)}${f.optional ? ' (if your app has one)' : ''}" ${f.optional ? '' : 'required'}>`).join('')}<button class="btn primary">Save</button></form>${(a.redirects || []).length ? `<p class="muted small">Your app lists ${a.redirects.length > 1 ? 'one of these redirect addresses (the second only with a secret)' : 'this redirect address'}:</p><ul class="app-redirects">${a.redirects.map((r) => `<li><code class="path">${esc(r)}</code></li>`).join('')}</ul>` : ''}${webUrl(a.get_one) ? `<p class="muted small"><a href="${esc(webUrl(a.get_one))}" target="_blank" rel="noopener">${icon('external')} Where to make one</a></p>` : ''}</details>`;
          // An app whose value lives on its own page (the AcoustID key with Identify): the way there, no form
          const appGo = (a) => a.page === 'library:node' ? ` <button class="btn ghost sm" data-applib="node">Open Node library</button>` : '';
          const appRow = (a) => `<div class="set-row"><div class="set-label"><div class="set-name">${esc(a.name)}</div>${help([appFrom(a), a.note ? esc(a.note) : ''].filter(Boolean).join(' · '))}${a.limits ? (() => { const [x, y] = splitNote(a.limits); return help(esc(x)) + (y ? `<details class="small morenote"><summary>More</summary><p class="set-help">${esc(y)}</p></details>` : ''); })() : ''}${appAdvanced(a)}</div><div class="set-ctl">${a.configured ? `<span class="pill ok">${a.from === 'builtin' ? 'Built in' : 'Set'}</span>` : '<span class="pill">Not set up</span>'}${a.from === 'settings' && a.set && (a.fields || []).length ? ` <button class="btn ghost sm" data-appreset="${esc(a.id)}">${a.builtin ? 'Use built-in' : 'Remove'}</button>` : ''}${appGo(a)}</div></div>`;
          // Whose TIDAL, Qobuz and Spotify sign-ins people play with (owner, 2026-11-03) is chosen on
          // People & access, with the rest of what the people you add can do: one line points there
          const policyTab = group('For the people you add', row('Whose music services they play with', 'This node’s TIDAL, Qobuz and Spotify sign-ins or their own: chosen with the rest of what they can do.', '<button class="btn ghost" data-settab="people">Open People &amp; access</button>'));
          const appsTab = policyTab + group('Set once, for everyone on this node', apps && apps.length ? apps.filter(appShown).map(appRow).join('') : block('', 'No list from this node.', ''));
          // This homeserver: the bundled one
          const hsTab = bundled && bundled.configured ? group('The homeserver', row('Address', esc(bundled.icann_domain || ''), bundled.pubky_address ? `<code class="small">${esc(bundled.pubky_address)}</code>` : '')
            + row('Status', '', bundled.reachable ? `<span class="pill ok">Running${bundled.version ? ` · ${esc(bundled.version)}` : ''}</span>` : `<span class="pill bad">Not reachable</span>${bundled.error ? ` <span class="muted small">${esc(bundled.error)}</span>` : ''}`)
            + row('People', '', `${countOf(bundled.users || 0, 'accounts')}${bundled.disabled_users ? ` · ${bundled.disabled_users} turned off` : ''}`)
            + row('Invites', '', countOf(bundled.unused_invites || 0, 'unused invites'))
            + row('Storage', '', bundled.disk_used_mb != null ? `${bundled.disk_used_mb} MB used` : '—')) : '';
          // This phone: the USB DAC this app plays through, what it takes, how it is played and who
          // sets its level (read-only: one behaviour)
          // The zone on this output: what its DAC does with what plays now
          const dz = myOut ? zoneList().find((z) => z.output === myOut.id) : null;
          const dline = dz && dz.now_playing ? dacLine(dz) : '';
          const dacTab = !dv ? '' : group(esc(dv.name), row('Playing through it', esc(dline ? `${dline}.${/^Shared/.test(dline) ? ` ${DAC_HELP.shared}` : ''}` : `${DAC_MODE[dacMode(dv)]}.${dacMode(dv) === 'shared' ? ` ${DAC_HELP.shared}` : ''}`), '')
            + (dacUnconfirmed(dv) ? `<div class="notice" role="status">${esc(dacUnconfirmed(dv))}</div>` : '')
            + (dacCaps(dv) ? row('What it takes', esc(dacCaps(dv)), '') : '')
            + (dv.note ? `<div class="notice" role="status">${esc(dv.note)}</div>` : '')
            // Who sets the level, read-only: one behaviour, nothing to choose
            + (dacVolumeWords(dv) ? row(esc(dacVolumeWords(dv)), dv.own_volume ? 'The app’s slider, the volume keys and Android’s volume panel set the DAC’s own volume; the music itself stays untouched. The first sound is quiet: the level you last played this DAC at, else a low one.' : 'This DAC has no volume of its own, so the app has no slider for it: set the level on the DAC or the amplifier. The music stays bit-perfect.', '') : '')
            // Android's permission for Exclusive USB (the app's own USB driver) was not given: ask again
            + (dv.usb_permission === false ? row('Exclusive USB', 'This app can drive the DAC itself with its own USB driver: bit-perfect on any Android, at the DAC’s own volume. Android asks you once to allow it.', '<button class="btn ghost sm" id="dvallow">Allow Exclusive USB</button>') : ''));
          main.innerHTML = tabs.page(`${dv ? panel('dac', dacTab) : ''}${panel('playfrom', personalPlayFrom)}${panel('home', homeTab)}${panel('quality', qualityTab)}${panel('apps', appsTab)}${panel('nodehome', nodeHomeTab)}${hsTab ? panel('homeserver', hsTab) : ''}${panel('playback', playback)}${panel('accounts', accounts)}${panel('people', '<div id="peoplebox" class="pa-box"><div class="skeleton" style="height:160px"></div></div>')}${tunnel && tunnel.available ? panel('remote', '<div id="remotebox"><div class="skeleton" style="height:160px"></div></div>') : ''}${panel('discover', discover)}${panel('backup', backup)}${panel('appearance', appearance)}${panel('about', about)}${panel('privacy', group('Usage insights', '<div id="usageme"></div>'))}${panel('usage', group('Usage insights', '<div id="usagenode"></div>'))}`);
          const showTab = tabs.wire(main, (k) => { state.settingsTab = k; });
          // A row that points to another tab: the focus goes with it, to that tab's button
          const goTab = (b) => { b.onclick = () => { const k = b.dataset.settab; showTab(k); const t = $(`#settab-${k}`, main); if (t && !phone) t.focus(); }; };
          $$('[data-settab]', main).forEach(goTab);
          // Usage insights (usage.js): yours, and the node's switch
          if (window.PM_USAGE) { PM_USAGE.renderPersonal($('#usageme', main)); PM_USAGE.renderNode($('#usagenode', main)); }
          // This phone: "Allow Exclusive USB" goes to the node, which passes it to the app's player,
          // and Android shows its dialog
          const dva = $('#dvallow', main); if (dva) dva.onclick = async () => { dva.disabled = true; try { await api('PUT', `/outputs/${encodeURIComponent(myOut.id)}/device`, { mode: 'exclusive' }); toast('Allow the DAC when Android asks: Exclusive USB from the next song'); refreshOutputs(); } catch (e) { dva.disabled = false; toast(e.message, 'error'); } };
          $$('[data-goto-sources]', main).forEach((b) => b.onclick = () => setView('sources'));
          $$('[data-view-go]', main).forEach((b) => b.onclick = () => setView(b.dataset.viewGo));
          const he = $('#sethomeedit'); if (he) he.onclick = () => { state.homeEdit = true; setView('home'); };
          // Service apps: Save sends the fields to the app's own route; Use built-in sends its first field empty
          const appSend = async (a, body, msg) => { const [method, path] = a.set.split(' '); await api(method, path.replace(/^\/v1/, ''), body); toast(msg); render(); };
          $$('[data-appform]').forEach((f) => { f.onsubmit = async (e) => { e.preventDefault(); const a = (apps || []).find((x) => x.id === f.dataset.appform); if (!a) return; const body = {}; for (const fl of a.fields || []) { const v = (f.elements[fl.name] && f.elements[fl.name].value || '').trim(); if (v || !fl.optional) body[fl.name] = v; } const b = f.querySelector('button'); b.disabled = true; try { await appSend(a, body, `${a.name}: your own app is in use`); } catch (err) { b.disabled = false; toast(err.message, 'error'); } }; });
          $$('[data-applib]', main).forEach((b) => b.onclick = () => setView('library', b.dataset.applib));
          $$('[data-appreset]').forEach((b) => { b.onclick = async () => { const a = (apps || []).find((x) => x.id === b.dataset.appreset); if (!a) return; b.disabled = true; try { await appSend(a, { [a.fields[0].name]: '' }, a.builtin ? `${a.name}: the built-in app is in use` : `${a.name}: your app is removed`); } catch (err) { b.disabled = false; toast(err.message, 'error'); } }; });
          const hd = $('#sethomedefault'); if (hd) hd.onclick = async () => { hd.disabled = true; try { await api('PUT', '/home/layout', { order: mine.order || [], hidden: mine.hidden || [] }); toast('Your Home is the default for everyone now'); render(); } catch (e) { hd.disabled = false; toast(e.message, 'error'); } };
          const hr = $('#sethomereset'); if (hr) hr.onclick = async () => { try { await api('PUT', '/me/prefs', { home_layout: null }); state.homeLayout = null; state.homeSaved = null; toast('Home follows the node’s default'); render(); } catch (e) { toast(e.message, 'error'); } };
          $$('[data-qreset]', main).forEach((b) => b.onclick = async () => { try { await api('PUT', '/me/prefs', { quality: { [b.dataset.qreset]: null } }); toast('Back to the node’s quality'); render(); } catch (e) { toast(e.message, 'error'); } });
          const pff = $('#preferfull'); if (pff) pff.onchange = async () => { try { await api('PUT', '/node/settings', { prefer_full: pff.checked }); toast(pff.checked ? 'Previews give way to full tracks from your Play from order' : 'A song page plays its own service, previews included'); } catch (e) { toast(e.message, 'error'); pff.checked = !pff.checked; } };
          const fst = $('#firststeps'); if (fst) fst.onchange = async () => { try { await api('PUT', '/node/settings', { first_steps_hidden: !fst.checked }); toast(fst.checked ? 'First steps show on Home again' : 'First steps are hidden for everyone'); render(); } catch (e) { toast(e.message, 'error'); fst.checked = !fst.checked; } };
          const dvo = $('#devouts'); if (dvo) dvo.onchange = async () => { try { await api('PUT', '/node/settings', { dev_outputs: dvo.checked }); state.devOutputs = dvo.checked; toast(dvo.checked ? 'Developer outputs are shown' : 'Developer outputs are hidden'); renderZoneName(); } catch (e) { toast(e.message, 'error'); dvo.checked = !dvo.checked; } };
          // The first sync after a sign-in runs in the background: follow it here
          if (pk && pk.sync && pk.sync.running) { clearTimeout(state.syncPoll); state.syncPoll = setTimeout(() => { if (state.view === 'settings' && state.settingsTab === 'accounts') render(); }, 2000); }
          $$('[data-zone-del]', main).forEach((b) => b.onclick = async () => { try { await api('DELETE', `/zones/${encodeURIComponent(b.dataset.zoneDel)}`); toast('Group removed'); if (state.zone === b.dataset.zoneDel) state.zone = BROWSER; render(); } catch (e) { toast(e.message, 'error'); } });
          // What the outputs picked become, and which can't join them (no sample rate in common)
          const groupBox = $('#groupouts');
          const order = []; // the order they were checked in: the first leads
          if (groupBox) {
            const byId = Object.fromEntries(shownOuts.map((o) => [o.id, o]));
            const update = () => {
              const picked = order.map((id) => byId[id]);
              const rates = picked.length ? picked.reduce((r, o) => r.filter((x) => o.caps.pcm_rates.includes(x)), picked[0].caps.pcm_rates) : null;
              const kind = picked.length ? groupKind(picked[0]) : null;
              const noRate = []; // of the picked kind but no sample rate in common: said under the speakers, not only in a title a touch screen never shows
              $$('input', groupBox).forEach((i) => { const o = byId[i.value]; const other = !i.checked && !!kind && groupKind(o) !== kind; const off = other || (!i.checked && (kind === 'node' || kind === 'net') && !o.caps.pcm_rates.some((x) => rates.includes(x))); if (off && !other) noRate.push(o.name); i.disabled = off; i.closest('label').title = other ? 'Another kind: speakers group with their own kind' : off ? 'Shares no sample rate with all of the ones picked' : ''; });
              const many = noRate.length > 1;
              const why = noRate.length ? `${listWords(noRate)} ${many ? 'share' : 'shares'} no sample rate with ${picked.length === 1 ? picked[0].name : 'all of the ones picked'}, so ${many ? 'they' : 'it'} can’t join.` : '';
              $('#groupsync').textContent = [groupSync(picked), why].filter(Boolean).join(' ');
            };
            $$('input', groupBox).forEach((i) => i.onchange = () => { const at = order.indexOf(i.value); if (i.checked && at < 0) order.push(i.value); if (!i.checked && at >= 0) order.splice(at, 1); update(); });
          }
          const gm = $('#groupmake'); if (gm) gm.onclick = async () => { const ids = [...order]; const name = $('#groupname').value.trim() || 'Group'; if (ids.length < 2) { toast('Pick at least two outputs', 'error'); return; } try { const z = await api('POST', '/zones', { name, outputs: ids }); state.zones[z.id] = z; toast(`Group “${name}” created`); render(); } catch (e) { toast(e.message, 'error'); } };
          $$('[data-dsp]', main).forEach((b) => b.onclick = () => { state.dspOpen = state.dspOpen === b.dataset.dsp ? null : b.dataset.dsp; render(); });
          if (state.dspOpen && state.zones[state.dspOpen]) {
            try { const cfg = await api('GET', `/zones/${encodeURIComponent(state.dspOpen)}/dsp`); $('#dspbox').innerHTML = dspEditor(state.zones[state.dspOpen], cfg); bindDspEditor(state.dspOpen, cfg); } catch (e) { $('#dspbox').innerHTML = `<div class="notice warn">${esc(e.message)}</div>`; }
          }
          const saveContact = async () => { const c = $('#contact').value.trim(); if (!c) return; try { await api('PUT', '/node/settings', { contact: c }); state.discoveryOff = false; homeCache.clear(); toast('Contact saved; Discover is on'); render(); } catch (e) { toast(e.message, 'error'); } };
          $('#contactsave').onclick = saveContact; $('#contact').onkeydown = (e) => { if (e.key === 'Enter') $('#contactsave').click(); };
          api('GET', '/home/location').then((l) => { const c = $('#loccur'); if (c) c.textContent = l ? `Now: ${l.label || `${l.lat.toFixed(2)}, ${l.lon.toFixed(2)}`}` : 'Not set'; }).catch(() => {});
          $('#locsave').onclick = async () => { const city = $('#loccity').value.trim(); if (!city) return; try { const l = await api('PUT', '/home/location', { city }); toast(`Concerts near ${l.label}`); render(); } catch (e) { toast(e.message, 'error'); } };
          $('#locclear').onclick = async () => { try { await api('PUT', '/home/location', {}); toast('City cleared'); render(); } catch (e) { toast(e.message, 'error'); } };
          $('#rescan').onclick = async () => { try { await api('POST', '/outputs/rescan'); toast('Outputs rescanned'); render(); } catch (e) { toast(e.message, 'error'); } };
          const showSaved = async () => { try { const r = await api('GET', '/outputs/discovery?quick=1'); const box = $('#rendsaved'); if (!box) return; box.innerHTML = r.saved_addresses.length ? 'Added by address: ' + r.saved_addresses.map((a) => `<span class="rendrow"><code>${esc(a)}</code> <button class="link" data-rendrm="${esc(a)}" aria-label="Forget ${esc(a)}">forget</button></span>`).join('<span class="rendsep"> · </span>') : ''; $$('[data-rendrm]', box).forEach((b) => b.onclick = async () => { try { await api('DELETE', `/outputs/upnp/${encodeURIComponent(b.dataset.rendrm)}`); toast('Address forgotten (its zone stays until restart)'); showSaved(); } catch (e) { toast(e.message, 'error'); } }); } catch { /* older node */ } };
          showSaved();
          needsText($('#rendaddr'), $('#rendadd'));
          const ra = $('#rendadd'); if (ra) ra.onclick = async () => {
            const address = $('#rendaddr').value.trim(); if (!address) return; ra.disabled = true;
            try { const outs = await api('POST', '/outputs/upnp', { address }); toast(`Renderer added · ${outs.length} outputs`); $('#rendaddr').value = ''; render(); } catch (e) { toast(e.message, 'error'); }
            ra.disabled = false;
          };
          const sc = $('#netscan'); if (sc) sc.onclick = async () => {
            const box = $('#netscanout'); sc.disabled = true; box.innerHTML = '<p class="muted small">Asking the network (about 5 s)…</p>';
            try {
              const r = await api('GET', '/outputs/discovery');
              box.innerHTML = (r.notes.length ? r.notes.map((n) => `<div class="notice">${esc(n)}</div>`).join('') : '')
                + `<p class="muted small">Scanned from <strong>${esc(r.node || '')}</strong>${r.local_address ? ` at <code>${esc(r.local_address)}</code>` : ''} · audio is fetched from <code>${esc(r.public_url)}</code></p>`
                + (r.upnp.length ? `<div class="list">${r.upnp.map((d) => `<div class="list-row"><div class="grow"><div class="name">${esc(d.friendly_name || d.location)}</div><div class="sub">${esc(d.from)}${d.server ? ' · ' + esc(d.server) : ''}</div></div><span class="pill ${d.renderer ? 'ok' : ''}">${d.renderer ? (d.output ? 'renderer · output' : 'renderer') : 'not a renderer'}</span></div>`).join('')}</div>` : (r.upnp_enabled ? '' : '<p class="muted small">UPnP discovery is off.</p>'))
                + (r.cast.length ? `<div class="list">${r.cast.map((d) => `<div class="list-row"><div class="grow"><div class="name">${esc(d.friendly_name)}</div><div class="sub">${esc(d.addr)}</div></div><span class="pill ok">Cast</span></div>`).join('')}</div>` : '');
            } catch (e) { box.innerHTML = `<p class="muted small">${esc(e.message)}</p>`; }
            sc.disabled = false;
          };
          const jz = $('#jukeboxzone'); if (jz) jz.onchange = async () => { try { await api('PUT', '/node/settings', { jukebox_zone: jz.value }); toast(jz.value ? 'Jukebox zone saved' : 'Jukebox follows the first zone'); } catch (e) { toast(e.message, 'error'); } };
          $$('[data-theme-pick]', main).forEach((b) => b.onclick = () => { setTheme(b.dataset.themePick); $$('[data-theme-pick]', main).forEach((x) => { const on = x === b; x.classList.toggle('on', on); x.setAttribute('aria-checked', String(on)); }); }); bindLayoutPick();
          const loadLf = () => signinChooser(lastfmSigninCfg($('#lfbody')));
          loadLf();
          const loadLb = async () => {
            const box = $('#lbbody'); if (!box) return;
            let lb; try { lb = await api('GET', '/listenbrainz'); } catch (e) { box.innerHTML = `<p class="muted small">${esc(e.message)}</p>`; return; }
            if (!lb.connected) {
              box.innerHTML = `<p class="muted small">Send what you play to ListenBrainz, bring your listens here, and get its recommendations. Your user token is on <a href="https://listenbrainz.org/settings/" target="_blank" rel="noopener">listenbrainz.org/settings</a>; it stays on this node.</p><form class="row" id="lbform"><input class="input" name="token" type="password" placeholder="User token" autocomplete="off" style="flex:1;min-width:200px" required><button class="btn primary" id="lbconnect" type="submit">Connect</button></form>`;
              $('#lbform').onsubmit = async (e) => { e.preventDefault(); const b = $('#lbconnect'); b.disabled = true; try { await api('PUT', '/listenbrainz', { token: new FormData($('#lbform')).get('token') }); toast('ListenBrainz connected'); loadLb(); } catch (er) { toast(er.message, 'error'); b.disabled = false; } };
              return;
            }
            let pls = []; try { pls = await api('GET', '/listenbrainz/playlists'); } catch { /* none */ }
            box.innerHTML = `<div class="toolbar"><span class="pill ok account">Connected · ${esc(lb.user || '')}</span>${lb.pending ? `<span class="pill warn" title="ListenBrainz did not take them yet; they go with the next listen">${lb.pending} waiting</span>` : ''}</div>`
              + `<div class="set-row"><div class="set-label"><div class="set-name">Send what I play</div></div><div class="set-ctl"><input type="checkbox" class="switch" role="switch" id="lbsubmit" ${lb.submit ? 'checked' : ''} aria-label="Send what I play to ListenBrainz"></div></div>`
              + `<div class="toolbar" style="margin-top:8px"><button class="btn sm" id="lbimport">${icon('download')} Bring my listens here</button><button class="btn sm ghost" id="lbout">Disconnect</button></div>`
              + (pls.length ? `<p class="muted small" style="margin-top:12px">Made for you</p><div class="list">${pls.slice(0, 6).map((p) => `<div class="list-row"><div class="grow"><div class="name" title="${esc(p.name)}">${esc(p.name)}</div><div class="sub">${p.songs ? p.songs + ' songs' : 'ListenBrainz playlist'}</div></div><button class="btn sm" data-lbpl="${esc(p.id)}" aria-label="Import ${esc(p.name)}">${icon('download')} Import</button></div>`).join('')}</div>` : '');
            $('#lbsubmit').onchange = async (e) => { try { await api('PUT', '/listenbrainz', { submit: e.target.checked }); toast(e.target.checked ? 'Plays go to ListenBrainz' : 'Plays stay on this node'); } catch (er) { toast(er.message, 'error'); e.target.checked = !e.target.checked; } };
            $('#lbimport').onclick = async (e) => { e.currentTarget.disabled = true; try { const r = await api('POST', '/listenbrainz/import'); toast(`${r.imported} listen${r.imported === 1 ? '' : 's'} added to your history${r.skipped ? `, ${r.skipped} without a MusicBrainz id skipped` : ''}`); } catch (er) { toast(er.message, 'error'); } loadLb(); };
            $('#lbout').onclick = async () => { try { await api('DELETE', '/listenbrainz'); toast('ListenBrainz disconnected'); loadLb(); } catch (er) { toast(er.message, 'error'); } };
            $$('[data-lbpl]', box).forEach((b) => b.onclick = async () => { b.disabled = true; try { const r = await api('POST', `/listenbrainz/playlists/${b.dataset.lbpl}/import`); state.playlists = await api('GET', '/playlists'); toast(`Imported “${r.playlist.summary.name}”: ${countOf(r.imported, 'songs')}`); } catch (er) { toast(er.message, 'error'); } b.disabled = false; });
          };
          loadLb();
          // ---- backup and restore (steward next-4, item 1)
          const tb = $('#tobackup'); if (tb) tb.onclick = () => $('#backupcard').scrollIntoView({ behavior: glide(), block: 'start' });
          const authH = () => (state.token ? { authorization: 'Bearer ' + state.token } : {});
          const saveBlob = (blob, name) => { const a = document.createElement('a'); a.href = URL.createObjectURL(blob); a.download = name; document.body.appendChild(a); a.click(); setTimeout(() => { URL.revokeObjectURL(a.href); a.remove(); }, 1000); };
          $('#bkpass').oninput = () => { $('#bkpass2').hidden = !$('#bkpass').value; };
          $('#bkform').onsubmit = async (e) => {
            e.preventDefault();
            const p = $('#bkpass').value; if (p && p !== $('#bkpass2').value) { toast('The two passphrases differ', 'error'); return; }
            const b = $('#bkdown'); b.disabled = true;
            try {
              const res = await fetch('/v1/backup', { method: 'POST', headers: { ...authH(), ...(p ? { 'x-backup-passphrase': p } : {}) } });
              if (!res.ok) throw new Error((await res.json().catch(() => ({}))).error || res.status);
              const name = (/filename="([^"]+)"/.exec(res.headers.get('content-disposition') || '') || [])[1] || 'pubky-music.pmbackup';
              saveBlob(await res.blob(), name); toast(p ? 'Backup saved, sign-ins locked with your passphrase' : 'Backup saved (without sign-ins)');
            } catch (er) { toast(er.message, 'error'); }
            b.disabled = false;
          };
          const when = (t) => (t ? new Date(t * 1000).toLocaleString() : 'never');
          const nOf = (n, one, many) => `${n} ${n === 1 ? one : many}`;
          const countsLine = (c) => `${nOf(c.tracks, 'song', 'songs')} (${c.identified} identified) · ${nOf(c.playlists, 'collection', 'collections')} · ${nOf(c.favourites, 'bookmark', 'bookmarks')} · ${nOf(c.tags, 'tag', 'tags')} · ${nOf(c.plays, 'play', 'plays')}`;
          const loadSched = async () => {
            const box = $('#bksched'); if (!box) return;
            let sc; try { sc = await api('GET', '/backup/schedule'); } catch (er) { box.innerHTML = `<p class="muted small">${esc(er.message)}</p>`; return; }
            box.innerHTML = `<p class="muted small">A folder on the node, best on another disk or a NAS. The newest backups are kept, older ones removed.</p>
              <div class="toolbar"><input class="input" id="bkfolder" placeholder="/path/to/backups" value="${esc(sc.folder)}" aria-label="Backup folder on the node"></div>
              <div class="toolbar"><label class="small">Every <select class="input" id="bkevery" aria-label="How often">${[[24, 'day'], [168, 'week']].map(([h, l]) => `<option value="${h}" ${sc.every_hours === h ? 'selected' : ''}>${l}</option>`).join('')}</select></label><label class="small">Keep <input class="input" id="bkkeep" type="number" min="1" max="365" value="${sc.keep}" style="width:80px" aria-label="How many to keep"></label></div>
              <div class="toolbar"><input class="input" id="bkspass" type="password" autocomplete="new-password" placeholder="${sc.with_signins ? 'Passphrase saved: type a new one to change it' : 'Passphrase for sign-ins (optional)'}" aria-label="Passphrase for scheduled backups"></div>
              <div class="actions"><button class="btn" id="bksave">Save</button>${sc.folder ? '<button class="btn ghost" id="bknow">Back up now</button><button class="btn ghost" id="bkoff">Turn off</button>' : ''}</div>
              ${sc.folder ? `<p class="small ${sc.last_error ? 'warn-text' : 'muted'}" role="status">${sc.last_error ? `The last backup failed: ${esc(sc.last_error)}` : `Last backup: ${esc(when(sc.last_at))}${sc.last_file ? ` · ${esc(sc.last_file)}` : ''}`} · ${sc.files.length} in the folder · ${sc.with_signins ? 'with sign-ins' : 'without sign-ins'}</p>` : '<p class="muted small">Off.</p>'}`;
            const put = async (body, msg) => { try { await api('PUT', '/backup/schedule', body); toast(msg); loadSched(); } catch (er) { toast(er.message, 'error'); } };
            $('#bksave').onclick = () => { const f = $('#bkfolder').value.trim(); if (!f) { toast('Type the folder on the node', 'error'); return; } const pp = $('#bkspass').value; put({ folder: f, every_hours: Number($('#bkevery').value), keep: Number($('#bkkeep').value) || 7, ...(pp ? { passphrase: pp } : {}) }, 'Backups scheduled'); };
            const now = $('#bknow'); if (now) now.onclick = async () => { now.disabled = true; try { const r = await api('POST', '/backup/schedule/run'); toast(`Backup written: ${r.last_file}`); loadSched(); } catch (er) { toast(er.message, 'error'); now.disabled = false; } };
            const off = $('#bkoff'); if (off) off.onclick = () => put({ folder: '', passphrase: '' }, 'Scheduled backups off');
          };
          loadSched();
          $('#rsfile').onchange = async () => {
            const f = $('#rsfile').files[0]; const out = $('#rsout'); if (!f) return;
            const bytes = await f.arrayBuffer();
            out.innerHTML = '<div class="skeleton" style="height:60px"></div>';
            let info; try { const res = await fetch('/v1/restore?check=true', { method: 'POST', headers: { 'content-type': 'application/octet-stream', ...authH() }, body: bytes }); info = await res.json(); if (!res.ok) throw new Error(info.error || res.status); } catch (er) { out.innerHTML = `<div class="notice warn">${esc(er.message)}</div>`; return; }
            const c = info.counts;
            out.innerHTML = `<div class="notice" style="margin-top:10px"><p><b>${esc(f.name)}</b>: from “${esc(info.from_node)}”, ${esc(when(info.made_at))}.</p><p class="small">${countsLine(c)}</p>
              ${info.locked_signins ? '<label class="small" for="rspass">Its sign-ins are locked with a passphrase</label><input class="input" id="rspass" type="password" autocomplete="current-password" placeholder="Passphrase"><label class="check small"><input type="checkbox" id="rsskip"> Restore without the sign-ins</label>' : (info.left_out.length ? `<p class="small muted">Not in this backup: ${esc(info.left_out.join(', '))}. Sign in to those again afterwards.</p>` : '')}
              <div class="actions"><button class="btn primary" id="rsgo">Restore this backup</button></div></div><div id="rsreport"></div>`;
            $('#rsgo').onclick = async () => {
              const b = $('#rsgo');
              if (!b.dataset.armed) { b.dataset.armed = '1'; b.textContent = 'Replace this node’s library?'; setTimeout(() => { if (b.isConnected) { delete b.dataset.armed; b.textContent = 'Restore this backup'; } }, 4000); return; }
              b.disabled = true; b.textContent = 'Restoring…';
              const pass = $('#rspass') ? $('#rspass').value : ''; const skip = $('#rsskip') && $('#rsskip').checked;
              try {
                const res = await fetch(`/v1/restore${skip ? '?skip_secrets=true' : ''}`, { method: 'POST', headers: { 'content-type': 'application/octet-stream', ...authH(), ...(pass ? { 'x-backup-passphrase': pass } : {}) }, body: bytes });
                const r = await res.json(); if (!res.ok) throw new Error(r.error || res.status);
                const rc = r.restored;
                $('#rsreport').innerHTML = `<div class="notice ok-notice" role="status" style="margin-top:10px"><p><b>Restored.</b> ${countsLine(rc)}.${r.rescan_started ? ' Checking your music folders now; unchanged files keep their matches.' : ''}</p>
                  ${r.signed_in.length ? `<p class="small">Signed in again: ${esc(r.signed_in.join(', '))}.</p>` : ''}${r.sign_in_again.length ? `<p class="small">Sign in again: ${esc(r.sign_in_again.join(', '))}.</p>` : ''}${r.notes.map((n) => `<p class="small muted">${esc(n)}</p>`).join('')}
                  ${r.missing_folders.map((m, k) => `<div class="toolbar" style="margin-top:8px"><span class="small">Not on this computer: <code>${esc(m)}</code>. Where is that music now?</span><input class="input" data-mvto="${k}" placeholder="/path/to/the/music" aria-label="New place of ${esc(m)}"><button class="btn sm" data-mv="${k}">Use this folder</button></div>`).join('')}</div>`;
                $$('[data-mv]', out).forEach((mb) => mb.onclick = async () => { const k = Number(mb.dataset.mv); const to = $(`[data-mvto="${k}"]`, out).value.trim(); if (!to) return; try { const m = await api('POST', '/library/folders/move', { from: r.missing_folders[k], to }); toast(`${m.moved_files} files found in their new place`); mb.closest('.toolbar').remove(); } catch (er) { toast(er.message, 'error'); } });
                b.textContent = 'Restored'; forgetPubkyUser(); forgetAlbums(); homeCache.clear(); toast('Backup restored');
              } catch (er) { toast(er.message, 'error'); b.disabled = false; b.textContent = 'Restore this backup'; delete b.dataset.armed; }
            };
          };
          // Zones' "Group outputs…" opens this tab at its Group outputs form, the focus on its first speaker
          if (toGroup && active === 'playback') { const gc = $('#groupcard'); if (gc) { gc.scrollIntoView({ block: 'start', behavior: 'instant' }); const f = $('#groupouts input:not(:disabled)'); if (f) f.focus({ preventScroll: true }); else { gc.tabIndex = -1; gc.focus({ preventScroll: true }); } } }
          if (state.focusRestore) { state.focusRestore = false; $('#backupcard').scrollIntoView({ block: 'start' }); $('#rsfile').focus(); }
          loadOrder();
          const sy = $('#sync'); if (sy) sy.onclick = async () => { try { const r = await api('POST', '/pubky/sync'); syncToast(r); render(); } catch (e) { toast(e.message, 'error'); } };
          // ---- Settings › People & access (the `people` tab): who can use this node, adding and removing
          // people, what they can do, the browsers and apps signed in, and the password for apps ----
          const ACCESS = { homeserver_music: 'Homeserver account and music', homeserver: 'Homeserver account only', music: 'Music only' };
          // Each kind as a plain choice: what they get, then what happens once it is chosen
          const HS_READS = 'You run this homeserver, so you can read every file on it, private ones too: tell them.';
          const INVITE_KINDS = {
            homeserver_music: ['They get a Pubky account on this node’s homeserver and play this node’s music.', `You get a link and a QR code to send them: Pubky Ring makes their account with it. Their bookmarks and collections live on that account. ${HS_READS}`],
            homeserver: ['They get a Pubky account on this node’s homeserver. They can’t play this node’s music.', `You get a link and a QR code to send them: Pubky Ring makes their account with it. ${HS_READS}`],
            music: ['They have a Pubky account elsewhere and play this node’s music.', 'Added by their key; they then log in here with Pubky Ring. Their bookmarks and collections stay on their own account.'],
          };
          const SRC_POLICIES = [
            ['node', 'This node’s', 'Everyone plays TIDAL, Qobuz and Spotify through the sign-ins made on this node. The people you add can’t sign in to their own.', 'with this node’s music services', 'Everyone plays through this node’s sign-ins'],
            ['own', 'Their own', 'Everyone signs in to their own accounts; someone without one gets “Open in” instead.', 'with their own TIDAL, Qobuz or Spotify', 'Everyone plays with their own sign-ins'],
            ['choice', 'They choose', 'This node’s by default; each person may switch to their own.', 'with this node’s music services or their own', 'Each person chooses whose sign-ins they play with'],
          ];
          // A storage limit as the node gives it: MB, "unlimited", or nothing set (the homeserver's own)
          const limitWords = (q) => (q == null ? 'the homeserver’s usual limit' : /^unlimited$/i.test(String(q)) ? 'no limit' : `a limit of ${fmtBytes(Number(q) * 1e6)}`);
          const inviteQr = (inv, hsKey) => (inv.url ? `<div class="notice qrbox pa-qr">${inv.qr_svg ? `<div class="qr">${inv.qr_svg}</div>` : ''}<div class="pa-qr-text"><p>Send this to ${inv.name ? esc(inv.name) : 'them'}: they scan it with Pubky Ring (or open the link on their phone), and Ring makes their account on this node’s homeserver.</p><p class="muted small">Older Ring versions can’t open it; they can sign up by hand with the code <code>${esc(inv.token || '')}</code>${hsKey ? ` and this homeserver’s key <code>${esc(hsKey)}</code>` : ''}.</p><div class="actions"><a class="btn sm" href="${esc(ringUrl(inv.url))}">Open in Pubky Ring</a><button class="btn sm ghost" data-copylink="${esc(inv.url)}">Copy link</button></div></div></div>` : '');
          // A dialog with one field: the field has the focus, and Enter in it is the first choice
          const askText = (opts) => {
            const asked = choiceDialog(opts);
            const f = $$('dialog.dialog [data-field]').pop();
            if (f) { f.focus(); f.select(); f.onkeydown = (e) => { if (e.key !== 'Enter') return; e.preventDefault(); const go = $('.choice', f.closest('dialog')); if (go) go.click(); }; }
            return asked;
          };
          // What Add someone last answered with (the invite, or what to send them): it stays over a redraw
          let pinvOut = '';
          // People whose Remove failed on this visit (key -> who they are and what the node said). A
          // removal can fail after the node already took their music access away, or took them off
          // its list: Remove stays there for them, since removing again finishes it.
          const removeFailed = new Map();
          // The zones were changed and not saved yet: only then is what is ticked kept over a redraw
          // (a choice nobody made is never carried into a list the node could not read)
          let zonesTouched = false;
          // `fresh`: the form that starts again from the node's answer ('invite' after an add, 'zones'
          // after a save); every other redraw keeps what was typed and ticked. `focus`: where the focus
          // goes when the control that had it is gone after the redraw (the first of these that is there).
          const loadPeople = async ({ fresh = '', focus = null } = {}) => {
            const box = $('#peoplebox'); if (!box) return;
            let r = null; let rErr = null;
            const [pr, zl0, pt0] = await Promise.all([
              api('GET', '/people').catch((e) => { rErr = e.message; return null; }),
              // A list the node can't read keeps the people you added off every zone: said here, not shown as "every zone"
              api('GET', '/people/zones').catch((e) => ({ zones: [], error: e.message })),
              // (a failed read is said as one, with Retry: not shown as "no parties")
              api('GET', '/party').catch((e) => ({ error: e.message })),
            ]);
            if (!box.isConnected) return;
            if (pr && Array.isArray(pr.people) && Array.isArray(pr.invites)) r = pr; else if (!rErr) rErr = 'the node’s answer was not a list of people';
            const zl = zl0 && Array.isArray(zl0.zones) ? zl0 : { zones: [], error: 'the node’s answer was not a list of zones' };
            const partyErr = Array.isArray(pt0) ? null : (pt0 && pt0.error) || 'the node’s answer was not a list';
            const parties = partyErr ? [] : pt0;
            const hs = (r && r.homeserver) || {};
            const onHs = !!(hs.configured && hs.reachable);
            const kinds = onHs ? ['homeserver_music', 'homeserver', 'music'] : ['music'];
            const zones = zoneList().filter((z) => z.id !== BROWSER);
            // The zones open to the people you add: none saved is every zone, unless the node says no
            // zone at all is the choice (`none`, or from a node before that the list of one zone
            // without a name: they then play in their own browser or app only). One that is saved but
            // not among the node's zones now stays in the list (and is saved again), said as "not here
            // now".
            const noZone = !zl.error && (zl.none === true || noZoneList(zl.zones));
            const open = zl.zones.filter((id) => id !== '');
            const zoneName = (id) => { const z = zones.find((x) => x.id === id) || state.zones[id]; return z ? z.name : null; };
            const gone = open.filter((id) => !zones.some((z) => z.id === id));
            // (the node's settings could not be read: no choice is known, and none is shown as made)
            const policyRow = ns.unread ? null : SRC_POLICIES.find(([v]) => v === ns.sources_policy) || SRC_POLICIES[1];
            const policy = policyRow ? policyRow[0] : '';
            const settingsErr = (fk) => `<div class="set-row" data-pa-error><div class="set-label"><p class="set-help pa-err" role="alert">Couldn’t read this node’s settings: ${esc(ns.unread)}</p></div><div class="set-ctl"><button class="btn sm" data-pa-reload data-fk="${fk}">Retry</button></div></div>`;
            const zonesPhrase = zl.error ? 'on no zone until the zones are saved again'
              : !open.length ? 'on every zone'
                : `on ${listWords([...open.map(zoneName).filter(Boolean), ...(open.some((id) => !zoneName(id)) ? [`${countOf(open.filter((id) => !zoneName(id)).length, 'zones')} not here now`] : [])])}`;
            // On phones each section folds (UX sweep 2026-11-01 S2); what was open stays open when the
            // page is drawn again after an action, and so does a list opened inside a row
            const was = new Map($$('[data-fold]', box).map((d) => [d.dataset.fold, d.open]));
            const wasOpen = new Set($$('details[data-keepopen]', box).filter((d) => d.open).map((d) => d.dataset.keepopen));
            const draft = {
              invite: fresh !== 'invite' && $('#pinvname', box) ? { kind: ($('input[name="pinvkind"]:checked', box) || {}).value, ...Object.fromEntries(['pinvname', 'pinvquota', 'pinvkey'].map((id) => [id, ($(`#${id}`, box) || {}).value || ''])) } : null,
              zones: fresh !== 'zones' && zonesTouched && $('#pzsave', box) ? { mode: ($('input[name="pzmode"]:checked', box) || {}).value, picked: $$('#pzones input:checked', box).map((i) => i.value) } : null,
            };
            if (!draft.zones) zonesTouched = false;
            const act = document.activeElement;
            const had = !!act && act !== document.body && box.contains(act);
            const fkWas = had ? ((act.closest('[data-fk]') || {}).dataset || {}).fk || act.id || null : null;
            const sec = (id, title, body, openFirst) => (shellMode() === 'phone'
              ? `<details class="set-group set-fold" data-fold="${id}" ${(was.has(id) ? was.get(id) : openFirst) ? 'open' : ''}><summary><h3 class="set-h">${title}</h3></summary><div class="set-rows">${body}</div></details>`
              : group(title, body));
            // A list or a link that opens inside a row (never inside the row's <p>)
            const fold = (key, summary, body) => `<details class="pa-fold" data-keepopen="${esc(key)}" ${wasOpen.has(key) ? 'open' : ''}><summary data-fk="${esc(key)}">${summary}</summary>${body}</details>`;
            const errRow = (text, fk) => `<div class="set-row" data-pa-error><div class="set-label"><p class="set-help pa-err" role="alert">${esc(text)}</p></div><div class="set-ctl"><button class="btn sm" data-pa-retry data-fk="${fk}">Retry</button></div></div>`;

            // 1. Who can use this node: you, then each person with what they can do in one sentence
            const who = (p) => p.name || 'this person';
            // People whose removal did not finish, as the node remembers them (GET /v1/people `unfinished`)
            const unfinished = r && Array.isArray(r.unfinished) ? r.unfinished : [];
            const whoLabel = (p) => p.name || `the person with the key ${shortKey(p.key)}`;
            const plays = (p) => !!p.music_enabled && p.access !== 'homeserver';
            const signInsOf = (p) => (Array.isArray(p.sign_ins) ? p.sign_ins : []);
            const says = (p) => (plays(p) ? `Plays music here${noZone ? ' in their own browser or app only' : `, ${zonesPhrase}`}${policyRow ? `, ${policyRow[3]}` : ''}.` : p.access === 'homeserver' ? 'Has storage on this node’s homeserver. No music.' : 'Music is switched off for them.');
            const store = (p) => {
              if (p.access === 'music') return '';
              const off = p.homeserver_enabled === false ? '<span class="warn-text">Their homeserver account is switched off: they can read and delete, not save.</span> ' : '';
              const used = p.used_bytes != null ? `${fmtBytes(p.used_bytes)} used` : '';
              const facts = [used, onHs ? limitWords(p.storage_quota_mb) : ''].filter(Boolean).join(', ');
              return off || facts ? `<p class="set-help" data-pstore>${off}${facts ? `Storage on this node’s homeserver: ${esc(facts)}.` : ''}</p>` : '';
            };
            const signIns = (p) => {
              const list = signInsOf(p);
              if (!list.length) return plays(p) ? '<p class="set-help" data-pnone>Not signed in anywhere.</p>' : '';
              return fold(`si:${p.key}`, `Signed in on ${list.length} ${list.length === 1 ? 'browser or app' : 'browsers or apps'}${plays(p) ? '' : ' (not usable while music is off)'}`,
                `<div class="pa-silist">${list.map((s) => `<div class="pa-si" data-psi="${esc(s.id)}"><div class="pa-si-text"><div class="pa-si-name">${esc(signInName(s))}</div><div class="set-help">${esc(signInNote(s))}</div></div><button class="btn sm" data-psi-end="${esc(s.id)}" data-pkey="${esc(p.key)}" data-fk="psi:${esc(p.key)}:${esc(s.id)}" aria-label="Sign out ${esc(signInName(s))} of ${esc(whoLabel(p))}, ${esc(signInNote(s))}">Sign out</button></div>`).join('')}<div class="pa-si"><div class="pa-si-text set-help">They log in again with Pubky Ring.</div><button class="btn sm" data-psi-all="${esc(p.key)}" data-fk="psiall:${esc(p.key)}" aria-label="Sign out ${esc(whoLabel(p))} everywhere">Sign out everywhere</button></div></div>`);
            };
            const acts = (p) => {
              const k = esc(p.key); const a = esc(whoLabel(p));
              const music = plays(p) ? 'Switch music off' : 'Switch music on';
              const hsOn = p.homeserver_enabled !== false;
              return `<button class="btn sm" data-prename="${k}" data-fk="rename:${k}" aria-label="Rename ${a}">Rename</button>`
                + `<button class="btn sm" data-pmusic="${k}" data-to="${plays(p) ? 'off' : 'on'}" data-fk="music:${k}" aria-label="${music} for ${a}">${music}</button>`
                // (their homeserver account: only for an account on this node's homeserver, while it answers)
                + (p.access !== 'music' && onHs ? `<button class="btn sm" data-phs="${k}" data-to="${hsOn ? 'off' : 'on'}" data-fk="hs:${k}" aria-label="Switch the homeserver account ${hsOn ? 'off' : 'on'} for ${a}">Switch homeserver account ${hsOn ? 'off' : 'on'}</button><button class="btn sm" data-pquota="${k}" data-fk="quota:${k}" aria-label="Storage limit for ${a}">Storage limit…</button>` : '')
                // (someone who only has a homeserver account and no sign-in left has nothing here to remove:
                // the account itself stays on the homeserver either way; unless their last Remove failed)
                + (p.access === 'homeserver' && !signInsOf(p).length && !removeFailed.has(p.key) && !unfinished.includes(p.key) ? '' : `<button class="btn sm danger" data-premove="${k}" data-fk="remove:${k}" aria-label="Remove ${a}">Remove…</button>`);
            };
            // (what the node said on this visit; after a reload the node only says that it did not finish)
            const failedNote = (key) => (removeFailed.has(key) ? `<p class="set-help pa-err" role="alert" data-premove-err>Removing them did not finish: ${esc(removeFailed.get(key).error)} Remove again to finish it.</p>` : unfinished.includes(key) ? '<p class="set-help pa-err" data-premove-err>Removing them did not finish. Remove again to finish it.</p>' : '');
            const person = (p) => `<div class="set-row set-block pa-person" data-person="${esc(p.key)}"><div class="set-label"><div class="set-name">${p.name ? esc(p.name) : 'No name yet'}</div><p class="set-help" data-psays>${esc(says(p))}</p>${failedNote(p.key)}${store(p)}${signIns(p)}${fold(`key:${p.key}`, 'Show key', `<div class="pa-keyrow"><code class="pa-code">${esc(p.key)}</code><button class="btn sm ghost" data-copykey="${esc(p.key)}" aria-label="Copy the key of ${esc(whoLabel(p))}">Copy key</button></div>`)}</div><div class="actions pa-acts" role="group" aria-label="Change ${esc(whoLabel(p))}">${acts(p)}</div></div>`;
            const youRow = row('You', 'Owner: you can change everything on this node.', '');
            // (someone the node already took off its list while their removal failed: still here, to finish it)
            const listed = (key) => !!r && r.people.some((p) => p.key === key);
            const halfGone = [...removeFailed].filter(([key]) => !listed(key)).map(([key, f]) => ({ key, name: f.name, access: f.access, sign_ins: [] }))
              // (and the ones the node remembers from before this visit: it keeps no name for them)
              .concat(unfinished.filter((key) => !listed(key) && !removeFailed.has(key)).map((key) => ({ key, name: null, access: 'music', sign_ins: [], gone: true })));
            const gonePerson = (p) => `<div class="set-row set-block pa-person" data-person="${esc(p.key)}" data-pgone><div class="set-label"><div class="set-name">${p.name ? esc(p.name) : p.gone ? `Someone you removed (key ${esc(shortKey(p.key))})` : 'No name yet'}</div>${failedNote(p.key)}</div><div class="actions pa-acts" role="group" aria-label="Change ${esc(whoLabel(p))}"><button class="btn sm danger" data-premove="${esc(p.key)}" data-fk="remove:${esc(p.key)}" aria-label="Remove ${esc(whoLabel(p))}">Remove…</button></div></div>`;
            const whoBody = youRow + (rErr ? errRow(`Couldn’t read who else can use this node: ${rErr}`, 'retry-people')
              : r.people.length || halfGone.length ? r.people.map(person).join('') : row('Nobody else yet', 'The people you add show here.', ''))
              + halfGone.map(gonePerson).join('');

            // 2. Add someone: the three kinds, then the invites not used yet
            const hsNote = !r ? '' : !hs.configured ? 'This node has no homeserver of its own, so the people you add need a Pubky account elsewhere.'
              : !onHs ? `This node’s homeserver isn’t answering${hs.error ? ` (${esc(hs.error)})` : ''}, so only someone with a Pubky account elsewhere can be added right now.` : '';
            const invite = (inv) => `<div class="set-row pa-top" data-pinv="${esc(inv.id)}"><div class="set-label"><div class="set-name">${inv.name ? esc(inv.name) : 'Invite without a name'}</div><p class="set-help">${esc(ACCESS[inv.access] || inv.access)} · made ${esc(new Date(inv.created_at * 1000).toLocaleDateString())}${inv.storage_quota_mb ? ` · ${esc(limitWords(inv.storage_quota_mb))}` : ''}</p>${inv.url ? fold(`inv:${inv.id}`, 'Show the invite again', inviteQr(inv, hs.public_key)) : ''}</div><div class="set-ctl"><button class="btn sm" data-pinv-del="${esc(inv.id)}" data-fk="invdel:${esc(inv.id)}" aria-label="Take ${inv.name ? `the invite for ${esc(inv.name)}` : 'this invite'} off the list">Take off the list</button></div></div>`;
            const addBody = !r ? '' : block('', hsNote, `<div class="ways" role="radiogroup" aria-label="What they get">${kinds.map((k) => `<label class="way ${k === kinds[0] ? 'on' : ''}"><input type="radio" name="pinvkind" value="${k}" ${k === kinds[0] ? 'checked' : ''} aria-describedby="pinv-${k}"><span class="way-text"><span class="way-name">${ACCESS[k]}</span><span class="way-note" id="pinv-${k}">${INVITE_KINDS[k][0]}</span><span class="way-more small" ${k === kinds[0] ? '' : 'hidden'}>${INVITE_KINDS[k][1]}</span></span></label>`).join('')}</div><div class="toolbar invite-fields"><input class="input" id="pinvname" placeholder="Their name (optional)" aria-label="Their name"><input class="input" id="pinvquota" type="number" min="1" placeholder="Storage limit in MB (optional)" aria-label="Storage limit in MB (empty: the homeserver’s usual limit)" ${onHs ? '' : 'hidden'}></div><div class="keyfield" id="pinvkeyrow" ${onHs ? 'hidden' : ''}><input class="input" id="pinvkey" placeholder="Their Pubky key" aria-label="Their Pubky key" aria-describedby="pinvkeyerr" autocomplete="off" autocapitalize="off" spellcheck="false"><button class="btn ghost" type="button" id="pinvpaste" hidden>${icon('paste')} Paste</button><button class="btn ghost" type="button" id="pinvscan" hidden>${icon('scan')} Scan QR</button></div><p class="field-err small" id="pinvkeyerr" role="alert" hidden></p><div class="actions invite-go"><button class="btn primary" id="pinvgo">${icon('plus')} <span data-pinvgo-label>Create invite</span></button></div><div id="pinvout" aria-live="polite">${pinvOut}</div>`)
              + (r.invites.length ? `<div class="set-row set-block" data-pinv-head><div class="set-label"><div class="set-name">Invites not used yet</div><p class="set-help">Taking one off this list doesn’t cancel it: an invite already sent stays usable on the homeserver.</p></div></div>${r.invites.map(invite).join('')}` : '');

            // 3. What the people you add can and can't do, whoever they are
            const canBody = `<div class="set-row set-block" id="pcando">${canLists({ has: (cap) => LISTENER_CAN.has(cap) && (cap !== 'own_sign_in' || (!!policy && policy !== 'node')) && !(cap === 'zone_use' && noZone), their: 'their', owners: 'your', zones: 'Use the zones you open to them', noZone: noZone ? 'Play on a zone: none is open to them' : '', canTitle: 'They can', cantTitle: 'They can’t' })}</div>`;

            // 4. The zones they can play on: every zone, only the ones ticked (never "only these" with
            // none ticked), or no zone at all
            const zoneOpts = [...zones.map((z) => [z.id, z.name]), ...gone.map((id) => [id, zoneName(id) || `${id} (not here now)`])];
            const zMode = zl.error ? '' : noZone ? 'none' : open.length ? 'some' : 'all';
            const zonesBody = block('', zl.error ? `<span class="pa-err">Couldn’t read the zones: ${esc(zl.error)}.</span> Until they are saved again, the people you add can play on no zone: choose and save.` : 'The same for everyone you add.',
              zoneOpts.length
                ? `<div class="pa-zmode" role="radiogroup" aria-label="Zones the people you add can play on"><label class="check"><input type="radio" name="pzmode" value="all" ${zMode === 'all' ? 'checked' : ''}> Every zone</label><label class="check"><input type="radio" name="pzmode" value="some" ${zMode === 'some' ? 'checked' : ''}> Only these</label><label class="check"><input type="radio" name="pzmode" value="none" aria-describedby="pznonehelp" ${zMode === 'none' ? 'checked' : ''}> No zone</label></div><p class="set-help pa-zhelp" id="pznonehelp" ${zMode === 'none' ? '' : 'hidden'}>They play in their own browser or app only.</p><div class="chips pa-zones" id="pzones" role="group" aria-label="Only these zones" ${zMode === 'some' ? '' : 'hidden'}>${zoneOpts.map(([id, name]) => `<label class="chip"><input type="checkbox" value="${esc(id)}" ${open.includes(id) ? 'checked' : ''}> ${esc(name)}</label>`).join('')}</div><p class="field-err small" id="pzerr" role="alert" hidden></p><div class="actions"><button class="btn" id="pzsave">Save zones</button></div>`
                : '<p class="muted small">No zones yet.</p>');

            // 5. Whose TIDAL, Qobuz and Spotify sign-ins they play with (owner, 2026-11-03)
            const srcBody = !policyRow ? settingsErr('reload-services') : block('', `${policyRow[2]} Switching keeps every sign-in, yours and theirs.`, `<div class="chips pa-policy" role="radiogroup" aria-label="Whose music services the people you add play with">${SRC_POLICIES.map(([v, l]) => `<button class="chip scope ${policy === v ? 'on' : ''}" role="radio" aria-checked="${policy === v}" data-srcpolicy="${v}" data-fk="policy:${v}">${l}</button>`).join('')}</div>`);

            // 6. The browsers and apps signed in as the owner; this one first, with Log out
            const mineBody = '<div id="pmine" class="pa-rows"></div>';

            // 7. Apps with a password of their own, then the password for apps (the node's API token)
            const appsBody = block('', 'Each app you connect (Home Assistant, a Subsonic app, a script) can have a password of its own. With it the app plays, controls and groups zones and browses the library and your collections; it can’t change settings, people or the library. Removing one stops that app alone.', '<div id="papps" class="pa-rows"></div>');
            const exposed = ns.api_token_exposed_at ? `<div class="notice warn" role="alert">This password was in a saved queue’s links, where everyone who could use that zone could read it (noticed ${esc(new Date(ns.api_token_exposed_at * 1000).toLocaleString())}). ${ns.token_replaceable ? 'Make a new one.' : 'Change <code>PM_TOKEN</code> (or <code>[api] token</code>) and restart the node.'}</div>` : '';
            const APPS_USE = 'It can do everything you can. Apps can log in with it, but one with a password of its own (above) can do only what an app needs.';
            const passBody = ns.unread ? settingsErr('reload-password') : ns.token_replaceable
              ? block('', `${APPS_USE} A new one stops the old one at once: every app then needs the new one, and your other browsers are logged out (log in again with Pubky Ring). The people you added stay signed in.`, `${exposed}<div class="actions"><button class="btn" id="tokennew">Make a new password</button></div><div id="tokenout" aria-live="polite"></div>`)
              : block('', `${APPS_USE} Set in the node’s config (<code>[api] token</code> in node.toml, <code>PM_TOKEN</code> in Docker): it is changed there.`, `${exposed}<div class="actions"><a class="btn ghost sm" href="${TOKEN_GUIDE}" target="_blank" rel="noopener">${icon('external')} How to change it</a></div>`);

            // 8. Party: a guest link for one zone, no account
            const partyRow = (pt) => `<div class="set-row pa-top" data-party="${esc(pt.id)}"><div class="set-label"><div class="set-name">${esc(pt.name)}</div><p class="set-help">${pt.zone_name ? `On ${esc(pt.zone_name)} · ` : ''}until ${esc(new Date(pt.expires_at * 1000).toLocaleString([], { weekday: 'short', hour: '2-digit', minute: '2-digit' }))} · ${countOf((pt.guests || []).length, 'guests')} · ${countOf(pt.added || 0, 'songs')} added · ${esc(pt.adds_per_hour)} per guest per hour, ${esc(pt.waiting_per_guest)} waiting at once</p>${fold(`party:${pt.id}`, 'Show the guest link', `<div class="notice qrbox pa-qr">${pt.qr_svg ? `<div class="qr">${pt.qr_svg}</div>` : ''}<div class="pa-qr-text"><p>Guests scan this or open the link: they give a name and add songs from the library to this zone. No account needed.</p><p><code class="pa-code">${esc(pt.url)}</code></p><div class="actions"><button class="btn sm ghost" data-copylink="${esc(pt.url)}">Copy link</button></div></div></div>`)}</div><div class="set-ctl"><button class="btn sm" data-party-end="${esc(pt.id)}" data-fk="partyend:${esc(pt.id)}" aria-label="End the party ${esc(pt.name)}">End party</button></div></div>`;
            const partyBody = (partyErr ? errRow(`Couldn’t read the parties: ${partyErr}`, 'retry-party') : parties.map(partyRow).join(''))
              + (zones.length ? block(parties.length ? 'Start another party' : 'Start a party', 'Anyone with the link adds songs from the library to one zone, without an account. Their songs play after the current one, in the order they came; the node checks the limits.', `<div class="toolbar"><select class="input" id="ptzone" aria-label="Party zone">${zones.map((z) => `<option value="${esc(z.id)}">${esc(z.name)}</option>`).join('')}</select><input class="input" id="ptname" placeholder="Name (optional)" aria-label="Party name"><select class="input" id="pthours" aria-label="How long the link works">${[[3, '3 hours'], [6, '6 hours'], [12, '12 hours'], [24, 'A day'], [72, '3 days']].map(([h, l]) => `<option value="${h}" ${h === 6 ? 'selected' : ''}>${l}</option>`).join('')}</select></div><div class="toolbar pa-limits"><label class="small" for="ptadds">Songs per guest per hour</label><input class="input pa-num" id="ptadds" type="number" min="1" max="100" value="5"><label class="small" for="ptwait">Waiting at once</label><input class="input pa-num" id="ptwait" type="number" min="1" max="50" value="3"><button class="btn primary" id="ptgo">Start party</button></div>`) : row('Start a party', 'Needs a zone to play on.', ''));

            // 9. What belongs to this screen's subject but is set elsewhere
            const rt = $('#settab-remote', main); const remoteTab = rt ? rt.textContent.trim() : ''; // (the tab is there only where a tunnel can be set up)
            const moreBody = (remoteTab ? row('Use this node away from home', 'A tunnel or a domain, so you and the people you add reach it when not on your network.', `<button class="btn ghost" data-settab="remote">Open ${esc(remoteTab)}</button>`) : '')
              + row('This node’s own Pubky identity', 'The Pubky account this node is signed in as: yours. Your bookmarks, collections and tags are kept on it.', '<button class="btn ghost" data-settab="accounts">Open My accounts</button>');

            box.innerHTML = sec('who', 'Who can use this node', whoBody, true)
              + (r ? sec('invite', 'Add someone', addBody, !r.people.length || r.invites.length > 0) : '')
              + sec('cando', 'What people you add can do', canBody, false)
              + sec('zones', 'Zones they can play on', zonesBody, !!zl.error)
              + sec('services', 'Whose music services they play with', srcBody, false)
              + sec('mine', 'Browsers and apps signed in as you', mineBody, false)
              + sec('apps', 'Apps with their own password', appsBody, false)
              + sec('password', 'Password for apps (API token)', passBody, !!ns.api_token_exposed_at)
              + sec('party', 'Party', partyBody, false)
              + group('Also in Settings', moreBody);
            box.tabIndex = -1;
            // Turning a desktop window into a phone (or back) folds the sections (or unfolds them)
            box.reshell = () => loadPeople();
            $$('[data-settab]', box).forEach(goTab);
            $$('[data-pa-retry]', box).forEach((b) => b.onclick = () => loadPeople());
            // (the node's settings are read with the rest of Settings: the page is drawn again)
            $$('[data-pa-reload]', box).forEach((b) => b.onclick = () => render());
            mountAppPasswords($('#papps', box));
            mountSignIns($('#pmine', box), {
              control: state.token ? `<button class="btn" id="logout">${icon('logout')} Log out</button>` : '',
              kept: keptSignIns(),
              wire: (host) => { const lo = $('#logout', host); if (lo) lo.onclick = logOut; },
            });

            // ---- the people ----
            const personOf = (key) => (r && r.people.find((p) => p.key === key)) || halfGone.find((p) => p.key === key) || null;
            const change = async (p, body, done, to) => { try { await api('PATCH', `/people/${encodeURIComponent(p.key)}`, body); toast(done); } catch (e) { toast(e.message, 'error'); } await loadPeople({ focus: to }); };
            $$('[data-prename]', box).forEach((b) => b.onclick = async () => {
              const p = personOf(b.dataset.prename); if (!p) return;
              const a = await askText({ title: p.name ? `Rename ${p.name}` : 'Give them a name', text: 'The name this screen shows for them.', field: { label: 'Name', value: p.name || '' }, choices: [{ label: 'Save', value: true, primary: true }] });
              if (!a || a.text === (p.name || '')) return;
              await change(p, { name: a.text }, a.text ? `Renamed to ${a.text}` : 'Name removed');
            });
            $$('[data-pmusic]', box).forEach((b) => b.onclick = async () => {
              const p = personOf(b.dataset.pmusic); if (!p) return;
              const on = b.dataset.to === 'on';
              // Off: the node refuses their sign-ins from now on (their pages go back to Log in) and forgets
              // their registered devices and what those keep for offline. Nothing here promises more.
              if (!on && !(await choiceDialog({ title: `Switch music off for ${who(p)}?`, text: `They can’t ${p.access === 'music' ? 'use this node' : 'play music here'} until you switch it back on. Their browsers and apps go back to Log in, and the node forgets their devices and what those keep for offline.${p.access === 'music' ? '' : ' Their homeserver account is not touched.'}`, choices: [{ label: 'Switch music off', value: true, primary: true, danger: true }] }))) return;
              await change(p, { music: on }, on ? `Music is on for ${who(p)}` : `Music is off for ${who(p)}`);
            });
            $$('[data-phs]', box).forEach((b) => b.onclick = async () => {
              const p = personOf(b.dataset.phs); if (!p) return;
              const on = b.dataset.to === 'on';
              if (!on && !(await choiceDialog({ title: `Switch the homeserver account off for ${who(p)}?`, text: 'They can still read and delete what is there, but can’t save anything new, their bookmarks and collections included, until you switch it back on.', choices: [{ label: 'Switch the account off', value: true, primary: true }] }))) return;
              await change(p, { homeserver_enabled: on }, on ? `The homeserver account is on for ${who(p)}` : `The homeserver account is off for ${who(p)}: they can read and delete, not save`);
            });
            // The storage limit: a number, "no limit" or the homeserver's usual one, each chosen in
            // so many words (an emptied field never means "no limit")
            $$('[data-pquota]', box).forEach((b) => b.onclick = async () => {
              const p = personOf(b.dataset.pquota); if (!p) return;
              let typed = typeof p.storage_quota_mb === 'number' ? String(p.storage_quota_mb) : '';
              // (what was wrong with the last answer is said in the dialog that asks again, where it is read)
              let wrong = '';
              for (;;) {
                const a = await askText({ title: `Storage limit for ${who(p)}`, text: `${wrong}Now: ${limitWords(p.storage_quota_mb)}. How much they can keep on this node’s homeserver.`, field: { label: 'Limit in MB', value: typed }, choices: [{ label: 'Set this limit', value: 'set', primary: true }, { label: 'No limit', value: 'unlimited', note: 'They can fill this homeserver’s disk' }, { label: 'The homeserver’s usual limit', value: 'usual', note: 'What an account gets when nothing is set for it' }] });
                if (!a) return;
                if (a.value === 'unlimited') { await change(p, { storage_quota_mb: 'unlimited' }, `${p.name || 'This person'} has no storage limit now`); return; }
                if (a.value === 'usual') { await change(p, { storage_quota_mb: null }, `${p.name || 'This person'} has the homeserver’s usual limit now`); return; }
                const mb = /^\d+$/.test(a.text) ? Number(a.text) : 0;
                if (mb > 0) { await change(p, { storage_quota_mb: mb }, `Storage limit for ${who(p)}: ${fmtBytes(mb * 1e6)}`); return; }
                typed = a.text; wrong = `${a.text ? `“${a.text}” is not a limit` : 'No limit was typed'}: type a whole number of MB, like 500, or choose one of the other two. `;
              }
            });
            $$('[data-premove]', box).forEach((b) => b.onclick = async () => {
              const p = personOf(b.dataset.premove); if (!p) return;
              const stays = p.access !== 'music';
              const go = await choiceDialog({ title: `Remove ${who(p)}?`, text: `They are signed out everywhere, and what this node keeps for them goes: their music service sign-ins here, preferences, devices and problem reports. Their Pubky account and what is on their own homeserver are not touched.${stays ? ' Their account on this node’s homeserver stays, so they stay in this list without music.' : ''}`, choices: [{ label: stays ? 'Remove their music access' : 'Remove', value: true, primary: true, danger: true }] });
              if (!go) return;
              const keys = r ? r.people.map((x) => x.key) : []; const at = keys.indexOf(p.key);
              try {
                await api('DELETE', `/people/${encodeURIComponent(p.key)}`);
                removeFailed.delete(p.key);
                pinvOut = ''; // (what Add someone said to send them no longer holds)
                toast(stays ? `${p.name || 'This person'} no longer plays music here. Their homeserver account stays.` : `Removed ${who(p)}`);
              } catch (e) {
                if (e.status === 404) {
                  // Nobody by that key and nothing of theirs left (finished elsewhere meanwhile): done
                  removeFailed.delete(p.key);
                  toast(`Nothing of ${who(p)} is left on this node`);
                } else {
                  // Kept with the node's words: Remove stays on their row, and removing again finishes it
                  removeFailed.set(p.key, { name: p.name || null, access: p.access, error: e.message });
                  toast(`Removing ${who(p)} did not finish: ${e.message}`, 'error');
                }
              }
              await loadPeople({ focus: [`rename:${p.key}`, ...[keys[at + 1], keys[at - 1]].filter(Boolean).map((k) => `rename:${k}`), 'pinvname'] });
            });
            $$('[data-psi-end]', box).forEach((b) => b.onclick = async () => {
              const p = personOf(b.dataset.pkey); if (!p) return;
              const ids = signInsOf(p).map((s) => s.id); const at = ids.indexOf(b.dataset.psiEnd); const s = signInsOf(p)[at];
              try { await api('DELETE', `/people/${encodeURIComponent(p.key)}/sign-ins/${encodeURIComponent(b.dataset.psiEnd)}`); toast(`Signed out: ${s ? signInName(s) : 'that sign-in'} of ${who(p)}`); } catch (e) { toast(e.message, 'error'); }
              await loadPeople({ focus: [...[ids[at + 1], ids[at - 1]].filter(Boolean).map((id) => `psi:${p.key}:${id}`), `rename:${p.key}`] });
            });
            $$('[data-psi-all]', box).forEach((b) => b.onclick = async () => {
              const p = personOf(b.dataset.psiAll); if (!p) return;
              const go = await choiceDialog({ title: `Sign ${who(p)} out everywhere?`, text: 'Each of their browsers and apps has to log in again with Pubky Ring. What they can do here stays as it is.', choices: [{ label: 'Sign out everywhere', value: true, primary: true, danger: true }] });
              if (!go) return;
              try { const d = await api('DELETE', `/people/${encodeURIComponent(p.key)}/sign-ins`); toast(`Signed ${who(p)} out${d && d.ended === 1 ? ' of one browser or app' : d && d.ended != null ? ` of ${d.ended} browsers or apps` : ' everywhere'}`); } catch (e) { toast(e.message, 'error'); }
              await loadPeople({ focus: [`rename:${p.key}`] });
            });

            // ---- add someone ----
            if (r) {
              // The kind of invite: radio cards, the chosen one's details shown (S1)
              const kindOf = () => ($('input[name="pinvkind"]:checked', box) || {}).value || kinds[0];
              const keyIn = $('#pinvkey', box); const keyErr = $('#pinvkeyerr', box);
              if (draft.invite) {
                const k = $$('input[name="pinvkind"]', box).find((i) => i.value === draft.invite.kind); if (k) k.checked = true;
                ['pinvname', 'pinvquota', 'pinvkey'].forEach((id) => { $(`#${id}`, box).value = draft.invite[id]; });
              }
              const fit = () => {
                const k = kindOf(); const music = k === 'music';
                $$('.way', box).forEach((w) => { const on = w.querySelector('input').value === k; w.classList.toggle('on', on); w.querySelector('.way-more').hidden = !on; });
                $('#pinvquota', box).hidden = music; $('#pinvkeyrow', box).hidden = !music; if (!music) keyErr.hidden = true;
                $('[data-pinvgo-label]', box).textContent = music ? 'Add them' : 'Create invite';
              };
              $$('input[name="pinvkind"]', box).forEach((i) => i.onchange = fit); fit();
              // Their key: typed, pasted or scanned; any of the spellings Pubky apps show
              const checkKey = () => { const v = keyIn.value.trim(); const bad = !!v && !keyFrom(v); keyErr.hidden = !bad; keyErr.textContent = bad ? 'That isn’t a Pubky key: it is 52 letters and digits, as Pubky Ring shows it.' : ''; keyIn.setAttribute('aria-invalid', String(bad)); };
              const setKey = (raw, quiet) => { const k = keyFrom(raw); if (k) keyIn.value = k; else if (raw != null && !quiet) keyIn.value = String(raw).trim(); checkKey(); return k; };
              keyIn.onchange = () => { setKey(keyIn.value); };
              keyIn.oninput = () => { if (!keyErr.hidden) checkKey(); };
              const paste = $('#pinvpaste', box); paste.hidden = !(navigator.clipboard && navigator.clipboard.readText);
              paste.onclick = async () => { try { const t = await navigator.clipboard.readText(); if (!setKey(t, true)) toast('The clipboard has no Pubky key', 'error'); } catch { toast(`${appPlatform() ? 'The app' : 'The browser'} didn’t allow reading the clipboard: paste into the field instead`, 'error'); } };
              const scan = $('#pinvscan', box); scan.hidden = !canScan();
              scan.onclick = async () => { const k = await scanKey(); if (k) setKey(k); };
              $('#pinvgo', box).onclick = async () => {
                const kind = kindOf(); const name = $('#pinvname', box).value.trim();
                const body = { access: kind, name: name || undefined };
                if (kind === 'music') { const k = keyFrom(keyIn.value); if (!k) { if (keyIn.value.trim()) checkKey(); else toast('Paste or scan their Pubky key', 'error'); keyIn.focus(); return; } body.key = k; } else { const q = Number($('#pinvquota', box).value); if (q > 0) body.storage_quota_mb = Math.round(q); }
                let v; try { v = await api('POST', '/people/invites', body); } catch (e) { toast(e.message, 'error'); return; }
                if (kind === 'music') {
                  // What to send them: this node's address (the page's own) and how they log in
                  const at = location.origin; const say = `Open ${at} and log in with Pubky Ring.`;
                  // (a loopback address reaches the node only from its own machine: nothing to copy then)
                  const local = /^(localhost|127\.\d+\.\d+\.\d+|\[::1\])$/i.test(location.hostname);
                  pinvOut = `<div class="notice pa-send" role="status"><p><b>${name ? esc(name) : 'They'} can log in now.</b> ${local
                    ? `This page is open at <code class="pa-code">${esc(location.host)}</code>, an address that only works on the node’s own machine.</p><p class="pa-msg">Send them the node’s address on your network instead, and tell them to log in there with Pubky Ring.</p>`
                    : `Send them this:</p><p class="pa-msg">Open <code class="pa-code">${esc(at)}</code> and log in with Pubky Ring.</p><div class="actions"><button class="btn sm" data-copylink="${esc(say)}" data-fk="pinvsent">Copy the message</button></div>`}</div>`;
                  toast(local ? `Added ${name || 'them'}: they log in at this node’s address on your network` : `Added ${name || 'them'}: send them this node’s address`);
                } else {
                  pinvOut = inviteQr(v, hs.public_key);
                  toast('Invite made: send them the link or the QR code');
                }
                await loadPeople({ fresh: 'invite', focus: ['pinvsent', 'pinvname'] });
                const out = $('#pinvout'); if (out && out.firstElementChild) out.scrollIntoView({ behavior: 'smooth', block: 'center' });
              };
              $$('[data-pinv-del]', box).forEach((b) => b.onclick = async () => {
                const ids = r.invites.map((i) => i.id); const at = ids.indexOf(b.dataset.pinvDel);
                // (the invite shown under the form may be this one: it goes with it)
                try { await api('DELETE', `/people/invites/${encodeURIComponent(b.dataset.pinvDel)}`); pinvOut = ''; toast('Invite taken off the list. If it was sent, it still works on the homeserver.'); } catch (e) { toast(e.message, 'error'); }
                await loadPeople({ focus: [...[ids[at + 1], ids[at - 1]].filter(Boolean).map((id) => `invdel:${id}`), 'pinvname'] });
              });
            }

            // ---- zones ----
            const pzs = $('#pzsave', box);
            if (pzs) {
              const err = $('#pzerr', box); const list = $('#pzones', box);
              const say = (t) => { err.textContent = t; err.hidden = !t; };
              const mode = () => ($('input[name="pzmode"]:checked', box) || {}).value || '';
              if (draft.zones) {
                $$('input[name="pzmode"]', box).forEach((i) => { i.checked = i.value === draft.zones.mode; });
                $$('input', list).forEach((i) => { i.checked = draft.zones.picked.includes(i.value); });
              }
              const fitZones = () => { list.hidden = mode() !== 'some'; $('#pznonehelp', box).hidden = mode() !== 'none'; };
              $$('input[name="pzmode"]', box).forEach((i) => i.onchange = () => { zonesTouched = true; say(''); fitZones(); }); fitZones();
              $$('input', list).forEach((i) => i.onchange = () => { zonesTouched = true; say(''); });
              pzs.onclick = async () => {
                const m = mode(); const picked = $$('input:checked', list).map((i) => i.value);
                if (!m) { say('Choose “Every zone”, “Only these” or “No zone” first.'); $('input[name="pzmode"]', box).focus(); return; }
                // ("Only these" with none ticked would save an empty list, which the node reads as every zone)
                if (m === 'some' && !picked.length) { say('Tick at least one zone, or choose “Every zone” or “No zone”.'); $('input', list).focus(); return; }
                say('');
                const sent = m === 'some' ? picked : [];
                // No zone goes out as `none` with the list of one zone without a name: a node before
                // `none` keeps that list, and it opens no zone there either (an empty list would be
                // every zone: the opposite of what was asked)
                let saved;
                try { saved = await api('PUT', '/people/zones', m === 'none' ? { zones: [''], none: true } : { zones: sent }); } catch (e) { toast(e.message, 'error'); return; }
                if (m === 'none' && !(saved && (saved.none === true || noZoneList(saved.zones)))) {
                  // The node answered and did not keep it: the screen is drawn again from what the
                  // node has now, and says so where the choice is
                  await loadPeople({ fresh: 'zones', focus: ['pzsave'] });
                  const again = $('#pzerr', $('#peoplebox') || box); if (again) { again.textContent = 'This node did not keep “No zone”. The choice above is what it has now.'; again.hidden = false; }
                  return;
                }
                toast(m === 'none' ? 'The people you add play in their own browser or app only' : sent.length ? `The people you add can play on ${listWords(sent.map((id) => zoneName(id) || id))}` : 'The people you add can play on every zone');
                await loadPeople({ fresh: 'zones', focus: ['pzsave'] });
              };
            }

            // ---- whose music services ----
            $$('[data-srcpolicy]', box).forEach((b) => b.onclick = async () => {
              const v = b.dataset.srcpolicy; if (v === policy) return;
              try { await api('PUT', '/node/settings', { sources_policy: v }); ns.sources_policy = v; toast(SRC_POLICIES.find(([x]) => x === v)[4]); } catch (e) { toast(e.message, 'error'); }
              await loadPeople();
            });

            // ---- the password for apps ----
            const tn = $('#tokennew', box);
            // The new one is shown once, until the page reloads (also over a redraw of this screen)
            const showToken = () => {
              const out = $('#tokenout', box); if (!out || !state.tokenShown) return;
              out.innerHTML = `<p class="small pa-newpass">The new password for apps, shown only now. Copy it into your apps; the page then reloads, so its streams and covers use it too.</p><div class="toolbar"><code class="pa-code">${esc(state.tokenShown)}</code><button class="btn sm primary" id="tokendone">Copy and reload</button></div>`;
              $('#tokendone', box).onclick = async () => { try { await navigator.clipboard.writeText(state.tokenShown); } catch { /* copied by hand */ } location.reload(); };
              if (tn) tn.disabled = true;
            };
            if (tn) tn.onclick = async () => {
              const go = await choiceDialog({ title: 'Make a new password for apps?', text: 'The old one stops working at once. Every app then needs the new one, and your other browsers are logged out; this one goes on with the new one.', choices: [{ label: 'Make a new password', value: true, primary: true, danger: true }] });
              if (!go) return;
              try {
                const made = await api('POST', '/node/token');
                // Every later call (and a reconnect of the event socket) uses it: the old one and every
                // Pubky Ring log in of the owner's browsers end, and this browser takes the new one
                state.token = made.token; try { localStorage.setItem('pm.token', made.token); } catch { /* this tab keeps it */ }
                state.tokenShown = made.token; showToken();
                // The node just ended the owner's other browser log ins: the lists are read again
                // (the new password stays shown: every draw of this screen shows it until the reload)
                await loadPeople();
                const done = $('#tokendone', $('#peoplebox') || box); if (done) done.focus();
              } catch (e) { toast(e.message, 'error'); }
            };
            showToken();

            // ---- party ----
            const ptg = $('#ptgo', box); if (ptg) ptg.onclick = async () => {
              const body = { zone: $('#ptzone', box).value, name: $('#ptname', box).value.trim() || undefined, hours: Number($('#pthours', box).value), adds_per_hour: Number($('#ptadds', box).value) || 5, waiting_per_guest: Number($('#ptwait', box).value) || 3 };
              let made; try { made = await api('POST', '/party', body); } catch (e) { toast(e.message, 'error'); return; }
              toast('Party started: share the guest link');
              await loadPeople();
              // Its guest link, opened: that is what to share now
              const links = $$('details[data-keepopen^="party:"]', $('#peoplebox') || box);
              const mine = links.find((d) => made && d.dataset.keepopen === `party:${made.id}`) || links.pop();
              if (mine) { mine.open = true; mine.scrollIntoView({ behavior: 'smooth', block: 'center' }); $('summary', mine).focus({ preventScroll: true }); }
            };
            $$('[data-party-end]', box).forEach((b) => b.onclick = async () => {
              const ids = parties.map((p) => p.id); const at = ids.indexOf(b.dataset.partyEnd);
              try { await api('DELETE', `/party/${encodeURIComponent(b.dataset.partyEnd)}`); toast('Party ended: the guest link no longer works'); } catch (e) { toast(e.message, 'error'); }
              await loadPeople({ focus: [...[ids[at + 1], ids[at - 1]].filter(Boolean).map((id) => `partyend:${id}`), 'ptgo'] });
            });

            // The focus stays where it was; when that control is gone (a removed row), on the nearest one left
            if (had && !box.contains(document.activeElement)) {
              const byFk = (k) => $$('[data-fk]', box).find((el) => el.dataset.fk === k) || (/^[\w-]+$/.test(k) ? $(`#${k}`, box) : null);
              const to = [fkWas, ...(focus || [])].filter(Boolean).map(byFk).find((el) => el && el.getClientRects().length) || box;
              to.focus({ preventScroll: to === box });
            }
          };
          // Settings › Remote access: the bundled install's Cloudflare Tunnel
          const loadRemote = async (check = false) => {
            const box = $('#remotebox'); if (!box) return;
            if (check) { const b = $('#rtcheck', box); if (b) { b.disabled = true; b.innerHTML = '<span class="spin"></span> Checking…'; } }
            let t; try { t = await api('GET', `/remote/tunnel${check ? '?check=1' : ''}`); } catch (e) { box.innerHTML = `<p class="muted small">${esc(e.message)}</p>`; return; }
            if (!box.isConnected) return;
            const yes = (v, good, bad) => (v == null ? '' : v ? `<span class="pill ok">${good}</span>` : `<span class="pill warn">${bad}</span>`);
            box.innerHTML = group('Cloudflare Tunnel', (t.configured
                ? row('Homeserver', `<code>${esc(t.homeserver_domain || '')}</code>${t.published === false ? ' · not published yet' : t.published ? ' · published: Pubky apps find it at this address' : ''}`, yes(t.homeserver_reachable, 'Reachable', 'Not reachable'))
                  + (t.music_hostname ? row('This node', `<code>${esc(t.music_hostname)}</code>`, yes(t.music_reachable, 'Reachable', 'Not reachable')) : '')
                  + (t.restart_needed ? `<div class="notice warn" role="status">Restart the homeserver once so it publishes the address. ${t.restart_hint ? esc(t.restart_hint) : `On the machine, in <code>deploy/bundle</code>: <code>${esc(t.restart_command || '')}</code> <button class="btn sm ghost" data-copylink="${esc(t.restart_command || '')}">Copy</button>`}</div>` : '')
                  + row('Check from the internet', 'Opens both addresses over HTTPS, as someone on the internet would.', '<button class="btn" id="rtcheck">Check now</button>')
                : row('Not set up', 'Reach this node and its homeserver from anywhere, with no open ports or static IP.', ''))
              + block(t.configured ? 'Change the tunnel' : 'Set up', 'In Cloudflare Zero Trust › Networks › Tunnels, create a tunnel and give it two public hostnames: one to <code>http://localhost:6286</code> (the homeserver), one to <code>http://localhost:7788</code> (this node). Paste the tunnel’s token and the hostnames here. With the compose bundle, start cloudflared once: <code>docker compose --profile tunnel up -d</code> (the bare-metal install starts it by itself).', `<form class="stack" id="rtform"><input class="input" id="rttoken" type="password" autocomplete="off" placeholder="${t.configured ? 'Tunnel token (saved; paste to replace)' : 'Tunnel token'}" aria-label="Cloudflare tunnel token"><input class="input" id="rths" placeholder="Homeserver hostname, e.g. pubky.example.org" aria-label="Homeserver hostname" value="${esc(t.homeserver_domain || '')}"><input class="input" id="rtmusic" placeholder="This node’s hostname (optional), e.g. music.example.org" aria-label="This node's hostname" value="${esc(t.music_hostname || '')}"><div class="actions"><button class="btn primary" id="rtsave" type="submit">Save tunnel</button>${t.configured ? '<button class="btn ghost" id="rtdel" type="button">Remove tunnel</button>' : ''}</div></form>`))
              + group('Other ways', row('Your own domain', 'A static IP, ports 80, 443 and 6287 forwarded, and Caddy in front: see “With its own Pubky homeserver” in the deploy guide (docs/deploy.md).', ''));
            const ck = $('#rtcheck', box); if (ck) ck.onclick = () => loadRemote(true);
            $('#rtform', box).onsubmit = async (e) => {
              e.preventDefault();
              const token = $('#rttoken', box).value.trim(); const homeserver_domain = $('#rths', box).value.trim(); const music_hostname = $('#rtmusic', box).value.trim() || undefined;
              if (!token) { toast('Paste the tunnel’s token', 'error'); return; }
              if (!homeserver_domain) { toast('Enter the homeserver’s hostname', 'error'); return; }
              try { await api('PUT', '/remote/tunnel', { token, homeserver_domain, music_hostname }); toast('Tunnel saved'); loadRemote(); } catch (err) { toast(err.message, 'error'); }
            };
            const del = $('#rtdel', box); if (del) del.onclick = async () => { try { await api('DELETE', '/remote/tunnel'); toast('Tunnel removed'); loadRemote(); } catch (e) { toast(e.message, 'error'); } };
          };
          // Pubky Ring: a QR and link, then wait for the approval (a sign-in, or creating the owner's
          // account on the bundled homeserver, which signs the node in the same way)
          const ringFlow = async (path, what) => {
            try {
              const r = await api('POST', path);
              $('#pkurl').innerHTML = `<div class="notice qrbox" style="margin-top:12px">${r.qr_svg ? `<div class="qr">${r.qr_svg}</div>` : ''}<div><p>Scan with Pubky Ring, or open the link on this device and approve${what}. This page updates by itself.</p><p class="muted small">Pubky Ring is the free app (iPhone and Android, from your app store) that holds your Pubky identity; this node never sees your keys.</p><a class="btn sm" href="${esc(ringUrl(r.url))}">Open in Pubky Ring</a> <button class="btn sm ghost" id="pkcopy">Copy link</button></div></div>`;
              $('#pkurl').scrollIntoView({ behavior: glide(), block: 'center' }); // the QR, not below the fold
              const pc = $('#pkcopy'); if (pc) pc.onclick = () => { navigator.clipboard && navigator.clipboard.writeText(r.url).then(() => toast('Link copied')).catch(() => toast(r.url)); };
              const t = setInterval(async () => { try { const st = await api('GET', '/pubky/login'); if (st.status === 'signed_in') { clearInterval(t); forgetPubkyUser(); toast('Signed in as ' + shortKey(st.user)); render(); } } catch (e) { clearInterval(t); toast(e.message, 'error'); } }, 2000);
            } catch (e) { toast(e.message, 'error'); }
          };
          const pl = $('#pklogin'); if (pl) pl.onclick = () => ringFlow('/pubky/login', '');
          const hc = $('#pkhscreate'); if (hc) hc.onclick = () => ringFlow('/homeserver/bundled/owner-signup', ': it creates your account on this node\u2019s homeserver');
          if (state.autoCreate && hc) { state.autoCreate = false; hc.click(); hc.scrollIntoView({ block: 'center' }); }
          loadPeople(); loadRemote();
          if (state.autoResign && pl) { state.autoResign = false; pl.click(); pl.scrollIntoView({ block: 'center' }); }
          break;
        }

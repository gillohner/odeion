        case 'item': {
          // This render's generation: an answer (or failure) from an older render, or from a page
          // left meanwhile, is dropped (the same page opened again is a newer render)
          const gen = state.pageGen = (state.pageGen || 0) + 1;
          const { kind, id } = state.viewArg;
          const live = () => gen === state.pageGen && state.view === 'item' && state.viewArg.id === id;
          await pubkyUser();
          // Account discovery can finish after Back has restored another page.
          if (!live()) return;
          // Retry asks the node to fetch the service's lists again, once
          const refresh = state.viewArg.refresh ? '?refresh=1' : ''; delete state.viewArg.refresh;
          const pageKey = `item:${kind}:${id}`; const again = !refresh && redrawing(pageKey);
          if (!again) loading();
          let pg;
          try { [pg] = await Promise.all([api('GET', `/item/${kind}/${encodeURIComponent(id)}${refresh}`), loadSvcRank()]); }
          catch (e) { if (!live()) return; state.drawnPage = null; if (state.placeHold) state.placeHold(); main.innerHTML = backBtn(kind === 'artist' ? 'artists' : 'albums', kind === 'artist' ? 'Artists' : 'Albums') + failedHtml(e); break; }
          if (!live()) return;
          // The node leaves empty lists out of its answer.
          for (const k of ['songs', 'albums', 'versions', 'also_on', 'about', 'open_on', 'connect']) pg[k] = pg[k] || [];

          state.itemTitle = pg.title || `Untitled on ${pg.source}`; state.itemSection = pg.section;
          $$('.nav-item').forEach((b) => b.classList.toggle('on', b.dataset.view === pg.section));
          const fallback = { artist: pg.artist && pg.artist.name, album: kind === 'album' ? pg.title : '', image: pg.image };
          const songs = pg.songs || [];
          const target = pg.target;
          // What comments are about: the song's own link, else this item's (a web link only). The
          // section is drawn, and the comments asked for, only for who may comment: can('owner_music')
          const commentUri = offers('pubky') ? [pg.song_uri, target && target.uri, pg.link].find((u) => /^https?:\/\//.test(u || '')) || '' : '';
          const qs = new URLSearchParams(Object.entries(target).map(([k, v]) => [k, String(v)])).toString();
          // Tags go on the song (every source) or on exactly this source's link (STEWARD-UX-INSTANCES 3),
          // each on its own URI: without a MusicBrainz identity there is only the link, never a guess
          const localRef = target.track_id != null || target.album_id != null || target.artist_id != null;
          const songRef = localRef ? target : pg.song_uri ? { uri: pg.song_uri } : /^https:\/\/musicbrainz\.org\//.test(target.uri || '') ? target : null;
          const linkRef = pg.link && pg.link !== (songRef && songRef.uri) ? { uri: pg.link } : !songRef && target.uri ? target : null;
          const hereName = pg.source === 'Your library' ? 'your library' : pg.source;
          const refQs = (r) => new URLSearchParams(Object.entries(r).map(([k, v]) => [k, String(v)])).toString();
          // An item with no identity yet (needs_match) takes no tags: the hero says so and leads to a match
          const ownTags = (ref) => (!ref || isListener() || !offers('tags') ? Promise.resolve({ tags: null }) : api('GET', `/tags?${refQs(ref)}`).then((r) => ({ tags: r.needs_match ? null : r.tags || [], needsMatch: !!r.needs_match, uri: r.uri || null })).catch(() => ({ tags: null })));
          const [songOwn, linkOwn, myReview] = await Promise.all([
            ownTags(songRef), ownTags(linkRef),
            isListener() || !offers('reviews') ? Promise.resolve(undefined) : api('GET', `/reviews?${qs}`).then((r) => (r.needs_match ? undefined : (r.reviews || [])[0] || null)).catch(() => undefined),
          ]);
          if (!live()) return;
          // The hero's tag groups. Public tags and people come from Pubky for each group's own URI: for a
          // library item the one the node keeps its tags on (GET /tags says; a listener's page asks
          // Pubky), never the page's guess. The page's on_pubky stands in only when it is that URI.
          const shownSong = !!songRef && !songOwn.needsMatch;
          const songUri = localRef ? songOwn.uri : songRef && songRef.uri;
          // Drawn again: the last answers this page had stand in until the new ones come (pubKey)
          if (!again || heroPub.page !== pageKey) heroPub = { page: pageKey, by: new Map() };
          const kept = heroPub.by;
          const pubKey = (g) => `${g.scope}|${refQs(g.ref)}`;
          // (kept: { ask, p }; p null when the newest answer said "no identity")
          const keptOf = (scope, ref) => { const k = kept.get(pubKey({ scope, ref })); return k && k.p; };
          const pubFrom = (uri, scope, ref) => (uri && pg.on_pubky && pg.on_pubky.uri === uri ? { state: 'ok', tags: pg.on_pubky.tags || [], faces: pg.on_pubky.faces || {} } : keptOf(scope, ref) ? { ...keptOf(scope, ref), kept: true } : { state: 'loading', tags: [], faces: {} });
          const linkName = pg.source || 'source';
          const tagGroups = !offers('tags') ? [] : [
            ...(shownSong ? [{ scope: 'song', name: `On this ${kind}`, ref: songRef, own: songOwn.tags, pub: pubFrom(songUri, 'song', songRef) }] : []),
            ...(linkRef ? [{ scope: 'link', name: `On the ${linkName} link`, ref: linkRef, own: linkOwn.tags, pub: pubFrom(linkRef.uri, 'link', linkRef) }] : []),
          ];
          const htKey = `${kind}:${id}:${++heroTagsGen}`;
          const ht = { key: htKey, page: `${kind}:${id}`, kind, linkName, groups: tagGroups, canEdit: !isListener(), note: '', match: null, open: state.heroTagsOpen && state.heroTagsOpen.page === `${kind}:${id}` ? state.heroTagsOpen.set : new Set() };
          // No identity: say so; the owner's album leads to its folder in the matches to confirm
          const unmatched = () => { ht.note = 'Not matched to MusicBrainz yet, so other apps can\'t tell what it is.'; ht.match = kind === 'album' && !isListener() ? { label: 'Match this album to tag it', tracks: songs.map((t) => t.track_id).filter((x) => x != null) } : null; };
          if (localRef && songOwn.needsMatch) unmatched();
          else if (!songRef && linkRef) ht.note = `Tags here go on this ${linkName} link only: no MusicBrainz ${kind} is known for it here.`;
          state.heroTagsOpen = { page: `${kind}:${id}`, set: ht.open };
          const favKey = pg.uri; const isFav = () => !!(state.favUris && ((favKey && state.favUris.has(favKey)) || (target.track_id != null && state.favUris.has(`track:${target.track_id}`)) || (target.album_id != null && state.favUris.has(`album:${target.album_id}`)) || (target.artist_id != null && state.favUris.has(`artist:${target.artist_id}`)))); const fav = isFav();
          const sectionName = { albums: 'Albums', artists: 'Artists', tracks: 'Songs', home: 'Home', playlists: 'Collections' }[pg.section] || 'Home';
          const pf = pg.plays_from;
          const where = pf ? (pf.service === 'library' ? 'your library' : (SERVICE_NAMES[pf.service] || pf.service) + (pf.mode === 'preview' ? ' (previews)' : '')) : null;
          // Honest about both: where this page is from, and how it plays here (STEWARD-UX-INSTANCES 7)
          const ownSvc = pg.source_id || (kind && String(id).startsWith('lib-') ? 'library' : 'link');
          const isGlobal = ownSvc === 'global';
          const ownName = ownSvc === 'library' ? 'Your library' : pg.source;
          // The cover's badge: an album or song page says where it is from; an artist has no cover
          const heroBadge = kind === 'artist' ? '' : coverBadge(ownSvc);
          // What Play will do, from the node's one decision (plays_from + plays_note)
          const playsHere = !pf ? (pg.connect.length ? `plays on ${pg.connect.map((c) => SERVICE_NAMES[c] || c).join(', ')}` : '') : pf.service === ownSvc ? (pf.mode === 'preview' ? '30 s preview' : '') : pf.mode === 'connect' ? `plays on your ${SERVICE_NAMES[pf.service] || pf.service} device` : `plays from ${where}${isGlobal ? '' : ' here'}`;
          const eyebrow = [`${{ song: 'Song', album: 'Album', artist: 'Artist' }[kind]}${isGlobal ? '' : ownSvc === 'library' ? ' in your library' : ` on ${pg.source}`}`, pg.plays_note && !isGlobal ? `${pg.plays_note.charAt(0).toLowerCase()}${pg.plays_note.slice(1)}` : '', playsHere].filter(Boolean).join(' · ');
          const by = [pg.artist ? (pg.artist.id ? `<a class="lnk" id="iartist"${hrefAttr(pathFor('item', { kind: 'artist', id: pg.artist.id }))}>${esc(pg.artist.name)}</a>` : `<a class="lnk" id="iartistq"${hrefAttr(pathFor('search', { q: pg.artist.name }))}>${esc(pg.artist.name)}</a>`) : '', pg.album ? (pg.album.id ? `<a class="lnk" id="ialbum"${hrefAttr(pathFor('item', { kind: 'album', id: pg.album.id }))}>${esc(pg.album.name)}</a>` : esc(pg.album.name)) : '', pg.year].filter(Boolean).join(' · ');
          const empty = (t) => `<p class="muted small">${t}</p>`;
          const songsTitle = kind === 'artist' ? 'Top songs' : kind === 'song' ? 'Song' : 'Songs';
          const songsInfo = songs.length ? `${songs.length} song${songs.length === 1 ? '' : 's'}${pg.total_ms ? ' · ' + mins(pg.total_ms) : ''}` : '';
          const instName = (v) => (v.source === 'library' ? 'Your library' : SERVICE_NAMES[v.source] || String(v.label || v.source).split(' · ')[0]);
          // The source line (STEWARD-UX-PLAYER-SOURCES §1): a quiet line under the title. This page's
          // own source first; then the same recording on services that are on and working, in the
          // owner's play order; the rest behind "more". Services switched off are left out.
          const rk = state.svcRank && !state.svcRank.failed ? state.svcRank : null;
          const alsoSvc = (v) => srcSvc(v.source);
          const alsoAll = rankSvc(pg.also_on.map((v, k) => ({ v, k })).filter(({ v }) => !(rk && rk.off.has(alsoSvc(v)))), ({ v }) => alsoSvc(v));
          const alsoFirst = rk ? alsoAll.filter(({ v }) => rk.healthy.has(alsoSvc(v))) : alsoAll;
          const alsoMore = rk ? alsoAll.filter(({ v }) => !rk.healthy.has(alsoSvc(v))) : [];
          const alsoItem = ({ v, k }) => `<span class="src">${svcBadge(alsoSvc(v))}${v.page ? `<button type="button" class="lnk" data-also="${k}" title="${esc(v.label)}">${esc(instName(v))}</button>` : `<span>${esc(instName(v))}</span>`}${v.uri || v.source === 'library' || /^(tidal|deezer)$/.test(v.source) || ytOf(v.url) ? `<button class="chiplink" data-aplay="${k}" title="Play from ${esc(instName(v))}" aria-label="Play from ${esc(instName(v))}">${icon('play')}</button>` : ''}${webUrl(v.url) ? `<a class="chiplink" href="${esc(webUrl(v.url))}" target="_blank" rel="noopener" title="Open in ${esc(instName(v))} (new tab)" aria-label="Open in ${esc(instName(v))} (new tab)">${icon('external')}</a>` : ''}</span>`;
          // Artists and albums (TASK-pinakes-social-ux F6): "Also on" is a row of service icons next to
          // Play/Shuffle, the working services first. One opens the service's page here when the node
          // plays from it (on and working: Play is there), else the service's site (every link has a
          // page here, so `page` alone says nothing). Songs keep the source line with its play buttons.
          const iconAlso = kind === 'artist' || kind === 'album';
          const playsHereFrom = (v) => !!v.page && (rk ? rk.healthy.has(alsoSvc(v)) : alsoSvc(v) === 'library');
          // Two groups (owner, 2026-11-04): "Also on" switches to that service's page here (the node
          // plays from it, or only its page exists); "Open in" leaves Odeion for the service's site in
          // a new tab. A service that does both sits in Also on; its site is in ⋯ › Open in…
          const switchHere = ({ v }) => playsHereFrom(v) || (!!v.page && !webUrl(v.url));
          const alsoHere = alsoAll.filter(switchHere); const openOut = alsoAll.filter((x) => !switchHere(x) && webUrl(x.v.url));
          const alsoIcon = ({ v, k }) => { const name = instName(v); return `<button type="button" class="icon-btn also-ico" data-also="${k}" title="Switch to ${esc(name)}" aria-label="Switch to ${esc(name)}">${svcBadge(alsoSvc(v))}</button>`; };
          const openIcon = ({ v }) => { const name = instName(v); return `<a class="icon-btn also-ico" href="${esc(webUrl(v.url))}" target="_blank" rel="noopener" title="Open in ${esc(name)} (new tab)" aria-label="Open in ${esc(name)} (new tab)">${svcBadge(alsoSvc(v))}</a>`; };
          const alsoIcons = !iconAlso ? '' : (alsoHere.length ? `<span class="also-icons" role="group" aria-labelledby="alsolabel"><span class="also-label muted small" id="alsolabel">Also on</span>${alsoHere.map(alsoIcon).join('')}</span>` : '')
            + (openOut.length ? `<span class="also-icons open-in" role="group" aria-labelledby="openinlabel"><span class="also-label muted small" id="openinlabel">Open in ${icon('external')}</span>${openOut.map(openIcon).join('')}</span>` : '');
          const [lineFirst, lineMore] = iconAlso ? [[], []] : [alsoFirst, alsoMore];
          const ownItem = `<span class="src own" title="This page is ${esc(ownName)}'s">${svcBadge(ownSvc === 'library' ? 'library' : ownSvc)}<span>${esc(ownName)}</span>${webUrl(pg.link) ? `<a class="chiplink" href="${esc(webUrl(pg.link))}" target="_blank" rel="noopener" title="Open in ${esc(ownName)} (new tab)" aria-label="Open in ${esc(ownName)} (new tab)">${icon('external')}</a>` : ''}</span>`;
          // (nothing but the page's own source, with no link out, says only what the eyebrow says:
          // "Album in your library" over "From Your library". No line then; the global page link stays)
          const lineEmpty = !lineFirst.length && !lineMore.length && (isGlobal || !webUrl(pg.link));
          const alsoLine = (lineEmpty ? '' : `<div class="srcline" id="ialso">${isGlobal ? '<span class="muted">On</span>' : `<span class="muted">From</span>${ownItem}`}${!isGlobal && lineFirst.length ? '<span class="muted">· also</span>' : ''}${lineFirst.map(alsoItem).join('')}${lineMore.length ? `${!isGlobal && !lineFirst.length ? '<span class="muted">·</span>' : ''}<button type="button" class="lnk srcmore" data-srcmore aria-label="Show ${lineMore.length} more source${lineMore.length === 1 ? '' : 's'}">${lineFirst.length || !isGlobal ? `${lineMore.length} more` : `${lineMore.length} services`}</button><span class="srcrest" hidden>${lineMore.map(alsoItem).join('')}</span>` : ''}</div>`)
            + (pg.source_id !== 'global' && pg.song_uri ? `<p class="small to-global"><a class="lnk" id="iglobal"${hrefAttr(pathFor('item', { kind, id: idOf(pg.song_uri) }))}>All about this ${kind} →</a> <span class="muted">every source, versions and tags</span></p>` : '');
          const versionChip = (v, k) => `<span class="chip">${svcBadge(v.source === 'library' ? 'library' : v.source)} ${esc(v.label)}${v.uri || /^(tidal|deezer)$/.test(v.source) || v.source === 'library' || ytOf(v.url) ? ` <button class="chiplink" data-vplay="${k}" title="Play this version">${icon('play')}</button>` : ''}${webUrl(v.url) ? ` <a class="chiplink" href="${esc(webUrl(v.url))}" target="_blank" rel="noopener" title="Open">${icon('external')}</a>` : ''}</span>`;
          // Deep link with no history: back goes to the parent (song → album → artist → section).
          const parent = kind === 'song' && pg.album && pg.album.id ? { kind: 'album', ...pg.album } : kind !== 'artist' && pg.artist && pg.artist.id ? { kind: 'artist', ...pg.artist } : null;
          const crumbs = kind === 'artist' ? [] : [pg.artist && pg.artist.id ? ['artist', pg.artist] : null, kind === 'song' && pg.album && pg.album.id && pg.album.name !== state.itemTitle ? ['album', pg.album] : null].filter(Boolean);
          const crumbBar = crumbs.length ? `<nav class="crumbs" aria-label="Breadcrumb"><ol>${crumbs.map(([k, l]) => `<li><button class="lnk" data-crumb-kind="${k}" data-crumb-id="${esc(l.id)}">${esc(l.name)}</button></li>`).join('')}<li aria-current="page">${esc(state.itemTitle || '')}</li></ol></nav>` : '';
          if (isListener()) pg.about = pg.about.filter((f) => f.label !== 'File');
          // About (F6): the article's opening (Wikipedia in the reader's language, or TheAudioDB) with
          // its link and licence, the picture's Commons credit, then the facts. With an article it
          // sits under the header; without one it stays at the end of the page. A function: when open
          // sources answer later (about_pending) it is redrawn in place.
          const aboutTop = !!(pg.about_text && pg.about_text.text); // artist, album and song (F5: the same About everywhere)
          // A file is its name; where it is on the node's disk is a tooltip, not page text (UX sweep A21)
          const fileFact = (f) => { if (f.label !== 'File' || !/[\\/]/.test(String(f.value || ''))) return esc(f.value); const v = String(f.value); const name = v.split(/[\\/]/).pop(); return `<span class="filefact" title="${esc(v)}">${esc(name)}</span>`; };
          // With an article the page shows the music first (owner, 2026-11-05: "the page shows
          // artist metadata and songs but not less relevant metadata directly"): the blurb and one
          // facts line under the hero, the ids, links and credits in Details at the foot. The node
          // says which fact is which (`kind`: fact or ref).
          const isRef = (f) => f.kind === 'ref';
          const factValue = (f) => (webUrl(f.href) ? `<a href="${esc(webUrl(f.href))}" target="_blank" rel="noopener">${esc(f.value)}</a>` : fileFact(f));
          const aboutTopHtml = (p) => {
            const at = p.about_text;
            // The text's source right after it (Wikipedia, TheAudioDB…); its licence sits in Details
            const wiki = at && at.url && at.source && webUrl(at.url) ? ` <a class="small wikilink" href="${esc(webUrl(at.url))}" target="_blank" rel="noopener">${esc(at.source)}</a>` : '';
            const facts = p.about.filter((f) => !isRef(f) && f.label !== 'In your library');
            const lib = p.about.find((f) => f.label === 'In your library');
            return `<section class="section about-top" id="iabout"><div class="section-head"><h3>About</h3></div><p class="blurb clamp" id="iblurb"${at.lang ? ` lang="${esc(at.lang)}"` : ''}>${esc(at.text)}</p><p class="blurb-foot"><button type="button" class="lnk small" id="iblurbmore" aria-controls="iblurb" aria-expanded="false" hidden>More</button>${wiki}</p>${facts.length ? `<p class="facts-line">${facts.map((f) => `<span class="fact" title="${esc(f.label)}">${f.label === 'Active' ? 'Active ' : ''}${esc(f.value)}</span>`).join('<span class="dot" aria-hidden="true"> · </span>')}</p>` : ''}${lib ? `<p class="muted small libline">In your library: ${esc(lib.value)}</p>` : ''}</section>`;
          };
          const aboutDetailsHtml = (p) => {
            const refs = p.about.filter(isRef);
            const credit = aboutCredit(p);
            if (!refs.length && !credit) return '';
            return `<details class="section idetails" id="idetails"${window.innerWidth >= 600 ? ' open' : ''}><summary><h3>Details</h3></summary>${credit}${refs.length ? `<dl class="kv">${refs.map((f) => `<dt>${esc(f.label)}</dt><dd>${factValue(f)}</dd>`).join('')}</dl>` : ''}</details>`;
          };
          const aboutHtml = (p) => {
            const at = p.about_text;
            const clamp = !!(at && at.text);
            const blurbText = (at && at.text) || p.blurb;
            return `<section class="section" id="iabout"><div class="section-head"><h3>About</h3></div>${blurbText ? `<p class="blurb${clamp ? ' clamp' : ''}" id="iblurb"${at && at.lang ? ` lang="${esc(at.lang)}"` : ''}>${esc(blurbText)}</p>${clamp ? '<button type="button" class="lnk small" id="iblurbmore" aria-controls="iblurb" aria-expanded="false" hidden>More</button>' : ''}` : ''}${aboutCredit(p)}${p.about.length ? `<dl class="kv">${p.about.map((f) => `<dt>${esc(f.label)}</dt><dd>${webUrl(f.href) ? `<a href="${esc(webUrl(f.href))}" target="_blank" rel="noopener">${esc(f.value)}</a>` : fileFact(f)}</dd>`).join('')}</dl>` : blurbText ? '' : empty('Nothing more is known yet.')}</section>`;
          };
          const place = again ? keepPlace() : null;
          main.innerHTML = `<div class="pagebar">${backBtn(parent ? `item:${parent.kind}:${parent.id}` : pg.section, parent ? parent.name : sectionName)}${crumbBar}</div>`
            + (pg.reading ? `<p class="muted small reading">Reading this link on Pinakes… versions and tags appear in a moment.</p>` : '')
            + (pg.pending ? `<p class="muted small reading" role="status"><span class="spin"></span> Getting the songs and albums${pg.plays_from && pg.plays_from.service ? ` from ${esc(SERVICE_NAMES[pg.plays_from.service] || pg.plays_from.service)}` : ''}…</p>` : '')
            + `<div class="hero item-hero"><div class="art ${pg.image ? (kind === 'artist' ? 'round ' : '') : 'placeholder '}monogram" style="background:linear-gradient(135deg, hsl(${hue(state.itemTitle)} 35% 34%), hsl(${hue(state.itemTitle)} 35% 20%));border-radius:${kind === 'artist' ? '50%' : '12px'}"><span aria-hidden="true">${esc(initials(state.itemTitle))}</span>${pg.image ? heroImg(pg.image, kind === 'artist') : ''}${heroBadge}</div><div><div class="eyebrow">${esc(eyebrow)}</div><h1>${esc(state.itemTitle)}</h1><p class="sub">${by}</p>${alsoLine}${tagGroups.length || ht.note ? `<div class="htags" id="htags" data-for="${esc(htKey)}"></div>` : ''}
              <div class="actions" style="margin-top:12px"><button class="btn primary" id="iplay">${icon('play')} Play</button>${kind !== 'song' ? `<button class="btn" id="ishuffle">${icon('shuffle')} Shuffle</button>` : ''}<button class="btn hero-zone" id="izone" aria-label="Play on…" title="Play on…">${icon('zone')}<span class="lbl"> Play on…</span></button><button class="btn icon-only ${fav ? 'on' : ''}" id="ifav" aria-pressed="${!!fav}" title="${fav ? 'Remove from Bookmarks' : 'Add to Bookmarks'}" ${favKey || target.track_id || target.album_id || target.artist_id ? '' : 'disabled'}>${icon(fav ? 'heart-fill' : 'heart')}</button><button aria-haspopup="menu" aria-expanded="false" class="btn ghost" id="imore" title="More">${icon('more')}</button>${alsoIcons}</div>
              ${myReview !== undefined ? reviewRow(myReview) : ''}
              <div class="hmatch-slot" id="hmatch"></div>
              </div></div>`
            + (aboutTop ? aboutTopHtml(pg) : '')
            + (kind === 'song' ? '' : `<section class="section" id="isongs"><div class="section-head"><h3>${songsTitle}</h3><span class="muted small">${esc(songsInfo)}</span></div>${songs.length ? songRows(songs, { showAlbum: kind !== 'album', covers: kind !== 'album', ...(kind === 'artist' ? { numbers: 'index', pageArtist: { name: pg.title, id: pg.id } } : {}), ...(kind === 'album' ? { listClass: 'nums', oneArtist: songs.length > 1 && songs.every((t) => (t.artist || '') === (songs[0].artist || '')) && !!songs[0].artist } : {}) }) : pg.unavailable ? empty(`${esc(pg.unavailable)}${pg.link ? ` <a href="${esc(webUrl(pg.link))}" target="_blank" rel="noopener">Open on ${esc(pg.source)}</a>` : ''}`) : empty(pg.connect.length ? `Not on your services · ${connectLink(pg.connect)}` : 'No connected service lists songs for this yet.')}</section>`)
            + (kind === 'song' && pg.albums.length ? `<section class="section" id="ialbums"><div class="section-head"><h3>On the album</h3><span class="muted small">${pg.albums.length}</span></div>${itemTileGrid(pg.albums, 'album')}</section>` : '')
            // An artist's releases: Albums, EPs & singles, Other releases (each with View all), or
            // what the list is when it isn't there: loading, an error with Retry, nothing found
            + (kind === 'artist' ? `<div id="ialbums" class="relsecs">${relStatus(pg, (state.viewArg.tries || 0) >= 12)}${pg.albums.length ? relSections(pg) : pg.pending ? `<section class="section"><div class="section-head"><h3>Releases</h3></div>${relSkeleton()}</section>` : pg.catalogue && pg.catalogue.error ? '' : `<section class="section"><div class="section-head"><h3>Releases</h3></div><p class="muted small relempty">No releases found on your services yet.</p></section>`}</div>` : '')
            + '<div id="iconcerts-slot"></div>'
            + (kind === 'song' && pg.versions.length ? `<section class="section" id="iversions"><div class="section-head"><h3>Versions</h3><span class="muted small">other recordings: live, remastered, edits</span></div><div class="list compact">${pg.versions.map((v, k) => `<div class="list-row">${svcBadge(v.source === 'library' ? 'library' : v.source)}<div class="grow"><div class="name">${v.page ? `<a class="lnk" data-vpage="${k}"${hrefAttr(pathFor('item', { kind, id: v.page }))}>${esc(v.label.split(' · ')[0])}</a>` : esc(v.label.split(' · ')[0])}</div><div class="sub">${esc(v.label.split(' · ').slice(1).join(' · '))}</div></div><button class="icon-btn" data-vplay="${k}" title="Play this version" aria-label="Play this version">${icon('play')}</button></div>`).join('')}</div></section>` : '')
            + '<div id="isimilar-slot"></div>'
            + '<div id="icollections-slot"></div>'
            + (commentUri && can('owner_music') ? `<section class="section" id="icomments" data-curi="${esc(commentUri)}"><div class="section-head"><h3>Comments</h3></div><div class="comments-body"><div class="skeleton" style="height:48px"></div></div></section>` : '')
            + (aboutTop ? aboutDetailsHtml(pg) : aboutHtml(pg));
          $('[data-back]', main).onclick = () => goBack(pg.section);
          // The article's opening is clamped to a few lines; More shows it all. Whether it is cut off
          // is measured again when its width or the text size changes (zoom, rotation, a late font)
          const bindBlurb = () => { const bl = $('#iblurb', main); const bm = $('#iblurbmore', main);
            if (bl && bm) {
              const sync = () => { if (bl.classList.contains('clamp')) bm.hidden = !(bl.scrollHeight > bl.clientHeight + 2); };
              bm.onclick = () => { const open = bl.classList.toggle('clamp') === false; bm.textContent = open ? 'Less' : 'More'; bm.setAttribute('aria-expanded', String(open)); };
              sync(); if (window.ResizeObserver) new ResizeObserver(sync).observe(bl);
            } };
          bindBlurb();
          // One way back: on wide screens the breadcrumb stands in for Back when Back goes to a crumb
          { const bk = $('.pagebar > [data-back]', main);
            const same = bk && $$('[data-crumb-id]', main).find((c) => bk.textContent.trim().endsWith(c.textContent.trim()));
            if (same) { bk.classList.add('dupback'); same.dataset.asBack = '1'; }
            // Back to a page with this page's name (a song from its album of the same name) says Back
            if (bk && bk.textContent.trim() === String(pg.title || '').trim()) bk.lastChild.textContent = ' Back'; }
          const ig = $('#iglobal'); if (ig) ig.onclick = inAppClick(() => setView('item', { kind, id: idOf(pg.song_uri) }));
          if ($('#isongs')) { bindSongRows($('#isongs'), songs, { plays_from: pf, fallback }); enableSelect($('#isongs'), songs, { plays_from: pf, fallback }); }
          if (kind === 'artist') { bindRelSections($('#ialbums'), pg, id); const rr = $('#irelretry', main); if (rr) rr.onclick = () => { state.viewArg.tries = 0; state.viewArg.refresh = 1; render(); }; }
          else if (pg.albums.length) bindItemTiles($('#ialbums'), pg.albums, 'album');
          // Related items and concerts come after the page (each outside call has a time budget)
          api('GET', `/item/${kind}/${encodeURIComponent(id)}/extras`).then((ex) => {
            if (!live()) return;
            pg.related = ex.related || []; pg.concerts = ex.concerts || [];
            // Similar (TASK-pinakes-social-ux A3): the library's own first (shared genres, then the
            // same artist; playable), Last.fm's similar artists on an artist page, then what people
            // tagged alike on Pubky. One section; each part only when it has something.
            const ss = $('#isimilar-slot'); if (ss) (async () => {
              let fm = null;
              if (kind === 'artist' && target.artist_id != null) { try { fm = await api('GET', `/library/artists/${target.artist_id}/similar`); } catch { fm = null; } }
              if (!ss.isConnected) return;
              const mine = ex.similar || []; const fmArtists = (fm && fm.artists) || [];
              const mineSongs = kind === 'song' ? mine.map((h) => ({ ...h, artist: h.subtitle })) : [];
              const mineTiles = kind === 'song' ? [] : mine.map((h) => ({ ...h, year: h.subtitle || h.year || '' }));
              const part = (label, body, id) => `<div class="simpart" id="${id}"><h4 class="simhead muted small">${label}</h4>${body}</div>`;
              // Tagged alike on Pubky: music only, by the item's own kind (a coin, a paper, a place or a
              // game tagged alike isn't similar music); drawn and bound from this one list
              const rel = pg.related.filter((it) => SIMILAR_KINDS.includes(it.kind));
              const parts = [
                mine.length ? part('In your library', kind === 'song' ? songRows(mineSongs, { showAlbum: true, covers: true }) : itemTileGrid(mineTiles, kind), 'isimlib') : '',
                fmArtists.length ? part('From Last.fm', `<div class="chips">${fmArtists.map((a) => a.library_artist_id != null ? `<button class="chip" data-simlib="${a.library_artist_id}">${esc(a.name)}</button>` : `<button class="chip" data-simq="${esc(a.name)}" title="Not in your library: search">${esc(a.name)}</button>`).join('')}</div>`, 'isimfm') : '',
                rel.length ? part('Tagged alike on Pubky', itemTileGrid(rel.map(asTile), 'album'), 'irelated') : '',
              ].filter(Boolean);
              if (!parts.length) { ss.remove(); return; }
              ss.outerHTML = `<section class="section" id="isimilar"><div class="section-head"><h3>Similar</h3></div>${parts.join('')}</section>`;
              const lib = $('#isimlib', main);
              if (lib && kind === 'song') bindSongRows(lib, mineSongs, {}); else if (lib) bindItemTiles(lib, mineTiles, kind);
              $$('[data-simlib]', main).forEach((b) => b.onclick = () => setView('item', { kind: 'artist', id: 'lib-' + b.dataset.simlib }));
              $$('[data-simq]', main).forEach((b) => b.onclick = () => { search.value = b.dataset.simq; runSearch(); });
              if (rel.length) bindDiscoverTiles($('#irelated', main), rel);
            })();
            // In collections (TASK-in-collections-client): the public collections on Pubky holding this
            // item, from Pinakes; the count, the first few (yours first, marked), each opening the
            // collection. No row while Pinakes doesn't list them, or nothing holds it. Bookmarks stay
            // private: the heart above is only your own, never a count of other people's
            const cl = $('#icollections-slot'); if (cl) {
              const ic = ex.collections;
              if (!ic || !ic.count || !ic.items.length) cl.remove();
              else {
                const nameOf = (c) => c.name || `Collection by ${shortKey(c.author)}`;
                cl.outerHTML = `<section class="section" id="icollections"><div class="section-head"><h3>In collections</h3><span class="muted small">${ic.count} public ${ic.count === 1 ? 'collection' : 'collections'} on Pubky</span></div><div class="list compact">${ic.items.map((c, k) => `<div class="list-row clickable" data-icoll="${k}" tabindex="0" role="link"><div class="avatar" style="background:hsl(${hue(nameOf(c))} 35% 28%)">${icon('playlist')}</div><div class="grow"><div class="name">${esc(nameOf(c))}${c.mine && isOwner() ? ' <span class="pill ok">Yours</span>' : ''}</div><div class="sub">${c.mine && isOwner() ? 'your collection' : 'by ' + esc(shortKey(c.author))}</div></div>${icon('chevron', 'ic right')}</div>`).join('')}</div>${ic.count > ic.items.length ? `<p class="muted small">and ${ic.count - ic.items.length} more</p>` : ''}</section>`;
                $$('[data-icoll]', main).forEach((r) => r.onclick = () => { const c = ic.items[Number(r.dataset.icoll)]; setView('shared', { user: c.author, id: c.id }); });
              }
            }
            const cs = $('#iconcerts-slot'); if (cs) { cs.outerHTML = (pg.concerts && pg.concerts.length ? `<section class="section" id="iconcerts"><div class="section-head"><h3>Upcoming concerts</h3></div><div class="list">${pg.concerts.map((c, k) => `<div class="list-row clickable" data-iconcert="${k}"><div class="grow"><div class="name">${esc(itemName(c))}</div><div class="sub">${esc([c.description, c.site].filter(Boolean).join(' · '))}</div></div>${icon('external')}</div>`).join('')}</div></section>` : ''); }
            $$('[data-iconcert]', main).forEach((r) => r.onclick = () => { const c = pg.concerts[Number(r.dataset.iconcert)]; openWeb(c.page || c.uri); });
          }).catch(() => { /* extras are a bonus */ });
          if (offers('tags')) drawHeroTags(ht);
          for (const g of tagGroups) {
            if (g.pub.state !== 'loading' && !g.pub.kept) continue;
            const was = g.pub; const at = heroPub.by; const ask = ++heroPubAsk;
            api('GET', `/discover/uri?${refQs(g.ref)}`).then((r) => ({ state: 'ok', tags: r.tags || [], faces: r.faces || {} }))
              // No contact for Pubky (412) or no identity (422): nothing to show, not a failure
              .catch((e) => ({ state: [404, 412, 422].includes(e.status) ? 'none' : 'error', tags: [], faces: {}, unknown: e.status === 422 }))
              .then((p) => {
                // A failed ask (network, 5xx) keeps what was kept and shows it: the tags drawn from it stay as they
                // are (the area doesn't collapse to an error under a thumb, and the next redraw stands it in), or
                // a newer one kept meanwhile (another redraw's ask) is drawn instead; a newer "no identity" kept
                // meanwhile (p null) draws as that answer, nothing, not the older tags shown
                if (p.state === 'error') { const k = at.get(pubKey(g)); if (k && k.p) p = { ...k.p, kept: true }; else if (k) p = { state: 'none', tags: [], faces: {}, unknown: true }; else if (was.kept) return; } else {
                  // Kept for this page's next redraw: an answer ("no identity" drops it), when newer than the one kept
                  const had = at.get(pubKey(g));
                  if (!had || had.ask < ask) at.set(pubKey(g), { ask, p: p.state === 'ok' || (p.state === 'none' && !p.unknown) ? p : null });
                }
                g.pub = p;
                // The same as the kept one already drawn: nothing to draw again
                if (was.kept && JSON.stringify([was.state, was.tags, was.faces, !!was.unknown]) === JSON.stringify([p.state, p.tags, p.faces, !!p.unknown])) return;
                // A library item with no identity, seen without own tags (a listener, or GET /tags failed)
                if (p.unknown && g.scope === 'song' && localRef && !g.own) { ht.groups = ht.groups.filter((x) => x !== g); unmatched(); }
                drawHeroTags(ht);
              });
          }
          if (myReview !== undefined) bindReviewRow(target, myReview);
          $$('[data-view="sources"]', main).forEach((a) => a.onclick = inAppClick(() => openConnect(a)));
          const ia = $('#iartist'); if (ia) ia.onclick = inAppClick(() => setView('item', { kind: 'artist', id: pg.artist.id }));
          const iq = $('#iartistq'); if (iq) iq.onclick = inAppClick(() => { search.value = pg.artist.name; setView('search', { q: pg.artist.name, in: 'everywhere' }); });
          const ial = $('#ialbum'); if (ial) ial.onclick = inAppClick(() => setView('item', { kind: 'album', id: pg.album.id }));
          const playAll = (shuffle, how = 'replace', anchorEl) => { if (pg.unavailable) return toast(pg.unavailable, 'error'); if (!songs.length) return discoverPlay({ uri: pg.uri || '', page: pg.open_on[0] ? pg.open_on[0].url : pg.uri, title: pg.title, artist: fallback.artist, image: pg.image }, null, anchorEl, how === 'replace' ? { shuffle, how } : { how }); return playSongs(songs, 0, how, pf, anchorEl, fallback, how === 'replace' ? { shuffle } : {}); };
          $('#iplay').onclick = (e) => playAll(false, 'replace', e.currentTarget);
          stickyHead(state.itemTitle, (el) => playAll(false, 'replace', el));
          const sh = $('#ishuffle'); if (sh) sh.onclick = (e) => playAll(true, 'replace', e.currentTarget);
          // Nothing here plays (the archive restricts the item): Play and Shuffle say why, off
          if (pg.unavailable) for (const b of [$('#iplay'), sh]) if (b) { b.disabled = true; b.title = pg.unavailable; }
          $('#izone').onclick = (e) => showMenu(e.currentTarget, [{ title: 'Play on' }, ...zoneList().map((z) => ({ label: z.name, icon: 'zone', run: () => { state.zone = z.id; try { localStorage.setItem('pm.zone', z.id); } catch { /* */ } renderZoneName(); renderBar(); playAll(false); } }))]);
          $('#ifav').onclick = async () => {
            // A file not matched yet: "Which song is this?" first, then the heart
            // As it is now (the page stays on screen while it is drawn again): a second press undoes the first
            try {
              if (!(await setFavourite(target, !isFav()))) return;
              // (saved for this item; another page opened meanwhile keeps its own heart)
              if (state.view !== 'item' || state.viewArg.kind !== kind || state.viewArg.id !== id) return;
              const b = $('#ifav', main); const on = isFav();
              if (b) { b.classList.toggle('on', on); b.setAttribute('aria-pressed', String(on)); b.title = on ? 'Remove from Bookmarks' : 'Add to Bookmarks'; b.innerHTML = icon(on ? 'heart-fill' : 'heart'); }
              render();
            } catch (er) { toast(er.message, 'error'); }
          };
          const extra = [...(commentUri ? [{ label: 'Post about this…', icon: 'notes', run: () => postAbout(commentUri, pg.title) }] : [])];
          $('#imore').onclick = (e) => itemMenu(e.currentTarget, { title: pg.title, songs, plays_from: pf, dl: kind === 'album' && /^lib-\d+$/.test(String(id)) ? { kind: 'album', id: String(id).slice(4), sub: pg.artist && pg.artist.name, songs: (songs || []).length } : null, fallback, artistId: pg.artist && pg.artist.id, artistName: pg.artist && pg.artist.name, albumId: kind === 'album' ? null : pg.album && pg.album.id, openOn: pg.open_on, alsoHere: iconAlso ? alsoHere.map(({ v, k }) => ({ label: instName(v), svc: alsoSvc(v), k })) : [], switchTo: (k) => setView('item', { kind, id: pg.also_on[k].page }), path: `/${kind}/${id}`, uri: pg.uri, extra });
          const playVersion = async (v, b) => { if (v.source === 'library') { if (v.uri && v.uri.startsWith('track:')) { const t = await api('GET', `/library/tracks/${v.uri.slice(6)}`); return playTracks([t], true); } return setView('item', { kind: 'album', id: 'lib-' + v.id }); } if (v.uri) return playSourceItems([qOf({ uri: v.uri, title: pg.title }, fallback)], true); if (ytOf(v.url)) return playSourceItems([qOf({ uri: `youtube:video:${ytOf(v.url)}`, title: pg.title }, fallback)], true); return discoverPlay({ uri: v.url, page: v.url, title: pg.title, artist: fallback.artist }, null, b); };
          $$('[data-vplay]', main).forEach((b) => b.onclick = () => playVersion(pg.versions[Number(b.dataset.vplay)], b));
          $$('[data-aplay]', main).forEach((b) => b.onclick = () => playVersion(pg.also_on[Number(b.dataset.aplay)], b));
          $$('[data-also]', main).forEach((a) => a.onclick = () => setView('item', { kind, id: pg.also_on[Number(a.dataset.also)].page }));
          const smore = $('[data-srcmore]', main); if (smore) smore.onclick = () => { const rest = smore.nextElementSibling; rest.hidden = false; smore.remove(); const first = $('a[href], button', rest); if (first) first.focus(); };
          $$('[data-vpage]', main).forEach((a) => a.onclick = inAppClick(() => setView('item', { kind, id: pg.versions[Number(a.dataset.vpage)].page })));
          const csec = $('#icomments'); if (csec) mountComments(csec);
          state.drawnPage = pageKey; if (place) place();
          // Not while someone types a comment: the render would take the box away under them
          if (pg.reading || pg.pending) { const tries = (state.viewArg.tries || 0) + 1; const again = () => { if (!live()) return; if (commentTyping()) { setTimeout(again, 3000); return; } state.viewArg.tries = tries; render(); }; if (tries <= 12) setTimeout(again, pg.reading ? 4000 : 3000); }
          // About from open sources comes in the background: ask for the page again and redraw only
          // About and the portrait in place (the page stays put: scroll, focus, a half-typed tag).
          // When it brings other sources ("Also on"), the page is drawn once more at the end.
          else if (pg.about_pending) {
            // The newest render only: Retry and the pending redraws draw this page again meanwhile
            const here = () => live();
            const poll = (tries) => setTimeout(async () => {
              if (!here()) return;
              let np; try { np = await api('GET', `/item/${kind}/${encodeURIComponent(id)}`); } catch { return; }
              if (!here()) return;
              if (np.about_pending) { if (tries < 12) poll(tries + 1); return; }
              const sources = (p) => JSON.stringify([(p.also_on || []).map((v) => v.url), (p.open_on || []).map((v) => v.url)]);
              if (sources(np) !== sources(pg)) { const redraw = () => { if (!here()) return; if (commentTyping()) { setTimeout(redraw, 3000); return; } render(); }; redraw(); return; }
              if (isListener()) np.about = (np.about || []).filter((f) => f.label !== 'File');
              // With an article now: the top part in place and Details at the foot; else About whole, as drawn
              const sec = $('#iabout', main); const got = { ...np, about: np.about || [] };
              if (sec) {
                if (got.about_text && got.about_text.text) {
                  // The music first: the top part goes under the hero (About was at the foot while pending)
                  sec.remove(); const old = $('#idetails', main); if (old) old.remove();
                  const hero = $('.hero', main); if (hero) hero.insertAdjacentHTML('afterend', aboutTopHtml(got)); else main.insertAdjacentHTML('afterbegin', aboutTopHtml(got));
                  main.insertAdjacentHTML('beforeend', aboutDetailsHtml(got));
                } else sec.outerHTML = aboutHtml(got);
                bindBlurb();
              }
              const ph = np.image && !pg.image && $('.hero .art.placeholder', main);
              // The picture goes over the monogram (and under the source badge), so one that fails to load leaves the monogram
              if (ph) { ph.classList.remove('placeholder'); if (kind === 'artist') ph.classList.add('round'); ph.firstElementChild.insertAdjacentHTML('afterend', heroImg(np.image, kind === 'artist')); }
              Object.assign(pg, { about: np.about, blurb: np.blurb, about_text: np.about_text, image_credit: np.image_credit, image: np.image || pg.image, about_pending: false });
            }, 3000);
            poll(1);
          }
          break;
        }

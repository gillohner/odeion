/* Odeion web UI. Talks to /v1 (bearer token), listens on /v1/events,
   and plays either through a node zone or through this browser's <audio>. */
(() => {
  'use strict';
  const $ = (s, root = document) => root.querySelector(s);
  const $$ = (s, root = document) => Array.from(root.querySelectorAll(s));
  const BROWSER = 'browser';
  // Where the page's own player is, in words. The apps (window.pmDesktop, set by pm-desktop before
  // this script) never say "browser": on Android the in-app player is "This phone", on a desktop app
  // "This computer"; the plain web UI keeps "This browser". Every string that names it goes through
  // these (TASK-android-no-browser-wording).
  const appPlatform = () => (window.pmDesktop ? (window.pmDesktop.platform === 'android' ? 'android' : 'desktop') : null);
  const hereName = () => ({ android: 'This phone', desktop: 'This computer' }[appPlatform()] || 'This browser');
  // (An Android System WebView too old for this page is told so by index.html's own small script,
  // which also runs where this file cannot be read.)
  // In a sentence: "in this browser", "on this phone", "on this computer"
  const hereIn = () => (appPlatform() ? `on ${hereName().toLowerCase()}` : 'in this browser');
  // "… only in This browser" / "… only on This phone"
  const hereOnly = () => `${appPlatform() ? 'on' : 'in'} ${hereName()}`;
  // What plays the sound: a browser decodes the file itself; the app decodes it and the system mixes
  // it (Android to its own rate: a USB DAC shows that rate, not the file's), no bit-perfect claim
  const playedBy = () => ({ android: 'Played by Android', desktop: 'Played by the app' }[appPlatform()] || 'Played by this browser');
  const decodedBy = () => (appPlatform() ? 'Decoded by the app' : 'Decoded by this browser');
  // index.html's own two sentences about the player (the YouTube panel's head, the token hint)
  const wordHere = () => { const y = $('#ytpanel .embedhead .muted'); if (y) y.textContent = `YouTube’s own player · its ads and terms apply · plays only ${hereOnly().replace('This', 'this')}`; const t = $('#tokhint'); if (t) t.innerHTML = `It’s in the <code>api-token</code> file in the node’s data folder, and stays ${hereIn()}.`; };
  const decodeNote = () => ({ android: 'The app decodes the file and Android mixes it to the system rate; the node makes no signal-path claim.', desktop: 'The app decodes the file and the system mixes it; the node makes no signal-path claim.' }[appPlatform()] || 'This browser decodes the file itself; the node makes no signal-path claim.');

  const state = {
    token: localStorage.getItem('pm.token') || '',
    zone: localStorage.getItem('pm.zone') || '',
    zones: {},
    lyrics: new Map(),
    formats: null,
    focus: '',
    sleepAt: {},
    browserSleepTimer: null,
    pubkyUser: undefined,
    view: 'home',
    viewArg: null,
    albums: null,
    artists: null,
    playlists: [],
    queue: [],
    info: null,
    ws: null,
    favUris: new Set(),
    trackCache: new Map(),
    browserQueue: (() => { try { const q = JSON.parse(localStorage.getItem('pm.browserQueue') || '[]'); return Array.isArray(q) ? q.filter((it) => it && it.uri) : []; } catch { return []; } })(),
    browserIndex: -1,
    history: [],
  };

  // ---- helpers -------------------------------------------------------------
  const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  // A link from data (a homepage, a station's site, a pasted link) only when it is http or https: esc keeps
  // it inside the attribute, but a javascript: one would still run (security review 2026-10-27, #7).
  const webUrl = (u) => { if (!u) return ''; try { const x = new URL(String(u), location.href); return x.protocol === 'http:' || x.protocol === 'https:' ? x.href : ''; } catch { return ''; } };
  // Pubky Ring's deep links (pubkyauth:, pubkyring:) besides web ones
  const ringUrl = (u) => (/^pubky(auth|ring):\/\//i.test(String(u || '')) ? String(u) : webUrl(u));
  const openWeb = (u) => { const x = webUrl(u); if (x) window.open(x, '_blank', 'noopener'); };
  // Images that fail to load are removed, or hidden where they hold their place. One listener instead of
  // inline onerror handlers, which the page's Content-Security-Policy does not run.
  document.addEventListener('error', (e) => { const t = e.target; const how = t && t.dataset && t.dataset.err; if (how === 'remove') t.remove(); else if (how === 'hide') t.style.visibility = 'hidden'; }, true);
  // A song row's (or a page hero's) cover that has loaded gets a backing of its own (.loaded), so the initials under it
  // don't show through a cover with transparent parts; while it loads it has none, and the initials show
  document.addEventListener('load', (e) => { const t = e.target; if (t instanceof HTMLImageElement && t.matches('.thumbwrap > img.thumb, .hero .art.monogram > img')) t.classList.add('loaded'); }, true);
  const icon = (name, cls = 'ic') => `<svg class="${cls}" aria-hidden="true"><use href="#i-${name}"/></svg>`;
  const fmt = (ms) => {
    if (ms === Infinity) return 'Live'; // a radio stream has no length
    const s = Math.floor((ms || 0) / 1000);
    const m = Math.floor(s / 60);
    const h = Math.floor(m / 60);
    const pad = (n) => String(n).padStart(2, '0');
    return h ? `${h}:${pad(m % 60)}:${pad(s % 60)}` : `${m}:${pad(s % 60)}`;
  };
  const fmtLong = (ms) => {
    const m = Math.round((ms || 0) / 60000);
    if (m < 1) return `${Math.round((ms || 0) / 1000)} s`;
    if (m < 60) return `${m} min`;
    const h = Math.floor(m / 60);
    return `${h} h ${m % 60} min`;
  };
  const ago = (unix) => {
    const s = Math.max(0, Math.floor(Date.now() / 1000) - unix);
    if (s < 60) return 'just now';
    if (s < 3600) return `${Math.floor(s / 60)} min ago`;
    if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
    return `${Math.floor(s / 86400)} d ago`;
  };
  const fmtFormat = (f) => !f ? '' : f.kind === 'dsd' ? `DSD${Math.round(f.rate_hz / 44100)}` : `${f.bits}/${(f.rate_hz / 1000).toFixed(1).replace('.0', '')}k`;
  // A lossy file shows its codec, not the PCM it decodes to ("MP3", not "24/44.1k")
  const LOSSY = { mp3: 'MP3', aac: 'AAC', vorbis: 'Vorbis', opus: 'Opus' };
  const fileFormat = (file) => (!file ? '' : LOSSY[file.container] || fmtFormat(file.format));
  // A count as people read it: 12,840, not 12840 (the reader's own separator; a string is kept as is)
  const fmtCount = (n) => (typeof n === 'number' ? n.toLocaleString() : n);
  // "1 track", "2 tracks", "12,840 tracks" (n may be a formatted number)
  const countOf = (n, many) => `${fmtCount(n)} ${String(n).replace(/[^0-9]/g, '') === '1' ? many.replace(/s$/, '') : many}`;
  // A playlist's size: "songs" when every item is a song (the node's tracks_only), else "items"
  // (a collection that also holds links, albums, artists or posts). The same words on every list of playlists.
  const plCount = (p) => countOf(p.items || 0, p.tracks_only ? 'songs' : 'items');
  // Which service a page link belongs to ("Open in Audius", not "Open in")
  const SERVICE_HOSTS = { 'audius.co': 'audius', 'archive.org': 'archive', 'deezer.com': 'deezer', 'tidal.com': 'tidal', 'listen.tidal.com': 'tidal', 'open.spotify.com': 'spotify', 'jamendo.com': 'jamendo', 'musicbrainz.org': 'musicbrainz' };
  const serviceOfUrl = (u) => { try { return SERVICE_HOSTS[new URL(u).hostname.replace(/^www\./, '')] || ''; } catch { return ''; } };
  // A Pubky key as people read it: no "pubky" prefix, shortened in the middle; the copy button
  // copies all of it (owner, 2026-10-22)
  const bareKey = (k) => String(k || '').replace(/^pubky:?\/*/i, '').replace(/^pk:/i, '');
  const shortKey = (k) => { const b = bareKey(k); return b.length > 20 ? `${b.slice(0, 6)}…${b.slice(-10)}` : b; };
  const keyHtml = (k) => `<span class="pkey"><code title="${esc(bareKey(k))}">${esc(shortKey(k))}</code><button class="icon-btn sm" data-copykey="${esc(bareKey(k))}" title="Copy the whole key" aria-label="Copy the whole key">${icon('copy')}</button></span>`;
  document.addEventListener('click', (e) => { const b = e.target.closest && e.target.closest('[data-copylink]'); if (!b) return; e.preventDefault(); const v = b.dataset.copylink; (navigator.clipboard ? navigator.clipboard.writeText(v) : Promise.reject()).then(() => toast('Copied')).catch(() => toast(v)); }, true);
  document.addEventListener('click', (e) => { const b = e.target.closest && e.target.closest('[data-addmusic]'); if (!b) return; e.preventDefault(); setView('add-music'); });
  document.addEventListener('click', (e) => { const b = e.target.closest && e.target.closest('[data-copykey]'); if (!b) return; e.preventDefault(); e.stopPropagation(); const k = b.dataset.copykey; (navigator.clipboard ? navigator.clipboard.writeText(k) : Promise.reject()).then(() => toast('Key copied')).catch(() => toast(k)); }, true);
  const artists = (t) => (t.artists || []).map((a) => a.name).join(', ');
  const hue = (s) => { let h = 0; for (const c of String(s || '')) h = (h * 31 + c.charCodeAt(0)) >>> 0; return h % 360; };
  // The credit Wikipedia's text (CC BY-SA) and a Commons picture need: source, licence and a link to each
  const creditLink = (href, text) => (webUrl(href) ? `<a href="${esc(webUrl(href))}" target="_blank" rel="noopener">${esc(text)}</a>` : esc(text));
  const aboutCredit = (pg) => {
    const t = pg.about_text; const c = pg.image_credit; const lines = [];
    if (t && t.source) lines.push(`Text: ${t.url ? creditLink(t.url, t.source === 'Wikipedia' ? 'Read more on Wikipedia' : t.source) : esc(t.source)}${t.licence ? ` · ${creditLink(t.licence_url, t.licence)}` : ''}`);
    if (c) lines.push(`Picture: ${c.author ? `${esc(c.author)} · ` : ''}${c.licence ? `${creditLink(c.licence_url, c.licence)} · ` : ''}${creditLink(c.file_page, c.source || 'Wikimedia Commons')}`);
    return lines.length ? `<p class="muted small credit">${lines.join('<br>')}</p>` : '';
  };
  // Two letters for an artwork placeholder: the first word and the next real word ("I'm in Love…"
  // gives "IL", not "II", which reads as a pause button)
  const initials = (s) => {
    const w = (s || '?').trim().split(/\s+/).filter((x) => /[\p{L}\p{N}]/u.test(x));
    if (!w.length) return '?';
    const second = w.slice(1).find((x) => x.replace(/[^\p{L}\p{N}]/gu, '').length > 2) || w[1];
    const letter = (x) => (x.match(/[\p{L}\p{N}]/u) || [''])[0];
    const out = (letter(w[0]) + (second ? letter(second) : '')).toUpperCase();
    return out === 'II' ? w[0].slice(0, 2).toUpperCase() : out;
  };
  const tokenQ = () => (state.token ? '?token=' + encodeURIComponent(state.token) : '');
  // `svc`: where it plays from, as a badge in the cover's corner (coverBadge); none when null.
  // `quiet`: the badge is decoration (the art sits in a button that names the source itself).
  // `inner`: markup placed over the cover (a tile's Play), inside the cover so it is positioned on it.
  const art = (albumId, title, cls = '', svc = null, quiet = false, inner = '') => {
    const h = hue(title);
    const bg = `background:linear-gradient(135deg, hsl(${h} 38% 30%), hsl(${(h + 40) % 360} 42% 18%))`;
    const img = albumId ? `<img loading="lazy" alt="" src="/v1/library/albums/${albumId}/cover${tokenQ()}" data-err="remove">` : '';
    return `<div class="art ${cls}" style="${bg}"><span>${esc(initials(title))}</span>${img}${coverBadge(svc, quiet)}${inner}</div>`;
  };
  // A playlist's art (UX review #8): four album covers as a 2×2 mosaic, one cover, or its initials
  // on a gradient; never an empty square.
  const mosaic = (covers, name, cls = '') => {
    const u = [...new Set((covers || []).filter(Boolean))].map(imgSrc);
    if (u.length >= 4) return `<div class="art mosaic ${cls}" aria-hidden="true">${u.slice(0, 4).map((x) => `<img loading="lazy" alt="" src="${esc(x)}" data-err="hide">`).join('')}</div>`;
    const h = hue(name);
    return `<div class="art ${cls}" aria-hidden="true" style="background:linear-gradient(135deg, hsl(${h} 38% 30%), hsl(${(h + 40) % 360} 42% 18%))"><span>${esc(initials(name))}</span>${u[0] ? `<img loading="lazy" alt="" src="${esc(u[0])}" data-err="remove">` : ''}</div>`;
  };
  // Smart playlists: the rules people can pick from, in words
  const SMART_FIELDS = {
    genre: { label: 'Genre', ops: [['contains', 'contains'], ['is', 'is']], input: 'text', hint: 'techno' },
    artist: { label: 'Artist', ops: [['contains', 'contains']], input: 'text', hint: 'name' },
    year: { label: 'Year', ops: [['between', 'between']], input: 'range' },
    favourite: { label: 'Bookmark', ops: [['is', 'is']], input: 'yesno' },
    plays: { label: 'Times played', ops: [['at_least', 'at least'], ['at_most', 'at most']], input: 'number' },
    last_played: { label: 'Last played', ops: [['within_days', 'in the last'], ['not_within_days', 'not in the last']], input: 'days' },
    tag: { label: 'Your Pubky tag', ops: [['is', 'is']], input: 'text', hint: 'chill' },
  };
  const SMART_SORTS = [['random', 'Random (new each time)'], ['most_played', 'Most played'], ['least_recent', 'Longest unplayed'], ['newest', 'Newest'], ['title', 'Title']];
  const smartRuleRow = (r, i) => {
    const f = SMART_FIELDS[r.field] || SMART_FIELDS.genre;
    const val = r.value == null ? '' : r.value;
    const input = f.input === 'yesno' ? `<select class="input sm" data-k="value" aria-label="Bookmark"><option value="yes" ${val !== false ? 'selected' : ''}>yes</option><option value="no" ${val === false ? 'selected' : ''}>no</option></select>`
      : f.input === 'range' ? `<input class="input sm" data-k="value" type="number" placeholder="from" value="${esc(val || '')}" aria-label="From year" style="width:90px"> and <input class="input sm" data-k="to" type="number" placeholder="to" value="${esc(r.to || '')}" aria-label="To year" style="width:90px">`
      : f.input === 'number' ? `<input class="input sm" data-k="value" type="number" min="0" value="${esc(val || 0)}" aria-label="Times" style="width:90px"> times`
      : f.input === 'days' ? `<input class="input sm" data-k="value" type="number" min="1" value="${esc(val || 30)}" aria-label="Days" style="width:90px"> days`
      : `<input class="input sm" data-k="value" value="${esc(val)}" placeholder="${esc(f.hint || '')}" aria-label="${esc(f.label)}">`;
    return `<div class="smrule" data-smrule="${i}"><select class="input sm" data-k="field" aria-label="What">${Object.entries(SMART_FIELDS).map(([k, x]) => `<option value="${k}" ${r.field === k ? 'selected' : ''}>${x.label}</option>`).join('')}</select><select class="input sm" data-k="op" aria-label="How">${f.ops.map(([k, l]) => `<option value="${k}" ${r.op === k ? 'selected' : ''}>${l}</option>`).join('')}</select>${input}<button class="icon-btn" data-smrm="${i}" aria-label="Remove this rule" title="Remove this rule">${icon('close')}</button></div>`;
  };
  const smartSummary = (x) => { const seeds = (x.seeds || []).map((sd) => sd.label || sd.kind); const rs = [...(seeds.length ? [`like ${seeds.join(', ')}`] : []), ...(x.rules || []).map((r) => { const f = SMART_FIELDS[r.field]; if (!f) return r.field; const op = (f.ops.find(([k]) => k === r.op) || f.ops[0])[1]; if (r.field === 'favourite') return r.value === false ? 'not bookmarked' : 'bookmarked'; if (r.field === 'year') return `year ${r.value || '…'}–${r.to || '…'}`; return `${f.label.toLowerCase()} ${op} ${r.value}${r.field === 'last_played' ? ' days' : ''}`; })]; return rs.join(x.matches === 'any' ? ' or ' : ' and ') + (x.minutes ? ` · ${x.minutes} min` : ''); };
  // Art for a queue item: a streaming source's image, else the library album cover; a badge for
  // where it plays from (`svc`; null for none), quiet when the art sits in a button.
  const itemArt = (it, cls = '', svc = null, quiet = false) => {
    if (it && it.image) { const h = hue(it.title || ''); return `<div class="art ${cls}" style="background:hsl(${h} 30% 22%)"><img loading="lazy" alt="" src="${esc(imgSrc(it.image))}" data-err="remove">${coverBadge(svc, quiet)}</div>`; }
    return art(albumIdOf(it), (it && (it.title || it.uri)) || '', cls, svc, quiet);
  };

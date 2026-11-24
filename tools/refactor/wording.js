// Odeion's own words: Bookmarks (never Favourites) and Collections (never Playlists).
//
//   node tools/refactor/wording.js --check          fail when the old words are in the page's copy
//   node tools/refactor/wording.js --write          rewrite the copy (run again on a newer main)
//   node tools/refactor/wording.js --list           every place the old words are shown, with its line
//   node tools/refactor/wording.js --tests --write  the fixture tests' expected words, by the same table
//
// Only what a person reads is looked at: text and the title, aria-label, placeholder and alt
// attributes inside the page's strings. Names in code stay as they are (view ids, routes, API paths,
// storage keys, classes, data attributes), and so do other products' own words: a service's
// playlists, an M3U playlist file, Roon's playlists and favourites (KEEP below).
// No dependencies.
const fs = require('node:fs');
const path = require('node:path');

const ROOT = path.join(__dirname, '..', '..');
const STATIC = path.join(ROOT, 'crates', 'pm-web', 'static');
// The page's script is kept as parts (crates/pm-web/ui/app/NN-name.js, joined into the one /app.js):
// each part is read and written as a file of its own. A tree without the parts has static/app.js.
const PARTS = path.join(ROOT, 'crates', 'pm-web', 'ui', 'app');
const scriptFiles = () => (fs.existsSync(PARTS)
  ? fs.readdirSync(PARTS).filter((n) => /^\d\d-[a-z0-9-]+\.js$/.test(n)).sort().map((n) => path.join(PARTS, n))
  : [path.join(STATIC, 'app.js')]);
const PAGE_FILES = [...['index.html', 'party.html'].map((f) => path.join(STATIC, f)), ...scriptFiles(),
  ...['feedback.js', 'usage.js', 'pwa.js'].map((f) => path.join(STATIC, f))];
const OLD = /\b(?:un)?favou?rit(?:es?|ed|ing)\b|\bplaylists?\b|\bheart(?:ed|ing)\b/gi;

// ---- scanning ------------------------------------------------------------------------------
// The parts of a JS source: code, comments, regexps, and the text of strings and template literals.
const scanJs = (src) => {
  const out = []; const n = src.length; let i = 0; let last = 0;
  const push = (kind, a, b) => { if (b > a) out.push({ kind, a, b }); };
  const flush = (to) => { push('code', last, to); last = to; };
  // (what stands before a `/`: an operator or a keyword means a regexp starts, a value means division)
  const regexAllowed = () => {
    let j = i - 1; while (j >= 0 && /\s/.test(src[j])) j--;
    if (j < 0) return true;
    const c = src[j];
    if ('=(:,[!&|?{};>+-*%~^<'.includes(c)) return true;
    if (c === ')' || c === ']' || c === '}') return false;
    const w = /[A-Za-z_$][\w$]*$/.exec(src.slice(Math.max(0, j - 12), j + 1));
    return !!w && ['return', 'typeof', 'case', 'throw', 'else', 'yield', 'void', 'in', 'of', 'do'].includes(w[0]);
  };
  const template = () => {
    // at the opening backtick
    i++; let a = i;
    while (i < n) {
      const c = src[i];
      if (c === '\\') { i += 2; continue; }
      if (c === '`') { push('tpl', a, i); i++; last = i; return; }
      if (c === '$' && src[i + 1] === '{') { push('tpl', a, i); i += 2; last = i - 2; code(true); a = i; last = i; continue; }
      i++;
    }
  };
  const code = (inTemplate) => {
    // (inTemplate: until the `}` that closes this ${…})
    let depth = 0; if (inTemplate) last = i - 2;
    while (i < n) {
      const c = src[i]; const d = src[i + 1];
      if (c === '/' && d === '/') { flush(i); const e = src.indexOf('\n', i); i = e < 0 ? n : e; push('comment', last, i); last = i; continue; }
      if (c === '/' && d === '*') { flush(i); const e = src.indexOf('*/', i + 2); i = e < 0 ? n : e + 2; push('comment', last, i); last = i; continue; }
      if (c === '/' && regexAllowed()) {
        flush(i); i++; let cls = false;
        while (i < n) { const r = src[i]; if (r === '\\') { i += 2; continue; } if (r === '\n') break; if (r === '[') cls = true; else if (r === ']') cls = false; else if (r === '/' && !cls) { i++; break; } i++; }
        while (i < n && /[a-z]/.test(src[i])) i++;
        push('regex', last, i); last = i; continue;
      }
      if (c === "'" || c === '"') {
        flush(i + 1); i++;
        while (i < n && src[i] !== c) { if (src[i] === '\\') i++; if (src[i] === '\n') break; i++; }
        push('str', last, i); last = i; i++; continue;
      }
      if (c === '`') { flush(i + 1); template(); continue; }
      if (c === '{') depth++;
      else if (c === '}') { if (inTemplate && depth === 0) { i++; flush(i); return; } depth--; }
      i++;
    }
    flush(n);
  };
  code(false);
  return out;
};
// An HTML file: its text and attributes are read as one string; its <script> bodies as JS
const scanHtml = (src) => {
  const out = []; const re = /<script\b[^>]*>([\s\S]*?)<\/script>/g; let last = 0; let m;
  while ((m = re.exec(src))) {
    const a = m.index + m[0].indexOf('>') + 1; const b = a + m[1].length;
    if (a > last) out.push({ kind: 'str', a: last, b: a });
    for (const p of scanJs(m[1])) out.push({ kind: p.kind, a: p.a + a, b: p.b + a });
    last = b;
  }
  if (last < src.length) out.push({ kind: 'str', a: last, b: src.length });
  return out;
};
// Inside a string: the stretches a person reads. Markup's tags are skipped but for the attributes
// that are read out or shown (title, aria-label, placeholder, alt).
const READ_ATTRS = /\b(title|aria-label|placeholder|alt|data-tip|label)=("|')/g;
const proseIn = (text) => {
  const out = []; let i = 0; const n = text.length; let a = 0; let inTag = false;
  while (i < n) {
    const c = text[i];
    if (!inTag && c === '<' && /[A-Za-z/!]/.test(text[i + 1] || '')) { if (i > a) out.push([a, i]); inTag = true; i++; continue; }
    if (inTag) {
      if (c === '>') { inTag = false; a = i + 1; i++; continue; }
      READ_ATTRS.lastIndex = i; const m = READ_ATTRS.exec(text);
      const close = text.indexOf('>', i);
      if (m && (close < 0 || m.index < close)) { const s = m.index + m[0].length; let e = text.indexOf(m[2], s); if (e < 0) e = n; out.push([s, e]); i = e + 1; continue; }
      i = close < 0 ? n : close; continue;
    }
    i++;
  }
  if (!inTag && n > a) out.push([a, n]);
  return out;
};

// ---- the words -----------------------------------------------------------------------------
// Other products' own words, left as they are (each a pattern over the sentence the word is in).
const KEEP = [
  /\b(?:TIDAL|Spotify|Qobuz|Deezer|SoundCloud|YouTube|Jellyfin|Plex|Navidrome|Subsonic|Audiobookshelf|ListenBrainz|Last\.fm|Roon|Audius|Jamendo|Bandcamp)(?:’s|'s)?(?: own)? (?:playlists?|favou?rites?)\b/gi,
  /\b(?:M3U8?|PLS|CSV|XSPF)(?: or [A-Z0-9]+)? playlists?(?: files?)?\b/gi,
  /\bplaylist files?\b/gi, /\bexport each playlist\b/gi, /\bplaylist exports?\b/gi,
  // a service's own page (Sources): the lists you keep on that service
  /^(?:Its|Your) playlists$/gi, /\bits playlists\b/g, /\bPlaylists on a service\b/gi, /\bNo (?:music )?playlists(?: of your own)? on (?:the server|[A-Z][A-Za-z.]+)\b/g, /\bNo music playlists\b/g,
  /\bImport this playlist\b/gi, /\bPlaylists from files\b/gi, /^Playlist “$/g,
  /\bservice playlists?\b/gi, /\b(?:TIDAL )?collection and playlists\b/g, /\blikes and playlists\b/gi, /\bopen the playlist on Sources\b/g,
];
// Odeion's own words, most specific first. Case follows the old word's first letter unless the new
// text says otherwise (a section's name is capitalised where it names the section).
const RULES = [
  [/\bNothing hearted yet\b/g, 'No bookmarks yet'],
  [/\bPress the heart on\b/g, 'Press the heart on'],
  [/\bhearted\b/g, 'bookmarked'], [/\bHearted\b/g, 'Bookmarked'],
  [/\bSmart playlists\b/g, 'Smart collections'], [/\bsmart playlists\b/g, 'smart collections'],
  [/\bSmart playlist\b/g, 'Smart collection'], [/\bsmart playlist\b/g, 'smart collection'],
  [/\bnot a favourite\b/g, 'not bookmarked'], [/\ba favourite\b/g, 'bookmarked'], [/\bfavourite (albums?|songs?|artists?)\b/g, 'bookmarked $1'],
  [/^ favourites$/g, ' Bookmarks'], [/\bIn favourites\b/g, 'In Bookmarks'],
  [/\bAdd to favourites\b/g, 'Add to Bookmarks'], [/\bRemove from favourites\b/g, 'Remove from Bookmarks'],
  [/\badded to favourites\b/g, 'added to Bookmarks'], [/\bremoved from favourites\b/g, 'removed from Bookmarks'],
  [/\bAdded to favourites\b/g, 'Added to Bookmarks'], [/\bRemoved from favourites\b/g, 'Removed from Bookmarks'],
  [/\bin favourites\b/g, 'in Bookmarks'], [/\bto favourites\b/g, 'to Bookmarks'], [/\bfrom favourites\b/g, 'from Bookmarks'],
  [/\bfavourited\b/g, 'bookmarked'], [/\bFavourited\b/g, 'Bookmarked'],
  [/\bunfavourite\b/g, 'remove the bookmark from'], [/\bUnfavourite\b/g, 'Remove bookmark'],
  [/\bFavourites\b/g, 'Bookmarks'], [/\bfavourites\b/g, 'bookmarks'],
  [/\bFavourite\b/g, 'Bookmark'], [/\bfavourite\b/g, 'bookmark'],
  [/\bFavorites\b/g, 'Bookmarks'], [/\bfavorites\b/g, 'bookmarks'], [/\bFavorite\b/g, 'Bookmark'], [/\bfavorite\b/g, 'bookmark'],
  [/\bPlaylists\b/g, 'Collections'], [/\bplaylists\b/g, 'collections'],
  [/\bPlaylist\b/g, 'Collection'], [/\bplaylist\b/g, 'collection'],
  // (a test reads a label as the page's CSS shows it, in capitals)
  [/\bPLAYLISTS\b/g, 'COLLECTIONS'], [/\bPLAYLIST\b/g, 'COLLECTION'], [/\bFAVOURITES\b/g, 'BOOKMARKS'], [/\bFAVOURITE\b/g, 'BOOKMARK'],
  // (an and a: "a collection", never "an collection"; "a bookmark")
];
const A_AN = [[/\ban (collection|bookmark)/g, 'a $1'], [/\bAn (collection|bookmark)/g, 'A $1']];

// A lower-case word alone is a name in code ('playlists' the view, '/playlists' the path) unless
// it stands in a sentence: another word beside it, in the same stretch of text.
const NAMEISH = /[-_./=#[\]:@${}\\]/;
const inSentence = (text, a, b) => {
  const before = text[a - 1] || ''; const after = text[b] || '';
  if (before && NAMEISH.test(before) && !(before === '.' && /\s/.test(text[a - 2] || ' '))) return false;
  if (after && NAMEISH.test(after) && !((after === '.' || after === ':') && (!text[b + 1] || /[\s<"'`)…]/.test(text[b + 1])))) return false;
  // (a space at the stretch's own edge: the sentence goes on in the next part of the template)
  const left = /(?:^|[A-Za-zÀ-ÿ0-9’'”"),.;:!?…›])\s+$/.test(text.slice(0, a)) && a > 0; const right = /^(?:’s|'s)?[,.;:!?…)]*\s+(?:$|[A-Za-zÀ-ÿ0-9“"(‘›·])/.test(text.slice(b));
  return left || right;
};
// A lone word that is shown after all: the second argument of countOf(n, 'playlists') and the like
const SHOWN_ALONE = [/countOf\([^()]*(?:\([^()]*\)[^()]*)*,\s*['"`]$/, /\bplural\([^()]*,\s*['"`]$/, /\bnoun\s*=\s*[^;]*['"`]$/,
  // (nOf(n, 'playlist', 'playlists'): both words; `const to = name ? … : 'playlist'`: the word a toast ends on)
  /\bnOf\([^()]*,\s*['"`]$/, /\bconst to = [^;]*:\s*['"`]$/];

// What to change in one stretch of prose: [[from, to, replacement], …]
const edits = (text, lead) => {
  const kept = []; for (const k of KEEP) { k.lastIndex = 0; let m; while ((m = k.exec(text))) kept.push([m.index, m.index + m[0].length]); }
  const isKept = (a, b) => kept.some(([x, y]) => a >= x && b <= y);
  const out = []; OLD.lastIndex = 0; let m;
  while ((m = OLD.exec(text))) {
    const a = m.index; const b = a + m[0].length;
    if (isKept(a, b)) continue;
    const capital = /^[A-Z]/.test(m[0]);
    const alone = text.trim() === m[0];
    if (!capital && !inSentence(text, a, b) && !(alone && SHOWN_ALONE.some((r) => r.test(lead)))) continue;
    out.push([a, b]);
  }
  return out;
};
const reword = (text, lead) => {
  const hits = edits(text, lead); if (!hits.length) return text;
  // (the rules see the whole stretch, so the phrases match; only stretches with a hit are touched,
  // and what KEEP names is put back)
  const marks = []; let t = text;
  for (const k of KEEP) t = t.replace(k, (s) => { marks.push(s); return `\u0001${marks.length - 1}\u0002`; });
  // (names in code inside the stretch, e.g. a path beside a sentence, are held back too)
  OLD.lastIndex = 0;
  t = t.replace(OLD, (s, off, whole) => {
    const capital = /^[A-Z]/.test(s);
    if (capital || inSentence(whole, off, off + s.length) || (whole.trim() === s && SHOWN_ALONE.some((r) => r.test(lead)))) return s;
    marks.push(s); return `\u0001${marks.length - 1}\u0002`;
  });
  for (const [from, to] of RULES) t = t.replace(from, to);
  for (const [from, to] of A_AN) t = t.replace(from, to);
  return t.replace(/\u0001(\d+)\u0002/g, (_, k) => marks[Number(k)]);
};

// ---- files ---------------------------------------------------------------------------------
const lineOf = (src, at) => src.slice(0, at).split('\n').length;
// Every stretch of prose in a file, as [start, end, lead] (lead: the code just before its string)
const stretches = (file, src, tests) => {
  const parts = file.endsWith('.html') ? scanHtml(src) : scanJs(src); const out = [];
  for (const p of parts) {
    // (a test also names the page's words in the patterns it looks for)
    // (the pattern's own text, without its slashes and flags: a word at its end is still in a sentence)
    if (tests && p.kind === 'regex') { const body = src.slice(p.a, p.b); const e = body.lastIndexOf('/'); if (e > 1) out.push([p.a + 1, p.a + e, '']); continue; }
    if (p.kind !== 'str' && p.kind !== 'tpl') continue;
    const text = src.slice(p.a, p.b); const lead = src.slice(Math.max(0, p.a - 200), p.a);
    for (const [a, b] of proseIn(text)) out.push([p.a + a, p.a + b, lead]);
  }
  return out;
};
const run = (file, mode, tests) => {
  const src = fs.readFileSync(file, 'utf8'); let out = ''; let last = 0; const found = [];
  for (const [a, b, lead] of stretches(file, src, tests)) {
    const text = src.slice(a, b); const hits = edits(text, lead);
    if (!hits.length) continue;
    const next = reword(text, lead);
    for (const [x, y] of hits) found.push({ line: lineOf(src, a + x), word: text.slice(x, y), around: text.slice(Math.max(0, x - 40), Math.min(text.length, y + 40)).replace(/\s+/g, ' ') });
    out += src.slice(last, a) + next; last = b;
  }
  out += src.slice(last);
  if (mode === 'write' && out !== src) fs.writeFileSync(file, out);
  return { found, changed: out !== src };
};

if (require.main === module) {
  const args = process.argv.slice(2); const mode = args.includes('--write') ? 'write' : args.includes('--list') ? 'list' : 'check';
  const named = args.filter((a) => !a.startsWith('--')); const tests = args.includes('--tests');
  const TOOLS = path.join(ROOT, 'tools');
  const files = named.length ? named.map((f) => path.resolve(f)) : tests ? fs.readdirSync(TOOLS).filter((f) => /^ui-.*\.js$/.test(f) && f !== 'ui-wording.js' /* (it names the old words on purpose) */).map((f) => path.join(TOOLS, f)) : PAGE_FILES;
  let total = 0;
  for (const f of files) {
    if (!fs.existsSync(f)) continue;
    const r = run(f, mode, tests); total += r.found.length;
    const rel = path.relative(ROOT, f);
    if (mode !== 'write') for (const h of r.found) console.log(`${rel}:${h.line}  ${h.word}  …${h.around}…`);
    else if (r.changed) console.log(`${rel}: ${r.found.length} reworded`);
  }
  if (mode === 'check') { if (total) { console.log(`\n${total} place${total === 1 ? '' : 's'} still say Favourites or Playlists (Odeion says Bookmarks and Collections; run with --write, or add another product's own word to KEEP)`); process.exit(1); } console.log('wording: ok (Bookmarks and Collections)'); }
  else if (mode === 'list') console.log(`\n${total} place${total === 1 ? '' : 's'}`);
}
module.exports = { scanJs, scanHtml, proseIn, reword, edits, run, PAGE_FILES, OLD };

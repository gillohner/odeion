// Tag colours are pubky.app's (TASK-tags-in-hero-pubky-design): app.js's tagColour is a port of
// generateRandomColor in pubky/pubky-app src/libs/utils/utils.ts (@7cb809a), so a label has the same
// colour in both apps. The expected colours below came from running that original with node; given
// a pubky-app checkout, the original runs here too and every label is compared with it.
//   node tools/tag-colour-check.js [path to pubky-app's src/libs/utils/utils.ts]
const fs = require('fs');
const path = require('path');
const src = fs.readFileSync(path.join(__dirname, '..', 'crates', 'pm-api', 'static', 'app.js'), 'utf8');
const block = /\n  const TAG_COLOURS = [\s\S]*?\n  const tagRgba = [^\n]*\n/.exec(src);
if (!block) { console.error('tagColour and tagRgba not found in app.js'); process.exit(1); }
const { tagColour, tagRgba } = new Function(`${block[0]}\nreturn { tagColour, tagRgba };`)();
const EXPECTED = {
  // pubky.app's own named labels, whatever the case
  bitcoin: '#FF9900', Bitcoin: '#FF9900', synonym: '#FF6600', bitkit: '#FF4400', pubky: '#C8FF00', blocktank: '#FFAE00', tether: '#26A17B', ai: '#00C8FF', AI: '#00C8FF',
  // hashed: the case as typed counts; a hash past 32 bits (Frenchcore); each of the six patterns;
  // the blue ones moved to teal (patterns 3 and 4 with a low byte); accents, CJK and an emoji
  frenchcore: '#FFc700', Frenchcore: '#00FFa7', hardcore: '#FF000a', techno: '#00FF29', jazz: '#b700FF', ambient: '#98FF00', funk: '#00ccFF', grime: '#00f6FF',
  'lo-fi': '#00c5FF', metal: '#00faFF', trance: '#00d9FF', kpop: '#00FFfa', rave: '#00FFe8', shoegaze: '#00FFf3',
  'électro': '#FF0090', '日本': '#e700FF', '🔥': '#FF3d00', a: '#FF6100', dub: '#f100FF', ska: '#FF0900',
};
const problems = [];
for (const [label, want] of Object.entries(EXPECTED)) {
  const got = tagColour(label);
  if (got !== want) problems.push(`${label}: ${got}, pubky.app has ${want}`);
}
// Names of Object.prototype are labels like any other
for (const label of ['constructor', 'toString', '__proto__', 'hasOwnProperty']) {
  if (!/^#[0-9a-fA-F]{6}$/.test(tagColour(label))) problems.push(`${label}: ${tagColour(label)} is no colour`);
}
if (tagRgba('#FF9900', 0.3) !== 'rgba(255, 153, 0, 0.3)') problems.push(`tagRgba('#FF9900', 0.3) is ${tagRgba('#FF9900', 0.3)}`);
if (tagRgba('#00faFF', 0.5) !== 'rgba(0, 250, 255, 0.5)') problems.push(`tagRgba('#00faFF', 0.5) is ${tagRgba('#00faFF', 0.5)}`);
let compared = 0;
if (process.argv[2]) {
  const ts = fs.readFileSync(process.argv[2], 'utf8');
  const cases = /const customCases = \[[\s\S]*?\];/.exec(ts)[0];
  const fn = /export function generateRandomColor\(str: string\): string \{[\s\S]*?\n\}/.exec(ts)[0].replace('export function generateRandomColor(str: string): string', 'function generateRandomColor(str)');
  const original = new Function(`${cases}\n${fn}\nreturn generateRandomColor;`)();
  const more = Array.from({ length: 2000 }, (_, i) => `tag${i}`).concat(['Drum & Bass', 'ÆØÅ', 'музыка', '🎵🎶', 'x'.repeat(20)]);
  for (const label of [...Object.keys(EXPECTED), ...more]) {
    compared += 1;
    if (tagColour(label) !== original(label)) problems.push(`${label}: ${tagColour(label)}, the original says ${original(label)}`);
  }
}
if (problems.length) { console.error(problems.join('\n')); process.exit(1); }
console.log(`tag colours ok: ${Object.keys(EXPECTED).length} pubky.app colours${compared ? `, ${compared} labels against the original` : ''}`);

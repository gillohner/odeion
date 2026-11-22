// The web UI's files as the node serves them, for the tools.
//
// app.js is kept as ordered parts, crates/pm-web/ui/app/NN-name.js. The node's build
// (crates/pm-web/build.rs) joins them end to end, in name order, into the one /app.js it serves;
// this does the same for the tools, so a fixture page runs the script the node would send. Every
// other file is read from crates/pm-web/static as it is.
//
//   const ui = require('./ui-src');
//   ui.read(ui.STATIC, 'app.js')        the script, joined (a Buffer)
//   ui.read(staticDir, 'index.html')    any other file of that static folder
//   ui.has(staticDir, name)             whether the node serves a file of that name
//   node tools/ui-src.js [name] [static dir]   print a file (app.js by default)
const fs = require('fs');
const path = require('path');

const STATIC = path.join(__dirname, '..', 'crates', 'pm-web', 'static');
// A part's name: two digits (its place), a dash, words. Anything else in the folder is a mistake
// the build refuses too (a part left out would be a script cut short).
const PART = /^\d\d-[a-z0-9-]+\.js$/;
const partsDir = (staticDir) => path.join(staticDir, '..', 'ui', 'app');

// The parts of app.js in order: [{ name, text }]
function parts(staticDir = STATIC) {
  const dir = partsDir(staticDir);
  // (an editor's own files beside a part it has open are not parts and not mistakes)
  const names = fs.readdirSync(dir).filter((n) => !/^[.#]|~$/.test(n)).sort();
  for (const n of names) if (!PART.test(n)) throw new Error(`${path.join(dir, n)}: not a part of app.js (parts are NN-name.js)`);
  return names.map((name) => ({ name, text: fs.readFileSync(path.join(dir, name)) }));
}

// A static folder with no parts beside it (a tree from before the split, a copy of the files)
// has the script as one file. Both at once is a mistake: the build would use the parts and the
// single file's changes would be lost without a word (run tools/refactor/web-ui-split.js on it).
const single = (staticDir) => path.join(staticDir, 'app.js');
const hasParts = (staticDir = STATIC) => fs.existsSync(partsDir(staticDir));
function appJs(staticDir = STATIC) {
  const whole = fs.existsSync(single(staticDir));
  if (!fs.existsSync(partsDir(staticDir))) return fs.readFileSync(single(staticDir));
  if (whole) throw new Error(`${single(staticDir)} exists beside the parts in ${partsDir(staticDir)}: the node serves the parts; cut the file into them and remove it (crates/pm-web/ui/README.md has the steps)`);
  return Buffer.concat(parts(staticDir).map((p) => p.text));
}
const has = (staticDir, name) => (name === 'app.js' ? fs.existsSync(partsDir(staticDir)) || fs.existsSync(single(staticDir)) : fs.existsSync(path.join(staticDir, name)));
const read = (staticDir, name) => (name === 'app.js' ? appJs(staticDir) : fs.readFileSync(path.join(staticDir, name)));

// Where a place in the joined script is: "17-menus.js:42" for a character offset of app.js
function where(offset, staticDir = STATIC) {
  let at = 0;
  for (const p of parts(staticDir)) {
    const text = p.text.toString('utf8');
    if (offset < at + text.length) return `${p.name}:${text.slice(0, offset - at).split('\n').length}`;
    at += text.length;
  }
  return 'app.js:end';
}

module.exports = { STATIC, parts, appJs, has, hasParts, read, where };

if (require.main === module) process.stdout.write(read(process.argv[3] || STATIC, process.argv[2] || 'app.js'));

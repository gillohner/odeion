// tools/ui-reconnect.js checks its arguments before it starts anything. It needs no browser, node or network:
//   node tools/ui-reconnect-args.js [path to ui-reconnect.js]
// The script under test only ever runs in a child process with a stub (--require) that records each attempt to
// start a process and then exits that child at once with 97, so a bad build can't start anything, let alone
// itself again. Each child also has a 15 s limit. Bad forms must exit 2 with a usage line, start nothing and
// print none of the given arguments; a good form must hand the command after `--` to spawn unchanged.
// The cleanup cases at the end run the script for real, with children that end by themselves within 30 s: a
// command that can't start or exits at once fails the run, and a node that never answers is stopped with
// everything it started.
const { spawnSync } = require('child_process');
const fs = require('fs');
const os = require('os');
const path = require('path');

const SCRIPT = path.resolve(process.argv[2] || path.join(__dirname, 'ui-reconnect.js'));
const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ui-reconnect-args-'));
const STUB = path.join(dir, 'spawn-stub.js');
const LOG = path.join(dir, 'spawns.log');
process.on('exit', () => fs.rmSync(dir, { recursive: true, force: true }));
fs.writeFileSync(STUB, `
const cp = require('child_process');
const fs = require('fs');
const record = (kind) => function (cmd, args, opts) {
  fs.appendFileSync(process.env.STUB_LOG, JSON.stringify({ kind, cmd, args, mark: (opts && opts.env || {}).PM_UI_RECONNECT_CHILD || null }) + '\\n');
  process.exit(97);
};
for (const k of ['spawn', 'fork', 'exec', 'execFile', 'spawnSync', 'execSync', 'execFileSync']) cp[k] = record(k);
`);

let failed = 0;
const check = (ok, msg) => { console.log((ok ? 'PASS ' : 'FAIL ') + msg); if (!ok) failed++; };
const spawns = () => (fs.existsSync(LOG) ? fs.readFileSync(LOG, 'utf8').trim().split('\n').filter(Boolean).map((l) => JSON.parse(l)) : []);
const run = (nodeArgs, env = {}) => {
  fs.rmSync(LOG, { force: true });
  const r = spawnSync(process.execPath, ['--require', STUB, ...nodeArgs], {
    encoding: 'utf8', timeout: 15000, killSignal: 'SIGKILL',
    env: { ...process.env, PM_UI_RECONNECT_CHILD: '', STUB_LOG: LOG, ...env },
  });
  return { status: r.status, signal: r.signal, out: (r.stdout || '') + (r.stderr || ''), spawns: spawns() };
};

const OUT = path.join(dir, 's-reconnect');
const TOK = 'tok-SENTINEL-4711';
const BAD = [
  // The form that started itself over and over: a worktree and a shots dir, no token, url or `--`.
  ['a worktree and a shots dir, nothing else', [path.dirname(__dirname), OUT]],
  ['three arguments and no `--`', [TOK, 'http://127.0.0.1:7799', OUT]],
  ['`--` with no node command after it', [TOK, 'http://127.0.0.1:7799', OUT, '--']],
  ['`--` followed by an empty command', [TOK, 'http://127.0.0.1:7799', OUT, '--', '']],
  ['`--` after one argument', [TOK, '--', '/bin/true']],
  ['four arguments before `--`', [TOK, 'http://127.0.0.1:7799', OUT, 'extra-SENTINEL', '--', '/bin/true']],
  ['a url that is not a url', [TOK, 'not-a-url-SENTINEL', OUT, '--', '/bin/true']],
  ['a url that is not http', [TOK, 'ftp://127.0.0.1:7799', OUT, '--', '/bin/true']],
  ['an empty token', ['', 'http://127.0.0.1:7799', OUT, '--', '/bin/true']],
];
for (const [name, args] of BAD) {
  const r = run([SCRIPT, ...args]);
  check(r.status === 2 && r.signal === null, `${name}: exits 2 (got ${r.status}${r.signal ? ' ' + r.signal : ''})`);
  check(r.spawns.length === 0, `${name}: starts nothing (${r.spawns.length} attempts)`);
  check(/^usage: /m.test(r.out) && r.out.trim().split('\n').length <= 2, `${name}: one reason and one usage line`);
  const leaked = args.filter((a) => a.length > 4 && r.out.includes(a));
  check(leaked.length === 0, `${name}: prints none of the given arguments${leaked.length ? ' (printed ' + leaked.length + ')' : ''}`);
}

// Started by itself (the mark it sets for its child): refuses even a good form.
const GOOD = [TOK, 'http://127.0.0.1:9', OUT, '--', '/bin/echo', 'serve', '--config', 'x.toml', '--', 'y'];
let r = run([SCRIPT, ...GOOD], { PM_UI_RECONNECT_CHILD: '1' });
check(r.status === 2 && r.spawns.length === 0, `started by itself: exits 2 and starts nothing (got ${r.status}, ${r.spawns.length} attempts)`);

// A good form goes on to start the node command, and nothing else: the stub records it and stops the run.
r = run([SCRIPT, ...GOOD]);
const s = r.spawns[0] || {};
check(r.status === 97 && r.spawns.length === 1, `a good form starts exactly one process (got ${r.status}, ${r.spawns.length} attempts)`);
check(s.kind === 'spawn' && s.cmd === '/bin/echo' && JSON.stringify(s.args) === JSON.stringify(['serve', '--config', 'x.toml', '--', 'y']),
  `that process is the command after the first \`--\`, with its own arguments unchanged (${JSON.stringify([s.cmd, s.args])})`);
check(s.mark === '1', 'the node is started with the mark that stops the script running under itself');

// parseArgs on its own, loaded as a module (still under the stub, so loading it can start nothing).
const probe = `
const { parseArgs } = require(${JSON.stringify(SCRIPT)});
const a = parseArgs(['node', 's', 'T', 'https://h:1', 'O', '--', 'n', '--', 'm']);
const b = parseArgs(['node', 's', 'T', 'https://h:1', 'O']);
console.log(JSON.stringify({ a, b }));`;
r = run(['-e', probe]);
let parsed = null;
try { parsed = JSON.parse(r.out.trim().split('\n').pop()); } catch { /* reported below */ }
check(r.status === 0 && r.spawns.length === 0 && parsed !== null, `loading the script as a module starts nothing (got ${r.status}, ${r.spawns.length} attempts)`);
check(parsed && JSON.stringify(parsed.a) === JSON.stringify({ token: 'T', base: 'https://h:1', out: 'O', node: ['n', '--', 'm'] }),
  'parseArgs splits a good form at the first `--`');
check(parsed && typeof (parsed.b || {}).error === 'string', 'parseArgs gives a reason for a form with no `--`');

// Cleanup, for real (no stub). Port 9 has no node, so the script waits its 10 s for one and gives up.
const real = (child, ms) => {
  const t = Date.now();
  const r = spawnSync(process.execPath, [SCRIPT, TOK, 'http://127.0.0.1:9', OUT, '--', ...child], {
    encoding: 'utf8', timeout: ms, killSignal: 'SIGKILL', env: { ...process.env, PM_UI_RECONNECT_CHILD: '' },
  });
  return { status: r.status, signal: r.signal, out: (r.stdout || '') + (r.stderr || ''), ms: Date.now() - t };
};
const gone = (pid) => {
  try { return /^\S+ \(.*\) Z/.test(fs.readFileSync(`/proc/${pid}/stat`, 'utf8')); } catch { return true; }
};
r = real([path.join(dir, 'no-such-dir', 'pm-node')], 15000);
check(r.status === 1 && /did not start: ENOENT/.test(r.out) && r.ms < 5000, `a node command that can't start fails the run at once (${r.status}, ${r.ms} ms)`);
r = real(['/bin/true'], 15000);
check(r.status === 1 && /exited at start/.test(r.out) && r.ms < 5000, `a node command that exits at once fails the run at once (${r.status}, ${r.ms} ms)`);
const PIDFILE = path.join(dir, 'sleep.pid');
r = real(['/bin/sh', '-c', `sleep 30 & echo $! > ${PIDFILE}; wait`], 30000);
check(r.status === 1 && r.signal === null && /node did not start/.test(r.out), `a node that never answers fails the run (${r.status}${r.signal ? ' ' + r.signal : ''}, ${r.ms} ms)`);
const pid = fs.existsSync(PIDFILE) ? Number(fs.readFileSync(PIDFILE, 'utf8')) : 0;
const t0 = Date.now();
while (pid && !gone(pid) && Date.now() - t0 < 3000) spawnSync('sleep', ['0.1']);
check(pid > 0 && gone(pid), `and what the node command started is stopped with it (pid ${pid ? 'recorded' : 'missing'})`);
if (pid && !gone(pid)) try { process.kill(pid, 'SIGKILL'); } catch { /* gone */ }

console.log(failed ? `${failed} FAILED` : 'all passed');
process.exitCode = failed ? 1 : 0;

#!/usr/bin/env python3
"""The bundle's scripts read settings as compose and the node do (docs/branding-compatibility.md),
checked with a stand-in `docker` in a throwaway folder: nothing real is started, backed up or read.

- deploy/bundle/settings.sh, used by backup.sh and restore.sh:
  - `setting TOKEN` / `setting BACKUP_PASSPHRASE`: ODEION_ before PM_, the shell's environment
    before .env (as compose reads them); empty counts as unset (never valid for these);
  - `node_port <container>`: the port from the container's ODEION_LISTEN, else PM_LISTEN (an empty
    ODEION_LISTEN wins and means the default), else 7788;
  - `NODE_BACKUP_SH`, run with `docker exec` (which sees the container's own environment, not what
    the entrypoint changed in the node's): the config ODEION_CONFIG, else PM_CONFIG, else
    <data dir>/node.toml, the data dir ODEION_DATA_DIR, else PM_DATA_DIR, else /data; ODEION_ wins
    even when empty, as in the entrypoint.
- setup.sh does not add PM_TOKEN when .env has ODEION_TOKEN.
- backup.sh and restore.sh use these, not their own PM_-only reading.

    python3 -I tools/bundle-settings-check.py [scratch dir]
"""

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
# The compose command: COMPOSE="/path/to/docker-compose" checks another version (CI has v2)
COMPOSE = os.environ.get("COMPOSE", "docker compose").split()
BUNDLE = REPO / "deploy" / "bundle"
problems = []


def check(ok, what):
    print(f"{'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        problems.append(what)


base = Path(tempfile.mkdtemp(prefix="odeion-bundle-", dir=sys.argv[1] if len(sys.argv) > 1 else None))
try:
    stubs = base / "bin"
    stubs.mkdir()
    # docker: `inspect -f … <ctr>` prints the container's environment; `exec <ctr> sh -c <code>`
    # runs the code in that environment only, with a stand-in pm-node/odeiond that records its -c
    (stubs / "docker").write_text(r'''#!/bin/bash
case "$1" in
  inspect) cat "$FIXTURE_ENV" ;;
  exec) shift 2; [ "$1" = sh ] && [ "$2" = -c ] || exit 9
    args=(); while IFS= read -r l; do [ -n "$l" ] && args+=("$l"); done < "$FIXTURE_ENV"
    exec env -i PATH="$NODE_BIN:/usr/bin:/bin" RECORD="$RECORD" "${args[@]}" sh -c "$3" ;;
  *) echo "unexpected docker $*" >&2; exit 9 ;;
esac
''')
    node_bin = base / "nodebin"
    node_bin.mkdir()
    for n in ("pm-node", "odeiond"):
        (node_bin / n).write_text(f'#!/bin/sh\necho "{n} $*" >> "$RECORD"\n')
    for p in list(stubs.iterdir()) + list(node_bin.iterdir()):
        p.chmod(0o755)

    def bash(code, env_file="", proc_env=None, container_env=""):
        d = Path(tempfile.mkdtemp(dir=base))
        (d / ".env").write_text(env_file)
        (d / "cenv").write_text(container_env)
        e = {"PATH": f"{stubs}:/usr/bin:/bin", "FIXTURE_ENV": str(d / "cenv"), "NODE_BIN": str(node_bin),
             "RECORD": str(d / "record")}
        e.update(proc_env or {})
        r = subprocess.run(["bash", "-euo", "pipefail", "-c", f'. "{BUNDLE}/settings.sh"\n{code}'], cwd=d, env=e,
                           capture_output=True, text=True)
        rec = (d / "record").read_text() if (d / "record").exists() else ""
        return r, rec

    def setting(name, env_file, proc_env=None):
        r, _ = bash(f"setting {name}", env_file, proc_env)
        return r.stdout if r.returncode == 0 else f"<error {r.stderr.strip()[:120]}>"

    check(setting("TOKEN", "PM_TOKEN=fixture-old-tok\n") == "fixture-old-tok", "token: the old name alone")
    check(setting("TOKEN", "ODEION_TOKEN=fixture-new-tok\n") == "fixture-new-tok", "token: the new name alone")
    check(setting("TOKEN", "PM_TOKEN=fixture-old-tok\nODEION_TOKEN=fixture-new-tok\n") == "fixture-new-tok", "token: the new name wins")
    check(setting("TOKEN", "PM_TOKEN=fixture-old-tok\nODEION_TOKEN=\n") == "fixture-old-tok", "token: an empty new name counts as unset")
    check(setting("TOKEN", "PM_TOKEN=fixture-old-tok\n", {"ODEION_TOKEN": "fixture-shell-tok"}) == "fixture-shell-tok",
          "token: the shell's environment before .env, as compose")
    check(setting("BACKUP_PASSPHRASE", "PM_BACKUP_PASSPHRASE=fixture old pass\n", {"ODEION_BACKUP_PASSPHRASE": "fixture new pass ♫"})
          == "fixture new pass ♫", "passphrase: the new name wins, value unchanged")
    check(setting("BACKUP_PASSPHRASE", "", {"PM_BACKUP_PASSPHRASE": "fixture old pass"}) == "fixture old pass", "passphrase: the old name alone")
    check(setting("BACKUP_PASSPHRASE", "") == "", "passphrase: none")

    def port(cenv):
        r, _ = bash('node_port fixture-container', container_env=cenv)
        return r.stdout
    check(port("PM_LISTEN=0.0.0.0:7801\n") == "7801", "port: the old name alone")
    check(port("ODEION_LISTEN=0.0.0.0:7802\n") == "7802", "port: the new name alone")
    check(port("PM_LISTEN=0.0.0.0:7801\nODEION_LISTEN=[::]:7802\n") == "7802", "port: the new name wins")
    check(port("PM_LISTEN=0.0.0.0:7801\nODEION_LISTEN=\n") == "7788", "port: an empty new name wins (the default)")
    check(port("HOME=/data\n") == "7788", "port: neither: the default")

    def conf(cenv):
        r, rec = bash('docker exec fixture-container sh -c "$NODE_BACKUP_SH"', container_env=cenv)
        return rec.strip() if r.returncode == 0 else f"<error {r.stderr.strip()[:160]}>"
    snap = "backup --force /tmp/pm-backup-library.db"
    check(conf("PM_DATA_DIR=/data\n") == f"pm-node -c /data/node.toml {snap}", "config: the image default")
    check(conf("PM_CONFIG=/data/old.toml\n") == f"pm-node -c /data/old.toml {snap}", "config: the old name alone")
    check(conf("ODEION_CONFIG=/data/new.toml\n") == f"pm-node -c /data/new.toml {snap}", "config: the new name alone")
    check(conf("PM_CONFIG=/data/old.toml\nODEION_CONFIG=/data/new.toml\n") == f"pm-node -c /data/new.toml {snap}",
          "config: the new name wins")
    check(conf("PM_CONFIG=/data/old.toml\nODEION_CONFIG=\nPM_DATA_DIR=/data\n") == f"pm-node -c /data/node.toml {snap}",
          "config: an empty new name wins (the data dir's node.toml)")
    check(conf("PM_DATA_DIR=/data\nODEION_DATA_DIR=/srv/odeion data\n") == f"pm-node -c /srv/odeion data/node.toml {snap}",
          "config: the new data dir wins")
    check(conf("PM_DATA_DIR=/old\nODEION_DATA_DIR=\n") == f"pm-node -c /data/node.toml {snap}",
          "config: an empty new data dir wins (/data)")

    # setup.sh keeps a token set under the new name
    d = base / "setup"
    shutil.copytree(BUNDLE, d, ignore=shutil.ignore_patterns(".env", "music", "hsdata", "ci"))
    (d / ".env").write_text("ODEION_TOKEN=fixture-fixture-new-token\nODEION_TAG=fixture\n")
    r = subprocess.run(["sh", str(d / "setup.sh")], cwd=d, env={"PATH": "/usr/bin:/bin", "HOME": str(base)},
                       capture_output=True, text=True)
    envf = (d / ".env").read_text()
    check(r.returncode == 0 and "PM_TOKEN=" not in envf and "PM_TAG=" not in envf and "ADMIN_PASSWORD=" in envf,
          "setup.sh adds no PM_TOKEN or PM_TAG next to the ODEION_ ones")
    check("fixture-fixture-new-token" not in r.stdout + r.stderr, "setup.sh prints no token")

    # An empty ODEION_TOKEN or ODEION_TAG is no token or tag (compose counts it as unset), so setup.sh
    # still writes the PM_ one; an empty ODEION_PUBLIC_URL wins in compose, so none is added for it
    d = base / "setup-empty"
    shutil.copytree(BUNDLE, d, ignore=shutil.ignore_patterns(".env", "music", "hsdata", "ci"))
    (d / ".env").write_text("ODEION_TOKEN=\nODEION_TAG=\nODEION_PUBLIC_URL=\n")
    r = subprocess.run(["sh", str(d / "setup.sh")], cwd=d, env={"PATH": "/usr/bin:/bin", "HOME": str(base)},
                       capture_output=True, text=True)
    envf = (d / ".env").read_text()
    tok = [l.split("=", 1)[1] for l in envf.splitlines() if l.startswith("PM_TOKEN=")]
    check(r.returncode == 0 and len(tok) == 1 and len(tok[0]) >= 32 and "PM_TAG=edge" in envf,
          "setup.sh writes a token and tag when the ODEION_ ones are empty")
    check("PM_PUBLIC_URL=" not in envf, "setup.sh adds no PM_PUBLIC_URL next to an empty ODEION_PUBLIC_URL (it wins)")

    # Values compose reads as empty (quoted empty, spaces, a comment) count as no token or tag: after
    # setup.sh, compose gives the node a token and the image a tag; a value compose reads keeps setup quiet
    import json as _json
    for i, envtext in enumerate(['ODEION_TOKEN=""\nODEION_TAG=""\n', "ODEION_TOKEN=''\nODEION_TAG=''\n",
                                 "ODEION_TOKEN=   \nODEION_TAG=   \n", "ODEION_TOKEN= # a note\nODEION_TAG= # a note\n",
                                 'PM_TOKEN=""\nPM_TAG=""\n', 'ODEION_TOKEN="fixture-kept"\nODEION_TAG="fixture-tag"\n']):
        d = base / f"setup-dotenv{i}"
        shutil.copytree(BUNDLE, d, ignore=shutil.ignore_patterns(".env", "music", "hsdata", "ci"))
        (d / ".env").write_text(envtext)
        r = subprocess.run(["sh", str(d / "setup.sh")], cwd=d, env={"PATH": "/usr/bin:/bin", "HOME": str(base)},
                           capture_output=True, text=True)
        (d / "home").mkdir()
        c = subprocess.run(COMPOSE + ["-f", str(d / "docker-compose.yml"), "--project-directory", str(d),
                            "config", "--format", "json"], cwd=d, env={"PATH": "/usr/bin:/bin", "HOME": str(d / "home")},
                           capture_output=True, text=True)
        music = _json.loads(c.stdout)["services"]["music"] if c.returncode == 0 else {}
        tok = music.get("environment", {}).get("PM_TOKEN", "")
        img = music.get("image", "")
        # compose reads "= # a note" as the value "# a note" (a comment needs a value before it): a token
        kept = "fixture-kept" in envtext or "# a note" in envtext
        if "# a note" in envtext:
            check(r.returncode == 0 and tok == "# a note" and "PM_TOKEN=" not in (d / ".env").read_text(),
                  "setup.sh then compose: '= # a note' is a value to compose, so setup adds no token (as compose reads it)")
            continue
        want_tok = (tok == "fixture-kept") if kept else len(tok) >= 32
        want_img = img.endswith(":fixture-tag") if kept else img.endswith(":edge")
        check(r.returncode == 0 and c.returncode == 0 and want_tok and want_img,
              f"setup.sh then compose: {envtext.strip()!r} gives a token and tag ({c.stderr.strip()[-100:]!r}, tag {img.rsplit(':', 1)[-1]!r})")

    # setting TOKEN picks what compose gives the node, for the shell and .env together
    import json
    compose_file = BUNDLE / "docker-compose.yml"
    cases = [
        ({"ODEION_TOKEN": ""}, "ODEION_TOKEN=fixture-env-new\nPM_TOKEN=fixture-env-old\n"),
        ({}, "ODEION_TOKEN=fixture-env-new\nPM_TOKEN=fixture-env-old\n"),
        ({"ODEION_TOKEN": "fixture-shell-new"}, "ODEION_TOKEN=fixture-env-new\nPM_TOKEN=fixture-env-old\n"),
        ({"PM_TOKEN": "fixture-shell-old"}, "ODEION_TOKEN=\nPM_TOKEN=fixture-env-old\n"),
        ({"PM_TOKEN": ""}, "PM_TOKEN=fixture-env-old\nODEION_TOKEN=\n"),
        ({"ODEION_TOKEN": "", "PM_TOKEN": "fixture-shell-old"}, "ODEION_TOKEN=fixture-env-new\n"),
        # compose's dotenv parser: quotes, spaces, inline comments, export, repeated keys
        ({}, 'ODEION_TOKEN=""\nPM_TOKEN=fixture-env-old\n'),
        ({}, "ODEION_TOKEN=''\nPM_TOKEN=fixture-env-old\n"),
        ({}, "ODEION_TOKEN=   \nPM_TOKEN=fixture-env-old\n"),
        ({}, "ODEION_TOKEN= # a note\nPM_TOKEN=fixture-env-old\n"),
        ({}, 'ODEION_TOKEN="fixture-abc"\nPM_TOKEN=fixture-env-old\n'),
        ({}, "ODEION_TOKEN='fixture-abc def'\nPM_TOKEN=fixture-env-old\n"),
        ({}, "ODEION_TOKEN=fixture-abc # a note\nPM_TOKEN=fixture-env-old\n"),
        ({}, "ODEION_TOKEN=fixture-abc#no-space\nPM_TOKEN=fixture-env-old\n"),
        ({}, "ODEION_TOKEN=  fixture-padded  \nPM_TOKEN=fixture-env-old\n"),
        ({}, 'ODEION_TOKEN="fixture-abc" # a note\nPM_TOKEN=fixture-env-old\n'),
        ({}, "export ODEION_TOKEN=fixture-exported\nPM_TOKEN=fixture-env-old\n"),
        ({}, 'PM_TOKEN=""\n'),
        ({}, "PM_TOKEN=fixture-first\nPM_TOKEN=fixture-last\n"),
        # the new name alone (Compose v2 once evaluated an inner :? first and refused this)
        ({}, "ODEION_TOKEN=fixture-env-new\n"),
        ({}, "ODEION_TOKEN=\nPM_TOKEN=\n"),
        # a comment starts only at space + #, not tab + #; `KEY: value` is a line too
        ({}, "ODEION_TOKEN=fixture-abc\t# a note\nPM_TOKEN=fixture-env-old\n"),
        ({}, "ODEION_TOKEN: fixture-colon-value\nPM_TOKEN=fixture-env-old\n"),
        ({}, "ODEION_TOKEN=fixture-first\nODEION_TOKEN:\nPM_TOKEN=fixture-env-old\n"),
    ]
    for i, (shell, envtext) in enumerate(cases):
        cd = base / f"cmp{i}"
        cd.mkdir()
        shutil.copy(compose_file, cd / "docker-compose.yml")
        (cd / ".env").write_text("ADMIN_PASSWORD=fixture-admin\nDB_PASSWORD=fixture-db\n" + envtext)
        (cd / "home").mkdir()
        e = {"PATH": "/usr/bin:/bin", "HOME": str(cd / "home"), **shell}
        r = subprocess.run(COMPOSE + ["-f", str(cd / "docker-compose.yml"), "--project-directory", str(cd),
                            "config", "--format", "json"], cwd=cd, env=e, capture_output=True, text=True)
        want = json.loads(r.stdout)["services"]["music"]["environment"].get("PM_TOKEN") if r.returncode == 0 else ("" if "missing a value" in r.stderr else f"<compose refuses: {r.stderr.strip()[-120:]}>")
        rr = subprocess.run(["bash", "-euo", "pipefail", "-c", f'. "{BUNDLE}/settings.sh"\nsetting TOKEN'], cwd=cd,
                            env={"PATH": "/usr/bin:/bin", **shell}, capture_output=True, text=True)
        got = rr.stdout if rr.returncode == 0 else f"<error {rr.stderr.strip()[:80]}>"
        check(got == want, f"setting TOKEN matches compose: shell {shell}, .env {envtext.strip()!r} (compose {want!r}, setting {got!r})")

    # ---- the API token and the passphrase for backup.sh and restore.sh ----------------------------
    # The node's real token is its config's [api] token, else the one it made (/data/api-token): an
    # existing node.toml is never rewritten, so a token setup.sh adds later is not the node's.
    check("run ./setup.sh" not in (BUNDLE / "backup.sh").read_text() + (BUNDLE / "restore.sh").read_text(),
          "no stop message recommends ./setup.sh for a node that has started (its node.toml is kept)")

    def node_token(files, cenv=""):
        d = Path(tempfile.mkdtemp(dir=base))
        data = d / "data"
        data.mkdir()
        for name, body in files.items():
            (data / name).write_text(body)
        envs = [f"PM_DATA_DIR={data}"] + [l for l in cenv.splitlines() if l]
        r = subprocess.run(["env", "-i", "PATH=/usr/bin:/bin", *envs, "sh", "-c",
                            f'. "{BUNDLE}/settings.sh"; sh -c "$NODE_TOKEN_SH"'], capture_output=True, text=True)
        node_token.rc = r.returncode
        return r.stdout
    check(node_token({"node.toml": 'data_dir = "/data"\n\n[api]\nlisten = "0.0.0.0:7788"\ntoken = "fixture-cfg-token"\n'}) == "fixture-cfg-token",
          "the node's token: [api] token from its config")
    check(node_token({"node.toml": '[api]\ntoken = ""\n', "api-token": "fixture-own-token\n"}) == "fixture-own-token",
          "the node's token: the one it made when its config has none")
    check(node_token({"node.toml": '[tidal]\ntoken = "fixture-not-this-one"\n[api]\nlisten = "x"\n', "api-token": "fixture-own\n"}) == "fixture-own",
          "the node's token: only [api]'s token line counts")
    check(node_token({"node.toml": '[api]\ntoken = "Change-Me"\n', "api-token": "fixture-own\n"}) == "fixture-own",
          "the node's token: a placeholder in the config counts as none (as api_token.rs)")
    check(node_token({"node.toml": '[api]\ntoken = "  fixture-spaced  "\n'}) == "fixture-spaced", "the node's token: trimmed (as api_token.rs)")
    check(node_token({}) == "", "the node's token: none")
    # The [api] token line as TOML writes it: '…' literal, "…" with \" and \\, a comment after it, a
    # comment after [api]; anything this does not read (multi-line, other escapes, a bare value) is a
    # stop (status 2), never a quiet fall-back to another token
    for label, toml, want in (
        ("single-quoted", "[api]\ntoken = 'fixture-single'\n", "fixture-single"),
        ("a comment after it", '[api]\ntoken = "fixture-commented"   # set by hand\n', "fixture-commented"),
        ("escaped quote and backslash", '[api]\ntoken = "fixture-a\\"b\\\\c"\n', 'fixture-a"b\\c'),
        ("a comment after [api]", '[api]  # the API\nlisten = "x"\ntoken = "fixture-after-header"\n', "fixture-after-header"),
        ("single-quoted with a comment", "[api]\ntoken = 'fixture-sq' # note\n", "fixture-sq"),
    ):
        got = node_token({"node.toml": toml, "api-token": "fixture-not-this\n"})
        check(got == want and node_token.rc == 0, f"the node's token, {label}: {got!r}")
    for label, toml in (("multi-line", '[api]\ntoken = """fixture-multi"""\n'),
                        ("a \\u escape", '[api]\ntoken = "fixture-\\u0041"\n'),
                        ("a bare value", "[api]\ntoken = fixture-bare\n"),
                        ("text after the string", '[api]\ntoken = "fixture-x" y\n')):
        got = node_token({"node.toml": toml, "api-token": "fixture-not-this\n"})
        check(node_token.rc == 2 and got == "", f"the node's token, {label}: not read (status 2), no fall-back ({got!r})")

    # backup.sh: the running node's own token when .env has none; a clear stop when there is none at all
    def backup_until_inspect(envtext, files):
        d = base / f"bk-{len(list(base.glob('bk-*')))}"
        shutil.copytree(BUNDLE, d, ignore=shutil.ignore_patterns(".env", "music", "hsdata", "ci"))
        (d / ".env").write_text("ADMIN_PASSWORD=fixture-admin\n" + envtext)
        data = d / "nodedata"
        data.mkdir()
        for name, body in files.items():
            (data / name).write_text(body)
        stub = d / "stub"
        stub.mkdir()
        (stub / "docker").write_text(f'''#!/bin/bash
echo "docker $*" >> "{d}/calls"
case "$1" in
  compose) [ "$2" = ps ] && echo "ctr-${{@: -1}}" ;;
  exec) shift; ctr=$1; shift; [ "$1" = sh ] && [ "$2" = -c ] && exec env -i PATH=/usr/bin:/bin PM_DATA_DIR="{data}" sh -c "$3" ;;
  inspect) exit 1 ;;
esac
''')
        (stub / "docker").chmod(0o755)
        r = subprocess.run(["bash", str(d / "backup.sh"), str(d / "out")], cwd=d, capture_output=True, text=True,
                           env={"PATH": f"{stub}:/usr/bin:/bin", "HOME": str(base)})
        calls = (d / "calls").read_text() if (d / "calls").exists() else ""
        return r, calls
    for label, envtext, files in (("the node's own token", "", {"api-token": "fixture-own-token\n"}),
                                  ("the node's config token", "PM_TOKEN=\n", {"node.toml": '[api]\ntoken = "fixture-cfg-token"\n'}),
                                  ("the .env token when the node keeps none", "ODEION_TOKEN=fixture-env-token\n", {})):
        r, calls = backup_until_inspect(envtext, files)
        check("docker inspect" in calls and "no API token" not in r.stderr and "fixture-" not in r.stdout + r.stderr + calls,
              f"backup.sh goes on with {label}, never showing it")
    r, calls = backup_until_inspect("ODEION_TOKEN=fixture-env-token\n", {"node.toml": '[api]\ntoken = """fixture-multi"""\n'})
    check(r.returncode != 0 and "cannot read" in r.stderr and "ODEION_TOKEN=" in r.stderr and "docker inspect" not in calls,
          f"backup.sh stops when the node's token line is more than it reads, not using .env's ({r.stderr.strip()[-140:]!r})")
    r, calls = backup_until_inspect("", {})
    check(r.returncode != 0 and "no API token" in r.stderr and "ODEION_TOKEN" in r.stderr and "docker inspect" not in calls,
          f"backup.sh without any token stops before it starts, saying where the token is ({r.stderr.strip()[-160:]!r})")

    # restore.sh, the whole script, against a stand-in docker that answers its steps and the node's API
    # (with skip_secrets: no sign-ins restored), a fresh folder and made-up backups
    import hashlib

    def make_backup(envtext, data_files, passphrase_file=None):
        b = Path(tempfile.mkdtemp(dir=base, prefix="backup-"))
        for sub in ("config", "node", "homeserver"):
            (b / sub).mkdir()
        (b / "config" / ".env").write_text("ADMIN_PASSWORD=fixture-admin\nDB_PASSWORD=fixture-db\n" + envtext)
        (b / "config" / "recreate-volumes.sh").write_text("")
        (b / "config" / "music-mounts.tsv").write_text("")
        for f in ("node/library.pmbackup", "node/library.db", "homeserver/database.dump"):
            (b / f).write_text("fixture")
        dd = b / "nodedata"
        dd.mkdir()
        for name, body in data_files.items():
            (dd / name).write_text(body)
        subprocess.run(["tar", "czf", str(b / "node" / "data.tar.gz"), "-C", str(dd), "."], check=True)
        shutil.rmtree(dd)
        subprocess.run(["tar", "czf", str(b / "homeserver" / "data.tar.gz"), "-C", str(b / "config"), "."], check=True)
        if passphrase_file is not None:
            (b / "node" / "passphrase").write_text(passphrase_file + "\n")
        subprocess.run("find . -type f ! -name SHA256SUMS -printf '%P\\n' | sort | xargs sha256sum > SHA256SUMS",
                       shell=True, cwd=b, check=True)
        return b

    # A stand-in docker that behaves like the bundle: it keeps the restored volumes in a folder, and its
    # node uses the token the real one would (node_token_from on its restored data; when that has none,
    # the entrypoint writes node.toml from compose's token, and the node makes one at start), answering
    # 401 to any other. Its restore honours skip_secrets (no sign-ins then).
    RESTORE_STUB = r'''#!/bin/bash
echo "docker $*" >> "$CALLS"
. "$SETTINGS"
NODEV="$STATE/vol-ctr-music"
node_tok() {
  if [ -n "${FORCE_NODE_TOKEN:-}" ]; then printf '%s' "$FORCE_NODE_TOKEN"; return; fi
  node_token_from "$NODEV/node.toml" "$NODEV" 2>/dev/null || true
}
case "$1" in
  compose)
    case "$2" in
      config) echo "name: pubky-music" ;;
      ps) [ "${4:-}" ] && echo "ctr-$4" ;;
      up)
        if [ "${3:-}" = -d ]; then
          mkdir -p "$NODEV"
          [ -f "$NODEV/node.toml" ] || printf '[api]\ntoken = "%s"\n' "$(setting TOKEN .env)" > "$NODEV/node.toml"
          [ -n "$(node_tok)" ] || printf 'fixture-made-at-start\n' > "$NODEV/api-token"
        fi ;;
    esac ;;
  volume) : ;;
  inspect) case "$*" in *Config.Env*) echo "PM_LISTEN=0.0.0.0:7788" ;; *) echo "vol-${@: -1}" ;; esac ;;
  logs) echo "init process complete" ;;
  exec)
    shift
    if [ "$1" = -i ]; then cat > /dev/null; exit 0; fi
    ctr=$1; shift
    if [ "$ctr" = ctr-music ] && [ "$1" = sh ] && [ "$2" = -c ]; then
      [ -z "${EXEC_FAILS:-}" ] || exit 1
      if [ -n "${FORCE_NODE_TOKEN:-}" ]; then printf '%s' "$FORCE_NODE_TOKEN"; exit 0; fi
      exec env -i PATH=/usr/bin:/bin PM_DATA_DIR="$NODEV" sh -c "$3"
    fi ;;
  run)
    if [[ " $* " == *" --network "* ]]; then
      read -r T; read -r P
      url=${@: -1}
      echo "api $url" >> "$CALLS.api"
      if [ "$T" != "$(node_tok)" ]; then
        echo '{"error":"unauthorized"}'; echo "curl: (22) The requested URL returned error: 401" >&2; exit 22
      fi
      case $url in
        */v1/info) echo '{"scanning":false}' ;;
        */v1/restore*)
          echo "restore $url" >> "$CALLS.restores"
          if [[ $url == *skip_secrets=true* ]]; then echo '{"restored":true,"signins_restored":0}'
          elif [ "$WANT_PASS" = NOT-A-BACKUP ]; then echo 'This is not an Odeion backup (the library in it does not open)'; exit 22
          elif [ "$P" = "$WANT_PASS" ]; then echo '{"restored":true,"signins_restored":3}'
          else echo 'That passphrase does not open this backup'; exit 22; fi ;;
      esac
    else
      vol=; prev=
      for a in "$@"; do [ "$prev" = -v ] && vol=${a%%:*}; prev=$a; done
      case " $* " in
        *" tar xzf - -C /v "*) mkdir -p "$STATE/$vol"; tar xzf - -C "$STATE/$vol" ;;
        *" -i "*) cat > /dev/null ;;
      esac
    fi ;;
esac
exit 0
'''

    def restore(b, want_pass, shell_env=None, folder=None, stub_env=None):
        d = folder or Path(tempfile.mkdtemp(dir=base, prefix="restore-"))
        if folder is None:
            shutil.copytree(BUNDLE, d, dirs_exist_ok=True, ignore=shutil.ignore_patterns(".env", "music", "hsdata", "ci"))
        stub = d / "stub"
        stub.mkdir(exist_ok=True)
        (stub / "docker").write_text(RESTORE_STUB)
        (stub / "sleep").write_text("#!/bin/sh\n:\n")
        for f in ("docker", "sleep"):
            (stub / f).chmod(0o755)
        calls = d / "calls"
        state = d / "state"
        for f in (calls, Path(str(calls) + ".restores"), Path(str(calls) + ".api")):
            if f.exists():
                f.unlink()
        if state.exists():
            shutil.rmtree(state)
        state.mkdir()
        env = {"PATH": f"{stub}:/usr/bin:/bin", "HOME": str(base), "CALLS": str(calls), "STATE": str(state),
               "SETTINGS": str(BUNDLE / "settings.sh"), "WANT_PASS": want_pass, **(shell_env or {}), **(stub_env or {})}
        r = subprocess.run(["bash", str(d / "restore.sh"), str(b)], cwd=d, capture_output=True, text=True, env=env)
        restores = Path(str(calls) + ".restores")
        api = Path(str(calls) + ".api")
        restore.api_calls = api.read_text().count("api ") if api.exists() else 0
        return d, r, (calls.read_text() if calls.exists() else ""), (restores.read_text().count("restore ") if restores.exists() else 0)

    def snapshot(d):
        return sorted((str(p.relative_to(d)), hashlib.sha256(p.read_bytes()).hexdigest())
                      for p in d.rglob("*") if p.is_file() and "stub" not in p.parts and p.name not in ("calls",))

    no_secret = lambda r: "fixture-" not in (r.stdout + r.stderr).replace("fixture-backup", "")
    # The backup's own token comes first: a newer token passed in the shell is not the restored node's
    b = make_backup("", {"node.toml": '[api]\ntoken = "fixture-backup-cfg"\n'})
    d, r, calls, n = restore(b, "", {"ODEION_TOKEN": "fixture-newer-shell"})
    check(r.returncode == 0 and '"restored":true' in r.stdout and restore.api_calls <= 4 and no_secret(r),
          f"restore.sh uses the backup's config token over a newer one in the shell ({(r.stdout + r.stderr).strip()[-140:]!r})")
    b = make_backup("PM_TOKEN=fixture-stale-env\n", {"node.toml": '[api]\ntoken = ""\n', "api-token": "fixture-backup-own\n"})
    d, r, calls, n = restore(b, "")
    check(r.returncode == 0 and '"restored":true' in r.stdout, "restore.sh uses the token the backup's node made, not a stale .env one")
    # No node.toml in the backup: the entrypoint writes it from compose's token, as the shell or .env give it
    b = make_backup("PM_TOKEN=fixture-env-token\n", {})
    d, r, calls, n = restore(b, "")
    check(r.returncode == 0 and '"restored":true' in r.stdout, "restore.sh with no node.toml in the backup: compose's token from .env")
    d, r, calls, n = restore(b, "", {"ODEION_TOKEN": "fixture-fixture-shell-token"})
    check(r.returncode == 0 and '"restored":true' in r.stdout, "restore.sh with no node.toml in the backup: compose's token from the shell")
    # No token anywhere: the restored node makes one at start; restore.sh reads it there (never shown)
    b = make_backup("", {"node.toml": '[api]\ntoken = ""\n'})
    d, r, calls, n = restore(b, "")
    check(r.returncode == 0 and '"restored":true' in r.stdout and no_secret(r),
          f"no token anywhere: the token the restored node made is read in its container ({(r.stdout + r.stderr).strip()[-140:]!r})")
    # A refused token (401): read the node's own once, then stop at once, saying how to find it
    b = make_backup("", {"node.toml": '[api]\ntoken = "fixture-backup-cfg"\n'})
    d, r, calls, n = restore(b, "", stub_env={"FORCE_NODE_TOKEN": "fixture-node-really-uses"})
    check(r.returncode == 0 and '"restored":true' in r.stdout, "a 401: the node's own token read in its container, then restored")
    d, r, calls, n = restore(b, "", stub_env={"FORCE_NODE_TOKEN": "fixture-node-really-uses", "EXEC_FAILS": "1"})
    check(r.returncode != 0 and "401" in r.stderr and "/data/api-token" in r.stderr and restore.api_calls <= 3,
          f"a 401 it cannot get past: a stop at once (not an hour's wait), saying where the token is ({restore.api_calls} calls; {r.stderr.strip()[-160:]!r})")
    # A token line this cannot read: a stop before anything changes, unless the shell gives the token
    b = make_backup("PM_TOKEN=fixture-env-token\n", {"node.toml": '[api]\ntoken = """fixture-multi"""\n'})
    d = Path(tempfile.mkdtemp(dir=base, prefix="restore-"))
    shutil.copytree(BUNDLE, d, dirs_exist_ok=True, ignore=shutil.ignore_patterns(".env", "music", "hsdata", "ci"))
    before = snapshot(d)
    d, r, calls, n = restore(b, "", folder=d, stub_env={"FORCE_NODE_TOKEN": "fixture-multi"})
    check(r.returncode != 0 and "cannot read" in r.stderr and "ODEION_TOKEN=" in r.stderr and not calls
          and snapshot(d) == before and not (d / ".env").exists(),
          f"a token line it cannot read: a stop before anything changes ({r.stderr.strip()[-150:]!r})")
    d, r, calls, n = restore(b, "", {"ODEION_TOKEN": "fixture-multi"}, folder=d, stub_env={"FORCE_NODE_TOKEN": "fixture-multi"})
    check(r.returncode == 0 and '"restored":true' in r.stdout, "then a rerun with ODEION_TOKEN=… in the shell restores")
    check("mktemp" not in (BUNDLE / "restore.sh").read_text(), "restore.sh reads the backup's token without writing it to disk")

    # Passphrases, in order: as compose reads .env, as v0.2.x read it (raw), the backup's node/passphrase
    tok = {"node.toml": '[api]\ntoken = "fixture-t"\n'}
    for label, envtext, pfile, locked in (
        ("quoted empty (v0.2.x locked under \"\")", 'PM_BACKUP_PASSPHRASE=""\n', None, '""'),
        ("single-quoted empty", "PM_BACKUP_PASSPHRASE=''\n", None, "''"),
        ("quoted words", 'PM_BACKUP_PASSPHRASE="fixture two words"\n', None, '"fixture two words"'),
        ("export line (v0.2.x made one: node/passphrase)", "export PM_BACKUP_PASSPHRASE=fixture-x\n", "fixture-made-1", "fixture-made-1"),
        ("colon line", "PM_BACKUP_PASSPHRASE: fixture-x\n", "fixture-made-2", "fixture-made-2"),
        ("leading space", " PM_BACKUP_PASSPHRASE=fixture-x\n", "fixture-made-3", "fixture-made-3"),
        ("today's ODEION_ name", "ODEION_BACKUP_PASSPHRASE=fixture-new\n", None, "fixture-new"),
        ("only node/passphrase", "", "fixture-made-4", "fixture-made-4"),
    ):
        b = make_backup(envtext, tok, pfile)
        d, r, calls, n = restore(b, locked)
        check(r.returncode == 0 and '"signins_restored":3' in r.stdout,
              f"passphrase {label}: the sign-ins come back ({(r.stdout + r.stderr).strip()[-140:]!r})")
    b = make_backup("", tok)
    d, r, calls, n = restore(b, "fixture-never")
    check(r.returncode == 0 and '"signins_restored":0' in r.stdout and n == 1,
          "no passphrase anywhere: restored without the sign-ins (skip_secrets), as before")
    b = make_backup("PM_BACKUP_PASSPHRASE=fixture-a\n", tok, "fixture-b")
    d, r, calls, n = restore(b, "fixture-wrong-everywhere")
    check(r.returncode != 0 and "does not open" in r.stderr and n == 2, "wrong every way: both tried once, then a stop saying why")
    b = make_backup('PM_BACKUP_PASSPHRASE="fixture-q"\n', tok, "fixture-f")
    d, r, calls, n = restore(b, "NOT-A-BACKUP")
    check(r.returncode != 0 and n == 1, "a backup the node cannot read is not retried with other passphrases")

    for script in ("backup.sh", "restore.sh"):
        s = (BUNDLE / script).read_text()
        check('. ./settings.sh' in s, f"{script} uses settings.sh")
        check("setting TOKEN" in s and "setting BACKUP_PASSPHRASE" in s and 'node_port "$NODE"' in s,
              f"{script} reads the token, passphrase and port through it")
        check("sed -n 's/^PM_LISTEN=" not in s and "env_get PM_TOKEN" not in s and "s/^PM_TOKEN=//p" not in s,
              f"{script} has no PM_-only reading left")
    check('docker exec "$NODE" sh -c "$NODE_BACKUP_SH"' in (BUNDLE / "backup.sh").read_text(),
          "backup.sh snapshots the database with the config the node uses")
finally:
    shutil.rmtree(base, ignore_errors=True)

print(f"\n{len(problems)} problem(s)")
sys.exit(1 if problems else 0)

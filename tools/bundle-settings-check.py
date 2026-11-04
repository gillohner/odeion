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

    check(setting("TOKEN", "PM_TOKEN=old-tok\n") == "old-tok", "token: the old name alone")
    check(setting("TOKEN", "ODEION_TOKEN=new-tok\n") == "new-tok", "token: the new name alone")
    check(setting("TOKEN", "PM_TOKEN=old-tok\nODEION_TOKEN=new-tok\n") == "new-tok", "token: the new name wins")
    check(setting("TOKEN", "PM_TOKEN=old-tok\nODEION_TOKEN=\n") == "old-tok", "token: an empty new name counts as unset")
    check(setting("TOKEN", "PM_TOKEN=old-tok\n", {"ODEION_TOKEN": "shell-tok"}) == "shell-tok",
          "token: the shell's environment before .env, as compose")
    check(setting("BACKUP_PASSPHRASE", "PM_BACKUP_PASSPHRASE=old pass\n", {"ODEION_BACKUP_PASSPHRASE": "new pass ♫"})
          == "new pass ♫", "passphrase: the new name wins, value unchanged")
    check(setting("BACKUP_PASSPHRASE", "", {"PM_BACKUP_PASSPHRASE": "old pass"}) == "old pass", "passphrase: the old name alone")
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
    (d / ".env").write_text("ODEION_TOKEN=fixture-new-token\nODEION_TAG=fixture\n")
    r = subprocess.run(["sh", str(d / "setup.sh")], cwd=d, env={"PATH": "/usr/bin:/bin", "HOME": str(base)},
                       capture_output=True, text=True)
    envf = (d / ".env").read_text()
    check(r.returncode == 0 and "PM_TOKEN=" not in envf and "PM_TAG=" not in envf and "ADMIN_PASSWORD=" in envf,
          "setup.sh adds no PM_TOKEN or PM_TAG next to the ODEION_ ones")
    check("fixture-new-token" not in r.stdout + r.stderr, "setup.sh prints no token")

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

    # setting TOKEN picks what compose gives the node, for the shell and .env together
    import json
    compose_file = BUNDLE / "docker-compose.yml"
    cases = [
        ({"ODEION_TOKEN": ""}, "ODEION_TOKEN=env-new\nPM_TOKEN=env-old\n"),
        ({}, "ODEION_TOKEN=env-new\nPM_TOKEN=env-old\n"),
        ({"ODEION_TOKEN": "shell-new"}, "ODEION_TOKEN=env-new\nPM_TOKEN=env-old\n"),
        ({"PM_TOKEN": "shell-old"}, "ODEION_TOKEN=\nPM_TOKEN=env-old\n"),
        ({"PM_TOKEN": ""}, "PM_TOKEN=env-old\nODEION_TOKEN=\n"),
        ({"ODEION_TOKEN": "", "PM_TOKEN": "shell-old"}, "ODEION_TOKEN=env-new\n"),
    ]
    for i, (shell, envtext) in enumerate(cases):
        cd = base / f"cmp{i}"
        cd.mkdir()
        shutil.copy(compose_file, cd / "docker-compose.yml")
        (cd / ".env").write_text("ADMIN_PASSWORD=fixture-admin\nDB_PASSWORD=fixture-db\n" + envtext)
        (cd / "home").mkdir()
        e = {"PATH": "/usr/bin:/bin", "HOME": str(cd / "home"), **shell}
        r = subprocess.run(["docker", "compose", "-f", str(cd / "docker-compose.yml"), "--project-directory", str(cd),
                            "config", "--format", "json"], cwd=cd, env=e, capture_output=True, text=True)
        want = json.loads(r.stdout)["services"]["music"]["environment"].get("PM_TOKEN") if r.returncode == 0 else ("" if "missing a value" in r.stderr else f"<compose refuses: {r.stderr.strip()[-120:]}>")
        rr = subprocess.run(["bash", "-euo", "pipefail", "-c", f'. "{BUNDLE}/settings.sh"\nsetting TOKEN'], cwd=cd,
                            env={"PATH": "/usr/bin:/bin", **shell}, capture_output=True, text=True)
        got = rr.stdout if rr.returncode == 0 else f"<error {rr.stderr.strip()[:80]}>"
        check(got == want, f"setting TOKEN matches compose: shell {shell}, .env {envtext.strip()!r} (compose {want!r}, setting {got!r})")

    for script in ("backup.sh", "restore.sh"):
        s = (BUNDLE / script).read_text()
        check('. ./settings.sh' in s, f"{script} uses settings.sh")
        check("TOKEN=$(setting TOKEN)" in s and "setting BACKUP_PASSPHRASE" in s and 'node_port "$NODE"' in s,
              f"{script} reads the token, passphrase and port through it")
        check("sed -n 's/^PM_LISTEN=" not in s and "env_get PM_TOKEN" not in s and "s/^PM_TOKEN=//p" not in s,
              f"{script} has no PM_-only reading left")
    check('docker exec "$NODE" sh -c "$NODE_BACKUP_SH"' in (BUNDLE / "backup.sh").read_text(),
          "backup.sh snapshots the database with the config the node uses")
finally:
    shutil.rmtree(base, ignore_errors=True)

print(f"\n{len(problems)} problem(s)")
sys.exit(1 if problems else 0)

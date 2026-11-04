#!/usr/bin/env python3
"""Checks the image's entrypoint (deploy/docker/entrypoint.sh) without Docker: it runs the script
with a stand-in `odeiond` on PATH, in a throwaway folder, with a cleared environment
(docs/branding-compatibility.md).

- ODEION_ settings are read; the PM_ names still work; the new name wins, even when empty;
- values pass through unchanged (spaces, unicode) and never become shell code;
- the canonical executable `odeiond` runs; the default node name is Odeion node.

    python3 -I tools/entrypoint-check.py [scratch dir]
"""

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
problems = []


def check(ok, what):
    print(f"{'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        problems.append(what)


base = Path(tempfile.mkdtemp(prefix="odeion-entrypoint-", dir=sys.argv[1] if len(sys.argv) > 1 else None))
try:
    bin_dir = base / "bin"
    bin_dir.mkdir()
    stub = bin_dir / "odeiond"
    stub.write_text('#!/bin/sh\nprintf "%s\\n" "ran odeiond $*" > "$STUB_OUT"\nenv | grep -E "^(PM|ODEION)_" | sort >> "$STUB_OUT"\n')
    stub.chmod(0o755)

    def run(env):
        data = Path(tempfile.mkdtemp(dir=base))
        out = data / "stub.out"
        e = {"PATH": f"{bin_dir}:/usr/bin:/bin", "STUB_OUT": str(out), "PM_DATA_DIR": str(data)}
        e.update(env)
        r = subprocess.run(["sh", str(REPO / "deploy" / "docker" / "entrypoint.sh"), "serve"], env=e,
                           cwd=base, capture_output=True, text=True)
        conf = (data / "node.toml").read_text() if (data / "node.toml").exists() else ""
        return r, conf, out.read_text() if out.exists() else ""

    r, conf, out = run({})
    check(r.returncode == 0, f"runs ({r.stderr.strip()[:200]})")
    check(out.startswith(f"ran odeiond -c {base}") and out.split("\n")[0].endswith("/node.toml serve"),
          "it runs odeiond on the written config")
    check('name = "Odeion node"' in conf, "the default node name is Odeion node")

    _, conf, _ = run({"ODEION_NAME": "Küche ♫ two words", "ODEION_FOLDERS": "/m1:/m2"})
    check('name = "Küche ♫ two words"' in conf, "ODEION_NAME is read, value unchanged")
    check('folders = ["/m1", "/m2"]' in conf, "ODEION_FOLDERS is read")
    _, conf, _ = run({"PM_NAME": "Old name", "PM_CONTACT": "old@example.invalid"})
    check('name = "Old name"' in conf and 'contact = "old@example.invalid"' in conf, "PM_ names still work")
    _, conf, out = run({"PM_NAME": "Old name", "ODEION_NAME": "New name"})
    check('name = "New name"' in conf, "the new name wins")
    check("PM_NAME=New name" in out, "the node sees the ODEION_ value under the PM_ name too")
    _, conf, _ = run({"PM_CONTACT": "old@example.invalid", "ODEION_CONTACT": ""})
    check('contact = ""' in conf, "the new name wins even when empty")
    _, conf, _ = run({"ODEION_NAME": "$(touch pwned)`touch pwned2`"})
    check(not (base / "pwned").exists() and not (base / "pwned2").exists() and "$(touch pwned)" in conf,
          "a value is never run as shell code")
    # The image's own PM_ defaults (and the compose files' PM_ keys) are not the user's leftovers: the
    # node gets each under both names, the same value, so it reports none of them as old names
    _, _, out = run({"PM_LISTEN": "0.0.0.0:7788", "PM_FOLDERS": "/music"})
    check("ODEION_LISTEN=0.0.0.0:7788" in out and "ODEION_FOLDERS=/music" in out and "PM_LISTEN=0.0.0.0:7788" in out,
          "PM_ settings reach the node under both names, the same value")
    _, _, out = run({"PM_NAME": "Old", "ODEION_NAME": "New"})
    check("ODEION_NAME=New" in out and "PM_NAME=New" in out, "both: both names carry the new value")
    _, _, out = run({"ODEIONX": "1", "ODEION_lower": "x"})
    check("PM_lower" not in out and "PMX" not in out, "names that are not ours are left alone")
finally:
    shutil.rmtree(base, ignore_errors=True)

print(f"\n{len(problems)} problem(s)")
sys.exit(1 if problems else 0)

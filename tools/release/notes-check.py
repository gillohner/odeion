#!/usr/bin/env python3
"""Runs the release workflow's `notes` step (.github/workflows/release.yml) for real, in bash, with
made-up values, and checks the notes it writes: every name is printed, nothing in them runs as a
command (stand-ins for `odeiond` and `pm-node` on PATH record any call), and bash says nothing on
stderr. A YAML parse cannot see a backtick that bash would run.

    python3 -I tools/release/notes-check.py [scratch dir]
"""

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
problems = []


def check(ok, what):
    print(f"{'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        problems.append(what)


def notes_script():
    lines = (REPO / ".github" / "workflows" / "release.yml").read_text().splitlines()
    start = next(i for i, l in enumerate(lines) if l.strip() == "- name: notes")
    run = next(i for i in range(start, len(lines)) if lines[i].strip() == "run: |")
    indent = len(lines[run]) - len(lines[run].lstrip()) + 2
    body = []
    for l in lines[run + 1:]:
        if l.strip() and len(l) - len(l.lstrip()) < indent:
            break
        body.append(l[indent:])
    return "\n".join(body) + "\n"


base = Path(tempfile.mkdtemp(prefix="odeion-notes-", dir=sys.argv[1] if len(sys.argv) > 1 else None))
try:
    script = notes_script()
    stubs = base / "bin"
    stubs.mkdir()
    calls = base / "calls"
    for name in ("odeiond", "pm-node"):
        s = stubs / name
        s.write_text(f'#!/bin/sh\necho "{name} $*" >> "{calls}"\n')
        s.chmod(0o755)
    for channel in ("edge", "v9.8.7"):
        work = base / channel
        work.mkdir()
        env = {"PATH": f"{stubs}:/usr/bin:/bin", "CHANNEL": channel, "GITHUB_SHA": "0123456789abcdef",
               "GITHUB_REPOSITORY": "example/fixture-repo"}
        r = subprocess.run(["bash", "-e", "-c", script], cwd=work, env=env, capture_output=True, text=True)
        notes = (work / "notes.md").read_text() if (work / "notes.md").exists() else ""
        check(r.returncode == 0, f"{channel}: the step succeeds")
        check(r.stderr == "", f"{channel}: bash says nothing on stderr ({r.stderr.strip()[:160]!r})")
        check(not calls.exists(), f"{channel}: no name in the notes runs as a command")
        check("`odeiond` (also as `pm-node`, its old name" in notes, f"{channel}: the node's names are printed")
        check("`pubky-music-node-linux-{x86_64,arm64,armv7}.tar.gz`" in notes, f"{channel}: download names are printed")
        check("`SHA256SUMS`" in notes, f"{channel}: the checksum line is printed")
        if channel != "edge":
            check(notes.startswith("Odeion v9.8.7. What changed"), f"{channel}: a release's notes start with Odeion")
            check("ghcr.io/example/fixture-repo:9.8.7" in notes, f"{channel}: the image tag is printed")
finally:
    shutil.rmtree(base, ignore_errors=True)

print(f"\n{len(problems)} problem(s)")
sys.exit(1 if problems else 0)

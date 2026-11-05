#!/usr/bin/env python3
"""Every action a workflow uses is pinned to a full commit SHA (`owner/repo@<40 hex> # vX.Y.Z`).

    python3 -I tools/workflow-pins-check.py [.github/workflows]

A tag or a branch can be moved to other code after review; a commit SHA can't. Local actions
(`./…`) and Docker images (`docker://…@sha256:…`) are not GitHub refs and are left alone.
Exit 1 naming each unpinned use."""
import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else ".github/workflows")
USES = re.compile(r"^\s*-?\s*uses:\s*(['\"]?)([^\s'\"#]+)\1")
PINNED = re.compile(r"^[^@\s]+@[0-9a-f]{40}$")
bad = []
for f in sorted(root.glob("*.y*ml")):
    for n, line in enumerate(f.read_text().splitlines(), 1):
        m = USES.match(line)
        if not m:
            continue
        ref = m.group(2)
        if ref.startswith("./") or ref.startswith("docker://"):
            continue
        if not PINNED.match(ref):
            bad.append(f"{f}:{n}: {ref} is not pinned to a commit SHA")
for b in bad:
    print(b)
print(f"{len(bad)} problem(s)")
sys.exit(1 if bad else 0)

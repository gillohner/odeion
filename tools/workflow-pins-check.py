#!/usr/bin/env python3
"""Every action a workflow uses is pinned to a full commit SHA (`owner/repo@<40 hex> # vX.Y.Z`).

    python3 -I tools/workflow-pins-check.py [.github/workflows]
    python3 -I tools/workflow-pins-check.py --self-test

A tag or a branch can be moved to other code after review; a commit SHA can't. Local actions
(`./…`) need no pin; Docker images must name a digest (`docker://…@sha256:<64 hex>`).
Any `uses` key counts, in block or flow style and quoted or not. A value this check can't read
on the key's own line (a block scalar, or the value on the next line) is reported too, so the
check fails rather than passes what it can't see. Exit 1 naming each problem."""
import pathlib
import re
import sys

# A `uses` key anywhere on the line: `uses:`, `'uses':`, `"uses":`, `{uses: …}`.
USES = re.compile(r"""(?:^|[\s{,-])(['"]?)uses\1\s*:(?=\s|$)\s*(.*)$""")
VALUE = re.compile(r"""^(['"]?)([^\s'",}#]+)\1""")
PINNED = re.compile(r"^[^@\s]+@[0-9a-f]{40}$")
DOCKER = re.compile(r"^docker://[^@\s]+@sha256:[0-9a-f]{64}$")


def problems(name, text):
    out = []
    for n, line in enumerate(text.splitlines(), 1):
        if line.lstrip().startswith("#"):
            continue
        m = USES.search(line)
        if not m:
            continue
        v = VALUE.match(m.group(2))
        if not v:
            out.append(f"{name}:{n}: a `uses` value this check can't read; put it on the key's line")
            continue
        ref = v.group(2)
        if ref.startswith("./"):
            continue
        if ref.startswith("docker://"):
            if not DOCKER.match(ref):
                out.append(f"{name}:{n}: {ref} names no image digest")
            continue
        if not PINNED.match(ref):
            out.append(f"{name}:{n}: {ref} is not pinned to a commit SHA")
    return out


def self_test():
    sha = "a" * 40
    good = [
        f"      - uses: actions/checkout@{sha} # v6.1.0",
        f"    uses: org/repo/.github/workflows/x.yml@{sha}",
        f"      - {{uses: actions/checkout@{sha}}}",
        f"      - 'uses': \"actions/checkout@{sha}\"",
        "      - uses: ./.github/actions/local",
        "      - uses: docker://alpine@sha256:" + "b" * 64,
        "      # uses: actions/checkout@v6",
        "      run: echo reuses: nothing",
    ]
    bad = [
        "      - uses: actions/checkout@v6",
        "      - {uses: actions/checkout@v6}",
        "      - { name: x, uses: actions/checkout@main }",
        "      - 'uses': actions/checkout@v6",
        '      - "uses": "actions/checkout@v6"',
        "    uses: org/repo/.github/workflows/x.yml@main",
        "      - uses: docker://alpine:latest",
        "      - uses: >-",
        "      - uses:",
        f"      - uses: actions/checkout@{sha[:39]}",
    ]
    for line in good:
        assert not problems("t", line), f"flagged a pinned use: {line!r}"
    for line in bad:
        assert problems("t", line), f"missed an unpinned use: {line!r}"
    print(f"self-test ok ({len(good)} pinned, {len(bad)} unpinned)")


if sys.argv[1:] == ["--self-test"]:
    self_test()
    sys.exit(0)
root = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else ".github/workflows")
bad = []
for f in sorted(root.glob("*.y*ml")):
    bad += problems(str(f), f.read_text())
for b in bad:
    print(b)
print(f"{len(bad)} problem(s)")
sys.exit(1 if bad else 0)

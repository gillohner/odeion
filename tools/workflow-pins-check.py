#!/usr/bin/env python3
"""Every action a workflow uses is pinned to a full commit SHA (`owner/repo@<40 hex> # vX.Y.Z`).

    python3 -I tools/workflow-pins-check.py [.github/workflows]
    python3 -I tools/workflow-pins-check.py --self-test

A tag or a branch can be moved to other code after review; a commit SHA can't. Local actions
(`./…`) need no pin; Docker images must name a digest (`docker://…@sha256:<64 hex>`).

Each workflow is read as YAML (PyYAML, on every GitHub runner) and every `uses` value in it is
checked, however it is written: block or flow style, quoted, escaped, several on one line. A
`uses` whose value isn't one plain string is reported. Without PyYAML the check reads the lines
instead: a `uses` key per line, and a value it can't read on the key's own line is reported, so
the check fails rather than passes what it can't see. Exit 1 naming each problem."""
import pathlib
import re
import sys

try:
    import yaml
except ImportError:  # the line reader then
    yaml = None

# A `uses` key anywhere on the line: `uses:`, `'uses':`, `"uses":`, `{uses: …}`.
USES = re.compile(r"""(?:^|[\s{,-])(['"]?)uses\1\s*:(?=\s|$)\s*(.*)$""")
VALUE = re.compile(r"""^(['"]?)([^\s'",}#]+)\1""")
PINNED = re.compile(r"^[^@\s]+@[0-9a-f]{40}$")
DOCKER = re.compile(r"^docker://[^@\s]+@sha256:[0-9a-f]{64}$")


def check(name, n, ref):
    """What is wrong with the action `ref` at line `n`, if anything."""
    if ref.startswith("./"):
        return None
    if ref.startswith("docker://"):
        if not DOCKER.match(ref):
            return f"{name}:{n}: {ref} names no image digest"
        return None
    if not PINNED.match(ref):
        return f"{name}:{n}: {ref} is not pinned to a commit SHA"
    return None


def problems_yaml(name, text):
    """Every `uses` in the YAML document, wherever it is."""
    try:
        root = yaml.compose(text)
    except yaml.YAMLError as e:
        return [f"{name}: not readable as YAML: {e}"]
    out = []

    def walk(node):
        if isinstance(node, yaml.MappingNode):
            for key, value in node.value:
                # An action's inputs and variables may be named anything; they are not actions
                if isinstance(key, yaml.ScalarNode) and key.value in ("with", "env"):
                    continue
                if isinstance(key, yaml.ScalarNode) and key.value == "uses":
                    n = value.start_mark.line + 1
                    if isinstance(value, yaml.ScalarNode) and value.value.strip():
                        bad = check(name, n, value.value.strip())
                        if bad:
                            out.append(bad)
                    else:
                        out.append(f"{name}:{n}: a `uses` that is not one action name")
                walk(value)
        elif isinstance(node, yaml.SequenceNode):
            for item in node.value:
                walk(item)

    if root is not None:
        walk(root)
    return out


def problems_lines(name, text):
    """A `uses` per line, read from the line itself (without PyYAML)."""
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
        bad = check(name, n, v.group(2))
        if bad:
            out.append(bad)
    return out


def problems(name, text):
    return problems_yaml(name, text) if yaml else problems_lines(name, text)


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
        "      run: 'echo reuses: nothing'",
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
    # Forms only the YAML reader sees: several on one line, an escaped key, a nested flow mapping
    # (an input named `uses` under `with` is not an action)
    yaml_only_bad = [
        f"steps: [{{uses: a/b@{sha}}}, {{uses: c/d@v1}}]",
        '      - "u\\x73es": actions/checkout@v1',
        f"steps: [{{with: {{x: 1}}, uses: a/b@{sha}}}, {{with: {{uses: fine}}, uses: c/d@main}}]",
    ]
    for line in good:
        assert not problems_lines("t", line), f"flagged a pinned use: {line!r}"
    for line in bad:
        assert problems_lines("t", line), f"missed an unpinned use: {line!r}"
    if yaml:
        for line in good:
            assert not problems_yaml("t", line), f"flagged a pinned use: {line!r}"
        for line in bad:
            assert problems_yaml("t", line), f"missed an unpinned use: {line!r}"
        for line in yaml_only_bad:
            found = problems_yaml("t", line)
            assert len(found) == 1 and ("@v1" in found[0] or "@main" in found[0]), f"{line!r}: {found}"
        assert problems_yaml("t", "steps: [") , "unreadable YAML is a problem"
        print(f"self-test ok ({len(good)} pinned, {len(bad) + len(yaml_only_bad)} unpinned, YAML read)")
    else:
        print(f"self-test ok ({len(good)} pinned, {len(bad)} unpinned; no PyYAML, lines only)")


if sys.argv[1:] == ["--self-test"]:
    self_test()
    sys.exit(0)
root = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else ".github/workflows")
bad = []
for f in sorted(root.glob("*.y*ml")):
    bad += problems(str(f), f.read_text())
for b in bad:
    print(b)
print(f"{len(bad)} problem(s)" + ("" if yaml else " (no PyYAML: read line by line)"))
sys.exit(1 if bad else 0)

#!/usr/bin/env python3
"""The two systemd units of the node on bare metal (deploy/bare-metal), and the move between them
(docs/branding-compatibility.md):

- odeiond.service and pubky-music-node.service conflict both ways, and exactly one ordering edge
  joins them (odeiond.service After= the old one, no edge back, so no ordering cycle): starting one
  while the other runs stops the other first, so the port is free;
- both run as the same user on the same config and data, and log at info level so the name-only
  notes about old names show in the journal;
- `systemd-analyze verify` accepts both (with the executables stood in by /bin/true);
- the move (unit header, Linux guide, compatibility page) looks at `systemctl cat` and carries the
  old unit's ReadWritePaths=, Environment= lines and drop-ins over, so no local change is lost.

    python3 -I tools/units-check.py [scratch dir]
"""

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
UNITS = REPO / "deploy" / "bare-metal"
problems = []


def check(ok, what):
    print(f"{'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        problems.append(what)


def directives(text):
    out = {}
    for line in text.splitlines():
        line = line.strip()
        if line and not line.startswith(("#", ";", "[")) and "=" in line:
            k, v = line.split("=", 1)
            out.setdefault(k.strip(), []).append(v.strip())
    return out


new_text = (UNITS / "odeiond.service").read_text()
old_text = (UNITS / "pubky-music-node.service").read_text()
new, old = directives(new_text), directives(old_text)
words = lambda d, k: " ".join(d.get(k, [])).split()

check("pubky-music-node.service" in words(new, "Conflicts"), "odeiond.service conflicts with the old unit")
check("odeiond.service" in words(old, "Conflicts"), "the old unit conflicts with odeiond.service")
edges_new = [k for k in ("After", "Before") if "pubky-music-node.service" in words(new, k)]
edges_old = [k for k in ("After", "Before") if "odeiond.service" in words(old, k)]
check(edges_new == ["After"] and edges_old == [], f"exactly one ordering edge, odeiond After= the old unit ({edges_new}, {edges_old})")
for name, d in (("odeiond.service", new), ("pubky-music-node.service", old)):
    check(d.get("User") == ["pubky-music"] and d.get("Group") == ["pubky-music"], f"{name}: the pubky-music user")
    check(d.get("ReadWritePaths") == ["/var/lib/pubky-music"], f"{name}: the same data folder")
    check(any("-c /etc/pubky-music/node.toml serve" in e for e in d.get("ExecStart", [])), f"{name}: the same config")
    check("RUST_LOG=info" in words(d, "Environment"), f"{name}: logs at info level")
    check(not d.get("Alias"), f"{name}: no Alias=")

for doc in ("deploy/bare-metal/odeiond.service", "docs/guides/install-linux.md", "docs/branding-compatibility.md"):
    t = (REPO / doc).read_text()
    check("systemctl cat pubky-music-node" in t, f"{doc}: the move starts by looking at the old unit")
    check("pubky-music-node.service.d" in t and "odeiond.service.d" in t, f"{doc}: drop-ins are carried over")
    check("ReadWritePaths=" in t and "Environment=" in t, f"{doc}: edited ReadWritePaths= and Environment= are carried over")
for doc in ("deploy/bare-metal/odeiond.service", "docs/guides/install-linux.md", "docs/branding-compatibility.md"):
    t = (REPO / doc).read_text()
    move = t[t.find("systemctl cat pubky-music-node"):]
    inst, link, enable = move.find("install -m 755"), move.find("ln -sf odeiond /usr/local/bin/pm-node"), move.find("enable --now odeiond")
    check(-1 < inst < enable and -1 < link < enable, f"{doc}: the move installs odeiond and the pm-node link before enabling the unit")
guide = (REPO / "docs/guides/backup-and-update.md").read_text()
check("odeiond.service" in guide and "ReadWritePaths=" in guide, "the backup guide names the unit it edits")

if shutil.which("systemd-analyze"):
    d = Path(tempfile.mkdtemp(prefix="odeion-units-", dir=sys.argv[1] if len(sys.argv) > 1 else None))
    try:
        for f in ("odeiond.service", "pubky-music-node.service", "pubky-homeserver.service"):
            t = (UNITS / f).read_text()
            t = "\n".join("ExecStart=/bin/true" if l.startswith("ExecStart=") else l for l in t.splitlines()) + "\n"
            (d / f).write_text(t)
        r = subprocess.run(["systemd-analyze", "verify", "--man=no", str(d / "odeiond.service"), str(d / "pubky-music-node.service")],
                           capture_output=True, text=True, env={"PATH": "/usr/bin:/bin", "SYSTEMD_LOG_LEVEL": "warning"})
        msgs = [l for l in (r.stdout + r.stderr).splitlines() if "odeiond" in l or "pubky-music-node" in l]
        check(r.returncode == 0 and not msgs, f"systemd-analyze verify accepts both ({' | '.join(msgs)[:300]})")
    finally:
        shutil.rmtree(d, ignore_errors=True)
else:
    print("skip systemd-analyze: not installed")

print(f"\n{len(problems)} problem(s)")
sys.exit(1 if problems else 0)

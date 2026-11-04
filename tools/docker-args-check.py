#!/usr/bin/env python3
"""The image's build arguments (Dockerfile, build stage): every PM_ build value (the node's version,
the project's app ids) has its ODEION_ counterpart, declared without a default, so that

- an ODEION_ value passed with --build-arg reaches the build (pm_model::brand::build_env reads it
  first), also when passed empty;
- an ODEION_ value not passed stays unset, so the PM_ one (the release workflow's) still counts.

The second half builds a throwaway Dockerfile made of the real ARG lines on busybox (with Docker,
when it is there; nothing is exported or pushed: --output type=cacheonly) and prints, per name,
whether it is set and to what (made-up values).

    python3 -I tools/docker-args-check.py [scratch dir]
"""

import re
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


text = (REPO / "Dockerfile").read_text()
stages = re.split(r"(?m)^FROM ", text)
build = stages[1]
args = re.findall(r"(?m)^ARG (\S+)", build)
legacy = [a for a in args if a.startswith("PM_")]
check(legacy, f"the build stage has PM_ build values ({len(legacy)})")
for a in legacy:
    new = "ODEION_" + a[3:]
    check(new in args, f"{new} is a build argument")
check(not [a for a in args if a.startswith("ODEION_") and "=" in a], "no ODEION_ build argument has a default")
build_line = build.index("cargo build")
check(all(build.index(f"ARG {a}") < build_line for a in args if a.startswith(("ODEION_", "PM_"))),
      "every build value is declared before the build runs")

if shutil.which("docker"):
    d = Path(tempfile.mkdtemp(prefix="odeion-dockerargs-", dir=sys.argv[1] if len(sys.argv) > 1 else None))
    try:
        names = ["PM_NODE_VERSION", "ODEION_NODE_VERSION", "PM_PROJECT_TIDAL_CLIENT_ID", "ODEION_PROJECT_TIDAL_CLIENT_ID"]
        lines = [l for l in build.splitlines() if re.match(r"ARG (PM|ODEION)_", l)]
        show = " ; ".join(f'if [ "${{{n}+set}}" = set ]; then echo "{n}=[${n}]"; else echo "{n} unset"; fi' for n in names)
        (d / "Dockerfile").write_text("FROM busybox:1.37\n" + "\n".join(lines) + f"\nRUN {show}\n")

        def build_with(*kv):
            cmd = ["docker", "build", "--no-cache", "--progress=plain", "--output", "type=cacheonly"]
            for k in kv:
                cmd += ["--build-arg", k]
            r = subprocess.run(cmd + [str(d)], capture_output=True, text=True)
            out = r.stdout + r.stderr
            return r.returncode, {n: (re.search(rf"{n}=\[([^\]]*)\]", out) or [None, None])[1] if f"{n} unset" not in out else None
                                  for n in names}, out

        rc, v, out = build_with("PM_NODE_VERSION=fixture-old")
        if rc != 0 and "operation not permitted" in out:
            # This machine's Docker cannot build at all (BuildKit may not mount): say so, pass nothing
            print("skip (NOT VERIFIED here): this Docker cannot build: " + out.strip().splitlines()[-1][:200])
            raise SystemExit(0 if not problems else 1)
        check(rc == 0, "the stand-in build runs" + ("" if rc == 0 else ": " + out[-300:]))
        check(v["PM_NODE_VERSION"] == "fixture-old" and v["ODEION_NODE_VERSION"] is None,
              "only the old name passed: the new one stays unset (the old one counts)")
        rc, v, _ = build_with("ODEION_NODE_VERSION=fixture-new", "ODEION_PROJECT_TIDAL_CLIENT_ID=fixture-id")
        check(v["ODEION_NODE_VERSION"] == "fixture-new" and v["ODEION_PROJECT_TIDAL_CLIENT_ID"] == "fixture-id"
              and v["PM_NODE_VERSION"] is None, "only the new name passed: it reaches the build")
        rc, v, _ = build_with("PM_NODE_VERSION=fixture-old", "ODEION_NODE_VERSION=")
        check(v["ODEION_NODE_VERSION"] == "" and v["PM_NODE_VERSION"] == "fixture-old",
              "the new name passed empty: set and empty (so it wins)")
        rc, v, _ = build_with()
        check(all(v[n] is None for n in names), "nothing passed: all unset")
    finally:
        shutil.rmtree(d, ignore_errors=True)
else:
    print("skip the build: docker is not installed")

print(f"\n{len(problems)} problem(s)")
sys.exit(1 if problems else 0)

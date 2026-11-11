#!/usr/bin/env python3
"""Checks how the shipped compose files read settings, with `docker compose config` (no daemon,
nothing started) on made-up `.env` files in a throwaway folder (docs/branding-compatibility.md):

- deploy/bundle: every setting a person sets in `.env` (`IMAGE`, `TAG`, `TOKEN`, `NAME`, `CONTACT`,
  `PUBLIC_URL`) may be spelled ODEION_ or PM_; the ODEION_ one wins. An ODEION_ setting set to
  empty wins too, except the required token, image and tag (empty is never valid there, so it
  counts as unset). The project, its volumes and the image name are unchanged; a missing token
  still stops the start.
- deploy/truenas: a setting added under `environment:` as ODEION_ reaches the container as it is
  (the image's entrypoint reads it; tools/entrypoint-check.py).

    python3 -I tools/compose-check.py [scratch dir]
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
# The compose command: COMPOSE="/path/to/docker-compose" checks another version (CI has v2)
COMPOSE = os.environ.get("COMPOSE", "docker compose").split()
problems = []


def check(ok, what):
    print(f"{'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        problems.append(what)


base = Path(tempfile.mkdtemp(prefix="odeion-compose-", dir=sys.argv[1] if len(sys.argv) > 1 else None))


def config(compose, env_text, extra_env=None, name="case"):
    d = base / name
    d.mkdir()
    shutil.copy(compose, d / compose.name)
    (d / ".env").write_text(env_text)
    (d / "home").mkdir()
    env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": str(d / "home")}
    env.update(extra_env or {})
    r = subprocess.run(COMPOSE + ["-f", str(d / compose.name), "--project-directory", str(d),
                        "config", "--format", "json"], env=env, capture_output=True, text=True, cwd=d)
    return r, (json.loads(r.stdout) if r.returncode == 0 else {})


BUNDLE = REPO / "deploy" / "bundle" / "docker-compose.yml"
base_env = "ADMIN_PASSWORD=fixture-admin\nDB_PASSWORD=fixture-db\n"
n = 0


def bundle(env_text, extra_env=None):
    global n
    n += 1
    r, c = config(BUNDLE, base_env + env_text, extra_env, f"bundle{n}")
    music = c.get("services", {}).get("music", {})
    return r, c, music.get("environment", {}), music.get("image", "")


try:
    r, c, e, image = bundle("PM_TOKEN=fixture-old-token\nPM_NAME=Old name\nPM_CONTACT=old@example.invalid\n"
                            "PM_PUBLIC_URL=http://192.0.2.1:7788\nPM_TAG=v0.2.2\n")
    check(r.returncode == 0, f"old names only: compose accepts it ({r.stderr.strip()[:200]})")
    check(e.get("PM_TOKEN") == "fixture-old-token" and e.get("PM_NAME") == "Old name"
          and e.get("PM_CONTACT") == "old@example.invalid" and e.get("PM_PUBLIC_URL") == "http://192.0.2.1:7788",
          "old names only: the node gets them")
    check(image == "ghcr.io/gillohner/odeion:v0.2.2", f"old names only: the same image ({image})")
    check(c.get("name") == "pubky-music", "the compose project keeps its name")
    check({"music-node", "cloudflare"} <= set(c.get("volumes", {})), "the volumes keep their names")
    check(all(v.get("name", "").startswith("pubky-music_") for v in c.get("volumes", {}).values()),
          "volumes stay under the pubky-music project")

    r, c, e, image = bundle("ODEION_TOKEN=fixture-new-token\nODEION_NAME=Küche ♫\nODEION_CONTACT=new@example.invalid\n"
                            "ODEION_PUBLIC_URL=http://192.0.2.2:7788\nODEION_IMAGE=registry.invalid/fixture/odeion\n"
                            "ODEION_TAG=fixture\n")
    check(r.returncode == 0, f"new names only: compose accepts it ({r.stderr.strip()[:200]})")
    check(e.get("PM_TOKEN") == "fixture-new-token" and e.get("PM_NAME") == "Küche ♫"
          and e.get("PM_CONTACT") == "new@example.invalid" and e.get("PM_PUBLIC_URL") == "http://192.0.2.2:7788",
          "new names only: the node gets them")
    check(image == "registry.invalid/fixture/odeion:fixture", f"new names only: image and tag ({image})")

    r, c, e, image = bundle("PM_TOKEN=fixture-old-token\nODEION_TOKEN=fixture-new-token\nPM_NAME=Old\nODEION_NAME=New\n"
                            "PM_TAG=old-tag\nODEION_TAG=new-tag\nPM_CONTACT=old@example.invalid\nODEION_CONTACT=new@example.invalid\n")
    check(e.get("PM_TOKEN") == "fixture-new-token" and e.get("PM_NAME") == "New" and e.get("PM_CONTACT") == "new@example.invalid"
          and image.endswith(":new-tag"), "both: the new names win")

    r, c, e, image = bundle("PM_TOKEN=fixture-old-token\nPM_NAME=Old\nODEION_NAME=\nPM_CONTACT=old@example.invalid\n"
                            "ODEION_CONTACT=\nPM_PUBLIC_URL=http://192.0.2.1:7788\nODEION_PUBLIC_URL=\n")
    check(e.get("PM_NAME") == "" and e.get("PM_CONTACT") == "" and e.get("PM_PUBLIC_URL") == "",
          f"an empty new name wins over the old one ({e.get('PM_NAME')!r}, {e.get('PM_CONTACT')!r})")

    r, c, e, image = bundle("PM_TOKEN=fixture-old-token\nODEION_TOKEN=\nPM_TAG=old-tag\nODEION_TAG=\n")
    check(e.get("PM_TOKEN") == "fixture-old-token" and image.endswith(":old-tag"),
          "an empty token or tag counts as unset (never valid)")

    r, c, e, image = bundle("PM_TOKEN=fixture-old-token\n", {"ODEION_NAME": "From the shell"})
    check(e.get("PM_NAME") == "From the shell", "a new name from the shell's environment is read too")

    # No token at all: compose passes an empty one and the node makes its own and keeps it (it listens
    # beyond localhost: crates/pm-node/src/api_token.rs), so the API is never open; a bundle without
    # setup.sh's ADMIN_PASSWORD still stops
    r, c, e, image = bundle("PM_NAME=No token\n")
    check(r.returncode == 0 and e.get("PM_TOKEN") == "" and e.get("PM_LISTEN") == "0.0.0.0:7788",
          "no token at all: an empty one, so the node makes its own (it listens beyond localhost)")
    keep, base_env = base_env, "DB_PASSWORD=fixture-db\n"
    r, *_ = bundle("PM_TOKEN=fixture-old-token\n")
    base_env = keep
    check(r.returncode != 0 and "run ./setup.sh first" in r.stderr, "without setup.sh's ADMIN_PASSWORD the start still stops")

    r, c, e, image = bundle("PM_TOKEN=fixture-old-token\n")
    check(e.get("PM_NAME") == "Odeion" and image == "ghcr.io/gillohner/odeion:edge",
          "nothing set: the default name and image")

    TRUENAS = REPO / "deploy" / "truenas" / "docker-compose.yaml"
    text = TRUENAS.read_text().replace('      PM_NAME: "TrueNAS"\n', '      ODEION_NAME: "TrueNAS ♫"\n      PM_NAME: "TrueNAS"\n')
    assert "ODEION_NAME" in text
    alt = base / "truenas-src"
    alt.mkdir()
    (alt / "docker-compose.yaml").write_text(text)
    r, c = config(alt / "docker-compose.yaml", "", None, "truenas")
    te = c.get("services", {}).get("node", {}).get("environment", {})
    check(r.returncode == 0 and te.get("ODEION_NAME") == "TrueNAS ♫" and te.get("PM_NAME") == "TrueNAS",
          "TrueNAS: an ODEION_ setting under environment reaches the container")
    check(c.get("services", {}).get("node", {}).get("image") == "ghcr.io/gillohner/odeion:edge",
          "TrueNAS: the same image")
finally:
    shutil.rmtree(base, ignore_errors=True)

print(f"\n{len(problems)} problem(s)")
sys.exit(1 if problems else 0)

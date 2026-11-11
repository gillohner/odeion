#!/usr/bin/env python3
"""Exercise the release tag-copy step with offline registry and Docker stubs."""
import json
from pathlib import Path
import subprocess

workflow = (Path(__file__).resolve().parents[1] / ".github/workflows/release.yml").read_text()
step = workflow.split("- name: copy older version tags to the new name", 1)[1]
assert "shell: bash" in step.split("run: |", 1)[0]
lines = []
for line in step.split("run: |\n", 1)[1].splitlines():
    if line and not line.startswith("          "):
        break
    lines.append(line[10:])
script = "\n".join(lines)
stub = r'''
curl() {
  local url="${@: -1}" key
  case "$url" in
    *token*repository:gillohner/odeion:pull) key=TARGET_TOKEN ;;
    *token*repository:gillohner/pubky-music-node:pull) key=SOURCE_TOKEN ;;
    *v2/gillohner/odeion/tags/*) key=TARGET_TAGS ;;
    *v2/gillohner/pubky-music-node/tags/*) key=SOURCE_TAGS ;;
    *) return 98 ;;
  esac
  if [ "$FAIL" = "$key" ]; then return 22; fi
  printf '%s\n' "${!key}"
}
docker() {
  printf 'COPY %s\n' "$*"
  if [ "$FAIL" = DOCKER ]; then return 42; fi
}
'''
defaults = {
    "PATH": "/usr/bin:/bin",
    "FAIL": "",
    "TARGET_TOKEN": '{"token":"fixture-token"}',
    "SOURCE_TOKEN": '{"token":"fixture-token"}',
    "TARGET_TAGS": '{"tags":["edge","0.2.2"]}',
    "SOURCE_TAGS": '{"tags":["0.2.2","latest"]}',
}
results = []


def case(name, changes, success=False, copies=None, code=None):
    run = subprocess.run(
        ["/bin/bash", "-e", "-o", "pipefail", "-c", stub + script],
        env=defaults | changes, text=True, capture_output=True, check=False,
    )
    actual = [line for line in run.stdout.splitlines() if line.startswith("COPY ")]
    assert (run.returncode == 0) == success, (name, run.returncode, run.stderr)
    if code is not None:
        assert run.returncode == code, (name, run.returncode)
    if not success and code != 42:
        assert not actual, (name, actual)
    if copies is not None:
        assert len(actual) == len(copies), (name, actual)
        for tag, line in zip(copies, actual):
            assert f"odeion:{tag} " in line, (name, line)
    results.append({"case": name, "exit": run.returncode, "copies": len(actual)})


for endpoint in ("TARGET_TOKEN", "SOURCE_TOKEN", "TARGET_TAGS", "SOURCE_TAGS"):
    case(endpoint + " HTTP failure", {"FAIL": endpoint})
    case(endpoint + " malformed JSON", {endpoint: "{"})
for endpoint in ("TARGET_TOKEN", "SOURCE_TOKEN"):
    for value in ({}, {"token": None}, {"token": ""}, {"token": 7}):
        case(endpoint + " invalid " + json.dumps(value), {endpoint: json.dumps(value)})
for endpoint in ("TARGET_TAGS", "SOURCE_TAGS"):
    for value in ({}, {"tags": "bad"}, {"tags": [None]}, {"tags": ["bad tag"]}):
        case(endpoint + " invalid " + json.dumps(value), {endpoint: json.dumps(value)})
case("existing tags preserved", {}, True, ["latest"])
case("valid empty destination", {"TARGET_TAGS": '{"tags":[]}'}, True, ["0.2.2", "latest"])
case("valid empty source", {"SOURCE_TAGS": '{"tags":[]}'}, True, [])
case("valid null source", {"SOURCE_TAGS": '{"tags":null}'}, True, [])
case("valid null destination", {"TARGET_TAGS": '{"tags":null}'}, True, ["0.2.2", "latest"])
case("version filter", {"SOURCE_TAGS": '{"tags":["edge","sha-abc","1.2.3-rc1","1.2","1.2.3","latest"]}'},
     True, ["1.2", "1.2.3", "latest"])
case("Docker failure", {"FAIL": "DOCKER"}, code=42)
print(json.dumps({"passed": len(results), "results": results}, indent=2))

#!/usr/bin/env python3
"""Freeze one fixture node's public search answers for all control-audit shards.

The audit still clicks every visible kind. Provider ranking/timing must not change
the inventory between shards or between a control's discovery and its click.
Only CI's isolated, account-free fixture node is started here.
"""
import json
import pathlib
import shutil
import subprocess
import sys
import time
import urllib.request


def main():
    binary, root, output = map(pathlib.Path, sys.argv[1:])
    root.mkdir(parents=True, exist_ok=True)
    music = root / "music"
    music.mkdir(exist_ok=True)
    for source in pathlib.Path("crates/pm-audio/tests/fixtures").glob("*.flac"):
        shutil.copyfile(source, music / source.name)
    config = root / "node.toml"
    config.write_text(f'''data_dir = {json.dumps(str(root / "data"))}
[library]
folders = [{json.dumps(str(music))}]
[api]
listen = "127.0.0.1:18783"
token = "ci-token"
[outputs]
discover = false
upnp = false
cast = false
slimproto_port = 0
[remote]
enabled = false
''')
    def get(path):
        request = urllib.request.Request("http://127.0.0.1:18783/v1" + path,
                                         headers={"Authorization": "Bearer ci-token"})
        return urllib.request.urlopen(request, timeout=40)

    with (root / "node.log").open("w") as log:
        node = subprocess.Popen([str(binary.resolve()), "-c", str(config), "serve"], stdout=log, stderr=log)
        try:
            for _ in range(60):
                if node.poll() is not None:
                    raise RuntimeError("fixture node exited before search capture")
                try:
                    with get("/library/tracks?limit=1") as response:
                        if json.load(response).get("tracks"):
                            break
                except (OSError, ValueError):
                    pass
                time.sleep(1)
            else:
                raise RuntimeError("fixture library did not become ready")
            with get("/search?q=queen&in=everywhere") as response:
                sources = response.headers.get("x-pm-sources", "").split(",")
                answers = [json.loads(line) for line in response.read().decode().splitlines() if line.strip()]
            if not sources or any(not s for s in sources) or {a["source"] for a in answers} != set(sources):
                raise RuntimeError("search did not return every advertised source")
            hits = [h for a in answers if a.get("status") == "ok" for h in a.get("hits", [])]
            # Preliminary check; every shard also checks the rendered, merged controls.
            for kind, minimum in [("song", 7), ("album", 12), ("artist", 1)]:
                if sum(h["kind"] == kind for h in hits) < minimum:
                    raise RuntimeError(f"search capture lacks the required {kind} controls")
            output.write_text(json.dumps({"query": "queen", "sources": sources, "answers": answers}, indent=2) + "\n")
        finally:
            node.terminate()
            node.wait(timeout=20)


if __name__ == "__main__":
    main()

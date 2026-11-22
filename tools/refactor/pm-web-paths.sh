#!/bin/bash
# The web UI's files moved from crates/pm-api/static to crates/pm-web/static (the pm-web crate).
# This rewrites every path to them in the tools, workflows and docs, and moves the folder if it is
# still at the old place. Safe to run again: on a branch made before the move, run it after
# merging or rebasing onto main and commit the result.
#   tools/refactor/pm-web-paths.sh
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
if [ -d crates/pm-api/static ]; then
  mkdir -p crates/pm-web
  if [ -d crates/pm-web/static ]; then
    # Both exist (a merge brought new files to the old place): move them over one by one
    git ls-files crates/pm-api/static | while read -r f; do git mv -f "$f" "crates/pm-web/static/${f#crates/pm-api/static/}"; done
  else
    git mv crates/pm-api/static crates/pm-web/static
  fi
fi
# Everything that names the folder, except the changelog (it says where things were at the time)
named() { git grep "$1" -E "pm-api/static|'pm-api', 'static'" -- . ':!docs/CHANGELOG.md' ':!tools/refactor/pm-web-paths.sh' || true; }
named -l | while read -r f; do
  sed -i \
    -e "s#crates/pm-api/static#crates/pm-web/static#g" \
    -e "s#\.\./pm-api/static#../pm-web/static#g" \
    -e "s#'crates', 'pm-api', 'static'#'crates', 'pm-web', 'static'#g" \
    "$f"
  echo "updated $f"
done
left=$(named -n)
if [ -n "$left" ]; then echo "still naming the old folder:"; echo "$left"; exit 1; fi
echo "ok: nothing names crates/pm-api/static"

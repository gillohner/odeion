#!/bin/bash
# The web UI's files moved from crates/pm-api/static to crates/pm-web/static (the pm-web crate).
# This rewrites every path to them in the tools, workflows and docs, and moves the folder if it is
# still at the old place. Safe to run again: on a branch made before the move, run it after
# merging or rebasing onto main and commit the result.
#   tools/refactor/pm-web-paths.sh
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
# git with each path taken as a name, not a pattern ("[ab].js" must not also match a.js)
lit() { GIT_LITERAL_PATHSPECS=1 git "$@"; }
# A link in either folder (e.g. the old folder kept as a link to the new one) would make a removal
# at the old place remove the file at the new place: the web UI has no links, so stop at one
links=$(find crates/pm-api/static crates/pm-web/static -type l 2>/dev/null || true)
if [ -n "$links" ]; then
  echo "links in the web UI's folders; replace each with the file or folder itself (or remove it), and run this again:"
  echo "$links" | sed 's/^/  /'
  exit 1
fi
# (also when the old folder was removed by hand but its files are still in the index)
if [ -d crates/pm-api/static ] || [ -n "$(git ls-files crates/pm-api/static)" ]; then
  mkdir -p crates/pm-web
  if [ -d crates/pm-web/static ] || [ ! -d crates/pm-api/static ]; then
    # Both exist (a merge brought files to the old place), or the old folder was removed by hand
    # but is still in the index: move over what the new place lacks, drop what is the same there,
    # and stop at a file that differs: writing one over the other would lose one side's changes
    # without a word
    differ=(); lost=()
    while IFS= read -r -d '' f; do
      to="crates/pm-web/static/${f#crates/pm-api/static/}"
      # (a file in an unresolved conflict is the person's to settle: it is listed below)
      if [ -n "$(lit ls-files -u -- "$f")" ]; then differ+=("$f")
      # (a file removed by hand but still in the index: dropped from the index when the new place
      # has the same file, otherwise listed, so that nothing is lost without a word)
      elif [ ! -e "$f" ] && [ ! -L "$f" ]; then
        if [ -e "$to" ] && [ "$(git rev-parse ":$f")" = "$(git hash-object -- "$to")" ]; then
          # (the copy at the new place is staged too: after a plain mv it is not tracked yet)
          lit add -- "$to"; lit rm -q --cached -- "$f"; echo "removed by hand, now out of the index too: $f"
        else lost+=("$f")
        fi
      elif [ ! -e "$to" ]; then mkdir -p "$(dirname "$to")"; lit mv -- "$f" "$to"
      elif cmp -s "$f" "$to"; then lit add -- "$to"; lit rm -q -f -- "$f"
      else differ+=("$f")
      fi
    done < <(git ls-files -z crates/pm-api/static | LC_ALL=C sort -zu)
    if [ ${#lost[@]} -gt 0 ]; then
      echo "these files were removed by hand, and crates/pm-web/static has no copy or a different one; bring each back from the index (GIT_LITERAL_PATHSPECS=1 git checkout -- <file>) or remove it from the index (GIT_LITERAL_PATHSPECS=1 git rm --cached -- <file>), and run this again:"
      for f in "${lost[@]}"; do echo "  $f"; done
    fi
    if [ ${#differ[@]} -gt 0 ]; then
      echo "these files are at both places with different content (or in an unresolved conflict); merge each into its copy under crates/pm-web/static and add that (GIT_LITERAL_PATHSPECS=1 git add -- <new file>), remove the old one (GIT_LITERAL_PATHSPECS=1 git rm -- <file>), and run this again:"
      for f in "${differ[@]}"; do echo "  $f"; done
    fi
    if [ ${#lost[@]} -gt 0 ] || [ ${#differ[@]} -gt 0 ]; then exit 1; fi
    # (files git does not track are not moved by the loop: say so, the build no longer reads them)
    if [ -d crates/pm-api/static ] && [ -n "$(find crates/pm-api/static ! -type d -print -quit)" ]; then
      echo "files git does not track are left in crates/pm-api/static; move or remove them, and run this again:"
      find crates/pm-api/static ! -type d | sed 's/^/  /'
      exit 1
    fi
    # (the folders the moves emptied)
    if [ -d crates/pm-api/static ]; then find crates/pm-api/static -depth -type d -empty -delete; fi
  else
    git mv crates/pm-api/static crates/pm-web/static
  fi
fi
# Everything that names the folder, except the changelog (it says where things were at the time)
# (git grep: 1 is "nothing found", anything else an error that must stop the run)
named() { git grep "$@" -E "pm-api/static|'pm-api', 'static'" -- . ':!docs/CHANGELOG.md' ':!tools/refactor/pm-web-paths.sh' || [ $? -eq 1 ]; }
named -z -l | while IFS= read -r -d '' f; do
  sed -i \
    -e "s#crates/pm-api/static#crates/pm-web/static#g" \
    -e "s#\.\./pm-api/static#../pm-web/static#g" \
    -e "s#'crates', 'pm-api', 'static'#'crates', 'pm-web', 'static'#g" \
    "$f"
  echo "updated $f"
done
# (a copy made at the new place by hand and never added would be left out of the commit)
untracked=$(git ls-files -o --exclude-standard -- crates/pm-web/static)
if [ -n "$untracked" ]; then
  echo "files in crates/pm-web/static git does not track yet; add them (GIT_LITERAL_PATHSPECS=1 git add -- <file>) or remove them, and run this again:"
  echo "$untracked" | sed 's/^/  /'
  exit 1
fi
left=$(named -n)
if [ -n "$left" ]; then echo "still naming the old folder:"; echo "$left"; exit 1; fi
echo "ok: nothing names crates/pm-api/static"

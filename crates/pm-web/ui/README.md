# The web UI's script, in parts

The node serves one `/app.js`. Its source is the files in `app/`, named `NN-name.js`: the build
(`../build.rs`) joins them end to end, in name order, and nothing else is added. The tools join
them the same way (`tools/ui-src.js`), so a fixture page runs the script the node would send.

The parts are pieces of one script, not modules: they share one closure (`00-core.js` opens it,
the last part closes it), and `22-` to `28-` are the pages inside `renderPage`'s `switch`. A part
on its own is not a complete script. Order matters where code runs as the page loads (event
listeners, the first render), so a statement moves between parts only with care.

- Change the UI: edit the part. Nothing to regenerate.
- A new part: give it a number between its neighbours; its first line should not start like any
  other line of the script (the split tool finds parts by their first line).
- Print the whole script: `node tools/ui-src.js > /tmp/app.js`.
- A branch that still has the single `app.js` (made before the split). The split tool cuts the
  branch's copy into the same parts, but only where the parts are still what that copy started
  from, so do it at the commit of the split (called `SPLIT` here) and let the rebase onto main
  merge the rest, part by part:
  1. `git rebase SPLIT^ <branch>`: the branch's `app.js` is now `SPLIT^`'s file plus the
     branch's changes.
  2. `git show SPLIT^:crates/pm-web/static/app.js > /tmp/base.js` and
     `git show <branch>:crates/pm-web/static/app.js > /tmp/mine.js`.
  3. `git switch -c <branch>-parts SPLIT`, then
     `node tools/refactor/web-ui-split.js /tmp/mine.js --base /tmp/base.js`, and bring over the
     branch's other changes as a patch, not as whole files (the split changed some of the same
     files: the changelog, a dozen tools, the workflow):
     `git diff --binary SPLIT^ <branch> -- . ':!crates/pm-web/static/app.js' | git apply --3way`
     (nothing to apply if the branch changed only `app.js`: skip it then). The patch is staged,
     the parts are not: `git add crates/pm-web/ui` and commit.
  4. `git rebase main`.
  The tool refuses to write unless the parts it finds join to `--base` byte for byte and the
  parts it writes join to the branch's file byte for byte. Run on a newer main without these
  steps it would put the branch's whole script over parts that others have changed since: that
  is what `--base` is there to stop.
- A `static/app.js` that is back beside the parts (a merge kept a branch's copy; the build and
  the tools refuse it): if it is the joined parts (`node tools/ui-src.js` says so only once the
  file is out of the way; `cmp` it with `cat app/[0-9][0-9]-*.js`), just `git rm` it. If not,
  it is a branch's script: carry it over with the steps above.

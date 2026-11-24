#!/bin/sh
# The node's shared library is a `pm_library::LibraryLock`, not a `std::sync::Mutex<Library>`.
# This rewrites every place that spells the old type (function parameters, struct fields) and the
# one place that makes it, so the change can be made again on a newer tree:
#   sh tools/refactor/library-lock.sh && cargo fmt --all && cargo clippy --workspace --all-targets
# A file left with an unused `Mutex` import afterwards is reported by clippy.
set -eu
cd "$(dirname "$0")/../.."
# (pm-library itself is left out: `LibraryLock` there is the one place that holds the mutex)
files=$(grep -rlE '(std::sync::)?Mutex<(pm_library::)?Library>|Mutex::new\(library\)' crates --include='*.rs' --exclude-dir=pm-library || true)
[ -n "$files" ] || { echo "nothing to rewrite"; exit 0; }
for f in $files; do
  sed -E -i \
    -e 's/(std::sync::)?Mutex<(pm_library::)?Library>/pm_library::LibraryLock/g' \
    -e 's/Arc::new\(Mutex::new\(library\)\)/Arc::new(pm_library::LibraryLock::new(library))/' \
    "$f"
  echo "rewrote $f"
done

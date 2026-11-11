#!/usr/bin/env bash
# Run before `apt-get update` (and Playwright's `install --with-deps`) on a GitHub runner.
#
# When the runner can't resolve the Ubuntu archive hosts (*.ubuntu.com), apt is pointed at
# another Ubuntu mirror that serves the same suites (release, -updates, -security, -backports).
# When the hosts resolve, nothing changes. Prints what it did. The mirror it picked goes to
# $GITHUB_ENV as APT_FALLBACK_MIRROR, for later steps that run apt in an Ubuntu container.
set -euo pipefail

[ -r /etc/os-release ] || { echo "apt-fallback: no /etc/os-release, skipped"; exit 0; }
# shellcheck disable=SC1091
. /etc/os-release
[ "${ID:-}" = ubuntu ] || { echo "apt-fallback: not Ubuntu, skipped"; exit 0; }
codename=${VERSION_CODENAME:?}

if [ "$(dpkg --print-architecture)" = amd64 ]; then
  hosts="azure.archive.ubuntu.com archive.ubuntu.com security.ubuntu.com"
  pattern='https?://([a-z0-9.-]+\.)?(archive|security)\.ubuntu\.com/ubuntu'
  mirrors="http://mirrors.edge.kernel.org/ubuntu http://mirror.leaseweb.net/ubuntu http://mirrors.ocf.berkeley.edu/ubuntu"
else
  hosts="ports.ubuntu.com"
  pattern='https?://ports\.ubuntu\.com/ubuntu-ports'
  mirrors="http://mirror.leaseweb.net/ubuntu-ports http://mirrors.ocf.berkeley.edu/ubuntu-ports http://ftp.tu-chemnitz.de/pub/linux/ubuntu-ports"
fi

down=""
for h in $hosts; do
  getent hosts "$h" >/dev/null || down="$down $h"
done
if [ -z "$down" ]; then
  echo "apt-fallback: Ubuntu archive resolves, apt sources unchanged"
  exit 0
fi
echo "apt-fallback: can't resolve:$down"

mirror=""
for m in $mirrors; do
  if curl -fsS -o /dev/null --max-time 15 "$m/dists/$codename-security/InRelease" &&
     curl -fsS -o /dev/null --max-time 15 "$m/dists/$codename-updates/InRelease"; then
    mirror=$m
    break
  fi
done
if [ -z "$mirror" ]; then
  echo "apt-fallback: no fallback mirror reachable, apt sources unchanged"
  exit 0
fi

# The runner image lists its mirrors in apt-mirrors.txt (used through mirror+file:); the Ubuntu
# sources name the archive hosts directly. Only Ubuntu archive URLs change; other repos stay.
for f in /etc/apt/sources.list /etc/apt/sources.list.d/*.sources /etc/apt/sources.list.d/*.list; do
  [ -f "$f" ] || continue
  sudo sed -E -i -e "s#mirror\+file:/etc/apt/apt-mirrors\.txt#$mirror#g" -e "s#$pattern#$mirror#g" "$f"
done
if [ -f /etc/apt/apt-mirrors.txt ]; then
  echo "$mirror" | sudo tee /etc/apt/apt-mirrors.txt >/dev/null
fi
echo "apt-fallback: apt now uses $mirror"
if [ -n "${GITHUB_ENV:-}" ]; then
  echo "APT_FALLBACK_MIRROR=$mirror" >>"$GITHUB_ENV"
fi

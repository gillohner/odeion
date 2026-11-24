#!/bin/sh
# Starts the bundle after every boot and keeps trying until the node runs.
#
# Docker restarts the containers itself at boot (restart: unless-stopped), but only once: when a
# container can't start then, for example because a network share it mounts as a volume does not
# answer yet, Docker never tries again, and the node stays down until someone starts it. This
# installs a systemd unit (odeion-bundle.service) that runs `docker compose up -d` in this folder
# after the network is up and tries again every 30 seconds until the node runs.
#
#   sudo ./start-at-boot.sh            # install and enable it (it runs from the next boot on)
#   sudo systemctl start odeion-bundle # run it now
#   sudo ./start-at-boot.sh --remove   # take it out again; the containers keep running
#   ./start-at-boot.sh --print         # show the unit it installs
#   ./start-at-boot.sh --run           # what the unit runs
set -eu
cd "$(dirname "$0")"
DIR=$(pwd -P)
UNIT=odeion-bundle.service
UNIT_FILE=/etc/systemd/system/$UNIT

unit() {
  cat <<EOF
# Written by $DIR/start-at-boot.sh. After a boot it starts the bundle in that folder and tries
# again every 30 s until the node runs (a network share that answers late, the homeserver not yet
# healthy). Stopping or removing it leaves the containers as they are.
[Unit]
Description=Odeion bundle in $DIR: start after boot, retry until the node runs
Documentation=https://github.com/gillohner/odeion/blob/main/docs/deploy.md
Wants=network-online.target docker.service
After=network-online.target docker.service
StartLimitIntervalSec=0

[Service]
Type=oneshot
RemainAfterExit=yes
WorkingDirectory=$DIR
ExecStart=/bin/sh ./start-at-boot.sh --run
Restart=on-failure
RestartSec=30
TimeoutStartSec=15min

[Install]
WantedBy=multi-user.target
EOF
}

need_root() {
  [ "$(id -u)" = 0 ] || { echo "Run it as root: sudo $0${*:+ $*}" >&2; exit 1; }
  [ -d /run/systemd/system ] || { echo "This machine doesn't run systemd; Docker's own restart is all there is." >&2; exit 1; }
}

case ${1:-} in
--print)
  unit
  ;;
--run)
  # `up -d` fails when a container can't start (a share volume that doesn't mount yet); the check
  # after it also catches a node that stopped again at once.
  docker compose up -d
  if [ -z "$(docker compose ps -q --status running music)" ]; then
    echo "The music container isn't running yet; trying again." >&2
    exit 1
  fi
  ;;
--remove)
  need_root --remove
  systemctl disable --now "$UNIT" 2>/dev/null || true
  rm -f "$UNIT_FILE"
  systemctl daemon-reload
  echo "Removed $UNIT. The containers keep running; Docker still restarts them at boot."
  ;;
'')
  need_root
  # A % would be read by systemd as a placeholder, a line break would end the line
  case $DIR in *%* | *'
'*) echo "The folder's path ($DIR) has a % or a line break; move the bundle to a plain path." >&2; exit 1 ;; esac
  [ -f docker-compose.yml ] || { echo "No docker-compose.yml in $DIR." >&2; exit 1; }
  # Type=oneshot with Restart= needs systemd 244 (Debian 11, Ubuntu 20.04)
  v=$(systemctl --version | sed -n '1s/^systemd \([0-9]*\).*/\1/p')
  [ "${v:-0}" -ge 244 ] || { echo "This needs systemd 244 or newer (this machine has ${v:-an unknown version})." >&2; exit 1; }
  tmp=$(mktemp "$UNIT_FILE.XXXXXX")
  unit > "$tmp"
  chmod 644 "$tmp"
  mv "$tmp" "$UNIT_FILE"
  systemctl daemon-reload
  systemctl enable "$UNIT"
  echo "Installed $UNIT: after every boot it starts the bundle in $DIR and retries until the node runs."
  echo "Run it now with: sudo systemctl start odeion-bundle. See it with: systemctl status odeion-bundle"
  ;;
*)
  echo "Usage: sudo $0 [--remove] | $0 --print | $0 --run" >&2
  exit 2
  ;;
esac

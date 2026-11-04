# shellcheck shell=bash
# Sourced by backup.sh and restore.sh: the bundle's settings, read as compose and the node read them
# (docs/branding-compatibility.md). Every setting is ODEION_<NAME>; the pre-Odeion PM_<NAME> still
# works and loses to the ODEION_ one.

# setting <NAME>: ODEION_<NAME>, else PM_<NAME>; each from the shell's environment, else .env (as
# compose reads .env). Only for the token and the backup passphrase, where empty is never valid, so
# an empty value counts as unset.
setting() {
  local k v
  for k in "ODEION_$1" "PM_$1"; do
    v=${!k:-}
    [ -n "$v" ] || v=$(sed -n "s/^$k=//p" .env 2>/dev/null | tail -n 1)
    if [ -n "$v" ]; then
      printf '%s' "$v"
      return 0
    fi
  done
}

# node_port <container>: the API port from the container's ODEION_LISTEN, else PM_LISTEN (an empty
# ODEION_LISTEN wins, as in the entrypoint, and means the default), else 7788.
node_port() {
  local env listen
  env=$(docker inspect -f '{{range .Config.Env}}{{println .}}{{end}}' "$1")
  if printf '%s\n' "$env" | grep -q '^ODEION_LISTEN='; then
    listen=$(printf '%s\n' "$env" | sed -n 's/^ODEION_LISTEN=//p' | tail -n 1)
  else
    listen=$(printf '%s\n' "$env" | sed -n 's/^PM_LISTEN=//p' | tail -n 1)
  fi
  listen=${listen##*:}
  case $listen in '' | *[!0-9]*) listen=7788 ;; esac
  printf '%s' "$listen"
}

# Run in the node's container with `docker exec`, which sees the container's environment, not the
# one the entrypoint made for the node: so it picks the config file the same way the entrypoint
# does (ODEION_ first, winning even when empty). pm-node is in every image, before and since Odeion.
# shellcheck disable=SC2016
NODE_BACKUP_SH='
if [ "${ODEION_DATA_DIR+set}" = set ]; then d=$ODEION_DATA_DIR; else d=${PM_DATA_DIR-}; fi
if [ "${ODEION_CONFIG+set}" = set ]; then c=$ODEION_CONFIG; else c=${PM_CONFIG-}; fi
c=${c:-${d:-/data}/node.toml}
pm-node -c "$c" backup --force /tmp/pm-backup-library.db >/dev/null'

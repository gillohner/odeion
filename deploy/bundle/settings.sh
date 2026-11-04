# shellcheck shell=sh
# Sourced by backup.sh, restore.sh and setup.sh: the bundle's settings, read as compose and the node
# read them (docs/branding-compatibility.md). Every setting is ODEION_<NAME>; the pre-Odeion
# PM_<NAME> still works and loses to the ODEION_ one. POSIX sh, so setup.sh (sh) can use it too.

# dotenv_get <KEY> [file]: KEY's value in .env (or file) as compose's .env parser reads it; status 1
# when no line sets KEY. The last line wins; `export KEY=` and `KEY: value` count; around the value,
# spaces go; "…" and '…' give what is between the quotes (anything after the closing quote is
# dropped); unquoted, a space then # starts a comment (not a tab then #; a value that starts with #
# is a value). Not handled (a documented follow-up; the values setup.sh writes are hex): backslash
# escapes in "…", $VAR / ${VAR} expansion, values over several lines.
dotenv_get() {
  local line v
  line=$(grep -E "^[[:space:]]*(export[[:space:]]+)?$1[[:space:]]*[=:]" "${2:-.env}" 2>/dev/null | tail -n 1) || true
  [ -n "$line" ] || return 1
  v=${line#*"$1"}
  v=${v#"${v%%[![:space:]]*}"}
  v=${v#?}
  v=${v#"${v%%[![:space:]]*}"}
  case $v in
    \"*) v=${v#\"}; v=${v%%\"*} ;;
    \'*) v=${v#\'}; v=${v%%\'*} ;;
    *)
      v=${v%%" #"*}
      v=${v%"${v##*[![:space:]]}"}
      ;;
  esac
  printf '%s' "$v"
}

# setting <NAME>: ODEION_<NAME>, else PM_<NAME>, exactly as compose resolves them: each name from the
# shell's environment when the shell has it (even empty: then .env is not read for that name), else
# from .env (dotenv_get). Only for the token and the backup passphrase, where empty is never valid, so
# an empty value goes on to the next name (compose's `${ODEION_TOKEN:-${PM_TOKEN…}}`).
setting() {
  local k v isset
  for k in "ODEION_$1" "PM_$1"; do
    eval "isset=\${$k+set}"
    if [ "$isset" = set ]; then
      eval "v=\${$k}"
    else
      v=$(dotenv_get "$k") || v=
    fi
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

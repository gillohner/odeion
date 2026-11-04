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

# setting <NAME> [env file]: ODEION_<NAME>, else PM_<NAME>, exactly as compose resolves them: each name
# from the shell's environment when the shell has it (even empty: then the file is not read for that
# name), else from .env or the file given (dotenv_get). Only for the token and the backup passphrase, where empty is never valid, so
# an empty value goes on to the next name (compose's `${ODEION_TOKEN:-${PM_TOKEN…}}`).
setting() {
  local k v isset
  for k in "ODEION_$1" "PM_$1"; do
    eval "isset=\${$k+set}"
    if [ "$isset" = set ]; then
      eval "v=\${$k}"
    else
      v=$(dotenv_get "$k" "${2:-.env}") || v=
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

# node_token_from <config file> <data dir>: the API token a node with these uses, as the node picks
# it (crates/pm-node/src/api_token.rs): its config's [api] token, trimmed, unless empty or a
# placeholder; else the one it made, <data dir>/api-token. node_token_pick <api-token text> does the
# same with the config's text on stdin (so a backup's need not be written to disk). Status 2 when the
# [api] token line is more than this reads: TOML's '…' and "…" with \" and \\ and a comment after it
# are read; """…""", other escapes or a bare value are not, and never quietly fall back to another
# token. Nothing printed but the token itself. Kept as text too, so the same functions run inside the
# node's container (NODE_TOKEN_SH).
NODE_TOKEN_FN=$(cat <<'EOF_FN'
toml_api_token() {
  local line v got
  line=$(sed -n '/^[[:space:]]*\[api\][[:space:]]*\(#.*\)\{0,1\}$/,/^[[:space:]]*\[/{/^[[:space:]]*token[[:space:]]*=/p;}' | head -n 1)
  [ -n "$line" ] || return 0
  v=${line#*=}
  v=${v#"${v%%[![:space:]]*}"}
  case $v in
    '"""'* | "'''"*) return 2 ;;
    \"*) got=$(printf '%s\n' "$v" | sed -nE 's/^"(([^"\\]|\\["\\])*)"[[:space:]]*(#.*)?$/ok:\1/p')
      [ -n "$got" ] || return 2
      got=$(printf '%s\n' "${got#ok:}" | sed 's/\\\(["\\]\)/\1/g') ;;
    \'*) got=$(printf '%s\n' "$v" | sed -nE "s/^'([^']*)'[[:space:]]*(#.*)?\$/ok:\1/p")
      [ -n "$got" ] || return 2
      got=${got#ok:} ;;
    *) return 2 ;;
  esac
  printf '%s' "$got"
}
node_token_pick() {
  local t rc=0
  t=$(toml_api_token) || rc=$?
  [ "$rc" = 0 ] || return "$rc"
  t=${t#"${t%%[![:space:]]*}"}
  t=${t%"${t##*[![:space:]]}"}
  case $(printf '%s' "$t" | tr 'A-Z' 'a-z') in '' | change-me | changeme | change-me-to-a-long-random-secret) t= ;; esac
  if [ -z "$t" ]; then
    t=$1
    t=${t#"${t%%[![:space:]]*}"}
    t=${t%"${t##*[![:space:]]}"}
  fi
  printf '%s' "$t"
}
node_token_from() {
  local own=
  [ ! -f "$2/api-token" ] || own=$(cat "$2/api-token")
  if [ -f "$1" ]; then node_token_pick "$own" < "$1"; else node_token_pick "$own" < /dev/null; fi
}
EOF_FN
)
eval "$NODE_TOKEN_FN"

# Run in the node's container with `docker exec`: the token the node uses, with the config and the
# data folder picked as the entrypoint picks them. Never shown: the caller keeps it.
# shellcheck disable=SC2016
NODE_TOKEN_SH="$NODE_TOKEN_FN"'
if [ "${ODEION_DATA_DIR+set}" = set ]; then d=$ODEION_DATA_DIR; else d=${PM_DATA_DIR-}; fi
if [ "${ODEION_CONFIG+set}" = set ]; then c=$ODEION_CONFIG; else c=${PM_CONFIG-}; fi
d=${d:-/data}
node_token_from "${c:-$d/node.toml}" "$d"'

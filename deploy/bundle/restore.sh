#!/usr/bin/env bash
# Puts a backup made by backup.sh back into this bundle folder, on a new disk or machine
# (docs/guides/backup-and-update.md, "Restore on a new disk or machine"):
#
#   ./restore.sh ~/backups/pubky-music/2026-10-29-030000
#
# It never overwrites a node: it stops when this bundle already has containers or data volumes.
# The music itself must be where it was (or mounted again); the network shares the node mounted
# as Docker volumes are created again from the backup.
set -euo pipefail
umask 077
cd "${BUNDLE_DIR:-$(dirname "$0")}"

die() { echo "restore: $*" >&2; exit 1; }
# setting, node_port: ODEION_ settings first, then PM_ ones
. ./settings.sh
[ $# -eq 1 ] || die "usage: ./restore.sh <backup folder>"
B=$(cd "$1" 2>/dev/null && pwd) || die "no folder $1"
[ -f "$B/SHA256SUMS" ] || die "$B is not a complete backup (no SHA256SUMS)"
(cd "$B" && sha256sum --quiet -c SHA256SUMS) || die "files in $B do not match their checksums"
# curl for the node's API (pinned), and the small image the compose file already uses
CURL=curlimages/curl:8.17.0@sha256:935d9100e9ba842cdb060de42472c7ca90cfe9a7c96e4dacb55e79e560b3ff40
ALPINE=$(sed -n 's/^ *image: *\(alpine:[^ ]*\).*/\1/p' docker-compose.yml | head -n 1)

# --- before anything is changed: the token and the passphrases ------------------------------------
# The backup's .env is the one this install will use (copied below); a folder's own .env otherwise
ENVSRC="$B/config/.env"
[ -f "$ENVSRC" ] || ENVSRC=.env
# The API token the restored node will use, as it will pick it (node_token_pick, read from the backup
# in memory, never written to disk or shown): the backup's node.toml [api] token, else the token that
# node made (api-token). Without a node.toml in the backup, the entrypoint writes one from compose's
# token: the shell's ODEION_TOKEN (or PM_TOKEN), else the backup's .env. When the backup's node has
# no token at all, the restored one makes a new one at start, read in its container once it runs.
shell_token() {
  local k v
  for k in ODEION_TOKEN PM_TOKEN; do
    eval "v=\${$k-}"
    if [ -n "$v" ]; then printf '%s' "$v"; return 0; fi
  done
}
TOKEN=
cfg=
own=
has_cfg=
if [ -f "$B/node/data.tar.gz" ]; then
  if cfg=$(tar -xzOf "$B/node/data.tar.gz" ./node.toml 2>/dev/null); then has_cfg=1; else cfg=; fi
  own=$(tar -xzOf "$B/node/data.tar.gz" ./api-token 2>/dev/null) || own=
fi
rc=0
TOKEN=$(printf '%s\n' "$cfg" | node_token_pick "$own") || rc=$?
cfg=
own=
if [ "$rc" = 2 ]; then
  TOKEN=$(shell_token)
  [ -n "$TOKEN" ] || die "cannot read the API token in the backup's node.toml: this reads only a plain [api] table with a plain token = \"…\" or '…' line (no quoted or escaped keys or headers, no api.… or api = { … }); pass the token: ODEION_TOKEN=… ./restore.sh $1 (nothing was changed)"
elif [ -z "$has_cfg" ]; then
  TOKEN=$(setting TOKEN "$ENVSRC")
elif [ -z "$TOKEN" ]; then
  TOKEN=$(shell_token)
fi
# The sign-ins' passphrase, the ways a backup may have been locked, tried in this order (only on the
# node's "That passphrase does not open this backup" the next): as compose reads .env; as written in
# .env, which backup.sh read up to v0.2.2 (quotes and comment included); the one backup.sh made when
# it found none (node/passphrase). None at all: the backup goes back without the sign-ins.
PASSES=()
for p in "$(setting BACKUP_PASSPHRASE "$ENVSRC")" \
  "${PM_BACKUP_PASSPHRASE:-$(sed -n 's/^PM_BACKUP_PASSPHRASE=//p' "$ENVSRC" 2>/dev/null | tail -n 1)}" \
  "$([ ! -f "$B/node/passphrase" ] || cat "$B/node/passphrase")"; do
  [ -n "$p" ] || continue
  seen=
  for c in "${PASSES[@]}"; do [ "$c" != "$p" ] || seen=1; done
  [ -n "$seen" ] || PASSES+=("$p")
done

# --- config -----------------------------------------------------------------------------------
for f in .env docker-compose.override.yml; do
  [ -f "$B/config/$f" ] || continue
  if [ -f "$f" ] && ! cmp -s "$f" "$B/config/$f"; then
    die "$f is already here and differs from the backup's: move it away first"
  fi
  cp "$B/config/$f" "$f"
done
chmod 600 .env
PROJECT=$(docker compose config 2>/dev/null | sed -n 's/^name: *//p' | head -n 1)
[ -n "$PROJECT" ] || die "docker compose cannot read this folder's compose files"
[ -z "$(docker compose ps -aq)" ] || die "the bundle $PROJECT already has containers: this restores into a fresh install only"
existing=$(docker volume ls -q --filter "label=com.docker.compose.project=$PROJECT")
[ -z "$existing" ] || die "the bundle $PROJECT already has data volumes ($(echo $existing)): this restores into a fresh install only"

# Network shares: the volumes, and their mount points in the music folder
sh "$B/config/recreate-volumes.sh" > /dev/null
music_dir=$(sed -n 's/^MUSIC_DIR=//p' .env | tail -n 1)
music_dir=${music_dir:-./music}
while IFS='	' read -r type _src dst _mode; do
  case $type:$dst in
    volume:/music/*)
      d="$music_dir/${dst#/music/}"
      [ -d "$d" ] || mkdir -p "$d" || echo "restore: create $d (the mount point of a share) and run docker compose up -d again" >&2 ;;
    bind:/music)
      [ -d "$music_dir" ] || echo "restore: the music folder $music_dir is missing: put the music back or set MUSIC_DIR in .env" >&2 ;;
  esac
done < "$B/config/music-mounts.tsv"

# --- data -------------------------------------------------------------------------------------
echo "Restoring $B into the bundle $PROJECT"
docker compose up --no-start --quiet-pull
vol_of() {
  docker inspect -f '{{range .Mounts}}{{if eq .Destination "'"$2"'"}}{{if .Name}}{{.Name}}{{else}}{{.Source}}{{end}}{{end}}{{end}}' "$1"
}
PG=$(docker compose ps -aq postgres)
HS=$(docker compose ps -aq homeserver)
NODE=$(docker compose ps -aq music)
NODE_DATA=$(vol_of "$NODE" /data)
TUNNEL=$(vol_of "$NODE" /tunnel)
HS_DATA=$(vol_of "$HS" /data)
PG_DATA=$(vol_of "$PG" /var/lib/postgresql/data)
# A host folder (HOMESERVER_DATA, POSTGRES_DATA) must be empty too
for v in "$HS_DATA" "$NODE_DATA" "$TUNNEL" "$PG_DATA"; do
  [ -z "$v" ] && continue
  docker run --rm -v "$v:/v:ro" "$ALPINE" sh -c '[ -z "$(ls -A /v | grep -vx lost+found)" ]' ||
    die "$v already holds data: this restores into a fresh install only (docker compose down to remove the new containers)"
done
# From here on a failure leaves a half-restored install: say how to start over
trap 'echo "restore: stopped half-way. To start over: docker compose down -v (this install only), empty HOMESERVER_DATA and POSTGRES_DATA if .env sets them, then ./restore.sh again" >&2' ERR
untar() { docker run --rm -i -v "$1:/v" "$ALPINE" tar xzf - -C /v < "$2"; }
untar "$HS_DATA" "$B/homeserver/data.tar.gz"
untar "$NODE_DATA" "$B/node/data.tar.gz"
docker run --rm -i -v "$NODE_DATA:/v" "$ALPINE" sh -c 'cat > /v/library.db && chown "$(stat -c %u:%g /v)" /v/library.db' \
  < "$B/node/library.db"
[ -z "$TUNNEL" ] || [ ! -f "$B/node/tunnel.tar.gz" ] || untar "$TUNNEL" "$B/node/tunnel.tar.gz"

# The homeserver's database, once PostgreSQL has made its empty one
docker compose start postgres > /dev/null
# (the image's set-up server answers on the socket only; the real one also on TCP)
ready=
for _ in $(seq 120); do
  if docker logs "$PG" 2>&1 | grep -q 'init process complete' &&
    docker exec "$PG" sh -c 'pg_isready -q -h 127.0.0.1 -U "$POSTGRES_USER" -d "$POSTGRES_DB"'; then
    ready=1
    break
  fi
  sleep 2
done
[ -n "$ready" ] || die "PostgreSQL did not start"
docker exec -i "$PG" sh -c 'pg_restore -U "$POSTGRES_USER" -d "$POSTGRES_DB" --no-owner --single-transaction --exit-on-error' \
  < "$B/homeserver/database.dump"

docker compose up -d
echo "The bundle is up. Putting the sign-ins back once the node's first scan is done..."

# --- the sign-ins, through the node's own restore (Settings › Backup) ---------------------------
PASS=${PASSES[0]:-}
PORT=$(node_port "$NODE")
node_api() {
  local mount=() body=()
  if [ -n "${3:-}" ]; then mount=(-v "$3:/in:ro"); body=(--data-binary @/in); fi
  printf '%s\n%s\n' "$TOKEN" "$PASS" |
    docker run --rm -i --user "$(id -u):$(id -g)" --network "container:$NODE" "${mount[@]}" "$CURL" sh -c '
      read -r T; read -r P
      printf "Authorization: Bearer %s\nx-backup-passphrase: %s\n" "$T" "$P" | curl -sS --fail-with-body -H @- "$@"' sh -X "$1" "${body[@]}" "http://127.0.0.1:$PORT$2"
}
# --- sign-ins: begin
tried=0
if [ -n "$PASS" ]; then q=''; else q='?skip_secrets=true'; fi
# The node scans its folders when it starts and refuses a restore meanwhile (409): wait for two
# idle answers in a row (the scan may not have begun at the first), and try again after a 409
report=""
idle=0
refused=0
for i in $(seq 720); do
  info=$(node_api GET /v1/info 2>&1) || true
  case $info in
    *"returned error: 401"*)
      # Not the token the restored node uses: read its own in its container (never shown) and use
      # that; when that is no different, stop at once rather than wait out the hour
      t=$(docker exec "$NODE" sh -c "$NODE_TOKEN_SH" 2>/dev/null) || t=
      refused=$(( refused + 1 ))
      if [ -n "$t" ] && [ "$t" != "$TOKEN" ]; then
        TOKEN=$t
        continue
      fi
      [ "$refused" -lt 3 ] || die "the node refuses the API token (401). Its token: docker compose exec -T music cat /data/api-token (or [api] token in /data/node.toml); with it, restore $B/node/library.pmbackup in Settings › Backup"
      sleep 5
      continue
      ;;
  esac
  case $info in *'"scanning":false'*) idle=$(( idle + 1 )) ;; *) idle=0 ;; esac
  if [ "$idle" -ge 2 ]; then
    if report=$(node_api POST "/v1/restore$q" "$B/node/library.pmbackup" 2>&1); then break; fi
    case $report in
      *scan*) idle=0 ;;
      *"That passphrase does not open this backup"*)
        tried=$(( tried + 1 ))
        if [ "$tried" -lt "${#PASSES[@]}" ]; then
          echo "  that passphrase does not open this backup; trying the next way it may have been locked"
          PASS=${PASSES[$tried]}
          idle=0
        else
          die "the node did not take $B/node/library.pmbackup ($report): restore it in Settings › Backup"
        fi
        ;;
      *) die "the node did not take $B/node/library.pmbackup ($report): restore it in Settings › Backup" ;;
    esac
    report=""
  fi
  [ $(( i % 30 )) -ne 0 ] || echo "  still waiting for the node (a first scan of a big library takes a while)"
  sleep 5
done
[ -n "$report" ] || die "the node did not finish its first scan: restore $B/node/library.pmbackup in Settings › Backup later"
# --- sign-ins: end
echo "Restored. The node's report (sign-ins put back, folders it cannot find, notes):"
printf '%s\n' "$report" | sed 's/,"/,\n  "/g'

#!/bin/sh
# Writes /data/node.toml from the environment unless one is mounted, then runs odeiond, the Odeion node.
# Every setting is ODEION_<NAME>; its pre-Odeion name PM_<NAME> still works this release, and the
# ODEION_ one wins, even when empty (docs/branding-compatibility.md). Below, the old names.
#   PM_LISTEN    host:port for the API (default 0.0.0.0:7788 in the image)
#   PM_TOKEN     bearer token clients must send. Empty while listening beyond localhost: the node
#                makes one on first start and keeps it in $PM_DATA_DIR/api-token (0600)
#   PM_DATA_DIR  where the SQLite library lives (default /data)
#   PM_FOLDERS   colon-separated music folders to scan (default /music)
#   PM_CONTACT   email or URL for outbound User-Agent strings (optional here; it can also be
#                set later in Settings, which turns Discover on without a restart)
#   PM_NAME      name shown to controllers (default Odeion node)
#   PM_PUBLIC_URL  how devices reach this node (http://<host-ip>:7788); renderers and endpoints
#                fetch audio from it. Empty: this machine's address on its way to the network, so
#                set it when the container is not on the host network or that is the wrong one
#   PM_TIDAL_CLIENT_ID / PM_TIDAL_CLIENT_SECRET   your own TIDAL app instead of the built-in one; optional
#                (docs/STREAMING-PROVIDERS.md, "Advanced: use your own app")
#   PM_TIDAL_SCOPES   scopes to request at sign-in (default: all the node can use; must be enabled on the app)
#   PM_SPOTIFY_CLIENT_ID / PM_SPOTIFY_CLIENT_SECRET   your own Spotify app; optional (Sources › Spotify
#                sets one too)
#   PM_ACOUSTID_KEY   your own AcoustID key instead of the built-in one; optional
set -eu
# ODEION_<NAME> onto PM_<NAME>, which the rest of this script and the node read. Only names made of
# A-Z, 0-9 and _ are ours; values are copied, never run.
for odeion_name in $(env | sed -n 's/^ODEION_\([A-Z0-9_][A-Z0-9_]*\)=.*/\1/p'); do
  eval "odeion_set=\${ODEION_${odeion_name}+set}"
  [ "$odeion_set" = set ] || continue
  eval "export PM_${odeion_name}=\"\${ODEION_${odeion_name}}\""
done
# Then every PM_<NAME> under its ODEION_ name too, the same value: the image's own defaults and the
# compose files still set PM_ names, which are not the user's to change, so the node has no old name
# to report here.
for odeion_name in $(env | sed -n 's/^PM_\([A-Z0-9_][A-Z0-9_]*\)=.*/\1/p'); do
  eval "odeion_set=\${PM_${odeion_name}+set}"
  [ "$odeion_set" = set ] || continue
  eval "export ODEION_${odeion_name}=\"\${PM_${odeion_name}}\""
done
unset odeion_name odeion_set
DATA_DIR="${PM_DATA_DIR:-/data}"
CONF="${PM_CONFIG:-$DATA_DIR/node.toml}"
if [ ! -f "$CONF" ] || [ "${PM_REWRITE_CONFIG:-0}" = "1" ]; then
  mkdir -p "$DATA_DIR"
  folders=""
  old_ifs="$IFS"; IFS=:
  for f in ${PM_FOLDERS:-/music}; do
    [ -n "$f" ] && folders="$folders\"$f\", "
  done
  IFS="$old_ifs"
  cat > "$CONF" <<TOML
data_dir = "$DATA_DIR"
contact = "${PM_CONTACT:-}"

[library]
folders = [${folders%, }]

[api]
listen = "${PM_LISTEN:-0.0.0.0:7788}"
token = "${PM_TOKEN:-}"
name = "${PM_NAME:-Odeion node}"
public_url = "${PM_PUBLIC_URL:-}"

[tidal]
client_id = "${PM_TIDAL_CLIENT_ID:-}"
client_secret = "${PM_TIDAL_CLIENT_SECRET:-}"
scopes = "${PM_TIDAL_SCOPES:-}"

[spotify]
client_id = "${PM_SPOTIFY_CLIENT_ID:-}"
client_secret = "${PM_SPOTIFY_CLIENT_SECRET:-}"

[acoustid]
api_key = "${PM_ACOUSTID_KEY:-}"
TOML
fi
exec odeiond -c "$CONF" "$@"

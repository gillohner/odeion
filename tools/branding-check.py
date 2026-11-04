#!/usr/bin/env python3
"""Odeion branding and compatibility check (docs/branding-compatibility.md).

Three parts, all against the files in the repository:

1. Display: the maintained surfaces people see say Odeion (web page and PWA, the app's window,
   dialogs and Android notification, the node's --help, the service unit, the image labels, the
   Home Assistant and Roon display names, README and the guides).
2. Compatibility: the identities that installs, accounts and data depend on are unchanged (Pubky
   paths, the mDNS service type, the HA domain, the Roon extension id, the app identifier and
   updater key, the image and repository names, the data and config paths, the old unit file
   and the old pm-node executable, PM_ names).
3. Legacy names: "Pubky Music" is left only in history (CHANGELOG, docs/archive, the docs/ux
   mockups), the specs crate, this checker, docs/branding-compatibility.md and lines that say
   why they keep it ("compat: pre-Odeion").

    python3 -I tools/branding-check.py [repo root]

Prints one line per check and exits 1 when any fails.
"""

import json
import re
import subprocess
import sys
from pathlib import Path

SELF_TEST = "--self-test" in sys.argv[1:]
args = [a for a in sys.argv[1:] if a != "--self-test"]
ROOT = Path(args[0] if args else Path(__file__).resolve().parent.parent)
problems = []


def check(ok, what):
    print(f"{'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        problems.append(what)


def channel_reads_new_first(src):
    """build_env(option_env!("ODEION_UPDATE_CHANNEL"), option_env!("PM_UPDATE_CHANNEL")), however laid out."""
    return re.search(r'build_env\(\s*option_env!\("ODEION_UPDATE_CHANNEL"\)\s*,\s*option_env!\("PM_UPDATE_CHANNEL"\)\s*,?\s*\)', src) is not None


MARK = "compat: pre-Odeion"


def legacy_left(rel, body):
    """Lines of body with the old product name that say neither why (MARK) nor are allowed. A mark
    counts on the same line, or on one of the next 3 lines when the statement is not finished yet
    (rustfmt moves a trailing comment to the line that closes a split statement)."""
    lines = body.splitlines()
    out = []
    for i, line in enumerate(lines):
        if "Pubky Music" not in line:  # the product name (not "the Pubky music index")
            continue
        if MARK in line or any(a in line for a in ALLOWED_LINES.get(rel, ())):
            continue
        ok = False
        if not line.rstrip().endswith((";", "}", "{")):
            for later in lines[i + 1:i + 4]:
                if MARK in later:
                    ok = True
                    break
                if later.rstrip().endswith((";", "{")) or "Pubky Music" in later:
                    break
        if not ok:
            out.append(f"{rel}:{i + 1}: {line.strip()[:120]}")
    return out


def text(rel):
    p = ROOT / rel
    return p.read_text(encoding="utf-8") if p.is_file() else ""


def has(rel, needle, what=None):
    check(needle in text(rel), what or f"{rel} has {needle!r}")


def lacks(rel, needle, what=None):
    check(needle not in text(rel), what or f"{rel} no longer has {needle!r}")


def json_of(rel):
    try:
        return json.loads(text(rel))
    except ValueError:
        return {}


# ---------------------------------------------------------------- 1. display: Odeion
web = "crates/pm-api/static/index.html"
has(web, "<title>Odeion</title>")
has(web, '<meta name="apple-mobile-web-app-title" content="Odeion">')
has(web, '<span class="brand-name">Odeion</span>')
manifest = json_of("crates/pm-api/static/manifest.webmanifest")
check(manifest.get("name") == "Odeion", "PWA manifest name is Odeion")
check(manifest.get("short_name") == "Odeion", "PWA manifest short_name is Odeion")
check("Pubky Music" not in json.dumps(manifest), "PWA manifest has no old name")
app_js = text("crates/pm-api/static/app.js")
check("‹ Odeion</a>" in app_js, "the dashboard's way home says Odeion")
check(app_js.startswith("/* Odeion web UI."), "app.js names Odeion")

node_main = "crates/pm-node/src/main.rs"
has(node_main, 'name = "odeiond"', "the daemon's command is odeiond")
has(node_main, 'about = "Odeion node', "the daemon's --help says Odeion node")
has("crates/pm-node/src/config.rs", 'name: "Odeion node".into()', "a new node is called Odeion node")
node_toml = text("crates/pm-node/Cargo.toml")
check('name = "odeiond"' in node_toml, "pm-node builds the odeiond executable")
has("crates/pm-endpoint/src/main.rs", 'about = "Odeion endpoint', "the endpoint's --help says Odeion")

has("crates/pm-desktop/src/lib.rs", '.title("Odeion")', "the app's window is titled Odeion")
desk = text("crates/pm-desktop/src/lib.rs")
run_body = desk[desk.find("pub fn run()"):]
check("apply_env_compat()" in run_body and run_body.find("apply_env_compat()") < run_body.find("std::env::var"),
      "the app takes ODEION_ settings before it reads any setting")
for f in ("crates/pm-node/src/main.rs", "crates/pm-endpoint/src/main.rs"):
    body = text(f)
    check("apply_env_compat()" in body, f"{f} takes ODEION_ settings at start")
check(channel_reads_new_first(text("crates/pm-desktop/src/update.rs")),
      "the app's update channel takes ODEION_UPDATE_CHANNEL first")
check("option_env!(\"ODEION_NODE_VERSION\")" in text("crates/pm-node/src/main.rs"), "the node's version takes ODEION_NODE_VERSION first")
pa = text("crates/pm-api/src/project_apps.rs")
for k in ("TIDAL_CLIENT_ID", "JAMENDO_CLIENT_ID", "ACOUSTID_KEY", "FANART_KEY", "AUDIODB_KEY", "GOOGLE_CLIENT_ID", "ONEDRIVE_CLIENT_ID"):
    check(f'option_env!("ODEION_PROJECT_{k}")' in pa and f'option_env!("PM_PROJECT_{k}")' in pa,
          f"the build value PROJECT_{k} takes ODEION_ first and PM_ still")
    check(f'"ODEION_PROJECT_{k}"' in text("crates/pm-api/build.rs"), f"a change of ODEION_PROJECT_{k} rebuilds")
has("crates/pm-desktop/src/update.rs", 'const TITLE: &str = "Update Odeion";', "the update dialog says Odeion")
has("crates/pm-desktop/src/update.rs", '"Odeion {new} is available', "the update question says Odeion")
has("crates/pm-desktop/src/crash.rs", "Odeion {} ({} {})", "the crash report says Odeion")
has("crates/pm-desktop/ui/index.html", "<title>Odeion</title>", "the app's connect page title")
has("crates/pm-desktop/ui/index.html", "<h1>Odeion</h1>", "the app's connect page heading")
tauri = json_of("crates/pm-desktop/tauri.conf.json")
bundle = tauri.get("bundle", {})
check("Odeion" in bundle.get("shortDescription", ""), "the app package's short description says Odeion")
check("Odeion" in bundle.get("longDescription", ""), "the app package's long description says Odeion")
has("tools/release/android/PlayerService.kt", '.setContentTitle("Odeion")', "the Android player notification says Odeion")
has("tools/release/android/CrashReport.kt", '"Odeion $version on', "the Android crash report says Odeion")
has("tools/release/android/CrashReport.kt", '"Odeion crash report"', "the Android crash mail subject says Odeion")
has("tools/release/android-patch.py", "Odeion", "the Android patch sets the launcher label")

unit = "deploy/bare-metal/odeiond.service"
u = text(unit)
check(bool(u), f"{unit} exists")
check("Description=Odeion node" in u, "odeiond.service describes the Odeion node")
check("ExecStart=/usr/local/bin/odeiond -c /etc/pubky-music/node.toml serve" in u,
      "odeiond.service runs odeiond on the same config file")
check("Conflicts=pubky-music-node.service" in u, "odeiond.service never runs next to the old unit")
check(not any(line.startswith("Alias=") for line in u.splitlines()), "odeiond.service has no Alias= (an alias cannot replace an installed old unit)")
docker = text("Dockerfile")
check('org.opencontainers.image.title="Odeion node"' in docker, "the image's title label says Odeion node")
check("/usr/local/bin/odeiond" in docker, "the image ships odeiond")
check(re.search(r"ln -s\S* +odeiond +/usr/local/bin/pm-node", docker) is not None,
      "the image keeps pm-node as a link to odeiond")
ep = text("deploy/docker/entrypoint.sh")
check('exec odeiond -c "$CONF" "$@"' in ep, "the image's entrypoint runs odeiond")
check("ODEION_" in ep, "the image's entrypoint reads ODEION_ settings")
check('${PM_NAME:-Odeion node}' in ep, "the image's default node name is Odeion node")
has("deploy/bundle/docker-compose.yml", "PM_NAME: ${ODEION_NAME-${PM_NAME:-Odeion}}", "the bundle's default node name is Odeion")
rel = text(".github/workflows/release.yml")
check(re.search(r"bins: odeiond pm-node\n", rel) is not None, "release archives ship odeiond and pm-node")
check("bins: odeiond.exe pm-node.exe" in rel, "the Windows archive ships odeiond.exe and pm-node.exe")
check('--title "Odeion $GITHUB_REF_NAME"' in rel, "a release is titled Odeion")
check('echo "Odeion $CHANNEL. What changed' in rel, "release notes say Odeion")
has("tools/release/build-armv7.sh", "odeiond", "the armv7 build collects odeiond")
meta = rel[rel.find("uses: docker/metadata-action"):]
meta = meta[:meta.find("- uses:")]
check("org.opencontainers.image.title=Odeion node" in meta and "org.opencontainers.image.description=Odeion node" in meta,
      "the published image's OCI title and description say Odeion (metadata-action labels override the Dockerfile's)")
bu = text("docs/guides/backup-and-update.md")
check(bu.count("ODEION_TAG") >= 2, "the update and rollback steps name ODEION_TAG (it wins over PM_TAG)")
dep = text("docs/deploy.md")
upd = dep[dep.find("- **Updating:**"):][:600]
check("odeiond" in upd and "ln -sf odeiond /usr/local/bin/pm-node" in upd, "deploy.md's update step installs odeiond and the pm-node link")

ha = "integrations/home-assistant/custom_components/pubky_music"
check(json_of(f"{ha}/manifest.json").get("name") == "Odeion", "the HA integration is named Odeion")
check(json_of("integrations/home-assistant/hacs.json").get("name") == "Odeion", "HACS lists Odeion")
for f in (f"{ha}/strings.json", f"{ha}/translations/en.json"):
    has(f, "Connect an Odeion node", f"{f}: the setup title says Odeion")
has(f"{ha}/__init__.py", 'manufacturer="Odeion"', "HA devices are made by Odeion")
has(f"{ha}/media_player.py", 'manufacturer="Odeion"', "HA zone devices are made by Odeion")
roon = text("sidecars/roon-bridge/index.js")
check('display_name: "Odeion Bridge"' in roon, "Roon shows the Odeion Bridge")
check('publisher: "Odeion"' in roon, "Roon shows the publisher Odeion")

check(text("README.md").startswith("# Odeion\n"), "README is titled Odeion")
has("README.md", "odeiond", "README names odeiond")
has("NOTICE", "Odeion", "NOTICE names Odeion")
for g in ("install-linux.md", "remote-access.md", "desktop-app.md"):
    has(f"docs/guides/{g}", "Odeion", f"docs/guides/{g} says Odeion")
has("docs/guides/install-linux.md", "odeiond.service", "the Linux guide installs odeiond.service")
has("docs/branding-compatibility.md", "# Odeion branding and compatibility", "the compatibility inventory exists")
cl = text("docs/CHANGELOG.md")
unreleased = cl.split("\n## ", 2)[1] if cl.count("\n## ") >= 1 else ""
check(unreleased.startswith("Unreleased") and "Odeion" in unreleased, "CHANGELOG's Unreleased section has the Odeion entry")

# ---------------------------------------------------------------- 2. compatibility identities kept
check(tauri.get("identifier") == "to.synonym.pubkymusic", "the app identifier (Android application id) is unchanged")
check(tauri.get("mainBinaryName") == "pubky-music", "the app's binary name is unchanged")
check(tauri.get("productName") == "Pubky Music",
      "the app's productName (install dir, MSI upgrade code, .deb name) is unchanged")
check(bundle.get("publisher") == "Pubky Music", "the app's publisher (Windows registry key) is unchanged")
updater_key = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDk1QTA0OUFGQTY5MkJFRjgK"
for f in ("tauri.linux.conf.json", "tauri.windows.conf.json", "tauri.macos.conf.json"):
    has(f"crates/pm-desktop/{f}", updater_key, f"the updater key in {f} is unchanged")
has("tools/release/android/PlayerService.kt", "package to.synonym.pubkymusic", "the Android package is unchanged")
has("tools/release/android/PlayerService.kt", 'getSharedPreferences("pubky-music"', "Android preferences keep their file")
check(json_of(f"{ha}/manifest.json").get("domain") == "pubky_music", "the HA domain is unchanged")
check(json_of(f"{ha}/manifest.json").get("zeroconf", [{}])[0].get("type") == "_pubky-music._tcp.local.",
      "HA still finds nodes by _pubky-music._tcp")
has(f"{ha}/const.py", 'DOMAIN = "pubky_music"', "the HA const DOMAIN is unchanged")
check('extension_id: "org.pubky.music.bridge"' in roon, "the Roon extension id is unchanged")
check(json_of("sidecars/roon-bridge/package.json").get("name") == "pubky-music-roon-bridge",
      "the Roon bridge's package name is unchanged")
has("crates/pm-discovery/src/lib.rs", "_pubky-music._tcp", "mDNS service type is unchanged")
has("crates/pm-remote/src/lib.rs", "pubky-music/http/1", "the iroh ALPN is unchanged")
has("crates/pm-appdata/src/paths.rs", "/priv/pubky.music/", "the homeserver app-data path is unchanged")
has("Cargo.toml", 'repository = "https://github.com/gillohner/pubky-music-node"', "the repository URL is unchanged")
check('"crates/pubky-music-specs"' in text("Cargo.toml"), "pubky-music-specs stays a workspace member")
check("images: ghcr.io/${{ github.repository }}" in rel, "the image is published under the repository's name")
check('name="pubky-music-node-${{ matrix.platform }}"' in rel, "node archives keep their download names")
check('"pubky-music-$platform.dmg"' in text("tools/release/collect-app.sh"), "app downloads keep their names")
has("deploy/bundle/docker-compose.yml", "name: pubky-music\n", "the bundle's compose project (its volumes) is unchanged")
has("deploy/bundle/docker-compose.yml", "${ODEION_IMAGE:-${PM_IMAGE:-ghcr.io/gillohner/pubky-music-node}}:${ODEION_TAG:-${PM_TAG:-edge}}",
    "the bundle keeps its image and still reads the PM_ .env names (ODEION_ first)")
has("deploy/truenas/docker-compose.yaml", "image: ghcr.io/gillohner/pubky-music-node:edge", "TrueNAS pulls the same image")
has("deploy/truenas/docker-compose.yaml", "container_name: pubky-music\n", "TrueNAS keeps its container name")
old_unit = text("deploy/bare-metal/pubky-music-node.service")
check(bool(old_unit), "the old unit file stays (published install commands use it)")
check("User=pubky-music" in old_unit and "User=pubky-music" in u, "both units run as the pubky-music user")
check("ReadWritePaths=/var/lib/pubky-music" in old_unit and "ReadWritePaths=/var/lib/pubky-music" in u,
      "both units keep /var/lib/pubky-music")
check("Conflicts=odeiond.service" in old_unit, "the old unit never runs next to odeiond.service")
check('name = "pm-node"' in node_toml, "the pm-node executable still builds")
check(re.search(r'\[package\]\nname = "pm-node"', node_toml) is not None, "the pm-node package id is unchanged")
check("PM_DATA_DIR" in ep and "PM_TOKEN" in ep and "PM_FOLDERS" in ep, "the entrypoint still reads PM_ settings")
has("crates/pm-api/src/opensubsonic.rs", 'Some("pubky-music-node")', "OpenSubsonic server type is unchanged")
has("crates/pm-api/static/app.js", "localStorage.getItem('pm.token')", "the web page keeps its stored token key")

# ---------------------------------------------------------------- 3. legacy names only where allowed
ALLOWED_PREFIXES = (
    "docs/CHANGELOG.md",  # history, and the Unreleased entry names the old name
    "docs/archive/",
    "docs/ux/",  # dated design mockups
    "crates/pubky-music-specs/",
    "docs/branding-compatibility.md",
    "tools/branding-check.py",
)
# Lines that keep the old display name on purpose (an identity, not a label).
ALLOWED_LINES = {
    "crates/pm-desktop/tauri.conf.json": ('"productName": "Pubky Music"', '"publisher": "Pubky Music"'),
    # the published test vector's text, shared with other implementations of VAULT.md
    "crates/pm-appdata/tests/vault.rs": ("Pubky Music VAULT.md v1",),
    "crates/pm-appdata/tests/vectors/vault-v1.json": ("Pubky Music VAULT.md v1",),
    # historical authorship stays as it was; the product line says what it was called
    "NOTICE": ("Copyright The Pubky Music Node authors.", "Odeion (formerly Pubky Music Node)"),
}
# Tracked files and new ones not ignored yet, so a file not yet added is checked too
files = subprocess.run(["git", "-C", str(ROOT), "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
                       capture_output=True, check=True).stdout
left = []
for rel in sorted(set(files.decode().split("\0"))):
    if not rel or rel.startswith(ALLOWED_PREFIXES):
        continue
    p = ROOT / rel
    try:
        body = p.read_text(encoding="utf-8")
    except (UnicodeDecodeError, OSError):
        continue
    left += legacy_left(rel, body)
check(not left, f"no 'Pubky Music' outside history and compatibility lines ({len(left)} left)")
for line in left[:40]:
    print("     ", line)
if len(left) > 40:
    print(f"      … and {len(left) - 40} more")

if SELF_TEST:
    # The checks must not depend on how rustfmt lays the code out (and must not let more through)
    one_line = 'pm_model::brand::build_env(option_env!("ODEION_UPDATE_CHANNEL"), option_env!("PM_UPDATE_CHANNEL")).filter(|c| !c.is_empty())'
    split = 'pm_model::brand::build_env(\n        option_env!("ODEION_UPDATE_CHANNEL"),\n        option_env!("PM_UPDATE_CHANNEL"),\n    )\n    .filter(|c| !c.is_empty())'
    reversed_ = 'build_env(option_env!("PM_UPDATE_CHANNEL"), option_env!("ODEION_UPDATE_CHANNEL"))'
    check(channel_reads_new_first(one_line), "self-test: the channel check passes on one line")
    check(channel_reads_new_first(split), "self-test: the channel check passes as rustfmt splits it")
    check(not channel_reads_new_first(reversed_), "self-test: the channel check fails when PM_ comes first")
    marked = 'assert!(!help.contains("Pubky Music")); // compat: pre-Odeion name must be gone\n'
    fmt_split = ('    assert!(\n        help.contains("Odeion endpoint") && !help.contains("Pubky Music"),\n'
                 '        "{help}"\n    ); // compat: pre-Odeion name must be gone\n')
    after_done = 'let s = "Pubky Music";\n// compat: pre-Odeion\n'
    far = 'x(\n  "Pubky Music",\n  a,\n  b,\n  c,\n  d); // compat: pre-Odeion\n'
    plain = 'title: "Pubky Music",\n'
    check(not legacy_left("t.rs", marked), "self-test: a marked line passes")
    check(not legacy_left("t.rs", fmt_split), "self-test: a marked statement rustfmt split over lines passes")
    check(legacy_left("t.rs", after_done), "self-test: a mark after a finished statement does not count")
    check(legacy_left("t.rs", far), "self-test: a mark more than 3 lines on does not count")
    check(legacy_left("t.rs", plain), "self-test: an unmarked old name fails")

print(f"\n{len(problems)} problem(s)")
sys.exit(1 if problems else 0)

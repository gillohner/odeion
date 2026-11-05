#!/usr/bin/env bash
# Builds the Odeion installer ISO (docs/guides/install-iso.md): Debian's official amd64 netinst,
# checked against Debian's signed SHA256SUMS, with our preseed, our boot menus and /odeion added.
# The boot records are replayed as they are (BIOS, UEFI, USB stick).
#
#   deploy/iso/build-iso.sh --out odeion-edge-amd64.iso --tag edge
#   deploy/iso/build-iso.sh --out test.iso --answers mode=node,disk=/dev/vda,hostname=odeion,password=x,console=ttyS0
#
# --tag      the image tag the installed system runs (a release's version; default edge)
# --answers  answers on the installer's command line, for unattended tests (CI); otherwise the same ISO
# --netinst  a netinst ISO already downloaded (its SHA256SUMS are still checked when given with --sums)
# --keyring  Debian's CD signing keys (default /usr/share/keyrings/debian-role-keys.gpg, debian-keyring)
# Needs: xorriso, gpgv, curl, sha256sum.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
MIRROR=${DEBIAN_CD_MIRROR:-https://cdimage.debian.org/debian-cd/current/amd64/iso-cd}
OUT=odeion-amd64.iso TAG=edge ANSWERS='' NETINST='' SUMS='' KEYRING=/usr/share/keyrings/debian-role-keys.gpg
die() { echo "build-iso: $*" >&2; exit 1; }
while [ $# -gt 0 ]; do
  case $1 in
    --out) OUT=$2; shift 2 ;;
    --tag) TAG=$2; shift 2 ;;
    --answers) ANSWERS=$2; shift 2 ;;
    --netinst) NETINST=$2; shift 2 ;;
    --sums) SUMS=$2; shift 2 ;;
    --keyring) KEYRING=$2; shift 2 ;;
    *) die "unknown argument $1" ;;
  esac
done
case $TAG in '' | *[!A-Za-z0-9._-]*) die "--tag: letters, digits, . _ - only" ;; esac
for t in xorriso gpgv curl sha256sum; do command -v $t >/dev/null || die "$t is not installed"; done

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# 1. Debian's netinst, checked: SHA256SUMS signed by Debian's CD key, the ISO against it
if [ -z "$NETINST" ] || [ -n "$SUMS" ]; then
  [ -f "$KEYRING" ] || die "no keyring at $KEYRING (install debian-keyring, or --keyring)"
  if [ -z "$SUMS" ]; then
    curl -fsSL --retry 5 -o "$WORK/SHA256SUMS" "$MIRROR/SHA256SUMS"
    curl -fsSL --retry 5 -o "$WORK/SHA256SUMS.sign" "$MIRROR/SHA256SUMS.sign"
    SUMS=$WORK/SHA256SUMS
    gpgv --keyring "$KEYRING" "$WORK/SHA256SUMS.sign" "$SUMS" || die "SHA256SUMS is not signed by Debian"
  fi
  name=$(grep -o 'debian-[0-9.]*-amd64-netinst\.iso' "$SUMS" | head -n 1)
  [ -n "$name" ] || die "no amd64 netinst in SHA256SUMS"
  if [ -z "$NETINST" ]; then
    echo "Downloading $name"
    curl -fL --retry 5 -o "$WORK/$name" "$MIRROR/$name"
    NETINST=$WORK/$name
  fi
  want=$(awk -v n="$name" '$2 == n { print $1 }' "$SUMS")
  got=$(sha256sum "$NETINST" | cut -d' ' -f1)
  [ "$want" = "$got" ] || die "$name does not match Debian's SHA256SUMS"
  echo "$name: checked against Debian's signed SHA256SUMS"
fi

# 2. What goes on the ISO
STAGE=$WORK/stage
mkdir -p "$STAGE/odeion/bundle"
cp -r "$HERE/odeion/." "$STAGE/odeion/"
for f in docker-compose.yml setup.sh settings.sh backup.sh restore.sh; do
  cp "$REPO/deploy/bundle/$f" "$STAGE/odeion/bundle/$f"
done
echo "$TAG" > "$STAGE/odeion/image-tag"
cp "$HERE/preseed.cfg" "$STAGE/preseed.cfg"

# Answers for unattended installs, on the installer's command line. Before `---`, so they are not
# copied into the installed system's boot line; the installer's logs (/var/log/installer) may still
# hold them, so only for test machines with throwaway passwords (CI). A preseeded `seen false`
# would ask again: dropped then.
args=
console=
IFS=, read -ra pairs <<< "$ANSWERS"
for p in "${pairs[@]}"; do
  [ -n "$p" ] || continue
  k=${p%%=*} v=${p#*=}
  case $v in *[[:space:]]*) die "--answers: no spaces in $k" ;; esac
  case $k in
    mode)
      case $v in bundle | node) ;; *) die "--answers mode: bundle or node" ;; esac
      args="$args odeion/mode=$v" ;;
    disk)
      args="$args partman-auto/disk=$v grub-installer/bootdev=$v partman/confirm=true partman/confirm_nooverwrite=true"
      args="$args partman-partitioning/confirm_write_new_label=true" ;;
    hostname)
      args="$args netcfg/get_hostname=$v netcfg/hostname=$v"
      sed -i '/^d-i netcfg\/get_hostname seen false$/d' "$STAGE/preseed.cfg" ;;
    password) args="$args passwd/user-password=$v passwd/user-password-again=$v" ;;
    console) console="console=$v,115200n8" ;;
    *) die "--answers: unknown key $k" ;;
  esac
done

boot="auto=true priority=high preseed/file=/cdrom/preseed.cfg${args}${console:+ $console} --- quiet${console:+ $console}"
cat > "$STAGE/isolinux.cfg" <<EOF
# Odeion: straight into the installer, which asks before it touches a disk
default odeion
prompt 0
timeout 0
label odeion
  kernel /install.amd/vmlinuz
  append vga=788 initrd=/install.amd/initrd.gz $boot
EOF
cat > "$STAGE/grub.cfg" <<EOF
# Odeion: straight into the installer, which asks before it touches a disk
set default=0
set timeout=0
menuentry 'Install Odeion' {
  linux /install.amd/vmlinuz $boot
  initrd /install.amd/initrd.gz
}
EOF

# md5sum.txt (the installer's "check the medium") with our files
xorriso -osirrox on -indev "$NETINST" -extract /md5sum.txt "$WORK/md5sum.txt" 2>/dev/null
chmod u+w "$WORK/md5sum.txt"
grep -v -e ' \./isolinux/isolinux\.cfg$' -e ' \./boot/grub/grub\.cfg$' "$WORK/md5sum.txt" > "$STAGE/md5sum.txt" || true
sum() { printf '%s  ./%s\n' "$(md5sum < "$STAGE/$1" | cut -d' ' -f1)" "$2"; }
{
  sum preseed.cfg preseed.cfg
  sum isolinux.cfg isolinux/isolinux.cfg
  sum grub.cfg boot/grub/grub.cfg
  (cd "$STAGE" && find odeion -type f | LC_ALL=C sort) | while read -r f; do sum "$f" "$f"; done
} >> "$STAGE/md5sum.txt"

# 3. The ISO: Debian's, with ours added and its boot records replayed
rm -f "$OUT"
xorriso -indev "$NETINST" -outdev "$OUT" \
  -map "$STAGE/preseed.cfg" /preseed.cfg \
  -map "$STAGE/odeion" /odeion \
  -map "$STAGE/isolinux.cfg" /isolinux/isolinux.cfg \
  -map "$STAGE/grub.cfg" /boot/grub/grub.cfg \
  -map "$STAGE/md5sum.txt" /md5sum.txt \
  -boot_image any replay
(cd "$(dirname "$OUT")" && sha256sum "$(basename "$OUT")" > "$(basename "$OUT").sha256")
echo "Built $OUT (image tag $TAG)"

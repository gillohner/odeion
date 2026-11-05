#!/usr/bin/env bash
# CI (iso.yml): installs an answers ISO (build-iso.sh --answers …,disk=/dev/vda,console=ttyS0) onto a
# fresh disk in QEMU/KVM, boots the installed system and waits for Odeion: the node on :7788 and,
# for `bundle`, the homeserver's Pubky TLS on :6287 (and for `node`, nothing there).
#
#   deploy/iso/test-qemu.sh odeion-test-bundle.iso bundle
set -euo pipefail
ISO=$1 MODE=$2
LOGS=${LOGS:-iso-logs}
INSTALL_MIN=${INSTALL_MIN:-60} UP_MIN=${UP_MIN:-30}
mkdir -p "$LOGS"
DISK=$(mktemp -d)/disk.qcow2
qemu-img create -f qcow2 "$DISK" 20G >/dev/null
VM=(qemu-system-x86_64 -enable-kvm -cpu host -m 4096 -smp 2
  -drive "file=$DISK,if=virtio,format=qcow2" -display none -no-reboot)

echo "== installing ($MODE), up to $INSTALL_MIN min; serial log: $LOGS/install-$MODE.log"
if ! timeout "${INSTALL_MIN}m" "${VM[@]}" -cdrom "$ISO" -boot d \
  -netdev user,id=n0 -device virtio-net-pci,netdev=n0 -serial "file:$LOGS/install-$MODE.log"; then
  echo "::error::the install did not finish in $INSTALL_MIN min"
  tail -n 80 "$LOGS/install-$MODE.log"
  exit 1
fi

echo "== booting the installed system"
CONSOLE=$(dirname "$DISK")/console.sock
"${VM[@]}" -boot c \
  -netdev "user,id=n0,hostfwd=tcp:127.0.0.1:17788-:7788,hostfwd=tcp:127.0.0.1:16287-:6287" \
  -device virtio-net-pci,netdev=n0 \
  -chardev "socket,id=s0,path=$CONSOLE,server=on,wait=off,logfile=$LOGS/boot-$MODE.log" -serial chardev:s0 &
QEMU=$!
trap 'kill $QEMU 2>/dev/null || true' EXIT

# What the VM itself says, through its serial console (the test's own login, TEST_PASSWORD)
diagnose() {
  [ -n "${TEST_PASSWORD:-}" ] && [ -S "$CONSOLE" ] || return 0
  echo "== inside the VM"
  timeout 600 python3 "$(dirname "$0")/vm-console.py" "$CONSOLE" odeion "$TEST_PASSWORD" \
    "systemctl --no-pager --failed; systemctl --no-pager status odeion-firstboot odeion nftables | tail -n 40" \
    "journalctl --no-pager -b -u odeion-firstboot | tail -n 60" \
    "cd /opt/odeion && odeion-compose ps -a" \
    "ss -ltnp" \
    "nft list ruleset" \
    "echo | timeout 10 openssl s_client -connect 127.0.0.1:6287 2>&1 | tail -n 25" \
    "cd /opt/odeion && odeion-compose logs --tail 40 homeserver homeserver-config" \
    | tee "$LOGS/inside-$MODE.log" || true
}

# The node: any HTTP answer from it
node_up() {
  local code
  code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 http://127.0.0.1:17788/ || true)
  case $code in 2?? | 3?? | 401) return 0 ;; *) return 1 ;; esac
}
# The homeserver: TLS bytes back on :6287 (Pubky TLS uses raw public keys, so the handshake itself
# may end in an alert; through QEMU's port forward nothing comes back when nothing listens)
tls_up() {
  echo | timeout 10 openssl s_client -connect 127.0.0.1:16287 2>&1 | grep -q 'SSL handshake has read [1-9]'
}

deadline=$(($(date +%s) + UP_MIN * 60))
node=no hs=no node_at=0
while [ "$(date +%s)" -lt "$deadline" ]; do
  kill -0 $QEMU 2>/dev/null || { echo "::error::the VM stopped"; tail -c 8000 "$LOGS/boot-$MODE.log"; exit 1; }
  if [ $node = no ] && node_up; then node=yes; node_at=$(date +%s); echo "node answers on :7788"; fi
  if [ "$MODE" = bundle ] && [ $hs = no ] && tls_up; then hs=yes; echo "homeserver answers TLS on :6287"; fi
  if [ $node = yes ] && { [ "$MODE" = node ] || [ $hs = yes ]; }; then
    if [ "$MODE" = node ] && tls_up; then
      echo "::error::Odeion only should not run a homeserver"
      diagnose
      exit 1
    fi
    echo "== Odeion is up ($MODE)"
    exit 0
  fi
  # The node starts only once the homeserver is healthy: 5 min after it, the homeserver should answer
  if [ "$MODE" = bundle ] && [ $node = yes ] && [ $hs = no ] && [ $(($(date +%s) - node_at)) -gt 300 ]; then
    echo "::error::the node is up but :6287 gives no TLS from outside"
    diagnose
    exit 1
  fi
  sleep 15
done
echo "::error::Odeion was not up within $UP_MIN min (node: $node, homeserver: $hs)"
diagnose
exit 1

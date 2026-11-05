#!/bin/sh
# Runs once, as root, inside the newly installed system (late.sh, `in-target`). $1: what to run,
# `bundle` (Odeion with its own homeserver) or `node` (Odeion only). Nothing is pulled or started
# here: the network and Docker come up on the first boot (odeion-firstboot.service).
set -eu
mode=$1
src=/opt/odeion-install
case $mode in bundle | node) ;; *) echo "install.sh: unknown mode $mode" >&2; exit 1 ;; esac

# What runs, and with which image tag (a release ISO pins its own version)
install -d -m 755 /opt/odeion /etc/odeion /var/lib/odeion /srv/music
install -d -m 700 /var/backups/odeion
echo "$mode" > /etc/odeion/mode
tag=$(cat "$src/image-tag" 2>/dev/null || echo edge)
echo "$tag" > /etc/odeion/image-tag
if [ "$mode" = bundle ]; then
  cp "$src"/bundle/* /opt/odeion/
else
  cp "$src"/node-only/docker-compose.yml /opt/odeion/docker-compose.yml
fi
chmod 755 /opt/odeion/*.sh 2>/dev/null || true
# The node's container runs as uid 1000; it only reads the music
chmod 755 /srv/music

# `docker compose` (what backup.sh and restore.sh run): Debian's docker-compose v2 works as the plugin
if ! docker compose version >/dev/null 2>&1 && [ -x /usr/bin/docker-compose ]; then
  install -d /usr/libexec/docker/cli-plugins
  ln -sf /usr/bin/docker-compose /usr/libexec/docker/cli-plugins/docker-compose
fi

# Odeion's own programs and units
install -m 755 "$src"/system/odeion-compose /usr/local/sbin/odeion-compose
install -m 755 "$src"/system/odeion-firstboot /usr/local/sbin/odeion-firstboot
install -m 755 "$src"/system/odeion-local-redirect /usr/local/sbin/odeion-local-redirect
for u in odeion-firstboot.service odeion.service odeion-backup.service odeion-backup.timer \
  odeion-local-redirect.service odeion-local-redirect.path; do
  install -m 644 "$src/system/$u" "/etc/systemd/system/$u"
done

# Firewall: our own nftables table, input only (Docker keeps its own rules)
install -m 755 -d /etc/nftables.d
if [ "$mode" = bundle ]; then
  sed 's/@HOMESERVER_PORT@/tcp dport 6287 accept/' "$src/system/odeion.nft" > /etc/nftables.d/odeion.nft
else
  sed '/@HOMESERVER_PORT@/d' "$src/system/odeion.nft" > /etc/nftables.d/odeion.nft
fi
cat > /etc/nftables.conf <<'EOF'
#!/usr/sbin/nft -f
# Odeion: no `flush ruleset` here, so a restart of nftables leaves Docker's rules alone.
include "/etc/nftables.d/*.nft"
EOF
# ...and its stop deletes only Odeion's tables (Debian's flushes every rule, Docker's too)
install -d /etc/systemd/system/nftables.service.d
install -m 644 "$src"/system/nftables-odeion.conf /etc/systemd/system/nftables.service.d/odeion.conf

# Hardware watchdog: systemd pets it; a hung machine restarts
install -d /etc/systemd/system.conf.d
install -m 644 "$src"/system/watchdog.conf /etc/systemd/system.conf.d/odeion-watchdog.conf

# Security updates by themselves (Debian's default origins are security only)
install -m 644 "$src"/system/20auto-upgrades /etc/apt/apt.conf.d/20auto-upgrades

# odeion.local on the network
sed -i 's/^#\?publish-workstation=.*/publish-workstation=no/' /etc/avahi/avahi-daemon.conf 2>/dev/null || true

# The console says where to go
cat > /etc/issue <<'EOF'
Odeion on \n (\l)

Open http://\n.local:7788 (or http://\4:7788) in a browser to set it up.

EOF

systemctl enable docker.service avahi-daemon.service nftables.service \
  odeion-firstboot.service odeion.service
if [ "$mode" = bundle ]; then
  systemctl enable odeion-backup.timer odeion-local-redirect.path odeion-local-redirect.service
fi
# No SSH server is installed; make sure none would start if one came along
systemctl mask ssh.service 2>/dev/null || true

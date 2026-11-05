#!/bin/sh
# debian-installer, preseed/late_command: copies Odeion into the installed system and sets it up
# there (install.sh, in the target's chroot). The answer to odeion/mode decides what it runs.
set -e
mode=$(debconf-get odeion/mode 2>/dev/null || true)
case $mode in bundle | node) ;; *) mode=bundle ;; esac
mkdir -p /target/opt/odeion-install
cp -r /cdrom/odeion/. /target/opt/odeion-install/
in-target sh /opt/odeion-install/install.sh "$mode"
rm -rf /target/opt/odeion-install

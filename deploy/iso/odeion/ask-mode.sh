#!/bin/sh
# debian-installer, preseed/early_command: asks what to run (odeion.templates). An answer on the
# kernel command line (odeion/mode=bundle or =node, as the CI test ISO has) is taken as it is.
. /usr/share/debconf/confmodule
debconf-loadtemplate odeion /cdrom/odeion/odeion.templates
for a in $(cat /proc/cmdline); do
  case $a in
    odeion/mode=bundle | odeion/mode=node)
      db_set odeion/mode "${a#odeion/mode=}"
      db_fset odeion/mode seen true
      exit 0
      ;;
  esac
done
db_input critical odeion/mode || true
db_go || true

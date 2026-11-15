#!/bin/sh
# SPDX-License-Identifier: FSL-1.1-ALv2
# The installed server preset's pryxd, as its own unit (deploy/pryxd.service) runs it, proved
# kernel network policy for process sandboxes at its current start: every program loaded,
# attached to a cgroup below the unit's, a process moved in, and all of it removed again. Read
# from that process's own journal line. The preset's control plane requires private-network,
# private-pids, memory-limit and cpu-limit of every sandbox, which the process tier doesn't
# give, so no process sandbox is placed through it here; CI runs the policy suite under the
# unit's settings ("Kernel network policy under the installed root unit"). Changes nothing.
set -eu
pid=$(systemctl show pryxd --property=MainPID --value)
test "$pid" -gt 0
line=$(sudo journalctl "_PID=$pid" _SYSTEMD_UNIT=pryxd.service --no-pager --output=cat | grep 'kernel network policy' | tail -n 1)
echo "$line"
case $line in
  *'"on":true'* | *'on=true'*) echo 'root_installed_kernel_network_policy=on' ;;
  *) echo 'The installed pryxd did not prove kernel network policy' >&2; exit 1 ;;
esac

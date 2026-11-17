#!/bin/bash
# SPDX-License-Identifier: FSL-1.1-ALv2
# Give a disposable runner a public-looking IPv4 address of its own, for the installed VM
# acceptance's firewall check (deploy/vm-installed-acceptance.ts, PRYX_VM_HOST_PUBLIC_ADDRESS):
# a /32 on a dummy interface. `up` adds it, if it is missing, and prints it; `down` removes it.
#
# The packet helper dials from the host's own network, so a host whose interface has a public
# address would otherwise let a guest reach what listens there. A runner's own addresses are
# private, which the helper refuses for being private; this one is refused only for being the
# host's. 192.88.99.1 is in RFC 7526's retired 6to4 relay range: outside the private, loopback,
# link-local, multicast and broadcast classes and the agent's non-public boundary
# (agent/src/transfer_target.rs, SPECIAL_V4), and nothing a job reaches. Uses sudo.
set -euo pipefail
address=192.88.99.1
device=pryx-public0
case "${1:?Say up or down}" in
  up)
    if ! ip -4 -o addr show dev "$device" 2>/dev/null | grep -q " $address/32 "; then
      sudo ip link add "$device" type dummy
      sudo ip addr add "$address/32" dev "$device"
      sudo ip link set "$device" up
    fi
    echo "$address"
    ;;
  down)
    if ip link show dev "$device" > /dev/null 2>&1; then sudo ip link del "$device"; fi
    ;;
  *) echo 'Say up or down' >&2; exit 1 ;;
esac

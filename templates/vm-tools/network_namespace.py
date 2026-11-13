#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Join the private pasta-created user/network namespace and attach the monitor's TAP.

Only this fixture builds a bridge. The runner's initial namespace and host firewall are not
modified. The unchanged monitor retains its seccomp and dies with its launcher on failure.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess

# Linux Documentation/networking/ip-sysctl.rst defines disable_ipv6=1 as disabling IPv6.
IPV6_DISABLED = '1'
IPV6_KEYS = ['net.ipv6.conf.all.disable_ipv6', 'net.ipv6.conf.default.disable_ipv6']
INTERFACES = ['lo', 'uplink', 'fc0', 'br0']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--host-netns', required=True)
    parser.add_argument('--host-uid', type=int, required=True)
    parser.add_argument('--record', type=Path, required=True)
    parser.add_argument('monitor', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    namespace = os.readlink('/proc/self/ns/net')
    mapping = Path('/proc/self/uid_map').read_text()
    if namespace == args.host_netns or args.host_uid == 0:
        raise ValueError('The TAP experiment requires a private ordinary-user namespace')
    maps = [tuple(map(int, line.split())) for line in mapping.splitlines()]
    if not maps or any(outside == 0 for _, outside, _ in maps):
        raise ValueError('The TAP experiment must not map host root')
    if not any(outside <= args.host_uid < outside + count for _, outside, count in maps):
        raise ValueError('The TAP namespace does not map the ordinary runner user')
    # This comparison selects IPv4 on both paths. Keep private bridge IPv6 control
    # frames from changing pasta's single learned guest MAC in the middle of a transfer.
    # The namespace and UID-map checks above precede every network mutation.
    for key in IPV6_KEYS:
        subprocess.run(['sysctl', '-w', key + '=' + IPV6_DISABLED], check=True, timeout=10)
    for arguments in [['link', 'add', 'br0', 'type', 'bridge'],
                      ['tuntap', 'add', 'dev', 'fc0', 'mode', 'tap'],
                      ['link', 'set', 'uplink', 'master', 'br0'],
                      ['link', 'set', 'fc0', 'master', 'br0'],
                      *[['link', 'set', name, 'up'] for name in INTERFACES]]:
        subprocess.run(['ip', *arguments], check=True, timeout=10)
    links = json.loads(subprocess.check_output(['ip', '-json', '-details', 'link', 'show'], timeout=10))
    keys = [*IPV6_KEYS, *[f'net.ipv6.conf.{name}.disable_ipv6' for name in INTERFACES]]
    ipv6 = {key: subprocess.check_output(['sysctl', '-n', key], text=True, timeout=10).strip() for key in keys}
    if any(value != IPV6_DISABLED for value in ipv6.values()):
        raise ValueError('IPv4-only TAP fixture retains an IPv6-enabled bridge interface')
    args.record.write_text(json.dumps({'monitorPid': os.getpid(), 'uidMap': mapping,
                                       'netns': namespace, 'hostNetns': args.host_netns,
                                       'links': links, 'ipv6': ipv6}) + '\n')
    if not args.monitor:
        raise ValueError('Missing monitor invocation')
    os.execvp('setpriv', ['setpriv', '--pdeathsig', 'KILL', '--', *args.monitor])


if __name__ == '__main__':
    main()

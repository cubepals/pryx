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
    for arguments in [['link', 'add', 'br0', 'type', 'bridge'],
                      ['tuntap', 'add', 'dev', 'fc0', 'mode', 'tap'],
                      ['link', 'set', 'uplink', 'master', 'br0'],
                      ['link', 'set', 'fc0', 'master', 'br0'],
                      *[['link', 'set', name, 'up'] for name in ['lo', 'uplink', 'fc0', 'br0']]]:
        subprocess.run(['ip', *arguments], check=True, timeout=10)
    args.record.write_text(json.dumps({'monitorPid': os.getpid(), 'uidMap': mapping,
                                       'netns': namespace, 'hostNetns': args.host_netns}) + '\n')
    if not args.monitor:
        raise ValueError('Missing monitor invocation')
    os.execvp('setpriv', ['setpriv', '--pdeathsig', 'KILL', '--', *args.monitor])


if __name__ == '__main__':
    main()

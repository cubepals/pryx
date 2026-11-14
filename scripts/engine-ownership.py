#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Measure three real user-namespace bind strategies without changing workload hardening.

This is diagnostic evidence, not pryx acceptance. It records unsuccessful strategies as well
as successful writes, preserving nonzero IDs, read-only roots and the daemon-owned data parent.
"""
import json
import os
from pathlib import Path
import subprocess
import tempfile


def measure():
    versions = json.loads(Path('.github/engine-versions.json').read_text())
    workload_uid = 1000
    workload_gid = 1000
    engine = os.environ['PRYX_TEST_ENGINE']
    endpoint = os.environ['DOCKER_HOST']
    prefix = (['docker', '--host', endpoint] if engine == 'docker'
              else ['podman', '--remote', '--url', endpoint])
    for mode in [None, 'host', f'keep-id:uid={workload_uid},gid={workload_gid}']:
        with tempfile.TemporaryDirectory(prefix='pryx-engine-ownership-', dir=Path.home()) as root:
            command = prefix + ['run', '--rm', '--pull=never', f'--user={workload_uid}:{workload_gid}',
                                '--cap-drop=ALL', '--security-opt=no-new-privileges', '--read-only',
                                '--mount', f'type=bind,source={root},target=/data']
            if mode is not None:
                command.append(f'--userns={mode}')
            if engine == 'podman':
                command.append('--read-only-tmpfs=false')
            command += [versions['image']['alias'], 'sh', '-c',
                        'id -u; id -g; cat /proc/self/uid_map; printf retained > /data/marker']
            result = subprocess.run(command, capture_output=True, text=True, timeout=45, check=False)
            marker = Path(root) / 'marker'
            owner = marker.stat().st_uid if marker.exists() else None
            report = {'engine': engine, 'userns': mode or 'default', 'exit': result.returncode,
                      'daemonUid': os.geteuid(), 'dataOwner': Path(root).stat().st_uid,
                      'writtenOwner': owner, 'stdout': result.stdout[:1024],
                      'stderr': result.stderr[:1024]}
            if result.returncode == 0:
                assert result.stdout.splitlines()[:2] == [str(workload_uid), str(workload_gid)], report
                assert marker.read_text() == 'retained', report
                assert owner == os.geteuid(), report
            print(json.dumps(report), flush=True)


if __name__ == '__main__':
    measure()

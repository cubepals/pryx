# SPDX-License-Identifier: FSL-1.1-ALv2
"""Read the hosted process-limit fixture's OOM count across sandbox retirement."""
import errno
import json
import os
from pathlib import Path
import subprocess
import sys


def oom_kills(events):
    events.seek(0)
    for line in events:
        name, count = line.split()
        if name == 'oom_kill':
            return int(count)
    return None


def observe(cgroup, node_log, run):
    # Keep the original leaf's counter open even if a new cgroup later reuses its path.
    with (cgroup / 'memory.events').open() as events, node_log.open('rb') as records:
        before = oom_kills(events)
        if before is None:
            raise RuntimeError('Missing initial cgroup oom_kill counter')
        records.seek(0, os.SEEK_END)
        status = run()
        if status == 0:
            raise RuntimeError('The allocation completed within the enforced memory limit')
        try:
            after = oom_kills(events)
        except OSError as error:
            if error.errno not in (errno.ENOENT, errno.ENODEV):
                raise
            after = None
        if after is not None and after > before:
            return status, after - before

        # resources::release logs the kernel count synchronously before removing the leaf.
        for line in records:
            try:
                record = json.loads(line)
            except (ValueError, UnicodeDecodeError):
                continue
            if not isinstance(record, dict) or record.get('target') != 'pryxd::runtime::process::resources':
                continue
            fields = record.get('fields')
            if isinstance(fields, dict) and fields.get('cgroup') == str(cgroup):
                count = fields.get('oom_kills')
                if type(count) is int:
                    if count > before:
                        return status, count - before
                    break
        raise RuntimeError('No increase in oom_kill for the tested sandbox cgroup')


if __name__ == '__main__':
    cgroup, node_log, output, *command = sys.argv[1:]
    with open(output, 'wb') as capture:
        status, delta = observe(Path(cgroup), Path(node_log), lambda: subprocess.run(
            command, stdout=capture, stderr=subprocess.STDOUT, check=False).returncode)
    print(f'oom_exit={status} oom_kills={delta}')

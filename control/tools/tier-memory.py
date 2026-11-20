# SPDX-License-Identifier: FSL-1.1-ALv2
"""Proportional resident bytes of explicit process roots and their descendants.

Run as the host administrator in CI so smaps_rollup includes every runtime UID. PSS avoids
counting shared executable pages once per process. Roots and missing processes are reported.
The optional VM breakdown counts sampled executables by fixed roles. A role with no measured
process has null PSS. The launcher execs the monitor in place before the VM reaches steady state.
"""
import argparse
import json
from pathlib import Path

VM_EXECUTABLES = {
    'vm-launcher': 'launcher',
    'firecracker': 'firecracker',
    'network-proxy': 'network-proxy',
    'vm-transport': 'vm-transport',
}


def process_tree(roots, proc):
    parents = {}
    for directory in proc.iterdir():
        if not directory.name.isdigit():
            continue
        try:
            lines = (directory / 'status').read_text().splitlines()
            parents[int(directory.name)] = int(next(line.split()[1] for line in lines if line.startswith('PPid:')))
        except (FileNotFoundError, ProcessLookupError):
            continue
    selected = set(roots)
    while True:
        children = {pid for pid, parent in parents.items() if parent in selected}
        if children <= selected:
            return selected
        selected |= children


def executable_role(directory):
    try:
        name = (directory / 'exe').readlink().name
    except (FileNotFoundError, ProcessLookupError, PermissionError):
        return None
    return VM_EXECUTABLES.get(name.removesuffix(' (deleted)'))


def sample(roots, vm_helpers=False):
    proc = Path('/proc')
    selected = process_tree(roots, proc)
    memory = 0
    missing = []
    helpers = {role: {'pssBytes': None, 'processes': 0} for role in VM_EXECUTABLES.values()}
    for pid in sorted(selected):
        directory = proc / str(pid)
        role = executable_role(directory) if vm_helpers else None
        try:
            lines = (directory / 'smaps_rollup').read_text().splitlines()
            pss = int(next(line.split()[1] for line in lines if line.startswith('Pss:'))) * 1024
        except (FileNotFoundError, ProcessLookupError):
            missing.append(pid)
            continue
        memory += pss
        # An exec during the measurement makes the executable attribution uncertain.
        if role and executable_role(directory) == role:
            observed = helpers[role]
            observed['pssBytes'] = (observed['pssBytes'] or 0) + pss
            observed['processes'] += 1
    result = {'pssBytes': memory, 'roots': sorted(roots), 'processes': len(selected), 'missing': missing}
    if vm_helpers:
        result['vmHelpers'] = helpers
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--vm-helpers', action='store_true')
    parser.add_argument('roots', nargs='*', type=int)
    args = parser.parse_args()
    print(json.dumps(sample(set(args.roots), args.vm_helpers)))


if __name__ == '__main__':
    main()

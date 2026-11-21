#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Compare one-vCPU PID 1 implementations with paired production and quiet boot.

Raw observations and a table survive a partial run. Only complete cohorts receive
a per-architecture verdict. The host clock never gets subtracted from the guest clock.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import statistics
import subprocess
import sys

from boot import sample

VM_TOOLS = Path(__file__).resolve().parents[2] / 'templates/vm-tools'
sys.path.insert(0, str(VM_TOOLS))
from filesystem import command
from kernel import root_image, selected


def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def percentiles(values):
    ordered = sorted(values)
    return statistics.median(ordered), ordered[math.ceil(0.95 * len(ordered)) - 1]


def summarize(rows, directory, labels, reference):
    by_label = {label: [row for row in rows if row['variant'] == label] for label in labels}
    table = ['| Variant | Boot to PID 1 p50/p95 (ms) | Launch to serial p50/p95 (ms) | '
             'Launch to bootstrap p50/p95 (ms) | Kernel/entry criterion |',
             '| --- | ---: | ---: | ---: | --- |']
    floor = percentiles([row['pid1BootNs'] for row in by_label[reference]])
    for name, group in by_label.items():
        kernel = percentiles([row['pid1BootNs'] for row in group])
        fields = []
        for key in ['pid1BootNs', 'launchToPid1SerialNs', 'launchToBootstrapNs']:
            series = [row[key] for row in group if key in row]
            fields.append(' / '.join(f'{v / 1e6:.3f}' for v in percentiles(series)) if series else '—')
        if name == reference:
            result = 'baseline'
        else:
            result = 'pass' if kernel[0] <= floor[0] * .90 and kernel[1] <= floor[1] * 1.05 else 'fail'
        table.append('| ' + ' | '.join([name, *fields, result]) + ' |')
    (directory / 'table.md').write_text('\n'.join(table) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--staged', type=Path, required=True, help='VM experiments RUNNER_TEMP')
    parser.add_argument('--build', type=Path, required=True, help='Build directory from build.sh')
    parser.add_argument('--agent', type=Path, required=True, help='Stamped bootstrap directory')
    parser.add_argument('--work', type=Path, required=True)
    parser.add_argument('--kernel-pair', type=Path, help='Use the matched control and changed kernel only')
    args = parser.parse_args()
    if platform.machine() != 'x86_64':
        raise ValueError('The pinned VM selection supports x86_64 only')
    if os.geteuid() == 0:
        raise ValueError('Run as the ordinary VM proof user')
    with open('/dev/kvm', 'r+b', buffering=0):
        pass
    work, stage = args.work.resolve(), args.staged.resolve()
    work.mkdir(mode=0o700)
    selection = json.loads((VM_TOOLS / 'kernel-selection.json').read_text())
    inspection = json.loads((stage / 'vm-archive-inspection.json').read_text())
    upstream = stage / 'vm-selected-files'
    monitor = selected(selection, inspection, upstream, 'monitor')
    kernel = selected(selection, inspection, upstream, 'kernel')
    monitor.chmod(0o700)
    bootstrap = json.loads((args.agent / 'bootstrap.json').read_text())
    agent = (args.agent / 'bootstrap.erofs').resolve()
    if digest(agent) != bootstrap['imageSha256']:
        raise ValueError('Bootstrap image digest mismatch')
    fixture = {'agent': agent}
    for variant, filename in ([] if args.kernel_pair else [('asm', 'init-S'), ('c', 'init-c')]):
        destination = work / variant
        destination.mkdir()
        builder = command(stage / 'vm-filesystem-component/tools', 'mkfs.erofs')
        fixture[variant] = root_image(args.build / filename, destination, builder)
    config = stage / 'vm-config'
    metadata = bootstrap['metadata']
    trials = [(kind if not options else f'{kind}-quiet', kind, kernel, options)
              for options in ([], ['quiet', 'loglevel=1']) for kind in fixture]
    if args.kernel_pair:
        trials = [(name, 'agent', (args.kernel_pair / f'{name}.elf').resolve(), [])
                  for name in ['control', 'no-print-time']]
    labels = [name for name, *_ in trials]
    report = {'host': platform.uname()._asdict(), 'kernel': digest(kernel),
              'monitor': digest(monitor), 'realAgent': bootstrap['agentSha256'],
              'kernelHashes': {name: digest(file) for name, _, file, _ in trials},
              'commit': subprocess.check_output(['git', '-C', VM_TOOLS, 'rev-parse', 'HEAD'], text=True).strip(),
              'cpu': Path('/proc/cpuinfo').read_text().split('\n\n', 1)[0],
              'compiler': subprocess.check_output(['cc', '--version'], text=True).splitlines()[0],
              'cache': {p.parent.name: p.read_text().strip() for p in
                        Path('/sys/devices/system/cpu/cpu0/cache').glob('index*/size')},
              'filesystem': subprocess.check_output(['stat', '-f', '-c', '%T', work], text=True).strip(),
              'images': {name: digest(path) for name, path in fixture.items()},
              'samples': [], 'complete': False}
    def save():
        (work / 'samples.json').write_text(json.dumps(report, indent=2) + '\n')
    save()
    try:
        for i in range(15):
            for label, kind, image, extra in trials[i % len(trials):] + trials[:i % len(trials)]:
                observation = sample(monitor, config, image, fixture[kind], work / f'{i:02}-{label}',
                                     metadata['vmInitMode'], metadata['vmBootPort'],
                                     extra, kind == 'agent')
                report['samples'].append({'round': i + 1, 'variant': label, **observation})
                save()
        summarize(report['samples'], work, labels, labels[0])
        report['complete'] = True
    finally:
        save()


if __name__ == '__main__':
    main()

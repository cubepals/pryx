#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Compare one-vCPU PID 1 implementations with paired production and quiet boot.

Raw observations and a table survive a partial run. Only complete cohorts receive
a per-architecture verdict. The host clock never gets subtracted from the guest clock.
"""
import argparse
import json
import math
import os
from pathlib import Path
import platform
import statistics
import sys

from boot import sample
from facts import digest, host

VM_TOOLS = Path(__file__).resolve().parents[2] / 'templates/vm-tools'
sys.path.insert(0, str(VM_TOOLS))
from filesystem import command
from kernel import root_image, selected


ROUNDS = 15


def schedule(trials):
    for round_number in range(1, ROUNDS + 1):
        forward = round_number % 2 == 1
        for position, trial in enumerate(trials if forward else reversed(trials), start=1):
            event = {'round': round_number, 'position': position, 'variant': trial[0],
                     'direction': 'forward' if forward else 'reverse'}
            yield event, trial


def percentiles(values):
    ordered = sorted(values)
    return statistics.median(ordered), ordered[math.ceil(0.95 * len(ordered)) - 1]


def summarize(rows, directory, labels, reference):
    by_label = {label: [row for row in rows if row['variant'] == label] for label in labels}
    table = ['| Variant | Boot to PID 1 p50/p95 (ms) | Launch to serial p50/p95 (ms) | '
             'Launch to bootstrap p50/p95 (ms) | Serial to bootstrap p50/p95 (ms) | '
             'Unordered/same-poll pairs | Kernel/entry criterion |',
             '| --- | ---: | ---: | ---: | ---: | ---: | --- |']
    floor = percentiles([row['pid1BootNs'] for row in by_label[reference]])
    for name, group in by_label.items():
        kernel = percentiles([row['pid1BootNs'] for row in group])
        fields = []
        for key in ['pid1BootNs', 'launchToPid1SerialNs', 'launchToBootstrapNs']:
            series = [row[key] for row in group if key in row]
            fields.append(' / '.join(f'{v / 1e6:.3f}' for v in percentiles(series)) if series else '—')
        phase = [row for row in group if 'serialToBootstrapNs' in row]
        ordered = [row['serialToBootstrapNs'] for row in phase
                   if row['serialToBootstrapNs'] >= 0 and not row['serialAndBootstrapSamePoll']]
        fields.append(' / '.join(f'{v / 1e6:.3f}' for v in percentiles(ordered)) if ordered else '—')
        fields.append(str(len(phase) - len(ordered)) if phase else '—')
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
    report = {**host(work), 'kernel': digest(kernel), 'monitor': digest(monitor),
              'realAgent': bootstrap['agentSha256'],
              'kernelHashes': {name: digest(file) for name, _, file, _ in trials},
              'images': {name: digest(path) for name, path in fixture.items()},
              'ordering': {'rounds': ROUNDS, 'oddRounds': labels, 'evenRounds': labels[::-1],
                           'forwardRounds': 8, 'reverseRounds': 7, 'positionBase': 1},
              'clocks': {'pid1BootNs': 'guest CLOCK_BOOTTIME at the first PID 1 clock read',
                         'hostIntervals': 'host monotonic_ns relative to monitor launch'},
              'samples': [], 'complete': False}
    def save():
        (work / 'samples.json').write_text(json.dumps(report, indent=2) + '\n')
    save()
    try:
        for event, (label, kind, image, extra) in schedule(trials):
            directory = work / f"{event['round'] - 1:02}-{label}"
            observation = sample(monitor, config, image, fixture[kind], directory,
                                 metadata['vmInitMode'], metadata['vmBootPort'], extra, kind == 'agent')
            report['samples'].append({**event, **observation})
            save()
        summarize(report['samples'], work, labels, labels[0])
        report['complete'] = True
    finally:
        save()


if __name__ == '__main__':
    main()

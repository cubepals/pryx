# SPDX-License-Identifier: FSL-1.1-ALv2
"""Retain the exact guest binary identities and native runner conditions for each cohort."""
import argparse
import hashlib
import json
from pathlib import Path
import platform
import subprocess

ROOT = Path(__file__).resolve().parents[2]
VM = ROOT / 'templates/vm-tools'


def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def optional(path):
    try:
        return path.read_text().strip()
    except FileNotFoundError:
        return None


def host(work):
    cpu = Path('/sys/devices/system/cpu/cpu0')
    huge = Path('/sys/kernel/mm/transparent_hugepage')
    return {
        'commit': subprocess.check_output(['git', '-C', ROOT, 'rev-parse', 'HEAD'], text=True).strip(),
        'host': platform.uname()._asdict(),
        'cpu': Path('/proc/cpuinfo').read_text().split('\n\n', 1)[0],
        'compiler': subprocess.check_output(['cc', '--version'], text=True).splitlines()[0],
        'rustc': subprocess.check_output(['rustc', '-vV'], text=True).strip(),
        'filesystem': subprocess.check_output(['stat', '-f', '-c', '%T', work], text=True).strip(),
        'cache': {index.name: {name: optional(index / name) for name in ('level', 'type', 'size')}
                  for index in sorted((cpu / 'cache').glob('index*'))},
        'frequencyGovernor': optional(cpu / 'cpufreq/scaling_governor'),
        'transparentHugePages': optional(huge / 'enabled'),
        'hugePageDefrag': optional(huge / 'defrag'),
        'hugePages': [s for s in Path('/proc/meminfo').read_text().splitlines() if s.startswith('Huge')],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('stage', type=Path)
    parser.add_argument('production_agent', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    root = args.stage / 'vm-agent-bootstrap'
    boot = json.loads((root / 'bootstrap.json').read_text())
    selection = json.loads((VM / 'kernel-selection.json').read_text())
    provenance = host(args.stage)
    for name in ['kernel', 'monitor']:
        expected = selection[name]['sha256']
        got = digest(args.stage / 'vm-selected-files' / expected)
        if got != expected:
            raise ValueError(f'{name} differs from committed selection')
        provenance[name] = got
    for path, name in [(args.production_agent, 'agentSha256'), (root / 'bootstrap.erofs', 'imageSha256')]:
        got = digest(path)
        if got != boot[name]:
            raise ValueError(f'{name} differs from trusted bootstrap manifest')
        provenance[name] = got
    args.output.write_text(json.dumps(provenance, indent=2) + '\n')


if __name__ == '__main__':
    main()

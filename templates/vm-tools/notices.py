#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Retain complete primary notices for the exact Go spike dependency closure.

The checksum-verified module cache supplies the original texts. Committed notice identities
must match before the fixture programs run; this does not package or license a product bundle.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

from inventory import ROOT


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--go', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--module-root', type=Path, default=ROOT)
    parser.add_argument('--catalog', type=Path, default=ROOT / 'spike-module-notices.json')
    parser.add_argument('--packages', nargs='+', default=['./cmd/config', './cmd/guest'])
    args = parser.parse_args()
    expected = json.loads(args.catalog.read_text())
    module_cache = Path(subprocess.check_output([str(args.go), 'env', 'GOMODCACHE'], text=True).strip())
    directories = subprocess.check_output([str(args.go), 'list', '-mod=readonly', '-deps', '-f',
                                            '{{if .Module}}{{.Module.Dir}}{{end}}',
                                            *args.packages], cwd=args.module_root, text=True).splitlines()
    actual = set()
    for value in sorted(set(directories)):
        if not value:
            continue
        path = Path(value)
        if path.resolve() in {ROOT.resolve(), args.module_root.resolve()}:
            continue
        identity = str(path.relative_to(module_cache))
        records = [item for item in expected if item['module'] == identity]
        if len(records) != 1:
            raise ValueError('Unreviewed experiment dependency: ' + identity)
        actual.add(identity)
        for notice in records[0]['notices']:
            data = (path / notice['name']).read_bytes()
            if hashlib.sha256(data).hexdigest() != notice['sha256']:
                raise ValueError('Experiment dependency notice changed: ' + identity)
            destination = args.output / identity / notice['name']
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(data)
    if actual != {item['module'] for item in expected}:
        raise ValueError('Experiment dependency closure differs from its reviewed inventory')
    (args.output / 'inventory.json').write_text(json.dumps(expected, indent=2) + '\n')
    print(f'Retained complete primary notices for {len(actual)} checksum-pinned modules')


if __name__ == '__main__':
    main()

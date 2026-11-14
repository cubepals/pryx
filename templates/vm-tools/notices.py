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
import shutil

from inventory import ROOT


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--go', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--module-root', type=Path, default=ROOT)
    parser.add_argument('--catalog', type=Path, default=ROOT / 'spike-module-notices.json')
    parser.add_argument('--packages', nargs='+', default=['./cmd/config', './cmd/guest'])
    parser.add_argument('--sources', type=Path, help='Retain authenticated original module archives too')
    args = parser.parse_args()
    expected = json.loads(args.catalog.read_text())
    module_cache = Path(subprocess.check_output([str(args.go), 'env', 'GOMODCACHE'], text=True).strip())
    output = subprocess.check_output([str(args.go), 'list', '-mod=readonly', '-deps', '-json=Module',
                                      *args.packages], cwd=args.module_root, text=True)
    modules = {}
    decoder = json.JSONDecoder()
    while output.strip():
        output = output.lstrip()
        package, end = decoder.raw_decode(output)
        output = output[end:]
        if module := package.get('Module'):
            modules[module['Dir']] = module
    actual = set()
    retained_sources = []
    for value, module in sorted(modules.items()):
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
        if args.sources is not None:
            # Go owns module identity, escaping, zip authentication and checksum format.
            downloaded = json.loads(subprocess.check_output(
                [str(args.go), 'mod', 'download', '-json', module['Path'] + '@' + module['Version']],
                cwd=args.module_root, text=True))
            if downloaded.get('Error') or not downloaded.get('Sum') or not downloaded.get('GoModSum'):
                raise ValueError('Go did not authenticate the original module archive')
            archive = Path(downloaded['Zip'])
            destination = args.sources / (identity + '.zip')
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(archive, destination)
            retained_sources.append({'module': module['Path'], 'version': module['Version'],
                                     'sum': downloaded['Sum'], 'goModSum': downloaded['GoModSum'],
                                     'archive': str(destination.relative_to(args.sources)),
                                     'sha256': hashlib.sha256(destination.read_bytes()).hexdigest()})
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
    if args.sources is not None:
        (args.sources / 'inventory.json').write_text(json.dumps(retained_sources, indent=2) + '\n')
    print(f'Retained complete primary notices for {len(actual)} checksum-pinned modules')


if __name__ == '__main__':
    main()

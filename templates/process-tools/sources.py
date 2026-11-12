#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Retain corresponding sources separately from the lazy runtime archive.

Source archives, complete package recipes/patches and notices stay unchanged. They are
data for redistribution, never executed, and the runtime records their pinned descriptor.
"""
import argparse
from contextlib import ExitStack
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import tomllib
import build
import package


def recipes(items, cache, output, extractor):
    for item in items:
        archive = build.acquire(item, cache)
        with tempfile.TemporaryDirectory(dir=cache) as temporary:
            payload = Path(temporary) / 'package'
            subprocess.run([str(extractor), 'package', 'extract', '--no-rc',
                            str(archive), str(payload)], check=True)
            source = payload / 'info/recipe'
            if not source.is_dir():
                raise ValueError(f"Missing corresponding build recipe: {item['name']}")
            target = output / 'recipes' / item['name']
            for path in sorted(source.rglob('*')):
                if path.is_file() or path.is_symlink():
                    build.copy_entry(path, target / path.relative_to(source), source)


def collect(runtime, cache, output):
    catalog = json.loads((build.ROOT / 'sources.lock.json').read_text())
    output.mkdir(parents=True, exist_ok=False)
    for item in catalog:
        archive = build.acquire(item, cache)
        destination = output / 'archives' / item['sha256'] / archive.name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(archive, destination)
    names = {entry['name'] for item in catalog for entry in item['packages']}
    packages = [item for lock in ['libraries.lock.json', 'helpers.lock.json']
                for item in json.loads((build.ROOT / lock).read_text())]
    missing = [item['name'] for item in packages
               if re.search('GPL|MPL|Sleepycat', item['declaredLicence']) and item['name'] not in names]
    if missing:
        raise ValueError(f'Missing corresponding-source inputs: {missing}')
    inputs = [item for item in packages if item['name'] in names]
    with ExitStack() as lifetime:
        recipes(inputs, cache, output, build.builder(cache, None, lifetime))
    # Exclude the previous descriptor: repeated collection must not hash itself.
    shutil.copytree(runtime / 'notices', output / 'notices',
                    ignore=shutil.ignore_patterns('source-distribution.json'))
    shutil.copy2(build.REPO / 'LICENSE.md', output / 'notices/pryx-LICENSE.md')
    shutil.copy2(build.ROOT / 'sources.lock.json', output / 'sources.lock.json')
    entries = []
    for path in sorted(output.rglob('*')):
        if path.is_symlink():
            if not path.resolve().is_relative_to(output):
                raise ValueError(f'Corresponding-source link escapes: {path}')
            entries.append(dict(path=str(path.relative_to(output)), link=str(path.readlink())))
        elif path.is_file():
            entries.append(dict(path=str(path.relative_to(output)), sha256=build.digest(path)))
    (output / 'manifest.json').write_text(json.dumps(dict(format=1, files=entries), indent=2) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--runtime', type=Path, required=True)
    parser.add_argument('--cache', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--archive', type=Path, required=True)
    parser.add_argument('--descriptor', type=Path, required=True)
    options = parser.parse_args()
    runtime = options.runtime.resolve()
    output = options.output.resolve()
    if output.is_relative_to(runtime) or runtime.is_relative_to(output):
        raise ValueError('Corresponding sources must be separate from runtime payloads')
    for path in [options.archive, options.descriptor]:
        if any(path.resolve().is_relative_to(root) for root in [runtime, output]):
            raise ValueError('Source archive and descriptor must be outside input directories')
    collect(runtime, options.cache, output)
    with (build.REPO / 'Cargo.toml').open('rb') as source:
        version = tomllib.load(source)['workspace']['package']['version']
    url = f'https://github.com/cubepals/pryx/releases/download/v{version}/pryx-process-tools-sources.tar.gz'
    descriptor = package.package(output, options.archive, url)
    options.descriptor.write_text(json.dumps(descriptor, indent=2) + '\n')
    record = dict(distribution=descriptor, inputs=json.loads((build.ROOT / 'sources.lock.json').read_text()))
    (runtime / 'notices/source-distribution.json').write_text(json.dumps(record, indent=2) + '\n')
    shutil.copy2(build.REPO / 'LICENSE.md', runtime / 'notices/pryx-LICENSE.md')
    build.inventory(runtime)
    print(json.dumps(descriptor))


if __name__ == '__main__':
    main()

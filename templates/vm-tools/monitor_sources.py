#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Retain native monitor source, LGPL libc source/recipes and primary runtime notices.

The separate compiler SDK is downloaded only for building and is not redistributed. Complete
libc and exact Buildroot recipes/configuration permit modification and relinking of this
separate Apache monitor. No copyleft library is linked into pryxd or the pryx agent.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tarfile

from inventory import ROOT, acquire
from sources import git_archive
import rust_notices


def download(item, cache, destination):
    digest = acquire(item['url'], item['bytes'], item['sha256'], cache=cache)
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(cache / (digest + '.archive'), destination)


def collect(args):
    component = args.component
    sources, notices = component / 'sources', component / 'notices'
    sources.mkdir()
    notices.mkdir()
    for path in args.source.iterdir():
        if path.is_file() and path.name.upper().startswith(('LICENSE', 'NOTICE', 'COPYRIGHT')):
            shutil.copyfile(path, notices / ('firecracker-' + path.name))
    cache = component.parent / 'source-cache'
    cache.mkdir()
    inputs = json.loads((ROOT / 'monitor-inputs.json').read_text())
    libc = inputs['sdk']['components']['glibc']
    archive = sources / Path(inputs['glibc']['url']).name
    download(inputs['glibc'], cache, archive)
    with tarfile.open(archive) as source:
        prefix = f'glibc-{libc}/'
        for member in source:
            if not member.name.startswith(prefix):
                continue
            relative = Path(member.name.removeprefix(prefix))
            if not member.isfile() or not relative.name.upper().startswith(('COPYING', 'LICENSE', 'NOTICE', 'COPYRIGHT')):
                continue
            if relative.is_absolute() or '..' in relative.parts or member.size > 2 * 1024 * 1024:
                raise ValueError('Unsafe primary libc notice')
            destination = notices / 'glibc' / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(source.extractfile(member).read(member.size))
    recipe = git_archive(inputs['recipe'], cache, sources)
    recipe_record = {**inputs['recipe'], 'archive': recipe.name,
                     'sha256': hashlib.file_digest(recipe.open('rb'), 'sha256').hexdigest()}
    for name, item in inputs['buildFiles'].items():
        download(item, cache, sources / 'compiler-recipe' / name)
    for name, item in inputs['notices'].items():
        download(item, cache, notices / name)
    rust_notices.collect(component, sources, manifest=args.source / 'Cargo.toml', target=inputs['target'],
                         root_package='firecracker', component='monitor-rust')
    sysroot = Path(subprocess.check_output(['rustc', '--print', 'sysroot'], text=True, timeout=10).strip())
    for name in ['COPYRIGHT.html', 'COPYRIGHT-library.html']:
        destination = notices / 'rust-standard-library' / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(sysroot / 'share/doc/rust' / name, destination)
    shutil.copytree(sysroot / 'share/doc/rust/licenses', notices / 'rust-standard-library/licenses')
    identity = json.loads((component / 'monitor.json').read_text())['source']
    original = args.original_sources / identity['archive']
    shutil.copyfile(original, sources / original.name)
    (sources / 'inputs.json').write_text(json.dumps([dict(identity, archive=original.name)], indent=2) + '\n')
    (sources / 'compiler-recipe.json').write_text(json.dumps(recipe_record, indent=2) + '\n')
    tools = sources / 'build-tools'
    tools.mkdir()
    for pattern in ['*.py', '*.json', '*.rs']:
        for path in ROOT.glob(pattern):
            shutil.copyfile(path, tools / path.name)
    shutil.copyfile(ROOT / 'monitor-REBUILD.md', sources / 'REBUILD.md')
    shutil.copyfile(ROOT / 'monitor-REBUILD.md', notices / 'REBUILD.md')
    (notices / 'inputs.json').write_text(json.dumps(inputs, indent=2) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['source', 'original-sources', 'component']:
        parser.add_argument('--' + name, type=Path, required=True)
    collect(parser.parse_args())


if __name__ == '__main__':
    main()

#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Retain original sources and notices for an actual resolved Linux Rust dependency graph.

Only normal/build edges are selected. Cargo.lock authenticates original registry archives;
git dependencies retain their full commit-verified tree. Archives are read as data, never
extracted onto host paths. The default remains the VM image helper's musl dependency graph.
"""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import tomllib
import urllib.parse

from inventory import ROOT
from sources import git_archive


def graph(manifest, target, root_package):
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--manifest-path', str(manifest),
        '--format-version', '1', '--filter-platform', target], timeout=180))
    nodes = {node['id']: node for node in metadata['resolve']['nodes']}
    root = metadata['resolve']['root'] if root_package is None else next(
        package['id'] for package in metadata['packages'] if package['name'] == root_package and package['source'] is None)
    selected, pending = set(), [root]
    while pending:
        identity = pending.pop()
        if identity in selected:
            continue
        selected.add(identity)
        pending.extend(dep['pkg'] for dep in nodes[identity]['deps']
                       if any(kind['kind'] != 'dev' for kind in dep['dep_kinds']))
    return [package for package in metadata['packages'] if package['id'] in selected and package['source'] is not None]


def original(package, checksums, directory, git_sources):
    identity = f"{package['name']}-{package['version']}"
    if package['source'].startswith('registry+'):
        root = Path(package['manifest_path']).parent
        archive = root.parent.parent.parent / 'cache' / root.parent.name / f'{identity}.crate'
        digest = hashlib.file_digest(archive.open('rb'), 'sha256').hexdigest()
        if digest != checksums[(package['name'], package['version'], package['source'])]:
            raise ValueError('VM dependency differs from its Cargo.lock archive')
        target = directory / archive.name
        shutil.copyfile(archive, target)
        return target, identity + '/', digest
    if not package['source'].startswith('git+'):
        raise ValueError('Unsupported VM Rust dependency source')
    if package['source'] not in git_sources:
        url = urllib.parse.urlsplit(package['source'].removeprefix('git+'))
        if url.scheme != 'https' or url.netloc != 'github.com':
            raise ValueError('Unsupported VM Rust git source origin')
        repository = url.path.removeprefix('/').removesuffix('.git')
        name = repository.rsplit('/', 1)[1]
        candidate = {'name': name, 'repository': repository, 'sourceCommit': url.fragment}
        archive = git_archive(candidate, directory, directory)
        git_sources[package['source']] = (archive, name + '/', hashlib.file_digest(archive.open('rb'), 'sha256').hexdigest())
    return git_sources[package['source']]


def notices(archive, prefix, package, directory):
    names = []
    with tarfile.open(archive) as source:
        for member in source:
            if member.isdir() and member.name == prefix.removesuffix('/'):
                continue
            if not member.name.startswith(prefix):
                raise ValueError('Unexpected Rust source archive prefix')
            relative = Path(member.name.removeprefix(prefix))
            notice = relative.name.upper().startswith(('LICENSE', 'LICENCE', 'COPYING', 'NOTICE', 'COPYRIGHT'))
            metadata_file = len(relative.parts) == 1 and relative.name in ('Cargo.toml', 'README.md', 'README')
            declared = package.get('license_file') == str(relative)
            if not member.isfile() or not (notice or metadata_file or declared):
                continue
            if relative.is_absolute() or '..' in relative.parts or member.size > 2 * 1024 * 1024:
                raise ValueError('Unsafe Rust primary-notice member')
            destination = directory / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(source.extractfile(member).read(member.size))
            names.append(str(relative))
    return sorted(names)


def collect(runtime, sources, *, manifest=None, target='x86_64-unknown-linux-musl', root_package=None, component='rust'):
    manifest = manifest or ROOT.parent.parent / 'vm-image/Cargo.toml'
    packages = graph(manifest, target, root_package)
    lock = tomllib.loads((manifest.parent / 'Cargo.lock').read_text())
    checksums = {(item['name'], item['version'], item.get('source')): item['checksum']
                 for item in lock['package'] if 'checksum' in item}
    source_directory, directory = sources / component, runtime / 'notices' / component
    source_directory.mkdir(parents=True, exist_ok=True)
    directory.mkdir(parents=True, exist_ok=True)
    inventory, git_sources = [], {}
    for package in packages:
        identity = f"{package['name']}-{package['version']}"
        archive, prefix, digest = original(package, checksums, source_directory, git_sources)
        names = notices(archive, prefix, package, directory / identity)
        inventory.append({'name': package['name'], 'version': package['version'], 'licence': package['license'],
                          'repository': package['repository'], 'source': package['source'],
                          'sourceSha256': digest, 'retained': names})
    (directory / 'inventory.json').write_text(json.dumps(inventory, indent=2) + '\n')

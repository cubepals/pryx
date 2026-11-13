#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Assemble unchanged VM filesystem programs with their musl closure and full sources.

The original Alpine package keeper verifies signatures before extraction. Only selected
programs and their actual ELF dependencies enter the component; no host packages, binary
rewriting, private glibc, privileged mounts or package scripts are involved.
"""
import argparse
from collections import deque
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import posixpath
import re
import shutil
import subprocess
import tarfile
import tempfile

from inventory import ROOT, acquire


def checked_file(archive, member, expected):
    if not member.isfile() or member.size != expected['bytes']:
        raise ValueError('Unexpected filesystem input notice')
    data = archive.extractfile(member).read(expected['bytes'] + 1)
    if hashlib.sha256(data).hexdigest() != expected['sha256']:
        raise ValueError('Filesystem input notice digest mismatch')
    return data


def local(item, cache):
    digest = acquire(item['url'], item['bytes'], item['sha256'], cache=cache)
    return cache / (digest + '.archive')


def source_inputs(catalog, cache, output):
    for item in [*catalog['sources'], *catalog['bootstrapSources'], *catalog['recipes']]:
        archive = local(item, cache)
        destination = output / 'sources' / item['origin'] / item['name']
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(archive, destination)
        if not item.get('archive'):
            continue
        with tarfile.open(archive) as source:
            for notice in catalog['notices']:
                if (notice['origin'], notice['archive']) != (item['origin'], item['name']):
                    continue
                relative = PurePosixPath(notice['path'])
                if relative.is_absolute() or '..' in relative.parts:
                    raise ValueError('Invalid notice destination')
                data = checked_file(source, source.getmember(notice['path']), notice)
                target = output / 'notices' / item['origin'] / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(data)


def bootstrap(archive, destination):
    # The already pinned image is trusted input. Still materialize only regular bytes
    # and resolve its archive links against the archive, never the host filesystem.
    with tarfile.open(archive) as source:
        members = {member.name.removeprefix('./'): member for member in source}
        for name, member in members.items():
            selected = (name == 'sbin/apk' or name.startswith('etc/apk/keys/')
                        or (name.startswith(('lib/', 'usr/lib/')) and '.so' in name))
            if not selected or member.isdir():
                continue
            if PurePosixPath(name).is_absolute() or '..' in PurePosixPath(name).parts:
                raise ValueError('Invalid bootstrap member')
            current = name
            for _ in range(16):
                entry = members[current]
                if entry.isfile():
                    break
                if entry.issym():
                    current = (entry.linkname.lstrip('/') if entry.linkname.startswith('/') else
                               posixpath.normpath(posixpath.join(posixpath.dirname(current), entry.linkname)))
                elif entry.islnk():
                    current = entry.linkname.removeprefix('./')
                else:
                    raise ValueError('Unsupported bootstrap member')
                if '..' in PurePosixPath(current).parts:
                    raise ValueError('Bootstrap link escapes its archive')
            else:
                raise ValueError('Cyclic bootstrap link')
            if entry.size > 16 * 1024 * 1024:
                raise ValueError('Oversized bootstrap executable or library')
            target = destination / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(source.extractfile(entry).read(entry.size + 1))
            target.chmod(0o755 if entry.mode & 0o111 else 0o644)


def command(root, program, *arguments):
    return [str(root / 'libraries/ld-musl-x86_64.so.1'), '--library-path',
            str(root / 'libraries'), str(root / 'helpers' / program), *map(str, arguments)]


def component(packages, output):
    tools = output / 'tools'
    (tools / 'helpers').mkdir(parents=True)
    (tools / 'libraries').mkdir()
    pending = deque([('usr/bin/mkfs.erofs', 'helpers/mkfs.erofs'),
                     ('usr/bin/fsck.erofs', 'helpers/fsck.erofs'),
                     ('sbin/mke2fs', 'helpers/mke2fs'), ('sbin/e2fsck', 'helpers/e2fsck'),
                     ('lib/ld-musl-x86_64.so.1', 'libraries/ld-musl-x86_64.so.1')])
    while pending:
        original, relative = pending.popleft()
        target = tools / relative
        if target.exists():
            continue
        source = (packages / original).resolve(strict=True)
        source.relative_to(packages.resolve())
        if not source.is_file():
            raise ValueError('Filesystem component requires regular ELF files')
        shutil.copyfile(source, target)
        target.chmod(0o755)
        dynamic = subprocess.check_output(['readelf', '--wide', '--dynamic', str(target)], text=True, timeout=10)
        for name in re.findall(r'\(NEEDED\).*\[([^\]]+)\]', dynamic):
            if Path(name).name != name:
                raise ValueError('ELF dependency must be a library name')
            candidates = [base / name for base in [packages / 'lib', packages / 'usr/lib']]
            library = next((path for path in candidates if path.exists()), None)
            if library is None:
                raise ValueError('Missing filesystem ELF dependency: ' + name)
            pending.append((str(library.relative_to(packages)), 'libraries/' + name))
    (tools / 'config').mkdir()
    shutil.copyfile(packages / 'etc/mke2fs.conf', tools / 'config/mke2fs.conf')
    for program, option in [('mkfs.erofs', '--version'), ('mke2fs', '-V')]:
        subprocess.run(command(tools, program, option), check=True, timeout=10)
    return tools


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--cache', type=Path, required=True)
    args = parser.parse_args()
    args.root = args.root.resolve()
    args.cache = args.cache.resolve()
    args.root.mkdir(mode=0o700, parents=True, exist_ok=False)
    catalog = json.loads((ROOT / 'filesystem-inputs.json').read_text())
    if os.uname().machine != catalog['architecture']:
        raise ValueError('This filesystem component requires ' + catalog['architecture'])
    source_inputs(catalog, args.cache, args.root)
    with tempfile.TemporaryDirectory(prefix='filesystem-', dir=args.root) as temporary:
        staging = Path(temporary)
        verifier = staging / 'bootstrap'
        bootstrap(local(catalog['bootstrap'], args.cache), verifier)
        packages = [local(item, args.cache) for item in catalog['packages']]
        extracted = staging / 'packages'
        extracted.mkdir()
        keeper = [str(verifier / 'lib/ld-musl-x86_64.so.1'), '--library-path',
                  str(verifier / 'lib') + ':' + str(verifier / 'usr/lib'), str(verifier / 'sbin/apk')]
        empty_keys = staging / 'untrusted-keys'
        empty_keys.mkdir()
        refusal = subprocess.run([*keeper, '--keys-dir', str(empty_keys), '--no-network',
                                  'verify', *map(str, packages)], timeout=60, capture_output=True, text=True)
        if refusal.returncode == 0:
            raise ValueError('Package keeper accepts packages without their trusted signing keys')
        print(json.dumps({'untrustedKeyRefusal': refusal.returncode,
                          'ownerDiagnostic': refusal.stdout + refusal.stderr}), flush=True)
        subprocess.run([*keeper, '--keys-dir', str(verifier / 'etc/apk/keys'),
                        '--no-network', 'extract', '--no-chown', '--destination', str(extracted),
                        *map(str, packages)], check=True, timeout=60)
        tools = component(extracted, args.root)
    inventory = []
    for path in sorted(tools.rglob('*')):
        if path.is_file():
            inventory.append({'path': str(path.relative_to(args.root)), 'bytes': path.stat().st_size,
                              'sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
                              'mode': path.stat().st_mode & 0o777})
    (args.root / 'filesystem-manifest.json').write_text(json.dumps(inventory, indent=2) + '\n')
    shutil.copyfile(ROOT / 'filesystem-inputs.json', args.root / 'filesystem-inputs.json')
    print(json.dumps({'component': 'vm-filesystems', 'files': len(inventory),
                      'sources': len(catalog['sources']), 'root': str(args.root)}), flush=True)


if __name__ == '__main__':
    main()

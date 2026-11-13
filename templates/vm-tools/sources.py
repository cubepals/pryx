#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Retain the published kernel's complete corresponding source and exact packaging recipes.

Git verifies the committed upstream tree; archives contain the full tree without checking it
out or executing it. Linux's original archive is checked against its primary published hash.
These are redistribution inputs, never a kernel build or a replacement for real boot proof.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile

from inventory import ROOT, acquire


def git_archive(candidate, cache, output):
    repository, commit = candidate['repository'], candidate['sourceCommit']
    if not re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', repository) or not re.fullmatch(r'[0-9a-f]{40}', commit):
        raise ValueError('Invalid pinned VM source identity')
    destination = output / f"{candidate['name']}-{commit}.tar.gz"
    environment = {**os.environ, 'GIT_TERMINAL_PROMPT': '0'}
    with tempfile.TemporaryDirectory(dir=cache) as temporary:
        checkout = Path(temporary)
        def git(*arguments, capture=False):
            result = subprocess.run(['git', '-C', str(checkout), *arguments], env=environment,
                                    check=True, timeout=180, stdout=subprocess.PIPE if capture else None)
            return result.stdout.decode().strip() if capture else None
        git('init', '--bare')
        git('fetch', '--depth=1', '--no-tags', f'https://github.com/{repository}.git', commit)
        if git('rev-parse', 'FETCH_HEAD^{commit}', capture=True) != commit:
            raise ValueError('VM source revision differs from its committed identity')
        git('fsck', '--strict', '--no-reflogs')
        git('archive', '--format=tar.gz', f'--prefix={candidate["name"]}/',
            f'--output={destination.resolve()}', commit)
    return destination


def notice(archive, member, output):
    # Never extract archive paths or links onto a host. This one original text has
    # an application-owned destination and a strict regular-file/size bound.
    with tarfile.open(archive) as source:
        entry = source.getmember(member)
        if not entry.isfile() or not 0 < entry.size <= 1024 * 1024:
            raise ValueError('VM source licence must be a bounded regular file')
        data = source.extractfile(entry).read(entry.size + 1)
        if len(data) != entry.size:
            raise ValueError('Incomplete primary VM source licence')
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_bytes(data)


def collect(cache, output):
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    cache.mkdir(mode=0o700, parents=True, exist_ok=True)
    sources = output / 'sources'
    sources.mkdir()
    records = []
    kernel = json.loads((ROOT / 'kernel-source.json').read_text())
    digest = acquire(kernel['url'], kernel['bytes'], kernel['sha256'], cache=cache)
    archive = sources / Path(kernel['url']).name
    shutil.copyfile(cache / f'{digest}.archive', archive)
    notice(archive, f'linux-{kernel["version"]}/COPYING', output / 'notices/linux/COPYING')
    # COPYING references Linux's SPDX exceptions and GPL text. Retain all primary
    # licence texts as data too; complete original sources remain alongside them.
    with tarfile.open(archive) as source:
        prefix = f'linux-{kernel["version"]}/LICENSES/'
        for entry in source:
            if entry.name.startswith(prefix) and entry.isfile():
                relative = Path(entry.name.removeprefix(prefix))
                if relative.is_absolute() or '..' in relative.parts or entry.size > 1024 * 1024:
                    raise ValueError('Invalid Linux primary licence member')
                target = output / 'notices/linux/LICENSES' / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(source.extractfile(entry).read(entry.size))
    records.append({**kernel, 'archive': str(archive.relative_to(output))})
    for candidate in json.loads((ROOT / 'candidates.json').read_text()):
        archive = git_archive(candidate, cache, sources)
        notice(archive, f'{candidate["name"]}/LICENSE', output / 'notices' / candidate['name'] / 'LICENSE')
        records.append({'repository': candidate['repository'], 'commit': candidate['sourceCommit'],
                        'archive': str(archive.relative_to(output)), 'bytes': archive.stat().st_size,
                        'sha256': hashlib.file_digest(archive.open('rb'), 'sha256').hexdigest()})
    (output / 'inputs.json').write_text(json.dumps(records, indent=2) + '\n')
    print(json.dumps({'scope': 'Complete pinned kernel, recipes/patches and monitor sources; no kernel built',
                      'inputs': records}), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cache', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    collect(args.cache, args.output)


if __name__ == '__main__':
    main()

#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Materialize a private measurement build from stock source and the selected lab revision.

Only explicitly dispatched measurements call this program. Every modified input
must be the exact original blob named by the fixture patch. The tracked production
guest, daemon, Cargo manifests and release recipes retain their stock bytes.
"""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile

STOCK = '35c28a630404d34fcc451528369432a54abba496'
HERE = Path(__file__).resolve().parents[2]


def git(repo, *args):
    return subprocess.check_output(['git', '-C', repo, *args])


def apply(stage, name):
    file = stage / 'experimental/guest' / f'{name}-fixture.patch'
    data = file.read_bytes()
    pairs = re.findall(rb'^diff --git a/(\S+) b/\1\nindex ([0-9a-f]{40})\.\.([0-9a-f]{40}) [0-7]+$', data, re.M)
    if len(pairs) != data.count(b'diff --git ') or not pairs:
        raise ValueError(f'{name} must modify only existing regular stock sources')
    for raw, expected, _ in pairs:
        path = raw.decode()
        member = stage / path
        if not member.is_file() or member.is_symlink() or not member.resolve().is_relative_to(stage):
            raise ValueError(f'Invalid original fixture path: {path}')
        if git(stage, 'hash-object', member).strip() != expected:
            raise ValueError(f'The original {name} fixture source changed: {path}')
    git(stage, 'apply', '--check', '--unidiff-zero', '--index', file)
    git(stage, 'apply', '--unidiff-zero', '--index', file)
    for raw, _, result in pairs:
        if git(stage, 'hash-object', stage / raw.decode()).strip() != result:
            raise ValueError('The staged fixture bytes differ from their target blob')
    return hashlib.sha256(data).hexdigest()


def prepare(repo, lab_revision, stage, candidate):
    stage = stage.resolve()
    if stage.exists():
        raise FileExistsError(stage)
    if subprocess.run(['git','-C',repo,'cat-file','-e',f'{STOCK}^{{commit}}'],
                      stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode:
        git(repo, 'fetch', '--no-tags', '--depth=1', 'https://github.com/cubepals/pryx.git', STOCK)
    if git(repo, 'rev-parse', f'{STOCK}^{{commit}}').decode().strip() != STOCK:
        raise ValueError('Unable to materialize the pinned production commit')
    git(repo, 'worktree', 'add', '--detach', stage, STOCK)
    archive = git(repo, 'archive', lab_revision, 'experimental/Cargo.toml', 'experimental/guest')
    with tarfile.open(fileobj=io.BytesIO(archive)) as lab:
        for entry in lab:
            if not (entry.isdir() or entry.isfile()) or (entry.name not in ('experimental','experimental/Cargo.toml','experimental/guest') and not entry.name.startswith('experimental/guest/')):
                raise ValueError('The experimental sources must contain only regular files or directories')
        lab.extractall(stage, filter='data')
    patches = {'host': apply(stage, 'host')}
    if candidate:
        patches['guest'] = apply(stage, 'guest')
        shutil.copytree(stage / 'experimental/guest/candidate/request', stage / 'templates/vm-tools/cmd/lab-gate-proof')
    receipt = {'stockCommit':STOCK, 'labRevision':lab_revision, 'patchSha256':patches}
    (stage / 'lab-source.json').write_text(json.dumps(receipt, indent=2)+'\n')


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('directory', type=Path)
    p.add_argument('--candidate', action='store_true')
    args = p.parse_args()
    os.umask(0o077)
    prepare(HERE, git(HERE, 'rev-parse','HEAD').decode().strip(), args.directory, args.candidate)

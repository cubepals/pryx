#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Inspect verified upstream VM archives without executing their contents.

Only bounded regular kernel/monitor files are staged, under content-addressed names. Archive
paths and links never become host paths. Configuration records and member identities are
retained for the subsequent real-KVM experiment; this step cannot establish boot support.
"""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import subprocess
import tarfile
import tempfile

from inventory import READ_BYTES

MAX_MEMBERS = 100_000
MAX_FILE_BYTES = 512 * 1024 * 1024
MAX_STAGED_BYTES = 1024 * 1024 * 1024
MAX_CONFIG_BYTES = 2 * 1024 * 1024
MAX_EXPANDED_BYTES = 16 * 1024 * 1024 * 1024


def inspect(candidate, cache, destination, zstd='zstd'):
    archive_path = cache / (candidate['sha256'] + '.archive')
    with archive_path.open('rb') as source:
        if hashlib.file_digest(source, 'sha256').hexdigest() != candidate['sha256']:
            raise ValueError('Staged archive does not match its verified identity')
    result = {'name': candidate['name'], 'archiveSha256': candidate['sha256'],
              'sourceCommit': candidate['sourceCommit'], 'members': [], 'executed': False}
    decoder = None
    if candidate['assetName'].endswith('.zst'):
        decoder = subprocess.Popen([str(zstd), '-dc', '--', str(archive_path)], stdout=subprocess.PIPE)
        stream = decoder.stdout
        mode = 'r|'
    else:
        stream = archive_path.open('rb')
        mode = 'r|*'
    staged, expanded = 0, 0
    try:
        with tarfile.open(fileobj=stream, mode=mode) as archive:
            for index, member in enumerate(archive):
                expanded += member.size
                if index >= MAX_MEMBERS or expanded > MAX_EXPANDED_BYTES:
                    raise ValueError('VM archive expansion exceeds its inspection bound')
                basename = PurePosixPath(member.name).name
                kernel = basename.startswith(('vmlinux', 'vmlinuz'))
                monitor = re.fullmatch(r'(?:firecracker|jailer)-v[0-9.]+-x86_64', basename)
                config = basename.startswith(('config-', 'configuration-')) or basename == '.config'
                interesting = kernel or monitor or config or 'kernel' in basename.lower()
                if not interesting:
                    continue
                entry = {'path': member.name, 'bytes': member.size,
                         'kind': 'file' if member.isfile() else 'link' if member.issym() else 'other'}
                if member.issym():
                    entry['target'] = member.linkname
                if member.isfile() and (kernel or monitor or config):
                    bound = MAX_CONFIG_BYTES if config else MAX_FILE_BYTES
                    if member.size > bound or staged + member.size > MAX_STAGED_BYTES:
                        raise ValueError('VM selected member exceeds its inspection bound')
                    staged += member.size
                    digest = hashlib.sha256()
                    with archive.extractfile(member) as source:
                        with tempfile.NamedTemporaryFile(dir=destination) as output:
                            while block := source.read(READ_BYTES):
                                digest.update(block)
                                output.write(block)
                            output.flush()
                            entry['sha256'] = digest.hexdigest()
                            if config:
                                output.seek(0)
                                entry['configuration'] = output.read().decode('utf-8')
                            else:
                                output.seek(0)
                                entry['prefixHex'] = output.read(16).hex()
                                staged_path = destination / digest.hexdigest()
                                # Copy regular bytes only; retain no archive path, link or executable bit.
                                if staged_path.exists():
                                    with staged_path.open('rb') as existing:
                                        if hashlib.file_digest(existing, 'sha256').hexdigest() != digest.hexdigest():
                                            raise ValueError('Existing staged member differs from its identity')
                                else:
                                    with staged_path.open('xb') as staged_file:
                                        output.seek(0)
                                        while block := output.read(READ_BYTES):
                                            staged_file.write(block)
                                staged_path.chmod(0o600)
                result['members'].append(entry)
        # Drain the bounded tar trailer before waiting for the external decoder's successful exit.
        if len(stream.read(MAX_CONFIG_BYTES + 1)) > MAX_CONFIG_BYTES:
            raise ValueError('Archive trailer exceeds its inspection bound')
        if decoder is not None and decoder.wait(timeout=30):
            raise ValueError('Archive decoder failed')
    finally:
        stream.close()
        if decoder is not None:
            if decoder.poll() is None:
                decoder.terminate()
            decoder.wait()
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--inventory', type=Path, required=True)
    parser.add_argument('--archive-cache', type=Path, required=True)
    parser.add_argument('--staged', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--zstd', type=Path, required=True)
    args = parser.parse_args()
    args.staged.mkdir(parents=True, exist_ok=True, mode=0o700)
    report = {'scope': 'Archive inspection only; no kernel or monitor executed', 'candidates': []}
    for candidate in json.loads(args.inventory.read_text())['candidates']:
        result = inspect(candidate, args.archive_cache, args.staged, args.zstd)
        report['candidates'].append(result)
        print(json.dumps(result), flush=True)
        args.output.write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    main()

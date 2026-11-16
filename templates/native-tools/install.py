# SPDX-License-Identifier: FSL-1.1-ALv2
"""Install audited immutable Debian archives, never resolve packages or start daemons.

Importing this module exposes only verified archive retrieval; installation runs from main.
"""
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import pathlib
import subprocess
import sys
import tempfile

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
import archive_retry


def download(package, temporary, cache=pathlib.Path('/build-cache')):
    path = pathlib.Path(temporary, package['package'] + '.deb')
    cached = cache / package['archive']
    if cached.is_file():
        data = cached.read_bytes()
    else:
        data = archive_retry.request(package['url'], lambda response: response.read())
    if hashlib.sha256(data).hexdigest() != package['sha256']:
        raise SystemExit('Package hash mismatch: ' + package['package'])
    path.write_bytes(data)
    print('Verified', package['package'], flush=True)
    return str(path)


def main():
    with tempfile.TemporaryDirectory(prefix='pryx-debs-') as temporary:
        packages = json.loads(pathlib.Path(__file__).with_name('debian.lock.json').read_text())
        with ThreadPoolExecutor(max_workers=4) as pool:
            archives = list(pool.map(lambda package: download(package, temporary), packages))
        # Daemons must never start or leave host keys in a build layer.
        policy = pathlib.Path('/usr/sbin/policy-rc.d')
        policy.write_text('#!/bin/sh\nexit 101\n')
        policy.chmod(0o755)
        subprocess.run(['dpkg', '--install', *archives], check=True)
        for key in pathlib.Path('/etc/ssh').glob('ssh_host_*'):
            key.unlink()
        policy.unlink()
        pathlib.Path('/run/sshd').mkdir(mode=0o755, exist_ok=True)
        pathlib.Path('/mnt').chmod(0o777)
        subprocess.run(['useradd', '--uid', '1000', '--home-dir', '/data', '--shell', '/bin/sh',
                        '--password', 'x', 'pryx'], check=True)
        # Nested user namespaces supply mount authority; setuid would discard ambient caps.
        # Execute-only, as the mount helpers that run it with credentials in their environment.
        pathlib.Path('/bin/fusermount3').chmod(0o111)


if __name__ == '__main__':
    main()

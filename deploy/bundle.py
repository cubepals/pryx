# SPDX-License-Identifier: FSL-1.1-ALv2
"""Pack the release image's programs and runtime closure for ordinary Linux users.

This runs only in the pinned image build; it does not provision the destination host,
sign artifacts, install packages, or replace sandbox confinement with a root filesystem.
"""
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile


def command(*args, **options):
    return subprocess.check_output(args, text=True, **options).strip()


class Bundle:
    def __init__(self, root):
        self.root = root
        self.libraries = root / 'lib'
        self.libraries.mkdir(parents=True)
        self.packages = set()

    def library(self, path):
        source = Path(path)
        shutil.copy2(source, self.libraries / source.name, follow_symlinks=True)
        self.owner(source)

    def owner(self, source):
        for candidate in [source, source.resolve()]:
            found = subprocess.run(['dpkg-query', '-S', str(candidate)], text=True,
                                   capture_output=True, check=False)
            for line in found.stdout.splitlines():
                self.packages.add(line.rsplit(': ', 1)[0])
            if found.returncode == 0:
                break

    def closure(self, binary):
        result = subprocess.run(['ldd', str(binary)], text=True, capture_output=True,
                                env={**os.environ, 'LD_LIBRARY_PATH': str(self.root / 'postgres/lib')},
                                check=False)
        if 'not found' in result.stdout:
            raise RuntimeError(f'Incomplete runtime closure for {binary}: {result.stdout}')
        for path in re.findall(r'(?:=>\s*|^\s*)(/[^\s]+)', result.stdout, re.M):
            self.library(path)

    def executable(self, source, target, postgres=False):
        self.owner(Path(source))
        target = self.root / target
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)
        with Path(source).open('rb') as stream:
            executable_format = stream.read(4)
        if executable_format != b'\x7fELF':
            return
        self.closure(target)
        dynamic = subprocess.run(['ldd', str(target)], capture_output=True, text=True,
                                 check=False).returncode == 0
        if not dynamic:
            return
        target.rename(str(target) + '.real')
        parent = '../..' if postgres else '..'
        preload = '''if [ -n "${NSS_WRAPPER_PASSWD:-}" ]; then
  exec "$bundle/lib/ld-linux-x86-64.so.2" --library-path "$bundle/postgres/lib:$bundle/lib" --preload "$bundle/lib/libnss_wrapper.so" --argv0 "$0" "$0.real" "$@"
fi
''' if postgres else ''
        target.write_text('''#!/bin/sh
set -eu
bundle=$(CDPATH= cd -- "$(dirname -- "$0")/''' + parent + '''" && pwd)
export PRYX_BUNDLE_DIR="$bundle"
export PRYX_ASSET_DIR="${PRYX_ASSET_DIR:-$bundle/control}"
''' + preload + '''exec "$bundle/lib/ld-linux-x86-64.so.2" --library-path "$bundle/postgres/lib:$bundle/lib" --argv0 "$0" "$0.real" "$@"
''')
        target.chmod(0o755)

    def notices(self):
        directory = self.root / 'licenses/runtime'
        directory.mkdir(parents=True)
        inventory = []
        for package in sorted(self.packages):
            name = package.split(':')[0]
            copyright_file = Path('/usr/share/doc') / name / 'copyright'
            if not copyright_file.exists():
                raise RuntimeError(f'Missing runtime copyright: {package}')
            shutil.copy2(copyright_file, directory / (name + '.txt'))
            inventory.append(command('dpkg-query', '-W', '-f=${binary:Package} ${Version} ${source:Package} ${source:Version}', package))
        (directory / 'packages.txt').write_text('\n'.join(inventory) + '\n')
        (directory / 'SOURCES.txt').write_text('Unmodified Debian runtime programs/libraries.\n'
            'Exact source package names and versions are in packages.txt.\n'
            'Corresponding source: https://sources.debian.org/ and https://snapshot.debian.org/\n')


def postgres(bundle, specification):
    with tempfile.TemporaryDirectory() as temporary:
        archive = Path(temporary) / 'postgres.tar.gz'
        subprocess.run(['curl', '--proto', '=https', '--proto-redir', '=https', '--fail',
                        '--location', '--retry', '12', '--retry-delay', '2',
                        '--retry-max-time', '180', '--connect-timeout', '15',
                        '--max-time', '60', specification['url'], '-o', str(archive)], check=True)
        if hashlib.sha256(archive.read_bytes()).hexdigest() != specification['sha256']:
            raise RuntimeError('PostgreSQL archive digest mismatch')
        extracted = Path(temporary) / 'extracted'
        with tarfile.open(archive) as package:
            package.extractall(extracted, filter='data')
        directories = list(extracted.iterdir())
        if len(directories) != 1 or not directories[0].is_dir():
            raise RuntimeError('Unexpected PostgreSQL archive layout')
        shutil.copytree(directories[0], bundle.root / 'postgres')
    for binary in list((bundle.root / 'postgres/bin').iterdir()):
        original = binary.with_suffix('.original')
        binary.rename(original)
        bundle.executable(original, binary.relative_to(bundle.root), postgres=True)
        original.unlink()


def pack(root, output):
    with tarfile.open(output, 'w:gz') as archive:
        for path in sorted(root.rglob('*')):
            info = archive.gettarinfo(str(path), str(path.relative_to(root)))
            info.uid = info.gid = info.mtime = 0
            info.uname = info.gname = ''
            if info.isfile():
                with path.open('rb') as stream:
                    archive.addfile(info, stream)
            else:
                archive.addfile(info)


def main():
    destination = Path(sys.argv[1])
    destination.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as temporary:
        bundle = Bundle(Path(temporary) / 'bundle')
        specification = json.loads(Path('/source/deploy/postgresql.json').read_text())
        postgres(bundle, specification)
        for source, target in [('/pryxd', 'pryxd'),
                ('/usr/local/lib/pryx/pryx-agent', 'pryx-agent'),
                ('/usr/local/bin/pryx', 'pryx'),
                ('/opt/pryx/pryx-control', 'pryx-control'),
                ('/opt/pryx/pryx-edge', 'pryx-edge'),
                ('/opt/pryx/pryx-image', 'pryx-image'),
                ('/usr/local/bin/rustfs', 'rustfs'),
                ('/usr/bin/setpriv', 'setpriv'),
                ('/usr/bin/flock', 'flock')]:
            bundle.executable(source, 'bin/' + target)
        bundle.library('/usr/lib/x86_64-linux-gnu/libnss_wrapper.so')
        bundle.closure('/usr/lib/x86_64-linux-gnu/libnss_wrapper.so')
        shutil.copytree('/usr/local/lib/pryx/agentos', bundle.root / 'agentos')
        shutil.copytree('/opt/pryx/control', bundle.root / 'control')
        shutil.copytree('/licenses', bundle.root / 'licenses', dirs_exist_ok=True)
        shutil.copy2('/LICENSE.md', bundle.root / 'licenses/pryx.txt')
        postgres_user = next(line.split(':') for line in Path('/etc/passwd').read_text().splitlines()
                             if line.startswith('postgres:'))
        metadata = {'version': command('/usr/local/bin/pryx', '--version').split()[1],
                    'architecture': 'x86_64', 'postgres': specification,
                    'serviceUid': int(postgres_user[2]), 'serviceGid': int(postgres_user[3])}
        (bundle.root / 'bundle.json').write_text(json.dumps(metadata, indent=2) + '\n')
        bundle.notices()
        pack(bundle.root, destination / 'pryx-bundle-linux-x64.tar.gz')
        bootstrap = Path(temporary) / 'bootstrap'
        (bootstrap / 'bin').mkdir(parents=True)
        for name in ['pryx', 'pryx.real', 'pryxd', 'pryx-agent']:
            shutil.copy2(bundle.root / 'bin' / name, bootstrap / 'bin' / name)
        shutil.copytree(bundle.root / 'lib', bootstrap / 'lib')
        shutil.copytree(bundle.root / 'licenses', bootstrap / 'licenses')
        pack(bootstrap, destination / 'pryx-bootstrap-linux-x64.tar.gz')


if __name__ == '__main__':
    main()

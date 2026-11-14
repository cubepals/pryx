#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Build the separate upstream nft batch program against verified musl packages.

The original source and build system are unchanged. Its documented no-interactive/no-JSON
configuration uses embedded mini-GMP under its GPLv2 option. No copyleft code enters pryx,
no host packages are installed, and only actual runtime ELF dependencies enter the component.
"""
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tarfile

# musl's documented x86_64 ELF interpreter ABI; the existing component runs it explicitly.
INTERPRETER = '/lib/ld-musl-x86_64.so.1'


def wrapper(path, arguments):
    path.write_text('#!/bin/sh\nexec ' + shlex.join(list(map(str, arguments))) + ' "$@"\n')
    path.chmod(0o700)
    return str(path)


def build(catalog, packages, sources, work):
    definition = catalog['firewall']
    expected = next(item for item in catalog['sources']
                    if (item['origin'], item['name']) == (definition['origin'], definition['archive']))
    archive = sources / definition['origin'] / definition['archive']
    if hashlib.sha256(archive.read_bytes()).hexdigest() != expected['sha256']:
        raise ValueError('Firewall source differs from its pinned original archive')
    extracted = work / 'firewall-source'
    extracted.mkdir()
    with tarfile.open(archive) as source:
        source.extractall(extracted, filter='data')
    source = extracted / f'nftables-{definition["version"]}'
    compiler = shutil.which('cc')
    if compiler is None or shutil.which('m4') is None:
        raise ValueError('The firewall build requires the host C compiler and m4')
    driver = wrapper(work / 'firewall-cc', [compiler, f'--sysroot={packages}',
                     f'-B{packages}/usr/lib/', '-static-libgcc', f'-Wl,--dynamic-linker={INTERPRETER}'])
    lexer = wrapper(work / 'firewall-flex', [packages / INTERPRETER.lstrip('/'), '--library-path',
                    packages / 'lib', packages / 'usr/bin/flex'])
    environment = {
        'PATH': os.environ['PATH'], 'LC_ALL': 'C', 'SOURCE_DATE_EPOCH': '0',
        'CC': driver, 'LEX': lexer, 'M4': shutil.which('m4'),
        'PKG_CONFIG_LIBDIR': str(packages / 'usr/lib/pkgconfig'),
        'PKG_CONFIG_SYSROOT_DIR': str(packages),
        'CFLAGS': '-O2 -fstack-protector-strong', 'LDFLAGS': '-Wl,-z,relro,-z,now',
    }
    output = work / 'firewall-build'
    output.mkdir()
    configure = [str(source / 'configure'), '--host=x86_64-linux-musl',
                 '--build=x86_64-pc-linux-gnu', *definition['configure']]
    subprocess.run(configure, cwd=output, env=environment, check=True, timeout=120)
    subprocess.run(['make', '-j2'], cwd=output, env=environment, check=True, timeout=180)
    binaries = output / 'src/.libs'
    target = packages / 'usr/sbin/nft'
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(binaries / 'nft', target)
    target.chmod(0o755)
    for library in binaries.glob('libnftables.so*'):
        original = library.resolve(strict=True)
        original.relative_to(binaries)
        shutil.copyfile(original, packages / 'usr/lib' / library.name)
    recipe = {
        'source': expected, 'configuration': definition,
        'compilerArguments': ['--sysroot=BUILD_PACKAGES', '-BBUILD_PACKAGES/usr/lib/',
                              '-static-libgcc', f'-Wl,--dynamic-linker={INTERPRETER}'],
        'configure': ['NFT_SOURCE/configure', '--host=x86_64-linux-musl',
                      '--build=x86_64-pc-linux-gnu', *definition['configure']],
        'lexer': ['BUILD_PACKAGES' + INTERPRETER, '--library-path',
                  'BUILD_PACKAGES/lib', 'BUILD_PACKAGES/usr/bin/flex'],
        'pkgConfig': {'PKG_CONFIG_LIBDIR': 'BUILD_PACKAGES/usr/lib/pkgconfig',
                      'PKG_CONFIG_SYSROOT_DIR': 'BUILD_PACKAGES'},
        'environment': {key: value for key, value in environment.items()
                        if key in ('CFLAGS', 'LDFLAGS', 'SOURCE_DATE_EPOCH', 'LC_ALL')},
        'build': ['make', '-j2'], 'target': 'x86_64-linux-musl',
    }
    (sources / definition['origin'] / 'pryx-build.json').write_text(json.dumps(recipe, indent=2) + '\n')

#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""What an aarch64 process tool bundle and VM bundle would take, tried on an arm64 host.

Evidence only, run by the Any host arm64 job: each stage fetches the aarch64 counterparts of the
x86_64 bundles' pinned inputs, checked against the digest their upstream publishes, and runs or
resolves them, printing one JSON line per stage. It builds no bundle, installs nothing and changes
no host policy; docs/decisions.md records what the stages found.

Stages:
- tool-runtimes: the browser, Python and Node of templates/process-tools/artifacts-aarch64.json,
  run on this host.
- tool-libraries: every conda-forge package the tool bundle pins (libraries, helper, micromamba
  builder) at the same version for linux-aarch64.
- tool-desktop: every Debian package the desktop pins at the same version for arm64, and every
  Python wheel the kernels pin for manylinux aarch64.
- vm-monitor: Firecracker's aarch64 release at the pinned tag, run on this host.
- vm-kernel: Kata Containers' arm64 static release at the pinned tag: its guest kernel, and the
  configuration the x86_64 selection requires (kernel-selection.json).
- vm-bootstrap: the guest agent built for aarch64-unknown-linux-musl (by the caller, beside this).
"""
import argparse
import bz2
import hashlib
import json
import lzma
import os
from pathlib import Path
import re
import subprocess
import tarfile
import tempfile
import urllib.request
import zipfile

REPO = Path(__file__).resolve().parent.parent
TOOLS = REPO / 'templates/process-tools'
VM = REPO / 'templates/vm-tools'
# conda-forge's subdirectory for this machine (conda's platform names).
CONDA_SUBDIR = 'linux-aarch64'
# Debian's architecture name for this machine, and the suites the desktop locks draw from.
DEBIAN_ARCH = 'arm64'
DEBIAN_INDEXES = [
    'https://deb.debian.org/debian/dists/bookworm/main/binary-arm64/Packages.xz',
    'https://deb.debian.org/debian/dists/bookworm-updates/main/binary-arm64/Packages.xz',
    'https://deb.debian.org/debian-security/dists/bookworm-security/main/binary-arm64/Packages.xz',
]


def fetch(url, path=None, token=None):
    headers = {'User-Agent': 'pryx-arm64-bundle-attempts'}
    if token and url.startswith('https://api.github.com/'):
        headers['Authorization'] = 'Bearer ' + token
    with urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=600) as response:
        if path is None:
            return response.read()
        with open(path, 'wb') as output:
            while chunk := response.read(1 << 20):
                output.write(chunk)
        return path


def sha256(path):
    with open(path, 'rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def pinned(item, cache):
    path = cache / Path(item['url']).name
    fetch(item['url'], path)
    if sha256(path) != item['sha256']:
        raise ValueError(f"{item['name']}: digest differs from its pin")
    return path


def tool_runtimes(cache):
    found = {}
    for item in json.loads((TOOLS / 'artifacts-aarch64.json').read_text()):
        archive = pinned(item, cache)
        target = cache / item['name']
        target.mkdir()
        if archive.suffix == '.zip':
            with zipfile.ZipFile(archive) as source:
                source.extractall(target)
                for entry in source.infolist():
                    (target / entry.filename).chmod((entry.external_attr >> 16) & 0o777 or 0o644)
        elif item['name'] != 'python-standalone-notices':
            with tarfile.open(archive) as source:
                source.extractall(target, filter='data')
    shell = cache / 'headless-shell/chrome-headless-shell-linux-arm64/chrome-headless-shell'
    page = subprocess.run([str(shell), '--headless', '--no-sandbox', '--disable-gpu', '--dump-dom',
                           'data:text/html,<p id=pryx>arm64</p>'], capture_output=True, text=True, timeout=120)
    found['browser'] = 'id="pryx">arm64</p>' in page.stdout
    python = cache / 'python-standalone/python/bin/python3'
    found['python'] = subprocess.check_output([str(python), '-c', 'import platform; print(platform.machine())'],
                                              text=True).strip()
    node = cache / 'node/node-v24.22.1-linux-arm64/bin/node'
    found['node'] = subprocess.check_output([str(node), '-p', 'process.arch'], text=True).strip()
    return found


def conda_index():
    index = json.loads(bz2.decompress(fetch(f'https://conda.anaconda.org/conda-forge/{CONDA_SUBDIR}/repodata.json.bz2')))
    by_version = {}
    for filename, package in {**index.get('packages', {}), **index.get('packages.conda', {})}.items():
        by_version.setdefault((package['name'], package['version']), []).append((filename, package))
    return by_version


def tool_libraries():
    items = json.loads((TOOLS / 'libraries.lock.json').read_text()) + json.loads((TOOLS / 'helpers.lock.json').read_text())
    items.append(json.loads((REPO / 'docs/checks/measurements/k23/micromamba-builder.json').read_text()))
    index = conda_index()
    missing, rebuilt, same = [], [], 0
    for item in items:
        if '/noarch/' in item['url']:
            same += 1
            continue
        name, version = item['name'], item['version']
        candidates = index.get((name, version), [])
        if not candidates:
            missing.append(f'{name}={version}')
        elif any(package.get('build') == item.get('build') for _, package in candidates):
            same += 1
        else:
            rebuilt.append(f"{name}={version} ({sorted({p.get('build') for _, p in candidates})})")
    return {'packages': len(items), 'sameBuildOrNoarch': same, 'otherBuildSameVersion': rebuilt, 'missing': missing}


def debian_index():
    versions = set()
    for url in DEBIAN_INDEXES:
        package = None
        for line in lzma.decompress(fetch(url)).decode().splitlines():
            if line.startswith('Package: '):
                package = line.split(': ', 1)[1]
            elif line.startswith('Version: ') and package:
                versions.add((package, line.split(': ', 1)[1]))
    return versions


def deb_identity(url):
    # Debian's pool file name: <package>_<version with the epoch dropped>_<architecture>.deb.
    name, version, _ = Path(urllib.request.url2pathname(url)).name.removesuffix('.deb').split('_')
    return name, version


def tool_desktop():
    versions = {(name, re.sub(r'^[0-9]+:', '', version)) for name, version in debian_index()}
    missing, debs = [], 0
    for lock in ['desktop.lock.json', 'desktop-full.lock.json']:
        for item in json.loads((TOOLS / lock).read_text()):
            if not item['url'].endswith('.deb'):
                continue
            debs += 1
            name, version = deb_identity(item['url'])
            if (name, version) not in versions:
                missing.append(f'{name}={version}')
    wheels, unwheeled = 0, []
    requirements = (REPO / 'templates/python-interpreter/requirements.txt').read_text()
    for name, version in re.findall(r'^([A-Za-z0-9_.-]+)==([^\s;\\]+)', requirements, re.M):
        wheels += 1
        files = json.loads(fetch(f'https://pypi.org/pypi/{name}/{version}/json'))['urls']
        if not any(f['filename'].endswith('-none-any.whl') or ('aarch64' in f['filename'] and 'manylinux' in f['filename']
                                                               and ('cp312' in f['filename'] or 'abi3' in f['filename']))
                   for f in files):
            unwheeled.append(f'{name}=={version}')
    return {'debs': debs, 'missingDebs': missing, 'wheels': wheels, 'noAarch64Wheel': unwheeled}


def release_asset(repository, tag, pattern, token, cache):
    release = json.loads(fetch(f'https://api.github.com/repos/{repository}/releases/tags/{tag}', token=token))
    asset = next(a for a in release['assets'] if re.match(pattern, a['name']))
    path = fetch(asset['browser_download_url'], cache / asset['name'])
    digest = sha256(path)
    # GitHub's own record of the asset's bytes, as the x86_64 pins' digestSource.
    if asset.get('digest') not in (None, f'sha256:{digest}'):
        raise ValueError(f"{asset['name']}: digest differs from the release's")
    return path, digest


def candidate(name):
    return next(c for c in json.loads((VM / 'candidates.json').read_text()) if c['name'] == name)


def vm_monitor(cache, token):
    pin = candidate('firecracker')
    archive, digest = release_asset(pin['repository'], pin['tag'], r'^firecracker-.*-aarch64\.tgz$', token, cache)
    with tarfile.open(archive) as source:
        names = source.getnames()
        source.extractall(cache / 'firecracker', filter='data')
    tag = pin['tag']
    monitor = cache / f'firecracker/release-{tag}-aarch64/firecracker-{tag}-aarch64'
    version = subprocess.run([str(monitor), '--version'], capture_output=True, text=True).stdout.splitlines()
    return {'asset': archive.name, 'sha256': digest, 'version': version[:1],
            'seccompPolicy': [n for n in names if 'seccomp' in n]}


def vm_kernel(cache, token):
    pin = candidate('kata-kernel-candidate')
    archive, digest = release_asset(pin['repository'], pin['tag'], r'^kata-static-.*-arm64\.tar\.(xz|zst|gz)$', token, cache)
    selection = json.loads((VM / 'kernel-selection.json').read_text())
    configuration = {}
    listing = subprocess.check_output(['tar', '-tf', str(archive)], text=True).splitlines()
    kernels = [n for n in listing if re.search(r'share/kata-containers/(vmlinu[xz]|Image)[^/]*$', n)]
    configs = [n for n in listing if re.search(r'share/kata-containers/config-[^/]*$', n)]
    if configs:
        text = subprocess.check_output(['tar', '-xOf', str(archive), configs[0]], text=True)
        enabled = {line.split('=')[0] for line in text.splitlines() if re.match(r'^CONFIG_[A-Z0-9_]+=(y|m)', line)}
        configuration = {option: option in enabled for option in selection['requiredConfiguration']}
        configuration['CONFIG_VMGENID'] = 'CONFIG_VMGENID' in enabled
    return {'asset': archive.name, 'sha256': digest, 'kernels': kernels, 'configurations': configs,
            'required': configuration}


def vm_bootstrap(agent):
    path = Path(agent)
    if not path.is_file():
        return {'agent': None}
    machine = subprocess.check_output(['file', '-b', str(path)], text=True).strip()
    return {'agent': machine, 'static': 'statically linked' in machine or 'static-pie' in machine}


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('--agent', required=True, help='the aarch64-unknown-linux-musl pryx-agent the caller built')
    options = parser.parse_args()
    token = os.environ.get('GH_TOKEN')
    stages = {
        'tool-runtimes': tool_runtimes,
        'tool-libraries': lambda _: tool_libraries(),
        'tool-desktop': lambda _: tool_desktop(),
        'vm-monitor': lambda cache: vm_monitor(cache, token),
        'vm-kernel': lambda cache: vm_kernel(cache, token),
        'vm-bootstrap': lambda _: vm_bootstrap(options.agent),
    }
    failed = False
    for name, stage in stages.items():
        with tempfile.TemporaryDirectory() as directory:
            try:
                print(json.dumps({'stage': name, 'outcome': 'ran', 'found': stage(Path(directory))}), flush=True)
            except Exception as problem:  # Evidence: every stage reports, whatever another did.
                failed = True
                print(json.dumps({'stage': name, 'outcome': 'failed', 'problem': f'{type(problem).__name__}: {problem}'}),
                      flush=True)
    raise SystemExit(1 if failed else 0)


if __name__ == '__main__':
    main()

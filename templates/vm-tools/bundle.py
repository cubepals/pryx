#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Assemble separately pinned VM runtime and corresponding-source distributions.

Only verified published monitor/kernel bytes and same-source static programs enter the runtime.
Original Linux, Kata recipes/patches, filesystem and registry sources stay in the source archive.
This build does not publish a release or advertise a VM compute tier.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil

from bootstrap import static
from inventory import ROOT
from kernel import selected
import rust_notices

REPO = ROOT.parent.parent
spec = importlib.util.spec_from_file_location('pryx_artifact_package', REPO / 'templates/process-tools/package.py')
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


def copy_tree(source, destination):
    if source.is_symlink() or not source.is_dir():
        raise ValueError('VM build input must be a real directory')
    for path in source.rglob('*'):
        if path.is_symlink() or not (path.is_file() or path.is_dir()):
            raise ValueError('VM distributions contain regular files and directories only')
    shutil.copytree(source, destination)


def inventory(root):
    entries = []
    for path in sorted(root.rglob('*')):
        if path.is_symlink() or not (path.is_file() or path.is_dir()):
            raise ValueError('VM distribution contains an unsupported member')
        if path.is_file():
            entries.append({'path': str(path.relative_to(root)), 'bytes': path.stat().st_size,
                            'sha256': package.digest(path), 'mode': path.stat().st_mode & 0o777})
    (root / 'manifest.json').write_text(json.dumps({'format': 1, 'files': entries}, indent=2) + '\n')


def build(args):
    runtime, sources = args.output, args.source_output
    if any(a.resolve().is_relative_to(b.resolve()) for a, b in [(runtime, sources), (sources, runtime)]):
        raise ValueError('VM runtime and source distributions must be separate')
    runtime.mkdir(mode=0o700, parents=True, exist_ok=False)
    copy_tree(args.sources, sources)
    selection = json.loads((ROOT / 'kernel-selection.json').read_text())
    inspection = json.loads(args.inspection.read_text())
    for key, relative in [('monitor', 'bin/firecracker'), ('jailer', 'bin/jailer'), ('kernel', 'kernel/vmlinux')]:
        path = selected(selection, inspection, args.staged, key)
        destination = runtime / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(path, destination)
        destination.chmod(0o755 if key != 'kernel' else 0o444)
    configuration = selected(selection, inspection, args.staged, 'configuration')
    if hashlib.sha256(configuration.encode()).hexdigest() != selection['configuration']['sha256']:
        raise ValueError('Published kernel configuration differs from its pin')
    (runtime / 'kernel/config').write_text(configuration)
    copy_tree(args.filesystem_component / 'tools', runtime / 'filesystem/tools')
    filesystem_manifest = json.loads((args.filesystem_component / 'filesystem-manifest.json').read_text())
    for member in filesystem_manifest:
        member['mode'] &= 0o555
        (runtime / 'filesystem' / member['path']).chmod(member['mode'])
    (runtime / 'filesystem/filesystem-manifest.json').write_text(json.dumps(filesystem_manifest, indent=2) + '\n')
    shutil.copyfile(args.filesystem_component / 'filesystem-manifest.json', sources / 'original-filesystem-manifest.json')
    copy_tree(args.filesystem_component / 'sources', sources / 'filesystem/sources')
    copy_tree(args.filesystem_component / 'notices', runtime / 'notices/filesystem')
    copy_tree(args.sources / 'notices', runtime / 'notices/kernel-monitor')
    copy_tree(args.sdk_notices, runtime / 'notices/sdk')
    copy_tree(args.network_notices, runtime / 'notices/network')
    copy_tree(REPO / 'LICENSES', runtime / 'notices/licence-texts')
    shutil.copyfile(REPO / 'LICENSE.md', runtime / 'notices/pryx-LICENSE.md')
    metadata = json.loads((args.bootstrap / 'bootstrap.json').read_text())
    image = args.bootstrap / 'bootstrap.erofs'
    if metadata['networkProbe'] is not None or package.digest(image) != metadata['imageSha256']:
        raise ValueError('VM runtime requires a whole production bootstrap without test programs')
    (runtime / 'bootstrap').mkdir()
    shutil.copyfile(image, runtime / 'bootstrap/bootstrap.erofs')
    shutil.copyfile(args.bootstrap / 'bootstrap.json', runtime / 'bootstrap/bootstrap.json')
    for source, name in [(args.image_helper, 'pryx-vm-image'), (args.configurator, 'vm-config'),
                         (args.transport, 'vm-transport'), (args.network_proxy, 'network-proxy')]:
        static(source)
        shutil.copyfile(source, runtime / 'bin' / name)
        (runtime / 'bin' / name).chmod(0o755)
    rust_notices.collect(runtime, sources)
    for name in ['candidates.json', 'kernel-selection.json', 'kernel-source.json', 'filesystem-inputs.json', 'cross-inputs.json']:
        shutil.copyfile(ROOT / name, sources / name)
    for name in ['Cargo.toml', 'Cargo.lock']:
        shutil.copyfile(REPO / 'vm-image' / name, sources / ('vm-image-' + name))
    inventory(sources)
    source_descriptor = package.package(sources, args.source_archive, args.url_base + '/pryx-vm-sources.tar.gz')
    args.source_descriptor.write_text(json.dumps(source_descriptor, indent=2) + '\n')
    (runtime / 'notices/source-distribution.json').write_text(json.dumps(source_descriptor, indent=2) + '\n')
    inventory(runtime)
    descriptor = package.package(runtime, args.archive, args.url_base + '/pryx-vm-linux-x64.tar.gz')
    args.descriptor.write_text(json.dumps(descriptor, indent=2) + '\n')
    print(json.dumps({'runtime': descriptor, 'sources': source_descriptor}), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['output', 'source-output', 'sources', 'inspection', 'staged', 'filesystem-component',
                 'sdk-notices', 'network-notices', 'bootstrap', 'image-helper', 'configurator', 'transport',
                 'network-proxy', 'archive', 'source-archive', 'descriptor', 'source-descriptor']:
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--url-base', required=True)
    build(parser.parse_args())


if __name__ == '__main__':
    main()

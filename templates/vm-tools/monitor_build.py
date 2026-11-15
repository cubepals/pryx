#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Build the unchanged pinned monitor with a confined native GNU thread ABI policy.

Only a verified upstream source archive is extracted. The publisher's pinned Rust version
and separate GNU SDK compile the monitor; host build tools retain host linkage. Compile the
expected restrictive BPF independently and require those exact bytes in the static PIE ELF.
No guest, privileged worker or host policy is started or installed here.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tomllib

from bootstrap import static
from inventory import ROOT


def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def original(sources, root):
    candidate = next(item for item in json.loads((ROOT / 'candidates.json').read_text()) if item['name'] == 'firecracker')
    identity = next(item for item in json.loads((sources / 'inputs.json').read_text())
                    if item.get('repository') == candidate['repository'])
    archive = sources / identity['archive']
    if identity['commit'] != candidate['sourceCommit'] or digest(archive) != identity['sha256']:
        raise ValueError('Native monitor source differs from its pinned original tree')
    with tarfile.open(archive) as source:
        source.extractall(root, filter='data')
    return root / candidate['name'], identity


def native_libraries(root):
    # The upstream build-time seccompiler needs the actual system libseccomp, even when
    # only its runtime soname is installed. No host files or global search paths change.
    entries = subprocess.check_output([shutil.which('ldconfig') or '/sbin/ldconfig', '-p'], text=True, timeout=10).splitlines()
    library = next(Path(line.split(' => ', 1)[1]) for line in entries
                   if line.strip().startswith('libseccomp.so.2 ') and '(libc6,x86-64)' in line)
    directory = root / 'native-libraries'
    directory.mkdir()
    (directory / 'libseccomp.so').symlink_to(library.resolve(strict=True))
    return directory


def build(args):
    root, target = args.root.resolve(), args.target_dir.resolve()
    root.mkdir(mode=0o700, parents=True, exist_ok=False)
    source, identity = original(args.sources.resolve(), root)
    inputs = json.loads((ROOT / 'monitor-inputs.json').read_text())
    if json.loads((args.sdk / 'sdk.json').read_text())['sdk'] != inputs['sdk']:
        raise ValueError('Native monitor SDK receipt differs from its committed input')
    sdk = args.sdk.resolve() / inputs['sdk']['name']
    compiler = sdk / 'bin/x86_64-buildroot-linux-gnu-gcc'
    channel = tomllib.loads((source / 'rust-toolchain.toml').read_text())['toolchain']['channel']
    subprocess.run(['rustup', 'toolchain', 'install', channel, '--profile', 'minimal'], check=True, timeout=600)
    environment = {key: value for key, value in os.environ.items() if key not in {
        'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_BUILD_TARGET',
        'CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER'}}
    environment.update({'RUSTUP_TOOLCHAIN': channel, 'CARGO_TARGET_DIR': str(target),
        'CARGO_PROFILE_RELEASE_DEBUG': '0', 'CARGO_PROFILE_RELEASE_LTO': 'false', 'CARGO_INCREMENTAL': '0',
        'LIBRARY_PATH': str(native_libraries(root)), 'CC_x86_64_unknown_linux_gnu': str(compiler),
        'CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS': f'-C target-feature=+crt-static -C linker={compiler}'})
    def run(arguments, *, capture=False, host=False, stdin=None):
        selected = {key: value for key, value in environment.items()
                    if not host or key != 'CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS'}
        return subprocess.check_output(arguments, cwd=source, env=selected, timeout=600,
                                       text=True) if capture else subprocess.run(
            arguments, cwd=source, env=selected, stdin=stdin, timeout=600, check=True)
    component = root / 'component'
    component.mkdir()
    example = source / 'src/vmm/examples/pryx_signal_abi.rs'
    example.parent.mkdir(exist_ok=True)
    shutil.copyfile(ROOT / 'monitor_abi.rs', example)
    shutil.copyfile(ROOT / 'monitor_threads.rs', source / 'src/vmm/examples/pryx_native_threads.rs')
    abi = run(['cargo', 'run', '--locked', '--release', '--target', inputs['target'],
               '-p', 'vmm', '--example', 'pryx_signal_abi'], capture=True)
    (component / 'abi.json').write_text(abi)
    policy = source / 'resources/seccomp' / (inputs['target'] + '.json')
    run([sys.executable, str(ROOT.resolve() / 'monitor_policy.py'),
         '--policy', str(source / 'resources/seccomp/x86_64-unknown-linux-musl.json'),
         '--abi', str(component / 'abi.json'), '--output', str(policy),
         '--receipt', str(component / 'policy.json')])
    run(['cargo', 'build', '--locked', '--release', '--target', inputs['target'], '-p', 'firecracker'])
    expected = component / 'seccomp_filter.bpf'
    run(['cargo', 'run', '--locked', '--release', '-p', 'seccompiler', '--bin', 'seccompiler-bin', '--',
         '--target-arch', 'x86_64', '--input-file', str(policy), '--output-file', str(expected)], host=True)
    master, terminal = os.openpty()
    try:
        run(['cargo', 'run', '--locked', '--release', '--target', inputs['target'],
             '-p', 'vmm', '--example', 'pryx_native_threads', '--', str(expected)], stdin=terminal)
    finally:
        os.close(terminal)
        os.close(master)
    executable = target / inputs['target'] / 'release/firecracker'
    static(executable)
    if not expected.read_bytes() or expected.read_bytes() not in executable.read_bytes():
        raise ValueError('Native monitor does not embed the expected restrictive policy')
    (component / 'bin').mkdir()
    shutil.copyfile(executable, component / 'bin/firecracker')
    (component / 'bin/firecracker').chmod(0o755)
    shutil.copyfile(policy, component / 'seccomp.json')
    receipt = {'source': identity, 'sdk': inputs['sdk'], 'target': inputs['target'], 'nativeThreadAbiVerified': True,
               'rust': run(['rustc', '--version'], capture=True).strip(),
               'version': run([str(executable), '--version'], capture=True).splitlines()[0],
               'monitorSha256': digest(executable), 'monitorBytes': executable.stat().st_size,
               'embeddedRestrictiveBpfSha256': digest(expected), 'embeddedRestrictiveBpfBytes': expected.stat().st_size}
    (component / 'monitor.json').write_text(json.dumps(receipt, indent=2) + '\n')
    run([sys.executable, str(ROOT.resolve() / 'monitor_sources.py'), '--source', str(source),
         '--original-sources', str(args.sources.resolve()), '--component', str(component)])
    print(json.dumps(receipt), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['sources', 'sdk', 'root', 'target-dir']:
        parser.add_argument('--' + name, type=Path, required=True)
    build(parser.parse_args())


if __name__ == '__main__':
    main()

#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Refuse a native-monitor build component with missing source or a weaker embedded policy.

Read the original commit-authenticated restrictive policy and reapply the reviewed ABI
translation before bundle selection. This verifies build inputs and correspondence only;
actual confined KVM boot and shutdown remain mandatory workflow acceptance.
"""
import hashlib
import json
from pathlib import Path
import tarfile

from bootstrap import static
from inventory import ROOT
from monitor_policy import adapt


def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def verified(component):
    inputs = json.loads((ROOT / 'monitor-inputs.json').read_text())
    candidate = next(item for item in json.loads((ROOT / 'candidates.json').read_text()) if item['name'] == 'firecracker')
    receipt = json.loads((component / 'monitor.json').read_text())
    identity = json.loads((component / 'sources/inputs.json').read_text())[0]
    if (receipt.get('nativeThreadAbiVerified') is not True or receipt['sdk'] != inputs['sdk'] or receipt['target'] != inputs['target']
            or identity['repository'] != candidate['repository'] or identity['commit'] != candidate['sourceCommit']
            or any(receipt['source'][key] != identity[key] for key in ['repository', 'commit', 'sha256', 'bytes'])):
        raise ValueError('Native monitor build/source identity differs from its committed inputs')
    relative = Path(identity['archive'])
    if len(relative.parts) != 1 or relative.is_absolute():
        raise ValueError('Native monitor source must be one original archive')
    archive = component / 'sources' / relative
    if digest(archive) != identity['sha256'] or archive.stat().st_size != identity['bytes']:
        raise ValueError('Native monitor original source is incomplete or changed')
    with tarfile.open(archive) as source:
        original = source.extractfile('firecracker/resources/seccomp/x86_64-unknown-linux-musl.json').read()
    abi = json.loads((component / 'abi.json').read_text())
    expected, changed = adapt(json.loads(original), abi)
    policy_path = component / 'seccomp.json'
    policy = json.loads(policy_path.read_text())
    translation = json.loads((component / 'policy.json').read_text())
    if (policy != expected or translation['adaptedRules'] != changed
            or translation['sourcePolicySha256'] != hashlib.sha256(original).hexdigest()
            or translation['nativePolicySha256'] != digest(policy_path)):
        raise ValueError('Native monitor does not retain its complete restrictive source policy')
    executable, binary_policy = component / 'bin/firecracker', component / 'seccomp_filter.bpf'
    static(executable)
    if (digest(executable) != receipt['monitorSha256'] or executable.stat().st_size != receipt['monitorBytes']
            or digest(binary_policy) != receipt['embeddedRestrictiveBpfSha256']
            or binary_policy.stat().st_size != receipt['embeddedRestrictiveBpfBytes']
            or not binary_policy.read_bytes() or binary_policy.read_bytes() not in executable.read_bytes()):
        raise ValueError('Native monitor ELF or embedded restrictive policy differs from its build')
    for relative in ['notices/firecracker-LICENSE', 'notices/glibc/COPYING.LESSERv2',
                     'notices/gcc/COPYING.RUNTIME', 'notices/monitor-rust/inventory.json',
                     'notices/rust-standard-library/COPYRIGHT-library.html', 'sources/REBUILD.md']:
        if not (component / relative).is_file():
            raise ValueError('Native monitor required source/relinking notice is missing')
    libc = component / 'sources' / Path(inputs['glibc']['url']).name
    if (not libc.is_file() or libc.stat().st_size != inputs['glibc']['bytes']
            or digest(libc) != inputs['glibc']['sha256']):
        raise ValueError('Native monitor complete corresponding libc source is missing or changed')
    return executable

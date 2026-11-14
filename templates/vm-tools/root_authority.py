#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Prove real jailed VM lifecycles in a separately bounded systemd authority fixture.

This runs only on the disposable root/KVM CI host. It installs no product service and
changes neither the main daemon's unit nor its capabilities. Raw output stays in CI artifacts.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import uuid


def run(*arguments, **options):
    return subprocess.run(arguments, check=True, **options)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cargo-artifacts', type=Path, required=True)
    parser.add_argument('--work', type=Path, required=True)
    args = parser.parse_args()
    # Cargo's documented compiler-artifact message owns executable discovery.
    artifacts = [json.loads(line) for line in args.cargo_artifacts.read_text().splitlines()]
    binaries = [Path(value['executable']) for value in artifacts
                if value.get('reason') == 'compiler-artifact' and value.get('executable')
                and value['target']['name'] == 'pryxd' and value['profile']['test']]
    if len(binaries) != 1:
        raise ValueError('Expected exactly the actual daemon library test executable')
    source = Path(os.environ['RUNNER_TEMP'])
    name = 'pryx-vm-ci-' + uuid.uuid4().hex
    programs = Path('/usr/local/libexec') / name
    state = Path('/var/lib') / name
    uid, gid = str(os.getuid()), str(os.getgid())
    if uid == '0':
        raise ValueError('Invoke the authority fixture as the ordinary CI runner')
    args.work.mkdir(mode=0o700)
    run('sudo', 'install', '-d', '-m', '755', str(programs))
    run('sudo', 'install', '-d', '-m', '700', str(state))
    try:
        for original, target, mode in [
            (binaries[0], 'acceptance', '555'),
            (source / 'vm-root-tap-probe', 'tap-probe', '555'),
            (source / 'vm-root-scope', 'scope', '555'),
            (source / 'vm-root-cni/tap', 'tap', '555'),
            (source / 'vm-network-probe', 'network-probe', '555'),
            (source / 'vm-runtime.json', 'vm-runtime.json', '444'),
            (source / 'pryx-vm-linux-x64.tar.gz', 'runtime.tar.gz', '444'),
        ]:
            run('sudo', 'install', '-m', mode, str(original), str(programs / target))
        # A system service defaults to root. Avoid a redundant identity transition:
        # systemd drops CAP_SETUID during its temporary seccomp-authority setup after
        # explicit User= even for root (v255 src/core/exec-invoke.c). Ambient stays empty.
        # The pinned jailer auto-exposes optional devices listed in /proc/misc.
        # Hide that catalogue: cold VM boot needs neither userfaultfd nor Nitro enclaves.
        # Keep DevicePolicy closed with only KVM/TUN, including creation permissions.
        properties = [
            'Type=exec', 'NoNewPrivileges=yes',
            'ProtectSystem=strict', 'ProtectHome=yes', 'PrivateTmp=yes', 'PrivateDevices=no',
            'ProtectKernelTunables=yes', 'ProtectKernelModules=yes', 'ProtectKernelLogs=yes',
            'ProtectControlGroups=yes', 'ProtectClock=yes', 'ProtectHostname=yes',
            'RestrictNamespaces=mnt net', 'RestrictRealtime=yes', 'RestrictSUIDSGID=yes',
            'InaccessiblePaths=/proc/misc',
            'LockPersonality=yes', 'MemoryDenyWriteExecute=yes', 'SystemCallArchitectures=native',
            'RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK',
            'DevicePolicy=closed', 'DeviceAllow=/dev/kvm rwm', 'DeviceAllow=/dev/net/tun rwm',
            'LimitNOFILE=65536', 'KillMode=control-group', f'ReadWritePaths={state}',
            'CapabilityBoundingSet=CAP_SYS_ADMIN CAP_SYS_CHROOT CAP_MKNOD CAP_NET_ADMIN '
            'CAP_SETUID CAP_SETGID CAP_SETPCAP CAP_CHOWN CAP_DAC_OVERRIDE CAP_FOWNER CAP_KILL CAP_SYS_PTRACE',
            'AmbientCapabilities=',
        ]
        environment = {
            'PRYX_VM_ROOT_TEST_UID': uid, 'PRYX_VM_ROOT_TEST_GID': gid,
            'PRYX_VM_RUNTIME_DESCRIPTOR': str(programs / 'vm-runtime.json'),
            'PRYX_VM_RUNTIME_ARCHIVE': str(programs / 'runtime.tar.gz'),
            'PRYX_VM_RUNTIME_DIAGNOSTICS': str(state / 'diagnostics'),
            'PRYX_VM_ROOT_SCOPE_TEST_PROGRAM': str(programs / 'scope'),
            'PRYX_VM_POLICY_PROBE': str(programs / 'network-probe'),
        }
        observed = subprocess.check_output(['systemd', '--version'], text=True).splitlines()[0]
        (args.work / 'profile.json').write_text(json.dumps({
            'systemd': observed, 'properties': properties, 'defaultSystemIdentity': 'root',
        }, indent=2) + '\n')
        base = ['sudo', 'systemd-run', '--quiet', '--wait', '--pipe', '--collect']
        for property in properties:
            base += ['--property', property]
        for key, value in environment.items():
            base += ['--setenv', f'{key}={value}']
        for suffix, command in [
            ('tap', [str(programs / 'tap-probe'), '--plugins', str(programs), '--uid', uid, '--gid', gid]),
            ('gateways', [str(programs / 'tap-probe'), '--plugins', str(programs), '--uid', uid, '--gid', gid, '--gateways']),
            ('scope', [str(programs / 'tap-probe'), '--plugins', str(programs), '--uid', uid, '--gid', gid,
                       '--scope-helper', str(programs / 'scope')]),
            ('adoption', [str(programs / 'acceptance'), '--exact',
                          'runtime::vm::root_scope::tests::actual_root_gateway_adoption_and_lease_retirement',
                          '--ignored', '--nocapture']),
            ('runtime', [str(programs / 'acceptance'), '--exact',
                         'runtime::vm::instance::root_lifecycle::actual_root_runtime_cold_boot_files_and_daemon_loss',
                         '--ignored', '--nocapture']),
        ]:
            with (args.work / f'{suffix}.log').open('wb') as log:
                result = subprocess.run([*base, '--unit', name + '-' + suffix, *command],
                                        stdout=log, stderr=subprocess.STDOUT, check=False)
            if result.returncode:
                raise RuntimeError(f'The separate {suffix} authority failed; inspect its retained CI log')
        # A passing command is not enough: require the actual Rust test to have executed.
        for suffix in ['adoption', 'runtime']:
            if '1 passed; 0 failed' not in (args.work / f'{suffix}.log').read_text():
                raise ValueError('The authority fixture did not execute its real Rust test')
        (args.work / 'summary.json').write_text(json.dumps({
            'systemd': observed, 'mainUnitChanged': False, 'separateTap': True,
            'separateRootRuntime': True, 'leaseBoundScope': True, 'images': 2, 'properties': properties,
            'rootHostnamePackets': True,
            'unproved': 'Production worker transport, installed service and root unrestricted IP forwarding',
        }, indent=2) + '\n')
    finally:
        # Stop the exact owned units before removing their private state, even after failure.
        for suffix in ['tap', 'gateways', 'scope', 'adoption', 'runtime']:
            subprocess.run(['sudo', 'systemctl', 'stop', name + '-' + suffix + '.service'],
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
            status = subprocess.run(['sudo', 'systemctl', 'show', name + '-' + suffix + '.service',
                                     '--property=ActiveState'], capture_output=True, text=True, check=False)
            if status.stdout.strip() not in {'ActiveState=inactive', 'ActiveState=failed'}:
                raise RuntimeError('Authority retirement is unconfirmed; preserve its private inputs and state')
        diagnostics = state / 'diagnostics'
        if subprocess.run(['sudo', 'test', '-d', str(diagnostics)], check=False).returncode == 0:
            run('sudo', 'cp', '-a', str(diagnostics), str(args.work / 'diagnostics'))
            run('sudo', 'chown', '-R', f'{uid}:{gid}', str(args.work / 'diagnostics'))
        run('sudo', 'rm', '-rf', '--', str(programs), str(state))


if __name__ == '__main__':
    main()

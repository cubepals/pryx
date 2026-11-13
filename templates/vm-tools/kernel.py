#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Boot the pinned published kernel with EROFS, writable ext4, overlay and real vsock.

The hosted runner's ordinary user launches the unchanged monitor with its normal seccomp.
Thirty cold boots and one disk-retaining reboot measure only this minimal fixture, not
production create/agent/image timings or monitor confinement. Failed serial logs are retained.
"""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import shutil
import socket
import subprocess
import tarfile
import time

from inventory import ROOT

SAMPLES = 30
REPORT_BYTES = 64 * 1024
DISK_BYTES = 128 * 1024 * 1024


def command(arguments):
    return subprocess.run([str(value) for value in arguments], check=True, timeout=180,
                          stdout=subprocess.PIPE, stderr=subprocess.STDOUT).stdout


def selected(selection, inspection, staged, key):
    expected = selection[key]
    members = [member for candidate in inspection['candidates'] for member in candidate['members']]
    matches = [member for member in members if member['path'] == expected['path']]
    if len(matches) != 1 or matches[0].get('sha256') != expected['sha256']:
        raise ValueError('Selected upstream member differs from its committed identity')
    if key == 'configuration':
        return matches[0]['configuration']
    path = staged / expected['sha256']
    with path.open('rb') as source:
        if hashlib.file_digest(source, 'sha256').hexdigest() != expected['sha256']:
            raise ValueError('Selected executable bytes differ from their identity')
    return path


def root_image(guest, work, mkfs, base=None, additions=None):
    archive_path = work / 'root.tar'
    with tarfile.open(archive_path, 'w') as archive:
        if base is not None:
            # Feed the maintained filesystem builder a tar, never extract image paths on the host.
            with tarfile.open(base, 'r|*') as upstream:
                for member in upstream:
                    if member.isfile():
                        with upstream.extractfile(member) as source:
                            archive.addfile(member, source)
                    else:
                        archive.addfile(member)
        for name, path in (additions or {}).items():
            entry = tarfile.TarInfo(name)
            entry.mode, entry.size = 0o755 if name in {'netprobe', 'gvforwarder'} else 0o600, path.stat().st_size
            with path.open('rb') as source:
                archive.addfile(entry, source)
        for name in ['dev', 'proc', 'sys', 'write', 'lower', 'merged']:
            entry = tarfile.TarInfo(name)
            entry.type, entry.mode = tarfile.DIRTYPE, 0o755
            archive.addfile(entry)
        entry = tarfile.TarInfo('init')
        entry.mode, entry.size = 0o755, guest.stat().st_size
        with guest.open('rb') as source:
            archive.addfile(entry, source)
        data = b'immutable-base\n'
        entry = tarfile.TarInfo('baseline')
        entry.mode, entry.size = 0o644, len(data)
        archive.addfile(entry, io.BytesIO(data))
    path = work / 'root.erofs'
    command([mkfs, '--tar=f', '--quiet', path, archive_path])
    archive_path.unlink()
    return path


def boot(configurator, monitor, kernel, root, writes, work, label, expected_boots, report_port,
         tap=None, launcher=None, timeout=30, monitor_record=None):
    channel = work / 'v.sock'
    # Reboot leaves the device's sockets behind, never a live monitor to be adopted here.
    channel.unlink(missing_ok=True)
    host = Path(str(channel) + '_' + str(report_port))
    host.unlink(missing_ok=True)
    arguments = [configurator, '--kernel', kernel, '--root', root, '--writes', writes, '--socket', channel]
    if tap:
        arguments.extend(['--tap', tap])
    config = command(arguments)
    config_path = work / 'machine.json'
    config_path.write_bytes(config)
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    listener.bind(str(host))
    listener.listen(1)
    listener.settimeout(timeout)
    process = None
    try:
        with (work / (label + '.serial.log')).open('wb') as serial:
            started = time.monotonic_ns()
            process = subprocess.Popen([*(launcher or []), str(monitor), '--no-api', '--config-file', str(config_path)],
                                       stdout=serial, stderr=subprocess.STDOUT)
            connection, _ = listener.accept()
            with connection:
                connection.settimeout(30)
                with connection.makefile('rb') as source:
                    raw = source.readline(REPORT_BYTES + 1)
                if len(raw) > REPORT_BYTES or not raw.endswith(b'\n'):
                    raise ValueError('Guest report exceeds its bound or is incomplete')
                report = json.loads(raw)
                elapsed = (time.monotonic_ns() - started) / 1_000_000
                for key in ['erofs', 'overlay', 'ext4', 'virtioBlock', 'vsock', 'rootUnchanged']:
                    if report.get(key) is not True:
                        raise ValueError(f'Guest did not prove {key}')
                if report['boots'] != expected_boots or report['pid'] != 1:
                    raise ValueError('Guest disk retention or PID 1 proof failed')
                if report['kernel'] == platform.release():
                    raise ValueError('Guest is using the host kernel')
                monitor_pid = process.pid if monitor_record is None else json.loads(monitor_record.read_text())['monitorPid']
                resident = (Path('/proc') / str(monitor_pid) / 'status').read_text()
                rss = next(line for line in resident.splitlines() if line.startswith('VmRSS:'))
                helper_rss = None
                if monitor_record is not None:
                    helper_status = (Path('/proc') / str(process.pid) / 'status').read_text()
                    helper_rss = next(line for line in helper_status.splitlines() if line.startswith('VmRSS:'))
                # Acknowledge once only after every guest assertion; this is not a retryable exec.
                connection.sendall(b'stop\n')
            code = process.wait(timeout=10)
            if code:
                raise ValueError(f'Monitor exited unsuccessfully: {code}')
            result = {**report, 'label': label, 'launchToFilesystemVsockReportMs': elapsed,
                      'monitorResident': rss, 'monitorExit': code}
            if helper_rss is not None:
                result['helperResident'] = helper_rss
            return result
    finally:
        listener.close()
        if process is not None and process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        host.unlink(missing_ok=True)
        channel.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--inspection', type=Path, required=True)
    parser.add_argument('--staged', type=Path, required=True)
    parser.add_argument('--configurator', type=Path, required=True)
    parser.add_argument('--guest', type=Path, required=True)
    parser.add_argument('--mkfs', type=Path, required=True)
    parser.add_argument('--work', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if os.geteuid() == 0:
        raise ValueError('Kernel spike must prove the ordinary-user KVM path')
    args.work.mkdir(mode=0o700)
    selection = json.loads((ROOT / 'kernel-selection.json').read_text())
    inspection = json.loads(args.inspection.read_text())
    monitor = selected(selection, inspection, args.staged, 'monitor')
    kernel = selected(selection, inspection, args.staged, 'kernel')
    configuration = selected(selection, inspection, args.staged, 'configuration')
    for key in selection['requiredConfiguration']:
        if key + '=y' not in configuration.splitlines():
            raise ValueError('Published configuration lacks built-in ' + key)
    monitor.chmod(0o700)
    metadata = json.loads(command([args.configurator, '--metadata']))
    report = {'scope': 'Minimal ordinary-user kernel fixture only; not production VM-tier proof',
              'hostKernel': platform.release(), 'uid': os.geteuid(), 'selection': selection,
              'samples': [], 'wake': [], 'status': 'in-progress'}

    def save():
        args.output.write_text(json.dumps(report, indent=2) + '\n')

    save()
    try:
        started = time.monotonic_ns()
        root = root_image(args.guest, args.work, args.mkfs)
        pristine = args.work / 'pristine.ext4'
        with pristine.open('xb') as output:
            output.truncate(DISK_BYTES)
        command(['/usr/sbin/mke2fs', '-q', '-t', 'ext4', '-F', pristine])
        report['imagePreparationMs'] = (time.monotonic_ns() - started) / 1_000_000
        report['rootBytes'] = root.stat().st_size
        for index in range(SAMPLES):
            writes = args.work / 'writes.ext4'
            shutil.copyfile(pristine, writes)
            sample = boot(args.configurator, monitor, kernel, root, writes, args.work,
                          f'cold-{index:02}', 1, metadata['reportPort'])
            if not sample['kernel'].startswith(selection['kernelVersion']):
                raise ValueError('Booted kernel version differs from selected publication')
            report['samples'].append(sample)
            save()
            print(json.dumps(sample), flush=True)
            if index == 0:
                report['wake'].append(boot(args.configurator, monitor, kernel, root, writes,
                                           args.work, 'disk-reboot', 2, metadata['reportPort']))
                save()
            writes.unlink()
        report['status'] = 'success'
    except Exception as failure:
        report['status'], report['failure'] = 'failure', str(failure)
        raise
    finally:
        save()


if __name__ == '__main__':
    main()

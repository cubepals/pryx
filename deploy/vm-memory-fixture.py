#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Read PSS for the disposable install account through the existing CI sampler.

The parent runner starts this read-only Unix fixture as root so nondumpable monitor processes
are included. Peers may request only roots owned by that fixed account, never paths or programs.
An optional boolean selects the sampler's fixed VM executable breakdown.
This is not installed with pryx and changes no process confinement or kernel setting.
"""
import argparse
import json
import os
from pathlib import Path
import signal
import socket
import struct
import subprocess
import sys


MAX_REQUEST_BYTES = 1024


def sample(uid, root, sampler, vm_helpers=False):
    if not isinstance(root, int) or isinstance(root, bool) or root <= 1:
        raise ValueError('Expected an owned live process root')
    status = Path(f'/proc/{root}/status').read_text().splitlines()
    actual = next(line.split()[1:] for line in status if line.startswith('Uid:'))
    if any(int(value) != uid for value in actual):
        raise ValueError('The measurement root belongs to another account')
    command = [sys.executable, str(sampler)]
    if vm_helpers:
        command.append('--vm-helpers')
    completed = subprocess.run([*command, str(root)], check=True,
                               capture_output=True, text=True, timeout=30)
    value = json.loads(completed.stdout)
    if root in value['missing'] or value['pssBytes'] <= 0:
        raise ValueError('The owned measurement root disappeared')
    result = {'pssBytes': value['pssBytes'], 'processes': value['processes']}
    if vm_helpers:
        result['vmHelpers'] = value['vmHelpers']
    return result


def respond(connection, uid, sampler):
    connection.settimeout(30)
    # Linux socket(7) SO_PEERCRED returns struct ucred: pid_t, uid_t and gid_t.
    _, peer, _ = struct.unpack('3i', connection.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED,
                                                         struct.calcsize('3i')))
    if peer != uid:
        raise ValueError('Only the fixed install account may request a sample')
    body = b''
    while True:
        part = connection.recv(MAX_REQUEST_BYTES + 1 - len(body))
        if not part:
            break
        body += part
        if len(body) > MAX_REQUEST_BYTES:
            raise ValueError('The measurement request exceeds its bound')
    request = json.loads(body)
    if not isinstance(request, dict) or set(request) not in ({'root'}, {'root', 'vmHelpers'}):
        raise ValueError('The measurement request must name an owned process root')
    vm_helpers = request.get('vmHelpers', False)
    if not isinstance(vm_helpers, bool):
        raise ValueError('The VM helper observation option must be boolean')
    return sample(uid, request['root'], sampler, vm_helpers)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--uid', type=int, required=True)
    parser.add_argument('--socket', type=Path, required=True)
    parser.add_argument('--sampler', type=Path, required=True)
    args = parser.parse_args()
    if args.uid <= 0 or os.geteuid() not in {0, args.uid}:
        raise ValueError('Run as the fixed ordinary account or its CI administrator')
    if not args.sampler.is_absolute() or not args.sampler.is_file():
        raise ValueError('The CI parent must supply its existing sampler program')
    def terminate(_signal, _frame):
        raise SystemExit(0)
    signal.signal(signal.SIGTERM, terminate)
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
        listener.bind(str(args.socket))
        try:
            args.socket.chmod(0o600)
            if os.geteuid() == 0:
                os.chown(args.socket, args.uid, -1)
            listener.listen(1)
            while True:
                connection, _ = listener.accept()
                with connection:
                    try:
                        value = respond(connection, args.uid, args.sampler)
                    except (ValueError, OSError, subprocess.SubprocessError):
                        value = {'error': 'Owned-process memory observation failed'}
                    connection.sendall(json.dumps(value).encode())
        finally:
            args.socket.unlink()


if __name__ == '__main__':
    main()

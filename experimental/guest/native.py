#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Execute the exact static inits as ordinary native processes and query KVM.
This proves architecture and executable compatibility. A VM boot requires a
separate native monitor, guest kernel, boot configuration and KVM-backed trial.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import time


def native_start(executable, poll_syscall):
    process = subprocess.Popen([executable], stdin=subprocess.DEVNULL, stdout=subprocess.PIPE)
    try:
        deadline = time.monotonic() + 5
        proc = Path('/proc') / str(process.pid)
        while time.monotonic() < deadline:
            if process.poll() is not None:
                raise RuntimeError(f'{executable.name} exited before blocking')
            fields = (proc / 'syscall').read_text().split()
            if fields and fields[0] == str(poll_syscall):
                if not (proc / 'exe').samefile(executable):
                    raise RuntimeError('Unexpected executable for native init test')
                break
            time.sleep(.01)
        else:
            raise RuntimeError(f'{executable.name} did not block in the expected syscall')
    finally:
        process.kill()
        output = process.communicate(timeout=5)[0]
    if output:
        raise RuntimeError('An ordinary child unexpectedly emitted a PID 1 marker')
    return {'sha256': hashlib.sha256(executable.read_bytes()).hexdigest(),
            'bytes': executable.stat().st_size, 'nativePpollObserved': True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('build', type=Path)
    parser.add_argument('report', type=Path)
    args = parser.parse_args()
    if os.geteuid() == 0:
        raise ValueError('Run the capability check as an ordinary user')
    src = Path(__file__).resolve().parent
    build = args.build.resolve()
    subprocess.run(['sh', src / 'build.sh', build], check=True, timeout=30)
    probe = build / 'kvm-api'
    subprocess.run(['cc', '-Wall', '-Wextra', '-Werror', '-o', probe, src / 'kvm_api.c'],
                   check=True, timeout=30)
    opened, api, api_err, expected, ppoll = map(int, subprocess.check_output([probe], text=True).split())
    kvm = {'expectedApi': expected, 'returnedApi': api if api >= 0 else None,
           'openErrno': opened or None, 'ioctlErrno': api_err or None}
    if opened or api_err:
        kvm['reason'] = os.strerror(opened or api_err)
    elif api != expected:
        kvm['reason'] = 'KVM API revision differs from the supported native UAPI'
    else:
        kvm['reason'] = 'Device opened and KVM API revision matched'
    report = {'machine': platform.machine(), 'kernel': platform.release(), 'uid': os.geteuid(),
              'commit': subprocess.check_output(['git', '-C', src, 'rev-parse', 'HEAD'], text=True).strip(),
              'kvm': kvm, 'inits': {}, 'guestBootMeasured': False}
    try:
        for name in ['init-S', 'init-c']:
            report['inits'][name] = native_start(build / name, ppoll)
    finally:
        args.report.write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps(report))


if __name__ == '__main__':
    main()

# SPDX-License-Identifier: FSL-1.1-ALv2
"""Observe an ordinary container's kernel policy without changing the parent process."""
import ctypes
import json
import os
import platform

# Linux include/uapi/linux/sched.h CLONE_NEW* flags and seccomp.h SECCOMP_MODE_FILTER (2).
libc = ctypes.CDLL(None, use_errno=True)
def child_result(work):
    pid = os.fork()
    if pid == 0:
        try:
            success = work()
        except OSError:
            success = False
        os._exit(0 if success else 1)
    return os.waitpid(pid, 0)[1] == 0

def switch_uid():
    os.setgid(20000)
    os.setuid(20000)
    return os.getuid() == 20000

def writable_cgroup():
    path = '/sys/fs/cgroup/pryx-capability-probe'
    try:
        os.mkdir(path)
    except OSError:
        return False
    os.rmdir(path)
    return True

with open('/proc/self/status', encoding='utf8') as status:
    facts = dict(line.rstrip().split(':', 1) for line in status if ':' in line)
print(json.dumps({
    'kernel': platform.release(), 'seccomp': int(facts['Seccomp']),
    'engineSocket': os.path.exists('/var/run/docker.sock'),
    'writableCgroup': writable_cgroup(),
    'canCreateNamespaces': {name: child_result(lambda flag=flag: libc.unshare(flag) == 0)
        for name, flag in {'user': 0x10000000, 'mount': 0x00020000, 'pid': 0x20000000, 'network': 0x40000000}.items()},
    'canSwitchUid': child_result(switch_uid),
}))

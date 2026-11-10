# SPDX-License-Identifier: FSL-1.1-ALv2
"""Assert refusal leaves an ordinary user's home unchanged on a real musl host whose
Landlock syscalls are denied. This tests the unavailable host, not a simulated probe.
"""
import hashlib
import os
from pathlib import Path
import stat
import subprocess


def snapshot():
    home = Path.home()
    found = {}
    for parent, directories, files in os.walk(home, followlinks=False):
        for name in directories + files:
            path = Path(parent) / name
            info = path.lstat()
            payload = (os.readlink(path) if stat.S_ISLNK(info.st_mode) else
                       hashlib.sha256(path.read_bytes()).hexdigest() if path.is_file() else None)
            found[str(path.relative_to(home))] = (info.st_mode, info.st_uid, info.st_gid, payload)
    return found


before = snapshot()
result = subprocess.run(['sh', '-c', 'curl -fsS http://127.0.0.1:9876/install.sh | sh'],
                        env={**os.environ, 'PRYX_RELEASE_BASE_URL': 'http://127.0.0.1:9876'},
                        capture_output=True, text=True, timeout=120, check=False)
assert result.returncode != 0, 'An unsupported host unexpectedly installed'
assert 'Landlock' in result.stdout + result.stderr, 'Refusal did not name the missing host requirement'
assert snapshot() == before, 'Unsupported preflight changed the home directory'
print('unsupported_requirement=Landlock home=unchanged')

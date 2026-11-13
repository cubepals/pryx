# SPDX-License-Identifier: FSL-1.1-ALv2
"""Print bounded local-install test diagnostics after redacting persisted credentials.

Logs and bounded read-only placement reasons are displayed; configuration, private keys
and environment files are never dumped. This is a CI failure tool, not the installed runtime.
"""
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys

home = Path.home()
private = Path(sys.argv[1]) if len(sys.argv) == 2 else (
    Path('/etc/pryx-local') if os.getuid() == 0 else home / '.config/pryx')
secrets = set()


def collect(value):
    if isinstance(value, str) and len(value) >= 8:
        secrets.add(value)
        secrets.add(json.dumps(value)[1:-1])
    elif isinstance(value, dict):
        for key, item in value.items():
            if key != 'url':
                collect(item)


for name in ['credentials.json', 'client.json']:
    try:
        collect(json.loads((private / name).read_text()))
    except (FileNotFoundError, ValueError):
        pass


def redact(text):
    for secret in sorted(secrets, key=len, reverse=True):
        text = text.replace(secret, '[redacted]')
    text = re.sub(r'-----BEGIN [^-]*PRIVATE KEY-----.*?-----END [^-]*PRIVATE KEY-----',
                  '[redacted private key]', text, flags=re.S)
    return re.sub(r'pryx_[A-Za-z0-9_-]+|bk1\.[A-Za-z0-9_.-]+|[a-f0-9]{32,}|[A-Za-z0-9+/]{40,}={0,2}', '[redacted]', text)


def inventory(root):
    """Identify retained fixture captures without reading credentials or workload data."""
    descriptor = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        remaining = 80
        for directory, children, names, parent in os.fwalk('.', dir_fd=descriptor, follow_symlinks=False):
            if directory.count('/') >= 2:
                children[:] = []
            for name in sorted(names):
                info = os.stat(name, dir_fd=parent, follow_symlinks=False)
                if not stat.S_ISREG(info.st_mode):
                    continue
                print('retained_fixture_file=' + redact(str(Path(directory) / name))
                      + ' bytes=' + str(info.st_size))
                remaining -= 1
                if remaining == 0:
                    return
    finally:
        os.close(descriptor)


if len(sys.argv) == 2:
    try:
        inventory(private)
    except OSError as error:
        print('fixture_inventory_unavailable=' + type(error).__name__)


try:
    record = json.loads((private / 'installation.json').read_text())
    values = json.loads((private / 'credentials.json').read_text())
    # Only the disposable installation's own database; no external database environment.
    environment = {'PATH': os.defpath, 'PGPASSWORD': values['password'], 'PGCONNECT_TIMEOUT': '2',
                   'PGOPTIONS': '-c default_transaction_read_only=on -c statement_timeout=2000'}
    result = subprocess.run([str(Path(record['bundle']) / 'postgres/bin/psql'), '-XAt',
                             '-h', str(Path(record['paths']['runtime']) / 'postgres'),
                             '-U', 'pryx', '-d', 'pryx', '-c',
                             "SELECT data->'considered' FROM fleet_events WHERE kind='placement.refused' ORDER BY at DESC LIMIT 5"],
                            env=environment, capture_output=True, text=True, timeout=5, check=True)
    for line in result.stdout.splitlines():
        for considered in json.loads(line) or []:
            reason = considered.get('reason')
            if isinstance(reason, str):
                print('placement_refused=' + redact(json.dumps(reason[:512])))
except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
    print('placement_diagnostics_unavailable=' + type(error).__name__)

logs = list(private.glob('*.log'))
if len(sys.argv) == 1:
    logs[:0] = [home / name for name in ['install.log', 'upgrade.log', 'unit.log', 'sdk-install.log']]
for path in logs:
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        with os.fdopen(descriptor, 'rb') as source:
            info = os.fstat(source.fileno())
            if not stat.S_ISREG(info.st_mode):
                continue
            source.seek(max(0, info.st_size - 16 * 1024))
            text = source.read(16 * 1024).decode(errors='replace')
    except OSError:
        continue
    text = redact(text)
    print(path.name + ':')
    print('\n'.join(text.splitlines()[-30:]))

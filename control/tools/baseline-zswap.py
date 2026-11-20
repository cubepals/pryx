#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Measure compressed and logical zswap bytes for an idle local development server.

The opt-in hosted-runner probe owns its cgroup and any backing swap it adds. It
records source data counts, cgroup counters and PSS, without retaining memory
contents or reading or printing application credentials.
"""
import argparse
from contextlib import ExitStack, contextmanager
import errno
from http import HTTPStatus
from http.client import HTTPConnection
import json
import os
from pathlib import Path
import platform
import signal
import socket
import subprocess
import sys
import tempfile
import time


MIB = 1024 * 1024
RECLAIM_BYTES = 32 * MIB
MINIMUM_SAMPLE_BYTES = MIB
BACKING_SWAP_BYTES = 256 * MIB
CGROUP_ROOT = Path('/sys/fs/cgroup')
ZSWAP_PARAMETERS = Path('/sys/module/zswap/parameters')
SOURCE_ROOT = Path(__file__).resolve().parents[2]
cancelled = False

# Run an actual Node HTTP listener with an in-memory source cache and line index.
# The cache is populated from the current checkout's tracked public source files.
DEVELOPMENT_SERVER = '''
const fs = require('node:fs');
const http = require('node:http');
const paths = JSON.parse(fs.readFileSync(0, 'utf8'));
const sources = paths.map(path => {
  const fd = fs.openSync(path, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW);
  try {
    const bytes = fs.readFileSync(fd);
    return {bytes, lines: bytes.toString('utf8').split('\\n')};
  } finally {
    fs.closeSync(fd);
  }
});
http.createServer((_, response) => {
  const sourceBytes = sources.reduce((size, file) => size + file.bytes.length, 0);
  const sourceLines = sources.reduce((lines, file) => lines + file.lines.length, 0);
  response.end(JSON.stringify({sourceFiles: sources.length, sourceBytes, sourceLines, node: process.versions.node}));
}).listen({fd: Number(process.argv[1])});
'''
RECLAIM = '''
import errno, pathlib, sys
try:
    pathlib.Path(sys.argv[1]).write_text(sys.argv[2])
except OSError as error:
    sys.exit(error.errno if error.errno in errno.errorcode else 1)
'''


class Unsupported(Exception):
    """A host cannot provide the measurement's prerequisites."""


def cancel(_signal, _frame):
    global cancelled
    cancelled = True


def check_cancellation():
    # Check after acquiring ownership of children and kernel resources, so cleanup can retire them.
    if cancelled:
        raise InterruptedError('zswap baseline cancelled')


def hosted_runner():
    if (os.environ.get('GITHUB_ACTIONS') != 'true' or
            os.environ.get('RUNNER_ENVIRONMENT') != 'github-hosted'):
        raise Unsupported('requires a disposable GitHub-hosted runner')
    if platform.freedesktop_os_release().get('VERSION_ID') != '24.04':
        raise Unsupported('requires an Ubuntu 24.04 runner')
    if os.geteuid() != 0 or not os.environ.get('SUDO_UID') or not os.environ.get('SUDO_GID'):
        raise Unsupported('requires sudo from the ordinary runner user')
    uid, gid = (int(os.environ[name]) for name in ('SUDO_UID', 'SUDO_GID'))
    if uid <= 0 or gid <= 0:
        raise Unsupported('the development server requires an ordinary runner UID and GID')
    temporary = os.environ.get('RUNNER_TEMP')
    if not temporary or not Path(temporary).is_dir():
        raise Unsupported('runner temporary directory is unavailable')
    return uid, gid, Path(temporary)


def swap_rows():
    return [line.split() for line in Path('/proc/swaps').read_bytes().splitlines()[1:]]


def available_swap():
    return sum((int(size) - int(used)) * 1024 for _, _, size, used, _ in swap_rows())


def command(*arguments):
    subprocess.run(arguments, check=True, timeout=20, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


@contextmanager
def add_swap(temporary):
    directory = Path(tempfile.mkdtemp(prefix='pryx-zswap-', dir=temporary))
    backing = directory / 'backing'
    # /proc/swaps represents whitespace and backslashes with octal escapes.
    proc_path = os.fsencode(backing).replace(b'\\', b'\\134').replace(b' ', b'\\040')
    proc_path = proc_path.replace(b'\t', b'\\011').replace(b'\n', b'\\012')
    try:
        with backing.open('xb') as stream:
            os.fchmod(stream.fileno(), 0o600)
            os.posix_fallocate(stream.fileno(), 0, BACKING_SWAP_BYTES)
        command('mkswap', str(backing))
        check_cancellation()
        command('swapon', str(backing))
        yield
    finally:
        # If swapoff fails, retain the active backing file so the runner can retire it.
        if any(row[0] == proc_path for row in swap_rows()):
            command('swapoff', str(backing))
        backing.unlink(missing_ok=True)
        directory.rmdir()


def source_paths():
    paths = subprocess.check_output(
        ['git', 'ls-files', '-z', '--', 'control/src', 'cli/src', 'edge/src', 'sdk/typescript/src'],
        cwd=SOURCE_ROOT, stderr=subprocess.DEVNULL, timeout=10,
    )
    selected = []
    for name in paths.split(b'\0'):
        path = Path(os.fsdecode(name))
        source = SOURCE_ROOT / path
        if (path.suffix in {'.ts', '.tsx', '.css'} and not source.is_symlink()
                and source.resolve().is_relative_to(SOURCE_ROOT)):
            selected.append(os.fspath(path))
    if not selected:
        raise RuntimeError('the development checkout has no tracked source fixtures')
    return selected


def request(port):
    connection = HTTPConnection('127.0.0.1', port, timeout=0.5)
    try:
        connection.request('GET', '/')
        reply = connection.getresponse()
        if reply.status != HTTPStatus.OK:
            raise RuntimeError('the development server returned an error')
        return json.loads(reply.read(512))
    finally:
        connection.close()


def ready(child, port):
    deadline = time.monotonic() + 10
    while child.poll() is None and time.monotonic() < deadline:
        check_cancellation()
        try:
            return request(port)
        except OSError:
            time.sleep(0.05)
    raise RuntimeError('the development server did not become ready')


def joined(cgroup, uid, gid):
    (cgroup / 'cgroup.procs').write_text(f'{os.getpid()}\n')
    os.setgroups([])
    os.setgid(gid)
    os.setuid(uid)


@contextmanager
def development_server(cgroup, node, uid, gid):
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(('127.0.0.1', 0))
        listener.listen()
        port = listener.getsockname()[1]
        descriptor = listener.fileno()
        child = subprocess.Popen(
            [node, '-e', DEVELOPMENT_SERVER, str(descriptor)], cwd=SOURCE_ROOT,
            env={'PATH': os.defpath}, start_new_session=True,
            pass_fds=(descriptor,), stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            preexec_fn=lambda: joined(cgroup, uid, gid),
        )
    try:
        check_cancellation()
        with child.stdin:
            child.stdin.write(json.dumps(source_paths()).encode())
        yield child.pid, port, ready(child, port)
    finally:
        if child.poll() is None:
            child.kill()
        child.wait(timeout=10)


def counters(path):
    return {key: int(value) for key, value in (line.split() for line in path.read_text().splitlines())}


def pss(pid):
    with Path(f'/proc/{pid}/smaps_rollup').open() as samples:
        for line in samples:
            if line.startswith('Pss:'):
                return int(line.split()[1]) * 1024
    raise RuntimeError('the development server has no PSS reading')


def sample(cgroup, pid):
    stat = counters(cgroup / 'memory.stat')
    if not {'zswap', 'zswapped'} <= stat.keys():
        raise Unsupported('memory.stat lacks zswap or zswapped for the measurement cgroup')
    return {
        'anonymousBytes': stat['anon'], 'compressedBytes': stat['zswap'], 'logicalBytes': stat['zswapped'],
        'residentPssBytes': pss(pid), 'swapBytes': int((cgroup / 'memory.swap.current').read_text()),
    }


def freeze(cgroup):
    (cgroup / 'cgroup.freeze').write_text('1\n')
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        check_cancellation()
        if counters(cgroup / 'cgroup.events').get('frozen') == 1:
            return
        time.sleep(0.05)
    raise RuntimeError('the development cgroup did not freeze')


def reclaim(cgroup):
    path = cgroup / 'memory.reclaim'
    # EAGAIN means the requested amount was not fully reclaimed. Actual counters decide.
    def run(value):
        return subprocess.run([sys.executable, '-c', RECLAIM, str(path), value], timeout=20, check=False).returncode

    value = f'{RECLAIM_BYTES}'
    status = run(f'{value} swappiness=max')
    check_cancellation()
    method = 'swappiness=max'
    if status == errno.EINVAL:
        status = run(value)
        method = 'kernel default swappiness'
    if status not in (0, errno.EAGAIN):
        raise Unsupported(f'memory.reclaim refused with {errno.errorcode.get(status, "unknown status")}')
    return {'requestedBytes': RECLAIM_BYTES, 'method': method, 'partial': status == errno.EAGAIN}


def stable_sample(cgroup, pid):
    previous = None
    matches = 0
    for _ in range(12):
        check_cancellation()
        time.sleep(0.2)
        current = sample(cgroup, pid)
        pair = current['compressedBytes'], current['logicalBytes']
        matches = matches + 1 if pair == previous else 1
        if matches >= 3:
            return current, True
        previous = pair
    return current, False


def compression(before, after, stable):
    logical = after['logicalBytes'] - before['logicalBytes']
    compressed = after['compressedBytes'] - before['compressedBytes']
    if not stable:
        return {'reason': 'the cgroup zswap counters did not stabilize'}
    if logical < MINIMUM_SAMPLE_BYTES or compressed <= 0:
        return {'reason': 'reclaim did not place at least 1 MiB of new pages in the zswap pool'}
    return {
        'newLogicalBytes': logical,
        'newCompressedBytes': compressed,
        'logicalToCompressedRatio': logical / compressed,
    }


def measure(report, node):
    uid, gid, temporary = hosted_runner()
    enabled = ZSWAP_PARAMETERS / 'enabled'
    if not enabled.is_file():
        raise Unsupported('host kernel exposes no zswap enable setting')
    original = enabled.read_text().strip()
    if original not in {'Y', 'N'}:
        raise Unsupported('host kernel reports an unknown zswap enable state')
    if 'memory' not in (CGROUP_ROOT / 'cgroup.subtree_control').read_text().split():
        raise Unsupported('host cgroup v2 does not enable the top-level memory controller')
    report['configuration'] = {
        'zswapEnabledAtStart': original == 'Y',
        'compressor': (ZSWAP_PARAMETERS / 'compressor').read_text().strip(),
        'hostSwapFreeBytesAtStart': available_swap(),
    }
    with ExitStack() as cleanup:
        cgroup = CGROUP_ROOT / f'pryx-zswap-{os.getpid()}'
        cgroup.mkdir()
        cleanup.callback(cgroup.rmdir)
        if not all((cgroup / name).exists() for name in ('memory.reclaim', 'memory.swap.current', 'cgroup.freeze')):
            raise Unsupported('cgroup v2 does not expose reclaim, swap and freeze for the fixture')
        if report['configuration']['hostSwapFreeBytesAtStart'] < 2 * RECLAIM_BYTES:
            try:
                cleanup.enter_context(add_swap(temporary))
            except InterruptedError:
                raise
            except (OSError, subprocess.SubprocessError) as error:
                raise Unsupported('host could not activate an owned scratch swap file') from error
            report['configuration']['addedSwapBytes'] = BACKING_SWAP_BYTES
        else:
            report['configuration']['addedSwapBytes'] = 0
        if original == 'N':
            cleanup.callback(enabled.write_text, 'N\n')
            enabled.write_text('Y\n')
        check_cancellation()
        with development_server(cgroup, node, uid, gid) as (pid, port, workload):
            report['workload'] = workload
            freeze(cgroup)
            report['before'], stable_before = stable_sample(cgroup, pid)
            report['reclaim'] = reclaim(cgroup)
            report['after'], stable_after = stable_sample(cgroup, pid)
            (cgroup / 'cgroup.freeze').write_text('0\n')
            if request(port) != workload:
                raise RuntimeError('the development server did not preserve its cache after thaw')
            measured = compression(report['before'], report['after'], stable_before and stable_after)
            report.update(measured)
            report['status'] = 'measured' if 'logicalToCompressedRatio' in measured else 'unconfirmed'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--node', required=True, type=Path)
    args = parser.parse_args()
    signal.signal(signal.SIGTERM, cancel)
    signal.signal(signal.SIGINT, cancel)
    report = {'benchmark': 'idle Node development HTTP server', 'kernel': platform.release()}
    result = 0
    try:
        measure(report, str(args.node))
    except Unsupported as error:
        report.update(status='unsupported', reason=str(error))
    except (RuntimeError, OSError, ValueError, subprocess.SubprocessError) as error:
        report.update(status='failed', reason=str(error))
        result = 1
    with args.output.open('x') as output:
        json.dump(report, output, indent=2)
        output.write('\n')
    return result


if __name__ == '__main__':
    sys.exit(main())

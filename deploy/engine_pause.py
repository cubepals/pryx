# SPDX-License-Identifier: FSL-1.1-ALv2
"""Pause only an isolated CI engine's verified listener holders.

SO_PEERCRED identifies the credentials at listen, which can be a waiting parent after an
engine reexec. This fixture follows the recorded launch identity and exact kernel socket,
not a process name or a host-wide group. It is not an installer or runtime controller.
"""
from contextlib import contextmanager
import json
import os
from pathlib import Path
import select
import signal
import socket
import stat
import struct
import sys
import time

# Linux include/linux/net.h: __SO_ACCEPTCON marks listening sockets in /proc/net/unix.
SO_ACCEPTCON = 1 << 16


def process(pid):
    path = Path('/proc') / str(pid)
    fields = (path / 'stat').read_text().rsplit(')', 1)[1].split()
    # proc_pid_stat(5): state, ppid and starttime are fields 3, 4 and 22.
    return {'pid': pid, 'state': fields[0], 'parent': int(fields[1]),
            'start': int(fields[19]), 'uid': path.stat().st_uid}


def pin(identity):
    descriptor = os.pidfd_open(identity['pid'])
    try:
        current = process(identity['pid'])
        assert current['start'] == identity['start'] and current['uid'] == os.getuid()
        assert not select.select([descriptor], [], [], 0)[0], 'The fixture process exited'
        return descriptor
    except BaseException:
        os.close(descriptor)
        raise


def private_path(owned, path, socket_file=False):
    relative = path.relative_to(owned)
    assert path.is_absolute() and '..' not in relative.parts
    for current in [owned, *[owned.joinpath(*relative.parts[:i]) for i in range(1, len(relative.parts) + 1)]]:
        entry = current.lstat()
        assert entry.st_uid == os.getuid() and not stat.S_ISLNK(entry.st_mode)
        if current == path and socket_file:
            assert stat.S_ISSOCK(entry.st_mode)
        else:
            assert stat.S_ISDIR(entry.st_mode)
        assert entry.st_mode & 0o077 == 0 or current == path and socket_file
    return (entry.st_dev, entry.st_ino)


def record(owned, pid):
    private_path(owned, owned)
    identity = process(pid)
    descriptor = pin(identity)
    try:
        identity['boot'] = Path('/proc/sys/kernel/random/boot_id').read_text().strip()
        target = os.open(owned / 'service.json', os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
        with os.fdopen(target, 'w') as output:
            json.dump(identity, output)
    finally:
        os.close(descriptor)


def table():
    entries = {}
    for path in Path('/proc').iterdir():
        if path.name.isdecimal():
            try:
                entry = process(int(path.name))
                entries[entry['pid']] = entry
            except (FileNotFoundError, ProcessLookupError):
                pass
    return entries


def within(pid, leader, entries):
    visited = set()
    while pid in entries and pid not in visited:
        if pid == leader:
            return True
        visited.add(pid)
        pid = entries[pid]['parent']
    return False


def listeners(peer, endpoint):
    found = set()
    for line in (Path('/proc') / str(peer) / 'net/unix').read_text().splitlines()[1:]:
        fields = line.split(maxsplit=7)
        if len(fields) == 8 and fields[7] == str(endpoint) and int(fields[3], 16) & SO_ACCEPTCON:
            found.add('socket:[' + fields[6] + ']')
    assert len(found) == 1, 'The fixture must have exactly one live API listener'
    return found


def holders(leader, sockets, entries):
    found = {}
    for pid, identity in entries.items():
        if identity['uid'] != os.getuid() or not within(pid, leader, entries):
            continue
        try:
            for entry in (Path('/proc') / str(pid) / 'fd').iterdir():
                try:
                    target = os.readlink(entry)
                except FileNotFoundError:
                    continue
                if target in sockets:
                    found[pid] = identity
                    break
        except (FileNotFoundError, ProcessLookupError):
            continue
    assert found, 'No listener holder belongs to the recorded fixture'
    return found


class Resumption:
    def __init__(self):
        self.stopped = {}
        self.started = False

    def __call__(self):
        if self.started:
            return
        self.started = True
        errors = []
        for _, descriptor in reversed(list(self.stopped.values())):
            try:
                signal.pidfd_send_signal(descriptor, signal.SIGCONT)
            except ProcessLookupError:
                pass
            except OSError as error:
                errors.append(error)
            finally:
                os.close(descriptor)
        if errors:
            raise RuntimeError('Could not resume every fixture listener holder') from errors[0]


@contextmanager
def cancellation(resume):
    cancelled = []

    def interrupted(number, _frame):
        cancelled.append(number)
        # Resume before unwinding. A second signal must not interrupt that cleanup.
        if not resume.started:
            resume()
            raise InterruptedError('The engine outage probe was interrupted')

    previous = {}
    try:
        for number in (signal.SIGTERM, signal.SIGINT):
            previous[number] = signal.getsignal(number)
            signal.signal(number, interrupted)
        yield
    finally:
        for number, handler in previous.items():
            signal.signal(number, handler)
        if cancelled:
            raise InterruptedError('The engine outage probe was interrupted')


@contextmanager
def paused(endpoint, owned):
    node = private_path(owned, endpoint, socket_file=True)
    source = os.open(owned / 'service.json', os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(source) as recorded:
        metadata = os.fstat(recorded.fileno())
        assert metadata.st_uid == os.getuid() and stat.S_ISREG(metadata.st_mode) and metadata.st_mode & 0o077 == 0
        identity = json.load(recorded)
    assert identity['pid'] > 1 and identity['boot'] == Path('/proc/sys/kernel/random/boot_id').read_text().strip()
    leader = pin(identity)
    resume = Resumption()
    try:
        with cancellation(resume):
            try:
                with socket.socket(socket.AF_UNIX) as peer:
                    peer.connect(str(endpoint))
                    pid, uid, _ = struct.unpack('3i', peer.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, struct.calcsize('3i')))
                    entries = table()
                    assert uid == os.getuid() and within(pid, identity['pid'], entries)
                    sockets = listeners(pid, endpoint)
                    protected = {candidate for candidate in entries if within(os.getpid(), candidate, entries)}
                    deadline = time.monotonic() + 5
                    while True:
                        assert not select.select([leader], [], [], 0)[0], 'The fixture launcher exited'
                        entries = table()
                        current = holders(identity['pid'], sockets, entries)
                        assert current.keys().isdisjoint(protected), 'The API listener belongs to the probe or an ancestor'
                        added = False
                        for holder, seen in current.items():
                            if holder not in resume.stopped:
                                assert seen['state'] not in {'T', 't'}, 'The fixture was already stopped'
                                descriptor = pin(seen)
                                resume.stopped[holder] = (seen['start'], descriptor)
                                signal.pidfd_send_signal(descriptor, signal.SIGSTOP)
                                added = True
                            start, descriptor = resume.stopped[holder]
                            assert start == seen['start'] and not select.select([descriptor], [], [], 0)[0]
                        assert time.monotonic() < deadline, 'The fixture listener did not stop'
                        if not added and all(process(holder)['state'] in {'T', 't'} for holder in current):
                            assert private_path(owned, endpoint, socket_file=True) == node
                            assert listeners(pid, endpoint) == sockets
                            break
                        time.sleep(0.01)
                yield
            finally:
                resume()
    finally:
        os.close(leader)


if __name__ == '__main__':
    assert sys.argv[1] == 'record'
    record(Path(sys.argv[2]), int(sys.argv[3]))

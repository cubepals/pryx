# SPDX-License-Identifier: FSL-1.1-ALv2
"""Real owned process controls for the isolated engine outage fixture.

The inherited listener exercises a fork/reexec peer identity; the probe must pause actual
holders, leave other fixture descendants alone, and resume on every exceptional path.
"""
import errno
import http.client
import json
import os
from pathlib import Path
import select
import signal
import socket
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

import engine_pause
from engine_pause import paused, process, record


SERVER = r'''
import ctypes, json, os, pathlib, signal, socket, sys, threading, time
protection = ctypes.CDLL(sys.argv[2], use_errno=True)
protection.protect.argtypes = []
protection.protect.restype = ctypes.c_int

def protect(role):
    if protection.protect() != 0:
        raise OSError(ctypes.get_errno(), 'Cannot protect fixture descendant')
    pathlib.Path(sys.argv[1]).with_suffix('.' + role + '-protected').touch()

listener = socket.socket(socket.AF_UNIX)
listener.bind(sys.argv[1])
listener.listen()
# A setup failure may terminate this owned session before readiness. Defer parent termination
# until its reaper is installed, while each child restores the original mask before serving.
previous_mask = signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGTERM, signal.SIGINT})
canary = os.fork()
if canary == 0:
    listener.close()
    signal.signal(signal.SIGUSR1, lambda _number, _frame: protect('canary'))
    pathlib.Path(sys.argv[1]).with_suffix('.canary-ready').touch()
    signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)
    while True:
        time.sleep(1)
canary_fd = os.pidfd_open(canary)
child = os.fork()
if child:
    child_fd = os.pidfd_open(child)
    retiring = False
    def retire():
        global retiring
        if retiring:
            return
        retiring = True
        for descriptor in (child_fd, canary_fd):
            try:
                signal.pidfd_send_signal(descriptor, signal.SIGTERM)
            except ProcessLookupError:
                pass
        for owned in (child, canary):
            os.waitpid(owned, 0)
        for descriptor in (child_fd, canary_fd):
            os.close(descriptor)
        pathlib.Path(sys.argv[1]).with_suffix('.reaped').write_text(json.dumps([child, canary]))
    def stop(_number, _frame):
        if not retiring:
            retire()
            raise SystemExit()
    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)
    print(json.dumps({'parent': os.getpid(), 'child': child, 'canary': canary}), flush=True)
    try:
        while True:
            signal.pause()
    finally:
        retire()
else:
    os.close(canary_fd)
    signal.signal(signal.SIGUSR1, lambda _number, _frame: protect('listener'))
    signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)
    def answer(peer):
        with peer:
            if peer.recv(4096):
                try:
                    peer.sendall(b'HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK')
                except (BrokenPipeError, ConnectionResetError):
                    pass
    while True:
        peer, _ = listener.accept()
        threading.Thread(target=answer, args=(peer,), daemon=True).start()
'''


class EnginePause(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix='pryx-engine-pause-')
        self.addCleanup(self.directory.cleanup)
        self.owned = Path(self.directory.name)
        self.endpoint = self.owned / 'engine.sock'
        protection = self.owned / 'protection.so'
        # Import the kernel interface's constant from its owner, rather than copy its number.
        subprocess.run(['cc', '-shared', '-fPIC', '-x', 'c', '-o', str(protection), '-'],
                       input='#include <sys/prctl.h>\nint protect(void) { return prctl(PR_SET_DUMPABLE, 0, 0, 0, 0); }\n',
                       text=True, capture_output=True, check=True, timeout=5)
        self.identity = None
        self.descriptors = []
        self.server = subprocess.Popen([sys.executable, '-c', SERVER, str(self.endpoint), str(protection)],
                                       stdout=subprocess.PIPE, text=True, start_new_session=True)
        self.addCleanup(self.cleanup_server)
        self.assertTrue(select.select([self.server.stdout], [], [], 5)[0], 'The fixture must report readiness')
        self.identity = json.loads(self.server.stdout.readline())
        for key in ('parent', 'child', 'canary'):
            self.descriptors.append(os.pidfd_open(self.identity[key]))
        record(self.owned, self.server.pid)
        self.wait_for_marker('canary-ready')

    def wait_for_marker(self, suffix):
        deadline = time.monotonic() + 5
        marker = self.endpoint.with_suffix('.' + suffix)
        while not marker.exists():
            self.assertLess(time.monotonic(), deadline, 'The owned descendant must report its state')
            time.sleep(0.01)

    def confirm_protected(self, role, pid):
        self.wait_for_marker(role + '-protected')
        with self.assertRaises(PermissionError):
            for descriptor in (Path('/proc') / str(pid) / 'fd').iterdir():
                os.readlink(descriptor)

    def protect_during_scan(self, role):
        scan = engine_pause.holders
        key = 'child' if role == 'listener' else 'canary'
        pid = self.identity[key]
        descriptor = self.descriptors[('parent', 'child', 'canary').index(key)]
        triggered = []

        def protected_scan(leader, sockets, entries, verified=()):
            if not triggered:
                # PR_SET_DUMPABLE can change /proc ownership. Capture the real same-UID
                # candidate first, then force that kernel transition before its fd scan.
                self.assertEqual(entries[pid]['uid'], os.getuid())
                signal.pidfd_send_signal(descriptor, signal.SIGUSR1)
                self.confirm_protected(role, pid)
                self.assertEqual(self.ping(), http.client.OK)
                triggered.append(True)
            return scan(leader, sockets, entries, verified)

        return protected_scan, triggered

    def cleanup_server(self):
        # The owned parent reaps both children before exiting; never abandon them to PID1.
        try:
            if len(self.descriptors) == 3:
                for descriptor in self.descriptors:
                    try:
                        signal.pidfd_send_signal(descriptor, signal.SIGCONT)
                    except ProcessLookupError:
                        pass
                signal.pidfd_send_signal(self.descriptors[0], signal.SIGTERM)
            else:
                # This session was created by Popen and is still pinned by its unreaped child.
                # The launcher's blocked setup signals ensure it installs its reaper first.
                for number in (signal.SIGCONT, signal.SIGTERM):
                    try:
                        os.killpg(self.server.pid, number)
                    except ProcessLookupError:
                        pass
            self.server.wait(timeout=5)
            reaped = self.endpoint.with_suffix('.reaped')
            if self.identity is not None:
                self.assertEqual(set(json.loads(reaped.read_text())),
                                 {self.identity['child'], self.identity['canary']})
            elif reaped.exists():
                self.assertEqual(len(set(json.loads(reaped.read_text()))), 2)
        finally:
            for descriptor in self.descriptors:
                os.close(descriptor)
            self.server.stdout.close()

    def ping(self):
        with socket.socket(socket.AF_UNIX) as peer:
            peer.settimeout(0.1)
            peer.connect(str(self.endpoint))
            connection = http.client.HTTPConnection('localhost', timeout=0.1)
            connection.sock = peer
            connection.request('GET', '/_ping')
            return connection.getresponse().status

    def test_inherited_listener_holders_stop_and_recover(self):
        self.assertEqual(self.ping(), http.client.OK)
        with paused(self.endpoint, self.owned):
            self.assertIn(process(self.identity['parent'])['state'], {'T', 't'})
            self.assertIn(process(self.identity['child'])['state'], {'T', 't'})
            self.assertNotIn(process(self.identity['canary'])['state'], {'T', 't'})
            with self.assertRaises(TimeoutError):
                self.ping()
        self.assertEqual(self.ping(), http.client.OK)

    def test_a_protected_nonlistener_is_never_signalled(self):
        pin = engine_pause.pin
        send = signal.pidfd_send_signal
        identities = {}
        sent = []

        def remember_pin(identity):
            descriptor = pin(identity)
            identities[descriptor] = identity['pid']
            return descriptor

        def remember_signal(descriptor, number):
            if number in (signal.SIGSTOP, signal.SIGCONT):
                sent.append((identities[descriptor], number))
            return send(descriptor, number)

        self.assertEqual(self.ping(), http.client.OK)
        scan, triggered = self.protect_during_scan('canary')
        with (
            patch.object(engine_pause, 'holders', scan),
            patch.object(engine_pause, 'pin', remember_pin),
            patch.object(engine_pause.signal, 'pidfd_send_signal', remember_signal),
        ):
            with paused(self.endpoint, self.owned):
                self.assertIn(process(self.identity['parent'])['state'], {'T', 't'})
                self.assertIn(process(self.identity['child'])['state'], {'T', 't'})
                self.assertNotIn(process(self.identity['canary'])['state'], {'T', 't'})
        self.assertTrue(triggered)
        self.assertEqual({pid for pid, _ in sent}, {self.identity['parent'], self.identity['child']})
        self.assertEqual(self.ping(), http.client.OK)

    def test_a_protected_listener_refuses_before_yield_and_resumes_holders(self):
        self.assertEqual(self.ping(), http.client.OK)
        scan, triggered = self.protect_during_scan('listener')
        with patch.object(engine_pause, 'holders', scan):
            with self.assertRaisesRegex(AssertionError, 'The paused engine still answered its API'):
                with paused(self.endpoint, self.owned):
                    self.fail('An unpaused protected responder was admitted')
        self.assertTrue(triggered)
        self.assertNotIn(process(self.identity['parent'])['state'], {'T', 't'})
        self.assertNotIn(process(self.identity['child'])['state'], {'T', 't'})
        self.assertNotIn(process(self.identity['canary'])['state'], {'T', 't'})
        self.assertEqual(self.ping(), http.client.OK)

    def test_verified_holder_fd_access_loss_refuses_and_resumes_all(self):
        scan = engine_pause.holders
        iterate = Path.iterdir
        denied = []

        def scan_with_access_loss(leader, sockets, entries, verified=()):
            if verified and not denied:
                holder = next(iter(verified))
                target = Path('/proc') / str(holder) / 'fd'

                def inaccessible(path):
                    if path == target:
                        denied.append(holder)
                        raise PermissionError(errno.EACCES, os.strerror(errno.EACCES), str(path))
                    return iterate(path)

                with patch.object(engine_pause.Path, 'iterdir', inaccessible):
                    return scan(leader, sockets, entries, verified)
            return scan(leader, sockets, entries, verified)

        with patch.object(engine_pause, 'holders', scan_with_access_loss):
            with self.assertRaises(PermissionError):
                with paused(self.endpoint, self.owned):
                    self.fail('A verified holder lost access without refusing the outage')
        self.assertTrue(denied)
        self.assertNotIn(process(self.identity['parent'])['state'], {'T', 't'})
        self.assertNotIn(process(self.identity['child'])['state'], {'T', 't'})
        self.assertEqual(self.ping(), http.client.OK)

    def test_error_resumes_every_holder(self):
        with self.assertRaisesRegex(RuntimeError, 'owned failure'):
            with paused(self.endpoint, self.owned):
                raise RuntimeError('owned failure')
        self.assertEqual(self.ping(), http.client.OK)
        self.assertNotIn(process(self.identity['parent'])['state'], {'T', 't'})
        self.assertNotIn(process(self.identity['child'])['state'], {'T', 't'})

    def test_interruptions_resume_and_restore_handlers(self):
        for number in (signal.SIGTERM, signal.SIGINT):
            previous = {owned: signal.getsignal(owned) for owned in (signal.SIGTERM, signal.SIGINT)}
            with self.assertRaises(InterruptedError):
                with paused(self.endpoint, self.owned):
                    signal.raise_signal(number)
            self.assertEqual(self.ping(), http.client.OK)
            self.assertEqual({owned: signal.getsignal(owned) for owned in previous}, previous)

    def test_partial_stop_error_resumes_prior_holder(self):
        send = signal.pidfd_send_signal
        stops = []

        def fail_second_stop(descriptor, number):
            if number == signal.SIGSTOP:
                stops.append(descriptor)
                if len(stops) == 2:
                    # Generate a real kernel EINVAL while the first holder is already stopped.
                    return send(descriptor, signal.SIGRTMAX + 1)
            return send(descriptor, number)

        with patch.object(engine_pause.signal, 'pidfd_send_signal', fail_second_stop):
            with self.assertRaises(OSError):
                with paused(self.endpoint, self.owned):
                    self.fail('An incomplete stop was admitted')
        self.assertEqual(len(stops), 2)
        self.assertEqual(self.ping(), http.client.OK)

    def test_repeated_termination_during_cleanup_resumes_all(self):
        send = signal.pidfd_send_signal
        again = []

        def interrupt_cleanup(descriptor, number):
            if number == signal.SIGCONT and not again:
                again.append(True)
                signal.raise_signal(signal.SIGTERM)
            return send(descriptor, number)

        with patch.object(engine_pause.signal, 'pidfd_send_signal', interrupt_cleanup):
            with self.assertRaises(InterruptedError):
                with paused(self.endpoint, self.owned):
                    signal.raise_signal(signal.SIGTERM)
        self.assertTrue(again)
        self.assertEqual(self.ping(), http.client.OK)

    def rewrite_identity(self, change):
        path = self.owned / 'service.json'
        identity = json.loads(path.read_text())
        change(identity)
        path.write_text(json.dumps(identity))

    def test_stale_start_refuses_without_a_signal(self):
        self.rewrite_identity(lambda identity: identity.update(start=identity['start'] + 1))
        with self.assertRaises(AssertionError):
            with paused(self.endpoint, self.owned):
                self.fail('Stale identity was admitted')
        self.assertEqual(self.ping(), http.client.OK)

    def test_other_launcher_refuses_without_a_signal(self):
        canary = process(self.identity['canary'])
        self.rewrite_identity(lambda identity: identity.update(pid=canary['pid'], start=canary['start']))
        with self.assertRaises(AssertionError):
            with paused(self.endpoint, self.owned):
                self.fail('Other launcher was admitted')
        self.assertEqual(self.ping(), http.client.OK)
        self.assertNotIn(process(canary['pid'])['state'], {'T', 't'})

    def test_symlink_endpoint_refuses(self):
        alias = self.owned / 'alias.sock'
        alias.symlink_to(self.endpoint)
        with self.assertRaises(AssertionError):
            with paused(alias, self.owned):
                self.fail('Socket alias was admitted')
        self.assertEqual(self.ping(), http.client.OK)

    def test_nonprivate_fixture_refuses(self):
        self.owned.chmod(0o755)
        try:
            with self.assertRaises(AssertionError):
                with paused(self.endpoint, self.owned):
                    self.fail('A nonprivate fixture was admitted')
        finally:
            self.owned.chmod(0o700)
        self.assertEqual(self.ping(), http.client.OK)

    def test_nonregular_identity_refuses_without_blocking(self):
        source = self.owned / 'service.json'
        source.unlink()
        os.mkfifo(source, 0o600)
        with self.assertRaises(AssertionError):
            with paused(self.endpoint, self.owned):
                self.fail('A FIFO identity was admitted')
        self.assertEqual(self.ping(), http.client.OK)


if __name__ == '__main__':
    unittest.main()

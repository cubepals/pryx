# SPDX-License-Identifier: FSL-1.1-ALv2
"""Real owned process controls for the isolated engine outage fixture.

The inherited listener exercises a fork/reexec peer identity; the probe must pause actual
holders, leave other fixture descendants alone, and resume on every exceptional path.
"""
import http.client
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import engine_pause
from engine_pause import paused, process, record


SERVER = r'''
import json, os, pathlib, signal, socket, sys, time
listener = socket.socket(socket.AF_UNIX)
listener.bind(sys.argv[1])
listener.listen()
canary = os.fork()
if canary == 0:
    listener.close()
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
    print(json.dumps({'parent': os.getpid(), 'child': child, 'canary': canary}), flush=True)
    try:
        while True:
            signal.pause()
    finally:
        retire()
else:
    os.close(canary_fd)
    while True:
        peer, _ = listener.accept()
        with peer:
            if peer.recv(4096):
                try:
                    peer.sendall(b'HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK')
                except (BrokenPipeError, ConnectionResetError):
                    pass
'''


class EnginePause(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix='pryx-engine-pause-')
        self.addCleanup(self.directory.cleanup)
        self.owned = Path(self.directory.name)
        self.endpoint = self.owned / 'engine.sock'
        self.server = subprocess.Popen([sys.executable, '-c', SERVER, str(self.endpoint)],
                                       stdout=subprocess.PIPE, text=True, start_new_session=True)
        self.addCleanup(self.cleanup_server)
        self.identity = json.loads(self.server.stdout.readline())
        self.descriptors = [os.pidfd_open(self.identity[key]) for key in ('parent', 'child', 'canary')]
        record(self.owned, self.server.pid)

    def cleanup_server(self):
        # The owned parent reaps both children before exiting; never abandon them to PID1.
        try:
            for descriptor in self.descriptors:
                try:
                    signal.pidfd_send_signal(descriptor, signal.SIGCONT)
                except ProcessLookupError:
                    pass
            signal.pidfd_send_signal(self.descriptors[0], signal.SIGTERM)
            self.server.wait(timeout=5)
            self.assertEqual(set(json.loads(self.endpoint.with_suffix('.reaped').read_text())),
                             {self.identity['child'], self.identity['canary']})
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

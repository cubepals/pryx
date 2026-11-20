#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Exercise the artifact watchdog against a maintained client and real loopback byte streams.

This tests transfer progress, stalled reads and failed clients without contacting GitHub or
using credentials. The shell fixture separately authenticates complete transport parts.
"""
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import importlib.util
import os
from pathlib import Path
import shutil
import select
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest

_spec = importlib.util.spec_from_file_location('artifact_read', Path(__file__).with_name('artifact-read.py'))
reader = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(reader)
PAYLOAD = bytes(range(256)) * 1024


class ReadTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.stage = self.root / 'stage'
        self.stage.mkdir()
        self.log = self.root / 'client.log'
        self.release = threading.Event()
        release = self.release

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_GET(self):
                if self.path == '/failure':
                    self.send_error(HTTPStatus.SERVICE_UNAVAILABLE)
                    return
                self.send_response(HTTPStatus.OK)
                self.send_header('Content-Length', str(len(PAYLOAD)))
                self.end_headers()
                try:
                    if self.path == '/stalled':
                        self.wfile.write(PAYLOAD[:32768])
                        self.wfile.flush()
                        release.wait(5)
                        return
                    for start in range(0, len(PAYLOAD), 32768):
                        self.wfile.write(PAYLOAD[start:start + 32768])
                        self.wfile.flush()
                        time.sleep(0.1)
                except (BrokenPipeError, ConnectionResetError):
                    pass

        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        thread.start()
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)
        self.addCleanup(self.release.set)

    def command(self, path):
        return [shutil.which('curl'), '--silent', '--show-error', '--fail',
                f'http://127.0.0.1:{self.server.server_port}/{path}',
                '--output', str(self.stage / 'part')]

    def test_progressing_transfer_outlives_the_idle_interval(self):
        control = self.command('flow')
        control[-1] = str(self.root / 'control-part')
        result = subprocess.run(['timeout', '0.35s', *control], check=False,
                                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.assertEqual(result.returncode, reader.TIMEOUT_EXIT_STATUS)
        self.assertGreater((self.root / 'control-part').stat().st_size, 0)
        self.assertLess((self.root / 'control-part').stat().st_size, len(PAYLOAD))
        started = time.monotonic()
        self.assertEqual(reader.read(self.command('flow'), self.stage, self.log, 0.35), 0)
        self.assertGreater(time.monotonic() - started, 0.7)
        self.assertEqual((self.stage / 'part').read_bytes(), PAYLOAD)

    def test_stalled_partial_transfer_is_stopped_and_diagnosed(self):
        started = time.monotonic()
        self.assertEqual(reader.read(self.command('stalled'), self.stage, self.log, 0.35),
                         reader.TIMEOUT_EXIT_STATUS)
        self.assertLess(time.monotonic() - started, 2)
        self.assertIn('idle timeout', self.log.read_text())
        self.assertIn('observed_bytes=32768', self.log.read_text())
        self.assertEqual((self.stage / 'part').stat().st_size, 32768)

    def test_client_failure_keeps_its_exit_and_safe_progress_metadata(self):
        control = subprocess.run(self.command('failure'), check=False, stdout=subprocess.DEVNULL,
                                 stderr=subprocess.DEVNULL)
        self.assertNotEqual(control.returncode, 0)
        self.assertEqual(reader.read(self.command('failure'), self.stage, self.log, 0.35),
                         control.returncode)
        self.assertIn(f'Artifact read exited {control.returncode}; observed_bytes=0', self.log.read_text())
        self.assertFalse((self.stage / 'part').exists())

    def test_client_messages_stay_out_of_the_watchdog_log(self):
        command = [sys.executable, '-c',
                   'import os, sys; print("fixture stdout"); '
                   'print("fixture stderr", file=sys.stderr); sys.exit(os.EX_UNAVAILABLE)']
        self.assertEqual(reader.read(command, self.stage, self.log, 0.35), os.EX_UNAVAILABLE)
        diagnostic = self.log.read_text()
        self.assertIn(f'Artifact read exited {os.EX_UNAVAILABLE}; observed_bytes=0', diagnostic)
        self.assertNotIn('fixture stdout', diagnostic)
        self.assertNotIn('fixture stderr', diagnostic)

    def test_cancellation_stops_the_client_and_cleans_its_temporary_directory(self):
        # Record the controlled client's PID before exec, without requiring procfs children files.
        client = [sys.executable, '-c',
                  'import os,pathlib,sys;pathlib.Path(sys.argv[1]).write_text(str(os.getpid()));'
                  'os.execv(sys.argv[2],sys.argv[2:])', str(self.root / 'client.pid'),
                  *self.command('stalled')]
        command = [sys.executable, str(Path(__file__).with_name('artifact-read.py')),
                   str(self.stage), str(self.log), '30', *client]
        process = subprocess.Popen(command, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            deadline = time.monotonic() + 3
            while not (self.stage / 'part').exists() or not (self.stage / 'part').stat().st_size:
                self.assertIsNone(process.poll())
                self.assertLess(time.monotonic(), deadline)
                time.sleep(0.01)
            child = int((self.root / 'client.pid').read_text())
            descriptor = os.pidfd_open(child)
            self.addCleanup(os.close, descriptor)
            poll = select.poll()
            poll.register(descriptor, select.POLLIN)
            process.send_signal(signal.SIGTERM)
            self.assertNotEqual(process.wait(timeout=2), 0)
            self.assertTrue(poll.poll(1000), 'Cancellation must stop the maintained client')
            self.assertEqual(list(self.root.glob('gh-artifact-read-*')), [])
        finally:
            if process.poll() is None:
                process.terminate()
            process.wait(timeout=2)


if __name__ == '__main__':
    unittest.main()

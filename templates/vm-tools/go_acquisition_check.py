#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Exercise real pinned Go acquisition after the separate tool has been verified.

An owned loopback HTTPS/HTTP2 proxy injects errors into generated module archives.
Go supplies the fixture checksums and module cache; no public module or credential is used.
"""
from contextlib import contextmanager, redirect_stderr
import io
import json
import os
from pathlib import Path
import select
import signal
import ssl
import subprocess
import sys
import tempfile
import time
import unittest
import urllib.request
import zipfile

from go_acquisition import module_archive, module_list


ROOT = Path(__file__).resolve().parent
MODULE = 'example.org/retry'
VERSION = 'v1.0.0'
ZIP_PATH = '/' + MODULE + '/@v/' + VERSION + '.zip'


class Acquisition(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.go = Path(os.environ['PRYX_TEST_GO']).resolve(strict=True)
        cls.temporary = tempfile.TemporaryDirectory()
        cls.base = Path(cls.temporary.name)
        cls.binary = cls.base / 'proxy'
        cls.environment = {**os.environ, 'GOTOOLCHAIN': 'local', 'GOWORK': 'off',
                           'GOENV': 'off', 'GOFLAGS': '', 'CGO_ENABLED': '0',
                           'GOSUMDB': 'off', 'GONOSUMDB': '', 'GOPRIVATE': '',
                           'GONOPROXY': '', 'GOMAXPROCS': '2', 'GODEBUG': ''}
        subprocess.run([str(cls.go), 'build', '-o', str(cls.binary),
                        str(ROOT / 'testdata/go-acquisition/main.go')],
                       env={**cls.environment, 'GOPROXY': 'off'}, check=True,
                       stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        cls.proxy = cls.base / 'fixture'
        endpoint = cls.proxy / MODULE / '@v'
        endpoint.mkdir(parents=True)
        cls.module = ('module ' + MODULE + '\n\ngo ' + next(
            line.split()[1] for line in (ROOT / 'go.mod').read_text().splitlines()
            if line.startswith('go ')) + '\n').encode()
        (endpoint / (VERSION + '.mod')).write_bytes(cls.module)
        (endpoint / (VERSION + '.info')).write_text(json.dumps(
            {'Version': VERSION, 'Time': '2020-01-01T00:00:00Z'}))
        cls.archive = endpoint / (VERSION + '.zip')
        cls.write_archive('const Value = 42\n')
        seed = cls.base / 'seed'
        seed.mkdir()
        (seed / 'go.mod').write_text(cls.main_module())
        subprocess.run([str(cls.go), 'mod', 'download', '-json', MODULE + '@' + VERSION],
                       cwd=seed, env={**cls.environment, 'GOPROXY': cls.proxy.as_uri(),
                                      'GOMODCACHE': str(cls.base / 'seed-cache')},
                       check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        cls.sums = (seed / 'go.sum').read_bytes()

    @classmethod
    def tearDownClass(cls):
        cls.temporary.cleanup()

    @classmethod
    def main_module(cls):
        version = cls.module.decode().split('go ')[1].strip()
        return f'module fixtures.pryx.invalid/main\n\ngo {version}\n\nrequire {MODULE} {VERSION}\n'

    @classmethod
    def write_archive(cls, source):
        with zipfile.ZipFile(cls.archive, 'w', zipfile.ZIP_DEFLATED) as archive:
            prefix = MODULE + '@' + VERSION + '/'
            archive.writestr(prefix + 'go.mod', cls.module)
            archive.writestr(prefix + 'retry.go', 'package retry\n\n' + source)

    @contextmanager
    def proxy_case(self, mode):
        with tempfile.TemporaryDirectory(dir=self.base) as directory:
            root = Path(directory)
            (root / 'go.mod').write_text(self.main_module())
            (root / 'go.sum').write_bytes(self.sums)
            (root / 'main.go').write_text('package main\nimport "' + MODULE + '"\nvar _ = retry.Value\n')
            locks = {name: (root / name).read_bytes() for name in ('go.mod', 'go.sum')}
            certificate = root / 'certificate.pem'
            with (root / 'server.log').open('w') as log:
                process = subprocess.Popen([str(self.binary), str(self.proxy), mode, str(certificate)],
                                           stdout=subprocess.PIPE, stderr=log, text=True)
                try:
                    self.assertTrue(select.select([process.stdout], [], [], 5)[0], 'Proxy did not start')
                    url = process.stdout.readline().strip()
                    self.assertTrue(url.startswith('https://127.0.0.1:'))
                    env = {**self.environment, 'GOPROXY': url, 'GOMODCACHE': str(root / 'cache'),
                           'SSL_CERT_FILE': str(certificate)}
                    diagnostics = io.StringIO()
                    handlers = {value: signal.getsignal(value) for value in (signal.SIGTERM, signal.SIGINT)}
                    with redirect_stderr(diagnostics):
                        yield root, env, diagnostics, url, certificate
                    for value, expected in handlers.items():
                        self.assertEqual(signal.getsignal(value), expected, 'Caller signal handler changed')
                    for name, expected in locks.items():
                        self.assertEqual((root / name).read_bytes(), expected, name + ' changed')
                finally:
                    process.terminate()
                    process.wait(timeout=5)
                    process.stdout.close()

    def count(self, url, certificate):
        with urllib.request.urlopen(url + '/counts', context=ssl.create_default_context(
                cafile=certificate)) as response:
            return json.load(response).get(ZIP_PATH, 0)

    def acquired(self, mode, archive=False):
        with self.proxy_case(mode) as (root, env, diagnostics, url, certificate):
            if archive:
                output = module_archive(self.go, root, MODULE + '@' + VERSION,
                                        env=env, budget=10, initial_wait=0.01)
                self.assertTrue(json.loads(output)['Sum'])
            else:
                output = module_list(self.go, root, ['./...'], env=env, budget=10, initial_wait=0.01)
                self.assertIn(MODULE, output)
            self.assertGreater(self.count(url, certificate), 1)
            self.assertIn('goModuleAcquisition', diagnostics.getvalue())
            self.assertNotIn('fixture response body', diagnostics.getvalue())
            self.assertNotIn('https://', diagnostics.getvalue())

    def test_partial_http2_stream_reset_is_recovered(self):
        self.acquired('reset')

    def test_source_archive_json_error_is_recovered(self):
        self.acquired('reset', archive=True)

    def test_service_unavailable_is_recovered(self):
        self.acquired('unavailable')

    def test_rate_limit_is_recovered(self):
        self.acquired('rate-limit')

    def test_other_http_refusals_are_not_retried(self):
        for mode in ('forbidden', 'missing'):
            with self.subTest(mode=mode), self.proxy_case(mode) as (root, env, diagnostics, url, certificate):
                control = subprocess.run([str(self.go), 'list', '-mod=readonly', '-deps',
                                          '-json=Module', './...'], cwd=root, env=env,
                                         stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=10)
                self.assertNotEqual(control.returncode, 0)
                baseline = self.count(url, certificate)
                self.assertGreater(baseline, 0)
                with self.assertRaisesRegex(RuntimeError, 'HTTP .*attempts=1'):
                    module_list(self.go, root, ['./...'], budget=10, initial_wait=0.01,
                                env={**env, 'GOMODCACHE': str(root / 'second-cache')})
                self.assertEqual(self.count(url, certificate) - baseline, baseline)
                self.assertEqual(diagnostics.getvalue(), '')

    def test_certificate_refusal_is_not_retried(self):
        with self.proxy_case('normal') as (root, env, diagnostics, _url, _certificate):
            with self.assertRaisesRegex(RuntimeError, 'certificate refusal; attempts=1'):
                module_list(self.go, root, ['./...'], budget=10, initial_wait=0.01,
                            env={**env, 'SSL_CERT_FILE': str(root / 'untrusted.pem')})
            self.assertEqual(diagnostics.getvalue(), '')

    def test_checksum_failure_is_not_retried(self):
        try:
            self.write_archive('const Value = 43\n')
            with self.proxy_case('normal') as (root, env, diagnostics, url, certificate):
                with self.assertRaisesRegex(RuntimeError, 'checksum refusal'):
                    module_list(self.go, root, ['./...'], env=env, budget=10, initial_wait=0.01)
                self.assertEqual(self.count(url, certificate), 1)
                self.assertEqual(diagnostics.getvalue(), '')
        finally:
            self.write_archive('const Value = 42\n')

    def test_stalled_read_has_a_deadline(self):
        with self.proxy_case('stall') as (root, env, _diagnostics, _url, _certificate):
            started = time.monotonic()
            with self.assertRaisesRegex(RuntimeError, 'read deadline'):
                module_list(self.go, root, ['./...'], env=env, budget=0.5, initial_wait=0.01)
            self.assertLess(time.monotonic() - started, 2)

    def test_retries_share_one_deadline(self):
        with self.proxy_case('persistent-unavailable') as (root, env, diagnostics, url, certificate):
            started = time.monotonic()
            with self.assertRaises(RuntimeError):
                module_list(self.go, root, ['./...'], env=env, budget=0.5, initial_wait=0.01)
            self.assertLess(time.monotonic() - started, 2)
            self.assertGreater(diagnostics.getvalue().count('goModuleAcquisition'), 1)
            self.assertGreater(self.count(url, certificate), 1)

    def cancelled_owner(self, value, *, custom=False, backoff=False, archive=False):
        with self.proxy_case('persistent-unavailable' if backoff else 'stall') as (
                root, env, _diagnostics, url, certificate):
            pid_file, marker = root / 'child.pid', root / 'handler.json'
            command = [sys.executable, str(ROOT / 'testdata/go-acquisition/owner.py'),
                       '--source', str(ROOT), '--go', str(self.go), '--root', str(root),
                       '--pid-file', str(pid_file)]
            if custom:
                command += ['--custom-handler', str(marker)]
            if archive:
                command += ['--archive']
            pid_fd = None
            with (root / 'owner.log').open('w+') as log:
                parent = subprocess.Popen(command, env=env, stdout=log, stderr=log)
                try:
                    deadline = time.monotonic() + 5
                    while not pid_file.exists():
                        self.assertIsNone(parent.poll(), 'Owner exited before starting Go')
                        self.assertLess(time.monotonic(), deadline, 'Go did not start')
                        time.sleep(0.01)
                    if not backoff:
                        pid_fd = os.pidfd_open(int(pid_file.read_text()))
                    while True:
                        log.flush()
                        if backoff:
                            ready = 'goModuleAcquisition' in (root / 'owner.log').read_text()
                        else:
                            ready = self.count(url, certificate) > 0
                        if ready:
                            break
                        self.assertLess(time.monotonic(), deadline, 'Go did not reach the read or backoff')
                        self.assertIsNone(parent.poll(), 'Owner exited before cancellation')
                        time.sleep(0.01)
                    started = time.monotonic()
                    parent.send_signal(value)
                    parent.wait(timeout=3)
                    self.assertLess(time.monotonic() - started, 2)
                    self.assertNotEqual(parent.returncode, 0)
                    if value == signal.SIGTERM and not custom:
                        self.assertEqual(parent.returncode, -signal.SIGTERM)
                    if pid_fd is not None:
                        self.assertTrue(select.select([pid_fd], [], [], 2)[0], 'Owned Go survived cancellation')
                    if custom:
                        self.assertEqual(json.loads(marker.read_text()), {'restored': True, 'childExited': True})
                finally:
                    if pid_fd is not None:
                        if not select.select([pid_fd], [], [], 0)[0]:
                            signal.pidfd_send_signal(pid_fd, signal.SIGKILL)
                        os.close(pid_fd)
                    if parent.poll() is None:
                        try:
                            parent.wait(timeout=2)
                        except subprocess.TimeoutExpired:
                            parent.kill()
                            parent.wait()

    def test_owner_sigterm_reaps_real_go(self):
        self.cancelled_owner(signal.SIGTERM)

    def test_owner_sigint_reaps_real_go_archive_reader(self):
        self.cancelled_owner(signal.SIGINT, archive=True)

    def test_caller_handler_runs_restored_after_go_exits(self):
        self.cancelled_owner(signal.SIGTERM, custom=True)

    def test_owner_cancellation_interrupts_retry_wait(self):
        self.cancelled_owner(signal.SIGTERM, backoff=True)

    def test_foreign_child_reaping_refuses_before_spawn(self):
        with self.proxy_case('normal') as (root, env, _diagnostics, _url, _certificate):
            pid_file = root / 'child.pid'
            result = subprocess.run([
                sys.executable, str(ROOT / 'testdata/go-acquisition/owner.py'), '--source', str(ROOT),
                '--go', str(self.go), '--root', str(root), '--pid-file', str(pid_file), '--auto-reap',
            ], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=5)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(b'requires owned child reaping', result.stderr)
            self.assertFalse(pid_file.exists())


if __name__ == '__main__':
    unittest.main()

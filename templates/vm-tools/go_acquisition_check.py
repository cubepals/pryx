#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Real Go module reads through go_acquisition.py, against an owned loopback HTTPS/HTTP2 proxy.

The pinned Go (PRYX_TEST_GO) builds the proxy fixture and fetches a module it generates, so every
error text the policy reads is Go's own: a stream reset, 503, 429, 403, 404, a checksum and a
certificate refusal. No public module or credential is used. The retry budget is shortened.
"""
from contextlib import contextmanager, redirect_stderr
import io
import json
import os
from pathlib import Path
import select
import ssl
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import urllib.request
import zipfile

from go_acquisition import archive_retry, module_archive, module_list

ROOT = Path(__file__).resolve().parent
MODULE = 'example.org/retry'
VERSION = 'v1.0.0'
ARCHIVE = '/' + MODULE + '/@v/' + VERSION + '.zip'


class Acquisition(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.go = Path(os.environ['PRYX_TEST_GO']).resolve(strict=True)
        cls.temporary = tempfile.TemporaryDirectory()
        cls.base = Path(cls.temporary.name)
        cls.environment = {**os.environ, 'GOTOOLCHAIN': 'local', 'GOWORK': 'off', 'GOENV': 'off',
                           'GOFLAGS': '-modcacherw', 'CGO_ENABLED': '0', 'GOSUMDB': 'off',
                           'GONOSUMDB': '', 'GOPRIVATE': '', 'GONOPROXY': '', 'GODEBUG': ''}
        cls.proxy_program = cls.base / 'proxy'
        subprocess.run([str(cls.go), 'build', '-o', str(cls.proxy_program),
                        str(ROOT / 'testdata/go-acquisition/main.go')], cwd=cls.base,
                       env={**cls.environment, 'GOPROXY': 'off'}, check=True, capture_output=True)
        language = subprocess.run([str(cls.go), 'env', 'GOVERSION'], env=cls.environment, check=True,
                                  capture_output=True, text=True).stdout.strip().removeprefix('go')
        cls.module_file = f'module {MODULE}\n\ngo {language}\n'.encode()
        cls.main_module = (f'module fixtures.pryx.invalid/main\n\ngo {language}\n\n'
                           f'require {MODULE} {VERSION}\n')
        cls.served = cls.base / 'served'
        versions = cls.served / MODULE / '@v'
        versions.mkdir(parents=True)
        (versions / (VERSION + '.mod')).write_bytes(cls.module_file)
        (versions / (VERSION + '.info')).write_text(json.dumps({'Version': VERSION,
                                                                'Time': '2020-01-01T00:00:00Z'}))
        cls.archive = versions / (VERSION + '.zip')
        cls.write_archive('const Value = 42\n')
        seed = cls.base / 'seed'
        seed.mkdir()
        (seed / 'go.mod').write_text(cls.main_module)
        subprocess.run([str(cls.go), 'mod', 'download', '-json', MODULE + '@' + VERSION], cwd=seed,
                       env={**cls.environment, 'GOPROXY': cls.served.as_uri(),
                            'GOMODCACHE': str(cls.base / 'seed-cache')}, check=True, capture_output=True)
        cls.sums = (seed / 'go.sum').read_bytes()

    @classmethod
    def tearDownClass(cls):
        cls.temporary.cleanup()

    @classmethod
    def write_archive(cls, source):
        with zipfile.ZipFile(cls.archive, 'w', zipfile.ZIP_DEFLATED) as archive:
            prefix = MODULE + '@' + VERSION + '/'
            archive.writestr(prefix + 'go.mod', cls.module_file)
            archive.writestr(prefix + 'retry.go', 'package retry\n\n' + source)

    def setUp(self):
        for name, value in [('RETRY_BUDGET', 3.0), ('FIRST_RETRY_WAIT', 0.01), ('LONGEST_RETRY_WAIT', 0.05)]:
            shortened = patch.object(archive_retry, name, value)
            shortened.start()
            self.addCleanup(shortened.stop)

    @contextmanager
    def proxy(self, mode):
        """A main module locked to the fixture module, and the proxy answering in mode."""
        with tempfile.TemporaryDirectory(dir=self.base) as directory:
            root = Path(directory)
            (root / 'go.mod').write_text(self.main_module)
            (root / 'go.sum').write_bytes(self.sums)
            (root / 'main.go').write_text('package main\n\nimport "' + MODULE + '"\n\nvar _ = retry.Value\n')
            certificate = root / 'certificate.pem'
            with (root / 'proxy.log').open('w') as log:
                process = subprocess.Popen([str(self.proxy_program), str(self.served), mode, str(certificate)],
                                           stdout=subprocess.PIPE, stderr=log, text=True)
                try:
                    self.assertTrue(select.select([process.stdout], [], [], 10)[0], 'The proxy did not start')
                    url = process.stdout.readline().strip()
                    self.assertTrue(url.startswith('https://127.0.0.1:'), url)
                    env = {**self.environment, 'GOPROXY': url, 'GOMODCACHE': str(root / 'cache'),
                           'SSL_CERT_FILE': str(certificate)}
                    diagnostics = io.StringIO()
                    with redirect_stderr(diagnostics):
                        yield root, env, diagnostics, lambda: self.archive_requests(url, certificate)
                    self.assertEqual((root / 'go.sum').read_bytes(), self.sums, 'go.sum changed')
                finally:
                    process.terminate()
                    process.wait(timeout=5)
                    process.stdout.close()

    def archive_requests(self, url, certificate):
        context = ssl.create_default_context(cafile=certificate)
        with urllib.request.urlopen(url + '/counts', context=context) as response:
            return json.load(response).get(ARCHIVE, 0)

    def recovered(self, mode, archive=False):
        with self.proxy(mode) as (root, env, log, requests):
            if archive:
                self.assertTrue(json.loads(module_archive(self.go, root, MODULE + '@' + VERSION, env))['Sum'])
            else:
                self.assertIn(MODULE, module_list(self.go, root, ['./...'], env))
            self.assertGreater(requests(), 1)
            self.assertIn('"goModuleAcquisition": "retry"', log.getvalue())

    def test_an_http2_stream_reset_is_read_again(self):
        # The failure seen in CI: `stream error: stream ID 11; INTERNAL_ERROR; received from peer`.
        self.recovered('reset')

    def test_an_archive_download_reset_is_read_again(self):
        self.recovered('reset', archive=True)

    def test_service_unavailable_is_waited_out(self):
        self.recovered('unavailable')

    def test_rate_limiting_is_waited_out(self):
        self.recovered('rate-limit')

    def test_other_refusals_fail_at_once(self):
        for mode in ['forbidden', 'missing']:
            with self.subTest(mode=mode), self.proxy(mode) as (root, env, log, requests):
                with self.assertRaisesRegex(RuntimeError, r'HTTP 40\d; attempts=1$'):
                    module_list(self.go, root, ['./...'], env)
                self.assertNotIn('goModuleAcquisition', log.getvalue())

    def test_an_untrusted_certificate_fails_at_once(self):
        with self.proxy('normal') as (root, env, log, requests):
            with self.assertRaisesRegex(RuntimeError, 'certificate refusal; attempts=1$'):
                module_list(self.go, root, ['./...'], {**env, 'SSL_CERT_FILE': os.devnull})
            self.assertEqual(requests(), 0)

    def test_other_bytes_fail_the_checksum_at_once(self):
        try:
            self.write_archive('const Value = 43\n')
            with self.proxy('normal') as (root, env, log, requests):
                with self.assertRaisesRegex(RuntimeError, 'checksum refusal; attempts=1$'):
                    module_list(self.go, root, ['./...'], env)
                self.assertEqual(requests(), 1)
        finally:
            self.write_archive('const Value = 42\n')

    def test_an_outage_ends_with_the_budget(self):
        with self.proxy('persistent-unavailable') as (root, env, log, requests):
            with self.assertRaisesRegex(RuntimeError, 'HTTP 503; attempts='):
                module_list(self.go, root, ['./...'], env)
            self.assertGreater(log.getvalue().count('goModuleAcquisition'), 1)
            self.assertGreater(requests(), 1)


if __name__ == '__main__':
    unittest.main()

# SPDX-License-Identifier: FSL-1.1-ALv2
"""Native package acquisition verifies immutable bytes before writing an installable archive.

Only loopback HTTP and temporary files are used; importing the installer never invokes dpkg
or changes a host's users, keys, policy or mount helpers.
"""
import contextlib
import hashlib
from http import HTTPStatus
import http.client
import importlib.util
import io
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from archive_retry_test import ArchiveFixture, Interrupted, PAYLOAD
import install


class InstallTests(ArchiveFixture):
    def setUp(self):
        super().setUp()
        self.cache = self.root / 'cache'
        self.cache.mkdir()
        self.destination = self.root / 'destination'
        self.destination.mkdir()
        self.package = {'package': 'fixture', 'archive': 'fixture.deb',
                        'url': self.http + '/package', 'sha256': hashlib.sha256(PAYLOAD).hexdigest()}
        response = patch.object(install.archive_retry.urllib.request, 'urlopen', self.opener.open)
        response.start()
        self.addCleanup(response.stop)

    def download(self):
        with contextlib.redirect_stdout(io.StringIO()):
            return Path(install.download(self.package, self.destination, self.cache))

    def test_importing_the_installer_does_not_install_anything(self):
        specification = importlib.util.spec_from_file_location('installer_import_fixture', install.__file__)
        module = importlib.util.module_from_spec(specification)
        with patch.object(subprocess, 'run', side_effect=AssertionError('Import must not execute a program')), \
                patch.object(install.tempfile, 'TemporaryDirectory',
                             side_effect=AssertionError('Import must not begin an installation')):
            specification.loader.exec_module(module)

    def test_outage_recovery_still_verifies_the_archive(self):
        self.server.script['/package'] = [HTTPStatus.SERVICE_UNAVAILABLE] * 3 + [PAYLOAD]
        self.assertEqual(self.download().read_bytes(), PAYLOAD)
        self.assertEqual(self.paths(), ['/package'] * 4)

    def test_incomplete_response_is_retried_without_writing_the_partial(self):
        self.server.script['/package'] = [Interrupted(PAYLOAD), PAYLOAD]
        self.assertEqual(self.download().read_bytes(), PAYLOAD)
        self.assertEqual(self.paths(), ['/package', '/package'])
        self.assertEqual([path.name for path in self.destination.iterdir()], ['fixture.deb'])

    def test_exhausted_incomplete_responses_never_become_installable(self):
        self.server.script['/package'] = [Interrupted(PAYLOAD)]
        with self.assertRaises(http.client.IncompleteRead):
            self.download()
        self.assertEqual(list(self.destination.iterdir()), [])

    def test_corrupt_bytes_are_never_written_or_retried(self):
        self.server.script['/package'] = [b'x' * len(PAYLOAD)]
        with self.assertRaisesRegex(SystemExit, 'Package hash mismatch'):
            self.download()
        self.assertEqual(list(self.destination.iterdir()), [])
        self.assertEqual(self.paths(), ['/package'])
        self.assertEqual(self.clock.waits, [])

    def test_a_cache_hit_is_verified_without_contacting_the_origin(self):
        (self.cache / self.package['archive']).write_bytes(PAYLOAD)
        self.assertEqual(self.download().read_bytes(), PAYLOAD)
        self.assertEqual(self.paths(), [])
        (self.cache / self.package['archive']).write_bytes(b'corrupt cached bytes')
        with self.assertRaisesRegex(SystemExit, 'Package hash mismatch'):
            self.download()
        self.assertEqual(self.paths(), [])
        self.assertEqual((self.destination / 'fixture.deb').read_bytes(), PAYLOAD)


if __name__ == '__main__':
    unittest.main()

# SPDX-License-Identifier: FSL-1.1-ALv2
"""VM metadata/archive transport against a real loopback HTTP server.

The fixture substitutes only HTTPS transport, leaving the production origin, redirect,
size, digest and cache checks in place; fixture authentication is generated per test.
"""
import hashlib
from http import HTTPStatus
import json
import os
from pathlib import Path
import secrets
import sys
import unittest
from unittest.mock import patch
import urllib.error

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from archive_retry_test import ArchiveFixture, Interrupted, PAYLOAD
import inventory


class InventoryTests(ArchiveFixture):
    def acquire(self, path='/archive', mirrors=(), upstream=None):
        return inventory.acquire(self.https + path, len(PAYLOAD), hashlib.sha256(PAYLOAD).hexdigest(),
                                 declared_digest=upstream, cache=self.root / 'cache', mirrors=mirrors)

    def test_server_outage_uses_a_mirror_and_verifies_the_same_bytes(self):
        self.server.script['/archive'] = [HTTPStatus.BAD_GATEWAY]
        self.server.script['/mirror'] = [PAYLOAD]
        mirrors = (self.https + path for path in ['/mirror'])
        self.assertEqual(self.acquire(mirrors=mirrors), hashlib.sha256(PAYLOAD).hexdigest())
        self.assertEqual(self.paths(), ['/archive', '/mirror'])
        self.assertEqual(self.clock.waits, [])

    def test_wrong_bytes_fail_once_without_an_accepted_cache_file(self):
        self.server.script['/archive'] = [b'x' * len(PAYLOAD)]
        with self.assertRaisesRegex(ValueError, 'committed archive digest'):
            self.acquire()
        self.assertEqual(self.paths(), ['/archive'])
        self.assertEqual(list((self.root / 'cache').iterdir()), [])
        self.assertEqual(self.clock.waits, [])

    def test_incomplete_attempt_is_discarded_before_a_complete_retry(self):
        self.server.script['/archive'] = [Interrupted(PAYLOAD), PAYLOAD]
        digest = self.acquire()
        self.assertEqual(self.paths(), ['/archive', '/archive'])
        cache = self.root / 'cache'
        self.assertEqual([path.name for path in cache.iterdir()], [digest + '.archive'])
        self.assertEqual((cache / (digest + '.archive')).read_bytes(), PAYLOAD)

    def test_incomplete_exhausted_attempts_leave_no_partial_cache(self):
        self.server.script['/archive'] = [Interrupted(PAYLOAD)]
        with self.assertRaises(ConnectionError):
            self.acquire()
        self.assertEqual(list((self.root / 'cache').iterdir()), [])

    def test_upstream_digest_is_not_replaced_by_the_committed_one(self):
        self.server.script['/archive'] = [PAYLOAD]
        wrong = hashlib.sha256(b'other upstream bytes').hexdigest()
        with self.assertRaisesRegex(ValueError, 'upstream release digest'):
            self.acquire(upstream='sha256:' + wrong)
        self.assertEqual(list((self.root / 'cache').iterdir()), [])
        self.assertEqual(self.clock.waits, [])

    def test_authenticated_metadata_never_follows_a_redirect(self):
        self.server.script['/metadata'] = [(HTTPStatus.FOUND, self.https + '/other')]
        self.server.script['/other'] = [b'{}']
        fixture_token = secrets.token_urlsafe()
        with patch.object(inventory, 'API', self.https), patch.dict(os.environ, {'GH_TOKEN': fixture_token}):
            with self.assertRaises(urllib.error.HTTPError) as refused:
                inventory.api('/metadata')
        self.assertEqual(refused.exception.code, HTTPStatus.FOUND)
        self.assertEqual(self.paths(), ['/metadata'])
        self.assertTrue(self.server.requests[0][1].get('Authorization') == 'Bearer ' + fixture_token)

    def test_incomplete_authenticated_metadata_retries_before_json_decoding(self):
        body = json.dumps({'source': 'complete'}).encode()
        self.server.script['/metadata'] = [Interrupted(body), body]
        with patch.object(inventory, 'API', self.https), patch.dict(os.environ, {'GH_TOKEN': ''}):
            self.assertEqual(inventory.api('/metadata'), {'source': 'complete'})
        self.assertEqual(self.paths(), ['/metadata', '/metadata'])

    def test_http_redirects_and_origins_are_refused(self):
        self.server.script['/archive'] = [(HTTPStatus.FOUND, self.http + '/downgraded')]
        with self.assertRaisesRegex(ValueError, 'redirects require HTTPS'):
            self.acquire()
        self.assertEqual(self.paths(), ['/archive'])
        with self.assertRaisesRegex(ValueError, 'requires HTTPS'):
            inventory.request(self.http + '/archive', lambda response: response.read())
        with patch.object(inventory, 'API', self.https):
            with self.assertRaisesRegex(ValueError, 'cannot leave'):
                inventory.request('https://another.invalid/metadata', lambda response: response.read(),
                                  authenticated=True)


if __name__ == '__main__':
    unittest.main()

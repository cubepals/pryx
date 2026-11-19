# SPDX-License-Identifier: FSL-1.1-ALv2
"""Pinned input downloads against a real loopback HTTP server: outages, mirrors, digests.

The server answers with real statuses, so urllib raises its own errors. Waiting is a fake
clock, so no test sleeps and none reaches the network. The mirror table's own tests read the
committed mirrors.json and locks.
"""
import contextlib
import hashlib
from http import HTTPStatus
import http.server
import io
import json
from pathlib import Path
import tempfile
import threading
import unittest
from unittest.mock import patch
import urllib.error
import urllib.parse
import archive_inputs
import archive_retry

PAYLOAD = b'pinned upstream archive\x00\xff'


class Archive(http.server.BaseHTTPRequestHandler):
    """Answers each path with its scripted statuses in turn; the last answer repeats."""

    def do_GET(self):
        self.server.requests.append(self.path)
        answers = self.server.script.get(self.path, [HTTPStatus.NOT_FOUND])
        answer = answers.pop(0) if len(answers) > 1 else answers[0]
        if isinstance(answer, bytes):
            self.send_response(HTTPStatus.OK)
            self.send_header('Content-Length', str(len(answer)))
            self.end_headers()
            self.wfile.write(answer)
        else:
            self.send_error(answer)

    def log_message(self, *arguments):
        pass


class Clock:
    """Time that passes only when the code under test waits."""

    def __init__(self):
        self.now = 0.0
        self.waits = []

    def monotonic(self):
        return self.now

    def sleep(self, seconds):
        self.waits.append(seconds)
        self.now += seconds


class AcquireTests(unittest.TestCase):
    def setUp(self):
        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Archive)
        self.server.requests, self.server.script = [], {}
        thread = threading.Thread(target=self.server.serve_forever, kwargs=dict(poll_interval=0.01),
                                  daemon=True)
        thread.start()
        self.addCleanup(thread.join)
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)
        self.base = f'http://127.0.0.1:{self.server.server_address[1]}'
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.cache = Path(temporary.name)
        self.clock = Clock()
        clock = patch.object(archive_retry, 'time', self.clock)
        clock.start()
        self.addCleanup(clock.stop)

    def item(self, path):
        return dict(name='input', url=self.base + path, sha256=hashlib.sha256(PAYLOAD).hexdigest())

    def acquire(self, item):
        log = io.StringIO()
        try:
            with contextlib.redirect_stderr(log):
                return archive_inputs.acquire(item, self.cache)
        finally:
            self.log = log.getvalue()

    def test_a_brief_outage_is_waited_out(self):
        # Three gateway timeouts in a row ended the job before; the waits now outlast them.
        self.server.script['/input.tar'] = [HTTPStatus.GATEWAY_TIMEOUT] * 3 + [PAYLOAD]
        item = self.item('/input.tar')
        self.assertEqual(self.acquire(item).read_bytes(), PAYLOAD)
        self.assertEqual(self.server.requests, ['/input.tar'] * 4)
        first = archive_retry.FIRST_RETRY_WAIT
        self.assertEqual(self.clock.waits, [first, 2 * first, 4 * first])
        self.assertEqual(self.log.count(f"Download failed: {item['url']}: "), 3)

    def test_a_missing_file_fails_at_once(self):
        with self.assertRaises(urllib.error.HTTPError) as refusal:
            self.acquire(self.item('/moved.tar'))
        self.assertEqual(refusal.exception.code, HTTPStatus.NOT_FOUND)
        self.assertEqual(self.server.requests, ['/moved.tar'])
        self.assertEqual(self.clock.waits, [])
        self.assertIn(self.base + '/moved.tar', self.log)

    def test_rate_limiting_is_waited_out(self):
        self.server.script['/input.tar'] = [HTTPStatus.TOO_MANY_REQUESTS, PAYLOAD]
        self.assertEqual(self.acquire(self.item('/input.tar')).read_bytes(), PAYLOAD)
        self.assertEqual(len(self.clock.waits), 1)

    def test_an_outage_that_outlasts_the_budget_fails(self):
        self.server.script['/input.tar'] = [HTTPStatus.SERVICE_UNAVAILABLE]
        with self.assertRaises(urllib.error.HTTPError) as refusal:
            self.acquire(self.item('/input.tar'))
        self.assertEqual(refusal.exception.code, HTTPStatus.SERVICE_UNAVAILABLE)
        self.assertLessEqual(sum(self.clock.waits), archive_retry.RETRY_BUDGET)
        self.assertLessEqual(max(self.clock.waits), archive_retry.LONGEST_RETRY_WAIT)
        self.assertEqual(len(self.server.requests), len(self.clock.waits) + 1)

    def test_other_bytes_fail_the_digest(self):
        self.server.script['/input.tar'] = [b'other bytes']
        item = self.item('/input.tar')
        with self.assertRaisesRegex(ValueError, 'Downloaded digest mismatch'):
            self.acquire(item)
        self.assertFalse((self.cache / item['sha256'] / 'input.tar').exists())
        self.assertEqual(self.clock.waits, [])

    def mirror(self):
        """Make the loopback server's /debian/ an archive whose mirror is its /mirror/debian/."""
        table = patch.object(archive_inputs, 'MIRRORS', [
            dict(prefix=self.base + '/debian/', mirrors=[self.base + '/mirror/debian/'])])
        table.start()
        self.addCleanup(table.stop)

    def test_an_archive_outage_falls_back_to_its_mirror(self):
        self.mirror()
        self.server.script['/debian/pool/input.deb'] = [HTTPStatus.GATEWAY_TIMEOUT]
        self.server.script['/mirror/debian/pool/input.deb'] = [PAYLOAD]
        self.assertEqual(self.acquire(self.item('/debian/pool/input.deb')).read_bytes(), PAYLOAD)
        self.assertEqual(self.server.requests, ['/debian/pool/input.deb', '/mirror/debian/pool/input.deb'])
        self.assertEqual(self.clock.waits, [])

    def test_a_stalled_origin_leaves_time_for_its_mirror(self):
        self.mirror()
        item = self.item('/debian/pool/input.deb')
        self.server.script['/mirror/debian/pool/input.deb'] = [PAYLOAD]
        real_open = urllib.request.urlopen

        def stalled_origin(address, timeout):
            if address.full_url == item['url']:
                self.clock.now += timeout
                raise urllib.error.URLError(TimeoutError('The origin stalled'))
            return real_open(address, timeout=timeout)

        with patch.object(archive_retry.urllib.request, 'urlopen', stalled_origin):
            self.assertEqual(self.acquire(item).read_bytes(), PAYLOAD)
        self.assertLess(self.clock.now, archive_retry.RETRY_BUDGET)
        self.assertEqual(self.server.requests, ['/mirror/debian/pool/input.deb'])
        self.assertEqual(self.clock.waits, [])

    def test_stalled_mirrors_share_the_retry_deadline(self):
        item = self.item('/debian/pool/input.deb')
        budget = archive_retry.REQUEST_TIMEOUT * 2.5
        attempts = []
        failure = urllib.error.URLError(TimeoutError('Every archive host stalled'))

        def stalled(address, timeout):
            attempts.append((address, timeout))
            self.clock.now += timeout
            raise failure

        mirrors = [dict(prefix=self.base + '/debian/',
                        mirrors=[self.base + f'/mirror-{index}/debian/' for index in range(8)])]
        with patch.object(archive_inputs, 'MIRRORS', mirrors), \
                patch.object(archive_retry, 'RETRY_BUDGET', budget), \
                patch.object(archive_retry.urllib.request, 'urlopen', stalled):
            with self.assertRaises(urllib.error.URLError) as refused:
                self.acquire(item)
        self.assertIs(refused.exception, failure)
        self.assertLessEqual(self.clock.now, budget)
        self.assertLess(attempts[-1][1], archive_retry.REQUEST_TIMEOUT)
        self.assertLess(len(attempts), 1 + len(mirrors[0]['mirrors']))
        self.assertEqual(self.clock.waits, [])

    def test_a_healthy_archive_can_finish_after_the_retry_deadline(self):
        class SlowArchive(io.BytesIO):
            def read(source, length):
                self.clock.now += archive_retry.RETRY_BUDGET
                return super().read(length)

        with patch.object(archive_retry.urllib.request, 'urlopen', return_value=SlowArchive(PAYLOAD)):
            self.assertEqual(self.acquire(self.item('/large.tar')).read_bytes(), PAYLOAD)
        self.assertGreater(self.clock.now, archive_retry.RETRY_BUDGET)
        self.assertEqual(self.clock.waits, [])

    def test_a_mirror_with_other_bytes_fails_the_digest(self):
        self.mirror()
        self.server.script['/debian/pool/input.deb'] = [HTTPStatus.GATEWAY_TIMEOUT]
        self.server.script['/mirror/debian/pool/input.deb'] = [b'other bytes']
        item = self.item('/debian/pool/input.deb')
        with self.assertRaisesRegex(ValueError, 'Downloaded digest mismatch'):
            self.acquire(item)
        self.assertFalse((self.cache / item['sha256'] / 'input.deb').exists())

    def test_a_mirror_that_refuses_is_left_out(self):
        self.mirror()
        self.server.script['/debian/pool/input.deb'] = [HTTPStatus.BAD_GATEWAY] * 2 + [PAYLOAD]
        self.assertEqual(self.acquire(self.item('/debian/pool/input.deb')).read_bytes(), PAYLOAD)
        self.assertEqual(self.server.requests, ['/debian/pool/input.deb', '/mirror/debian/pool/input.deb',
                                                '/debian/pool/input.deb', '/debian/pool/input.deb'])

    def test_a_refused_pin_is_not_looked_for_on_mirrors(self):
        self.mirror()
        self.server.script['/mirror/debian/pool/input.deb'] = [PAYLOAD]
        with self.assertRaises(urllib.error.HTTPError) as refusal:
            self.acquire(self.item('/debian/pool/input.deb'))
        self.assertEqual(refusal.exception.code, HTTPStatus.NOT_FOUND)
        self.assertEqual(self.server.requests, ['/debian/pool/input.deb'])


def pinned(value):
    if isinstance(value, dict):
        for key, inner in value.items():
            if key == 'url' and isinstance(inner, str):
                yield inner
            else:
                yield from pinned(inner)
    elif isinstance(value, list):
        for inner in value:
            yield from pinned(inner)


class MirrorTable(unittest.TestCase):
    def test_pinned_gnu_archives_have_an_independent_mirror(self):
        lock = archive_inputs.ROOT / 'sources.lock.json'
        urls = [url for url in pinned(json.loads(lock.read_text()))
                if urllib.parse.urlsplit(url).hostname == 'ftp.gnu.org']
        self.assertTrue(urls)
        for url in urls:
            with self.subTest(url=url):
                self.assertTrue(any(urllib.parse.urlsplit(address).hostname != 'ftp.gnu.org'
                                    for address in archive_inputs.addresses(url)[1:]))

    def test_a_mirror_keeps_each_pinned_path(self):
        urls = {url for lock in archive_inputs.ROOT.glob('*.json') for url in pinned(json.loads(lock.read_text()))}
        mirrored = [url for url in urls if len(archive_inputs.addresses(url)) > 1]
        self.assertTrue(mirrored)
        for url in mirrored:
            path = urllib.parse.urlsplit(url).path
            for address in archive_inputs.addresses(url)[1:]:
                self.assertTrue(address.startswith('https://'), address)
                self.assertEqual(urllib.parse.urlsplit(address).path, path, url)

    def test_only_an_archive_s_own_prefix_is_rewritten(self):
        for archive in archive_inputs.MIRRORS:
            prefix = urllib.parse.urlsplit(archive['prefix'])
            neighbour = archive['prefix'].rstrip('/') + '-ports/pool/input.deb'
            lookalike = prefix._replace(netloc=prefix.netloc + '.example').geturl() + 'pool/input.deb'
            for url in [neighbour, lookalike]:
                self.assertEqual(archive_inputs.addresses(url), [url])
            self.assertEqual(len(archive_inputs.addresses(archive['prefix'] + 'pool/input.deb')),
                             1 + len(archive['mirrors']))


if __name__ == '__main__':
    unittest.main()

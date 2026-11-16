# SPDX-License-Identifier: FSL-1.1-ALv2
"""Pinned input downloads against a real loopback HTTP server: outages, refusals, digests.

The server answers with real statuses, so urllib raises its own errors. Waiting is a fake
clock, so no test sleeps and none reaches the network.
"""
import contextlib
import hashlib
from http import HTTPStatus
import http.server
import io
from pathlib import Path
import tempfile
import threading
import unittest
from unittest.mock import patch
import urllib.error
import archive_inputs

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
        clock = patch.object(archive_inputs, 'time', self.clock)
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
        first = archive_inputs.FIRST_RETRY_WAIT
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
        self.assertLessEqual(sum(self.clock.waits), archive_inputs.RETRY_BUDGET)
        self.assertLessEqual(max(self.clock.waits), archive_inputs.LONGEST_RETRY_WAIT)
        self.assertEqual(len(self.server.requests), len(self.clock.waits) + 1)

    def test_other_bytes_fail_the_digest(self):
        self.server.script['/input.tar'] = [b'other bytes']
        item = self.item('/input.tar')
        with self.assertRaisesRegex(ValueError, 'Downloaded digest mismatch'):
            self.acquire(item)
        self.assertFalse((self.cache / item['sha256'] / 'input.tar').exists())
        self.assertEqual(self.clock.waits, [])


if __name__ == '__main__':
    unittest.main()

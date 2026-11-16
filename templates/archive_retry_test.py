# SPDX-License-Identifier: FSL-1.1-ALv2
"""Immutable archive retries against real HTTP responses, with a clock that never sleeps.

The client-specific suites reuse the loopback transport; no public archive or credential is
needed, and the HTTPS shim exists only inside these tests.
"""
import contextlib
from http import HTTPStatus
import http.client
import http.server
import io
from pathlib import Path
import tempfile
import threading
import unittest
from unittest.mock import patch
import urllib.error
import urllib.request

import archive_retry

PAYLOAD = b'complete immutable archive\x00\xff'


class Clock:
    def __init__(self):
        self.now = 0.0
        self.waits = []

    def monotonic(self):
        return self.now

    def sleep(self, seconds):
        self.now += seconds
        self.waits.append(seconds)


class Interrupted:
    """A real HTTP body that ends before its declared length."""

    def __init__(self, payload):
        self.payload = payload


class Archive(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.server.requests.append((self.path, dict(self.headers)))
        answers = self.server.script.get(self.path, [HTTPStatus.NOT_FOUND])
        answer = answers.pop(0) if len(answers) > 1 else answers[0]
        if isinstance(answer, tuple):
            self.send_response(answer[0])
            self.send_header('Location', answer[1])
            self.send_header('Content-Length', '0')
            self.end_headers()
        elif isinstance(answer, (bytes, Interrupted)):
            payload = answer.payload if isinstance(answer, Interrupted) else answer
            self.send_response(HTTPStatus.OK)
            self.send_header('Content-Length', str(len(payload)))
            self.end_headers()
            self.wfile.write(payload[:len(payload) // 2] if isinstance(answer, Interrupted) else payload)
            self.close_connection = True
        else:
            self.send_error(answer)

    def log_message(self, *arguments):
        pass


class LoopbackHTTPS(urllib.request.HTTPSHandler):
    """Keep HTTPS URLs/redirect checks while using this test's plain loopback HTTP server."""

    def https_open(self, request):
        return self.do_open(http.client.HTTPConnection, request)


class ArchiveFixture(unittest.TestCase):
    def setUp(self):
        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Archive)
        self.server.requests, self.server.script = [], {}
        thread = threading.Thread(target=self.server.serve_forever,
                                  kwargs={'poll_interval': 0.01}, daemon=True)
        thread.start()
        self.addCleanup(thread.join)
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)
        self.http = f'http://127.0.0.1:{self.server.server_port}'
        self.https = f'https://127.0.0.1:{self.server.server_port}'
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.clock = Clock()
        mocked_time = patch.object(archive_retry, 'time', self.clock)
        mocked_time.start()
        self.addCleanup(mocked_time.stop)
        # This fixture never uses an inherited proxy for its local HTTP transport.
        build_opener = urllib.request.build_opener
        self.opener = build_opener(urllib.request.ProxyHandler({}), LoopbackHTTPS())
        factory = patch.object(urllib.request, 'build_opener',
                               lambda *handlers: build_opener(urllib.request.ProxyHandler({}),
                                                             LoopbackHTTPS(), *handlers))
        factory.start()
        self.addCleanup(factory.stop)
        output = contextlib.redirect_stderr(io.StringIO())
        output.__enter__()
        self.addCleanup(output.__exit__, None, None, None)

    def paths(self):
        return [path for path, _ in self.server.requests]

    def retrieve(self, path='/archive', mirrors=()):
        return archive_retry.request(self.http + path, lambda response: response.read(),
                                     opener=self.opener, mirrors=mirrors)


class RetryTests(ArchiveFixture):
    def test_outages_and_rate_limits_recover_with_doubling_waits(self):
        self.server.script['/archive'] = [HTTPStatus.GATEWAY_TIMEOUT, HTTPStatus.TOO_MANY_REQUESTS,
                                          HTTPStatus.SERVICE_UNAVAILABLE, PAYLOAD]
        self.assertEqual(self.retrieve(), PAYLOAD)
        self.assertEqual(self.clock.waits, [5, 10, 20])
        self.assertEqual(self.paths(), ['/archive'] * 4)

    def test_the_entire_server_error_class_is_retried(self):
        for status in archive_retry.SERVER_ERRORS:
            with self.subTest(status=status):
                self.server.script['/archive'] = [status, PAYLOAD]
                self.assertEqual(self.retrieve(), PAYLOAD)
        unnamed = archive_retry.SERVER_ERRORS.stop - 1
        with self.assertRaises(ValueError):
            HTTPStatus(unnamed)

    def test_origin_client_refusals_do_not_wait_or_contact_a_mirror(self):
        self.server.script['/mirror'] = [PAYLOAD]
        for status in [HTTPStatus.NOT_FOUND, HTTPStatus.FORBIDDEN, HTTPStatus.REQUEST_TIMEOUT]:
            with self.subTest(status=status):
                self.server.requests.clear()
                self.server.script['/archive'] = [status]
                with self.assertRaises(urllib.error.HTTPError) as refused:
                    self.retrieve(mirrors=(self.http + '/mirror',))
                self.assertEqual(refused.exception.code, status)
                self.assertEqual(self.paths(), ['/archive'])
                self.assertEqual(self.clock.waits, [])

    def test_a_refusing_mirror_is_removed_while_the_origin_recovers(self):
        self.server.script['/archive'] = [HTTPStatus.BAD_GATEWAY] * 2 + [PAYLOAD]
        self.assertEqual(self.retrieve(mirrors=(self.http + '/missing',)), PAYLOAD)
        self.assertEqual(self.paths(), ['/archive', '/missing', '/archive', '/archive'])

    def test_stalled_sources_share_one_deadline(self):
        opened = []

        class Stalled:
            def open(inner, request, timeout):
                opened.append((request.full_url, timeout))
                self.clock.now += timeout
                raise TimeoutError('The fixture source stalled')

        with self.assertRaises(TimeoutError):
            archive_retry.request(self.http + '/archive', lambda response: response.read(),
                                  opener=Stalled(), mirrors=(self.http + '/mirror',))
        self.assertLessEqual(self.clock.now, archive_retry.RETRY_BUDGET)
        self.assertLessEqual(sum(self.clock.waits), archive_retry.RETRY_BUDGET)
        self.assertLess(opened[-1][1], archive_retry.REQUEST_TIMEOUT)
        self.assertTrue(any(url.endswith('/mirror') for url, _ in opened))

    def test_a_connection_timeout_retries_then_recovers(self):
        self.server.script['/archive'] = [PAYLOAD]
        first = True

        class Once:
            def open(inner, request, timeout):
                nonlocal first
                if first:
                    first = False
                    raise TimeoutError('The first connection timed out')
                return self.opener.open(request, timeout=timeout)

        self.assertEqual(archive_retry.request(self.http + '/archive', lambda response: response.read(),
                                               opener=Once()), PAYLOAD)
        self.assertEqual(len(self.clock.waits), 1)

    def test_a_healthy_stream_may_finish_after_the_retry_deadline(self):
        self.server.script['/archive'] = [PAYLOAD]

        def consume(response):
            self.clock.now += archive_retry.RETRY_BUDGET + 1
            return response.read()

        self.assertEqual(archive_retry.request(self.http + '/archive', consume, opener=self.opener), PAYLOAD)
        self.assertEqual(self.clock.waits, [])

    def test_an_incomplete_response_is_retried(self):
        self.server.script['/archive'] = [Interrupted(PAYLOAD), PAYLOAD]
        self.assertEqual(self.retrieve(), PAYLOAD)
        self.assertEqual(self.paths(), ['/archive', '/archive'])

    def test_an_outage_exhausts_its_budget(self):
        self.server.script['/archive'] = [HTTPStatus.SERVICE_UNAVAILABLE]
        with self.assertRaises(urllib.error.HTTPError) as refused:
            self.retrieve()
        self.assertEqual(refused.exception.code, HTTPStatus.SERVICE_UNAVAILABLE)
        self.assertLessEqual(sum(self.clock.waits), archive_retry.RETRY_BUDGET)
        self.assertEqual(len(self.paths()), len(self.clock.waits) + 1)


if __name__ == '__main__':
    unittest.main()

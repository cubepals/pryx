# SPDX-License-Identifier: FSL-1.1-ALv2
"""Bounded retries for immutable build inputs, not package resolution or mutations.

Callers retain their own authentication, redirect, size and digest policies. Every source of
one input shares a retry deadline; only transport errors, HTTP 429 and server errors retry.
"""
import datetime
import http.client
from http import HTTPStatus
import ssl
import sys
import time
import urllib.error
import urllib.request

RETRY_BUDGET = datetime.timedelta(minutes=3).total_seconds()
FIRST_RETRY_WAIT = datetime.timedelta(seconds=5).total_seconds()
LONGEST_RETRY_WAIT = datetime.timedelta(minutes=1).total_seconds()
REQUEST_TIMEOUT = datetime.timedelta(seconds=30).total_seconds()
# RFC 9110 section 15.6 defines all 5xx answers as server errors, including CDN extensions.
SERVER_ERRORS = range(500, 600)
NETWORK_ERRORS = (urllib.error.URLError, ConnectionError, TimeoutError, ssl.SSLError,
                  http.client.HTTPException)


def transient(problem):
    if isinstance(problem, urllib.error.HTTPError):
        return problem.code in SERVER_ERRORS or problem.code == HTTPStatus.TOO_MANY_REQUESTS
    return True


def request(url, consume, *, opener=None, headers=None, mirrors=()):
    """Consume a successful response with one deadline across the origin and its mirrors.

    A permanent origin refusal fails at once; a refusing mirror is removed. Consumers must
    reject incomplete or incorrect bytes, and every attempt receives a fresh response.
    """
    open_response = opener.open if opener is not None else urllib.request.urlopen
    sources = [url, *mirrors]
    deadline = time.monotonic() + RETRY_BUDGET
    wait = FIRST_RETRY_WAIT
    failure = TimeoutError('Archive download retry budget exhausted')
    attempted = False
    while True:
        for source in list(sources):
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise failure
            actual_headers = dict(headers or {})
            if attempted:
                actual_headers['Cache-Control'] = 'no-cache'
            attempted = True
            try:
                with open_response(urllib.request.Request(source, headers=actual_headers),
                                   timeout=min(REQUEST_TIMEOUT, remaining)) as response:
                    return consume(response)
            except NETWORK_ERRORS as problem:
                if isinstance(problem, urllib.error.HTTPError):
                    problem.close()  # Its error page is never an input.
                print(f'Archive download failed: {source}: {problem}', file=sys.stderr, flush=True)
                if transient(problem):
                    failure = problem
                elif source == url:
                    raise
                else:
                    sources.remove(source)
        if time.monotonic() + wait > deadline:
            raise failure
        print(f'Trying archive again in {wait:g} s', file=sys.stderr, flush=True)
        time.sleep(wait)
        wait = min(2 * wait, LONGEST_RETRY_WAIT)

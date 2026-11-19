# SPDX-License-Identifier: FSL-1.1-ALv2
"""The one retry policy for downloading pinned build inputs: process, VM and native tools.

A failed attempt is tried again with doubling waits until about three minutes after the first
try, across the input's mirrors. A timeout, a dropped or incomplete response, a 429 and any 5xx
are tried again; any other refusal of the pinned address fails at once. Callers keep their own
authentication, redirect, size and digest checks: the digest alone decides whether bytes are
kept. Locked Go module reads follow the same policy (vm-tools/go_acquisition.py). This module
neither caches nor verifies bytes, and package managers' own resolution never comes here.
"""
import datetime
import http.client
from http import HTTPStatus
import ssl
import sys
import time
import urllib.error
import urllib.request

# A failed download is tried again, with doubling waits, until this long after its first try:
# long enough to wait out an archive's brief outage, short enough to end a job that can't.
RETRY_BUDGET = datetime.timedelta(minutes=3).total_seconds()
FIRST_RETRY_WAIT = datetime.timedelta(seconds=5).total_seconds()
LONGEST_RETRY_WAIT = datetime.timedelta(minutes=1).total_seconds()
# A stalled upstream leaves time to try independent mirrors. This bounds socket inactivity;
# a healthy archive stream may take longer than the retry budget to finish.
REQUEST_TIMEOUT = datetime.timedelta(seconds=30).total_seconds()
# RFC 9110 section 15.6: 5xx is the server-error class, including codes Python's HTTPStatus
# doesn't name, such as a CDN's own 52x.
SERVER_ERRORS = range(500, 600)
# ConnectionError covers a short body a caller counts itself; HTTPException covers urllib's own
# IncompleteRead and a connection closed before the status line.
NETWORK_ERRORS = (urllib.error.URLError, ConnectionError, TimeoutError, ssl.SSLError,
                  http.client.HTTPException)


def transient_status(code):
    """Whether an HTTP answer could change on another try: 5xx or 429, never another refusal."""
    return code in SERVER_ERRORS or code == HTTPStatus.TOO_MANY_REQUESTS


def transient(error):
    """Whether trying again could help: a refusal other than 5xx or 429 means a wrong pin."""
    if isinstance(error, urllib.error.HTTPError):
        return transient_status(error.code)
    return True


def request(url, consume, mirrors=(), opener=None, headers=None):
    """Return consume(response) for url, or for a mirror's copy, trying again within the budget.

    Every attempt gets a fresh response, and consume must refuse incomplete or other bytes. A
    refusal of the pinned address itself fails at once; a mirror that refuses is left out.
    """
    open_response = urllib.request.urlopen if opener is None else opener.open
    candidates = [url, *mirrors]
    deadline = time.monotonic() + RETRY_BUDGET
    wait = FIRST_RETRY_WAIT
    failure = TimeoutError('Archive download retry budget exhausted')
    attempted = False
    while True:
        for address in list(candidates):
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise failure
            asked = dict(headers or {})
            if attempted:
                asked['Cache-Control'] = 'no-cache'  # A cache in between may keep a failed answer.
            attempted = True
            try:
                with open_response(urllib.request.Request(address, headers=asked),
                                   timeout=min(REQUEST_TIMEOUT, remaining)) as response:
                    return consume(response)
            except NETWORK_ERRORS as error:
                if isinstance(error, urllib.error.HTTPError):
                    error.close()  # Its body is an error page, never read.
                print(f'Download failed: {address}: {error}', file=sys.stderr, flush=True)
                if transient(error):
                    failure = error
                elif address == url:
                    raise
                else:
                    candidates.remove(address)
        if time.monotonic() + wait > deadline:
            raise failure
        print(f'Trying again in {wait:g} s', file=sys.stderr, flush=True)
        time.sleep(wait)
        wait = min(2 * wait, LONGEST_RETRY_WAIT)

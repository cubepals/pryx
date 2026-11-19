#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Read the locked Go dependency graph and module archives, under the templates' retry policy.

Go owns the proxy, the module cache, checksums and module selection; this only runs the same
`go list` or `go mod download` again when Go reports what the policy retries (../archive_retry.py):
a 5xx or 429, an HTTP/2 stream reset or another transport failure, with doubling waits for about
three minutes. A checksum or certificate refusal, any other HTTP refusal and every other failure
end at once. Each attempt's own output stays in the log. Compilation and tests never come here.
"""
import json
from pathlib import Path
import re
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import archive_retry

# cmd/go/internal/web reports an HTTP refusal as `reading <url>: <status> <text>`, in stderr or a
# module's `go mod download -json` Error. Nothing exports these words; go_acquisition_check.py
# holds them to what the pinned Go prints.
HTTP_STATUS = re.compile(r'(?:reading|read) (?:"https?://[^"\n]+"|https?://\S+): (\d{3})(?:\s|$)')
STREAM_RESET = 'stream error:'  # net/http's HTTP/2 StreamError
TRANSPORT_FAILURE = re.compile(
    r'unexpected EOF|connection reset by peer|connection refused|connection timed out|'
    r'i/o timeout|TLS handshake timeout|context deadline exceeded|network is unreachable|'
    r'temporary failure in name resolution', re.IGNORECASE)


def failure(text):
    """Whether Go's error text is one the policy retries, and its kind, naming no URL or body."""
    if 'checksum mismatch' in text or 'SECURITY ERROR' in text:
        return False, 'checksum refusal'
    if 'x509:' in text or 'failed to verify certificate' in text:
        return False, 'certificate refusal'
    statuses = [int(status) for status in HTTP_STATUS.findall(text)]
    refused = [status for status in statuses if not archive_retry.transient_status(status)]
    if refused:
        return False, f'HTTP {refused[0]}'
    if statuses:
        return True, f'HTTP {statuses[0]}'
    if STREAM_RESET in text:
        return True, 'HTTP/2 stream reset'
    if TRANSPORT_FAILURE.search(text):
        return True, 'transport failure'
    return False, 'other Go failure'


def reported_errors(output):
    """The Error fields of `go mod download -json`'s stream of objects."""
    errors, decoder, rest = [], json.JSONDecoder(), output.lstrip()
    while rest:
        try:
            item, end = decoder.raw_decode(rest)
        except ValueError:
            break
        if isinstance(item, dict) and isinstance(item.get('Error'), str):
            errors.append(item['Error'])
        rest = rest[end:].lstrip()
    return errors


def read(command, root, env=None):
    deadline = time.monotonic() + archive_retry.RETRY_BUDGET
    wait = archive_retry.FIRST_RETRY_WAIT
    attempt = 0
    while True:
        attempt += 1
        result = subprocess.run(command, cwd=root, env=env, capture_output=True, text=True)
        sys.stderr.write(result.stderr)
        sys.stderr.flush()
        if result.returncode == 0:
            return result.stdout
        retry, reason = failure('\n'.join([result.stderr, *reported_errors(result.stdout)]))
        if not retry or time.monotonic() + wait > deadline:
            raise RuntimeError(f'Go module acquisition failed: {reason}; attempts={attempt}')
        print(json.dumps({'goModuleAcquisition': 'retry', 'reason': reason, 'attempt': attempt,
                          'waitSeconds': wait}), file=sys.stderr, flush=True)
        time.sleep(wait)
        wait = min(2 * wait, archive_retry.LONGEST_RETRY_WAIT)


def module_list(go, root, packages, env=None):
    return read([str(go), 'list', '-mod=readonly', '-deps', '-json=Module', *packages], root, env)


def module_archive(go, root, module, env=None):
    return read([str(go), 'mod', 'download', '-json', module], root, env)

#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Read the locked Go dependency graph and module archives, under the templates' retry policy.

Go owns the proxy, the module cache, checksums and module selection; this only runs the same
`go list` or `go mod download` again when Go reports what the policy retries (../archive_retry.py):
a 5xx or 429, an HTTP/2 stream reset or another transport failure, with doubling waits for about
three minutes. A checksum or certificate refusal, any other HTTP refusal and every other failure
end at once. Direct VCS failures are terminal because the subprocess stderr has no trusted
transport reason. Only retry summaries and terminal reasons leave this wrapper's diagnostic
output. Compilation and tests never come here.
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
HTTP_STATUS = re.compile(r'(?:reading|read) (?:"https?://[^"\n]+"|https?://\S+): ([0-9]{3})(?:[ \t\r]|$)')
# cmd/go/internal/modfetch/codehost's RunError prints a VCS command, its directory and the
# process error. A final colon announces a block containing the subprocess stderr.
VCS_RUN_ERROR = re.compile(
    r'(?:^|[ \t])(?:git|hg|svn|fossil)(?: [^\r\n]+)? in [^\r\n]*: '
    r'(?:exit status [0-9]+|signal: [^:\r\n]+|context (?:canceled|deadline exceeded))(:)?\r?$')
STREAM_RESET = 'stream error:'  # net/http's HTTP/2 StreamError
TRANSPORT_FAILURE = re.compile(
    r'unexpected EOF|connection reset by peer|connection refused|connection timed out|'
    r'i/o timeout|TLS handshake timeout|context deadline exceeded|network is unreachable|'
    r'temporary failure in name resolution', re.IGNORECASE)


def first_heading(line):
    """Choose the heading before any HTTP reason phrase or VCS arguments."""
    status = HTTP_STATUS.search(line)
    vcs = VCS_RUN_ERROR.search(line)
    if vcs is not None and (status is None or vcs.start() < status.start()):
        return vcs
    return status


def diagnostic_headings(text):
    """Skip Go's tabbed HTTP response body and VCS subprocess detail blocks."""
    detail = False
    # Go splits the body on literal LF. Other whitespace does not start a new heading.
    for line in text.split('\n'):
        if line.startswith('\tserver response:'):
            detail = True
        elif not detail or not line.startswith('\t'):
            heading = first_heading(line)
            detail = heading is not None and heading.re is VCS_RUN_ERROR and heading[1] is not None
            yield line


def failure(*diagnostics):
    """Whether the pinned Go error headings warrant a retry, without URLs or server prose."""
    statuses, non_http = [], []
    direct_vcs_failed = False
    for diagnostic in diagnostics:
        for heading in diagnostic_headings(diagnostic):
            match = first_heading(heading)
            if match is None:
                non_http.append(heading)
            elif match.re is VCS_RUN_ERROR:
                direct_vcs_failed = True
            else:
                # HTTP reason phrases, like response bodies, can come from the server.
                statuses.append(int(match[1]))
    text = '\n'.join(non_http)
    if 'checksum mismatch' in text or 'SECURITY ERROR' in text:
        return False, 'checksum refusal'
    if 'x509:' in text or 'failed to verify certificate' in text:
        return False, 'certificate refusal'
    refused = [status for status in statuses if not archive_retry.transient_status(status)]
    if refused:
        return False, f'HTTP {refused[0]}'
    if direct_vcs_failed:
        return False, 'direct VCS failure'
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
        # Text mode translates CR into LF before the response detail boundary can be recognized.
        result = subprocess.run(command, cwd=root, env=env, capture_output=True)
        if result.returncode == 0:
            return result.stdout.decode()
        retry, reason = failure(result.stderr.decode(errors='replace'),
                                *reported_errors(result.stdout.decode(errors='replace')))
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

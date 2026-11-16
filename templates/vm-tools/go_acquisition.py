#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Read the locked Go dependency graph through Go's authenticated module cache.

Only module listing and archive acquisition may retry. Go still owns the proxy,
cache, checksums and module selection; compilation and tests never enter here.
"""
from contextlib import contextmanager
from http import HTTPStatus
import json
import os
import re
import selectors
import signal
import subprocess
import sys
import threading
import time


READ_BUDGET = 180.0
INITIAL_WAIT = 5.0
MAX_WAIT = 60.0
CANCEL_POLL = 0.1
# cmd/go/internal/web.HTTPError and net/http/internal/http2.StreamError expose
# only text through the pinned Go CLI, including `go mod download -json` Error.
HTTP_ERROR = re.compile(r'(?:reading|read) (?:"https?://[^"\n]+"|https?://\S+): (\d{3})(?:\s|$)')
TRANSPORT_ERROR = re.compile(
    r'unexpected EOF|connection reset by peer|connection refused|'
    r'connection timed out|i/o timeout|TLS handshake timeout|'
    r'context deadline exceeded|network is unreachable|temporary failure in name resolution',
    re.IGNORECASE)


def failure(output, stderr):
    """Classify Go's error, without exposing proxy credentials or response bodies."""
    errors = [stderr]
    decoder = json.JSONDecoder()
    while output.strip():
        try:
            item, end = decoder.raw_decode(output.lstrip())
        except (ValueError, TypeError):
            break
        output = output.lstrip()[end:]
        if isinstance(item, dict) and isinstance(item.get('Error'), str):
            errors.append(item['Error'])
    text = '\n'.join(errors)
    if 'checksum mismatch' in text or 'SECURITY ERROR' in text:
        return False, 'checksum refusal'
    if 'x509:' in text or 'failed to verify certificate' in text:
        return False, 'certificate refusal'
    statuses = HTTP_ERROR.findall(text)
    for status in statuses:
        if status.startswith(str(HTTPStatus.BAD_REQUEST)[0]) and int(status) != HTTPStatus.TOO_MANY_REQUESTS:
            return False, f'HTTP {status}'
    for status in statuses:
        if int(status) == HTTPStatus.TOO_MANY_REQUESTS or status.startswith(str(HTTPStatus.INTERNAL_SERVER_ERROR)[0]):
            return True, f'HTTP {status}'
    if 'stream error:' in text:
        return True, 'HTTP/2 stream interruption'
    if TRANSPORT_ERROR.search(text):
        return True, 'transport interruption'
    return False, 'Go configuration or module refusal'


class AcquisitionCancelled(BaseException):
    """Do not turn caller cancellation into an acquisition retry."""


@contextmanager
def cancellation():
    """Defer caller signal policy until the owned command has been cleaned up."""
    if threading.current_thread() is not threading.main_thread():
        raise RuntimeError('Go module acquisition requires main-thread cancellation ownership')
    if signal.getsignal(signal.SIGCHLD) != signal.SIG_DFL:
        raise RuntimeError('Go module acquisition requires owned child reaping')
    previous = {value: signal.getsignal(value) for value in (signal.SIGTERM, signal.SIGINT)}
    if any(value is None for value in previous.values()):
        raise RuntimeError('Go module acquisition cannot preserve an unknown caller signal handler')
    pending = []

    def received(value, frame):
        # Recording only also covers signals during Popen: cleanup starts once
        # the new child has been bound to its owner, never before it exists.
        if not pending:
            pending.append((value, frame))

    try:
        for value, handler in previous.items():
            if handler != signal.SIG_IGN:
                signal.signal(value, received)
        try:
            yield pending
        except BaseException:
            if not pending:
                raise
    finally:
        for value, handler in previous.items():
            signal.signal(value, handler)
    if pending:
        value, frame = pending[0]
        handler = previous[value]
        if handler == signal.SIG_DFL:
            signal.raise_signal(value)
        elif callable(handler):
            handler(value, frame)
        raise AcquisitionCancelled(f'Go module acquisition cancelled by {signal.Signals(value).name}')


def check_cancel(pending):
    if pending:
        raise AcquisitionCancelled()


def terminate(process):
    """Kill the owned group while its leader's PID is still held, then reap it."""
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    finally:
        try:
            process.wait()
        finally:
            process.stdout.close()
            process.stderr.close()


def communicate(process, deadline, pending):
    """Read pipes without reaping the group leader before descendant cleanup."""
    outputs = {process.stdout: bytearray(), process.stderr: bytearray()}
    with selectors.DefaultSelector() as reader:
        for pipe in outputs:
            reader.register(pipe, selectors.EVENT_READ)
        while reader.get_map():
            check_cancel(pending)
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise RuntimeError('Go module acquisition exceeded its read deadline')
            for key, _mask in reader.select(min(CANCEL_POLL, remaining)):
                data = os.read(key.fd, 64 * 1024)
                if data:
                    outputs[key.fileobj].extend(data)
                else:
                    reader.unregister(key.fileobj)
    # WNOWAIT holds the exited leader even if a descendant closed its pipes.
    # Only terminate() reaps it, after killing its still-owned process group.
    while os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOWAIT | os.WNOHANG) is None:
        check_cancel(pending)
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise RuntimeError('Go module acquisition exceeded its read deadline')
        time.sleep(min(CANCEL_POLL, remaining))
    check_cancel(pending)
    return tuple(outputs[pipe].decode() for pipe in (process.stdout, process.stderr))


def pause(duration, pending):
    deadline = time.monotonic() + duration
    while True:
        check_cancel(pending)
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            return
        time.sleep(min(CANCEL_POLL, remaining))


def _read(command, root, *, budget=READ_BUDGET, initial_wait=INITIAL_WAIT, env=None):
    with cancellation() as pending:
        return _read_owned(command, root, budget, initial_wait, env, pending)


def _read_owned(command, root, budget, initial_wait, env, pending):
    deadline = time.monotonic() + budget
    delay = initial_wait
    attempt = 0
    while True:
        check_cancel(pending)
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise RuntimeError('Go module acquisition exceeded its read deadline')
        attempt += 1
        process = subprocess.Popen(command, cwd=root, env=env,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                   start_new_session=True)
        try:
            output, stderr = communicate(process, deadline, pending)
        finally:
            terminate(process)
        check_cancel(pending)
        if process.returncode == 0:
            return output
        retry, reason = failure(output, stderr)
        remaining = deadline - time.monotonic()
        if not retry or remaining <= 0:
            raise RuntimeError(f'Go module acquisition failed: {reason}; attempts={attempt}')
        wait = min(delay, remaining)
        print(json.dumps({'goModuleAcquisition': 'retry', 'reason': reason,
                          'attempt': attempt, 'waitSeconds': wait}), file=sys.stderr, flush=True)
        pause(wait, pending)
        delay = min(delay * 2, MAX_WAIT)


def module_list(go, root, packages, **policy):
    return _read([str(go), 'list', '-mod=readonly', '-deps', '-json=Module', *packages], root, **policy)


def module_archive(go, root, module, **policy):
    return _read([str(go), 'mod', 'download', '-json', module], root, **policy)

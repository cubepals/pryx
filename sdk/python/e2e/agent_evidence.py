# SPDX-License-Identifier: FSL-1.1-ALv2
"""Own the existing agent acceptance child and report positions only after failure.
The child registers faulthandler before loading the acceptance module. Neither
phase messages nor stack formatting inspect arguments, locals or the environment.
"""
import ctypes
import faulthandler
import os
import re
import runpy
import select
import signal
import subprocess
import sys
import tempfile
import time

# POSIX shells encode signal termination as 128 plus the signal number.
SHELL_SIGNAL_OFFSET = 128


def instrument(target, phase_fd, stack_fd):
    faulthandler.register(signal.SIGUSR1, file=stack_fd, all_threads=True)
    os.set_blocking(phase_fd, False)
    os.write(phase_fd, b'ready\n')
    source = os.path.realpath(target)

    def position(frame, event, _argument):
        if frame.f_code.co_filename != source:
            return None
        if event == 'line':
            try:
                os.write(phase_fd, f'{frame.f_lineno} {time.monotonic()}\n'.encode('ascii'))
            except BlockingIOError:
                pass
        return position

    sys.argv = [source]
    sys.settrace(position)
    runpy.run_path(source, run_name='__main__')


class Evidence:
    def __init__(self, phase_fd, stack):
        self.phase_fd = phase_fd
        self.stack = stack
        self.pending = b''
        self.ready = False
        self.line = None
        self.since = time.monotonic()
        self.started = self.since
        self.failure = None
        os.set_blocking(phase_fd, False)

    def observe(self, delay):
        if not select.select([self.phase_fd], [], [], delay)[0]:
            return
        data = os.read(self.phase_fd, 65536)
        self.pending += data
        messages = self.pending.split(b'\n')
        self.pending = messages.pop()
        for message in messages:
            if message == b'ready':
                self.ready = True
            elif re.fullmatch(rb'[0-9]+ [0-9]+\.[0-9]+', message):
                line, since = message.split()
                self.line, self.since = int(line), float(since)

    def freeze(self):
        self.observe(0)
        self.failure = self.line, self.since, time.monotonic()

    def report(self, reason):
        line, since, now = self.failure or (self.line, self.since, time.monotonic())
        position = f'agent.py:{line}' if line is not None else 'bootstrap'
        print(f'Python agent {reason}: {position}; phase {now-since:.3f}s; '
              f'child {now-self.started:.3f}s', file=sys.stderr, flush=True)
        self.stack.seek(0)
        # faulthandler includes thread addresses; retain only source positions.
        for line in self.stack.read().decode('utf-8', errors='replace').splitlines():
            if re.fullmatch(r'  File "[^"\r\n]+", line [0-9]+ in [^\r\n]+', line):
                print(line, file=sys.stderr, flush=True)


def exited(child):
    # Keep the leader waitable until its group is retired: its PID cannot be reused.
    return os.waitid(os.P_PID, child.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT) is not None


def signal_group(child, value):
    try:
        os.killpg(child.pid, value)
    except ProcessLookupError:
        pass


def adopt_descendants():
    # Linux prctl(2): PR_SET_CHILD_SUBREAPER makes orphaned PTY sessions our children.
    pr_set_child_subreaper = 36
    if ctypes.CDLL(None, use_errno=True).prctl(pr_set_child_subreaper, 1, 0, 0, 0) != 0:
        raise RuntimeError('Python acceptance child ownership unavailable')


def own_pty_child(parent):
    # Linux prctl(2): PR_SET_PDEATHSIG covers the existing pty.fork session escape.
    pr_set_pdeathsig = 1
    try:
        armed = ctypes.CDLL(None, use_errno=True).prctl(pr_set_pdeathsig, signal.SIGKILL, 0, 0, 0) == 0
    except Exception:
        armed = False
    if not armed:
        # A failed post-fork child must never unwind the parent's unittest cleanup.
        os._exit(os.EX_OSERR)
    if os.getppid() != parent:
        os.kill(os.getpid(), signal.SIGKILL)


def retire(child, grace):
    signal_group(child, signal.SIGKILL)
    child.wait(timeout=grace)
    deadline = time.monotonic() + grace
    while True:
        try:
            while os.waitpid(-1, os.WNOHANG)[0]:
                pass
        except ChildProcessError:
            return
        if time.monotonic() >= deadline:
            raise RuntimeError('Python acceptance descendants did not retire')
        time.sleep(min(0.01, max(0, deadline-time.monotonic())))


def supervise(target, budget, grace, capture):
    cancelled = []

    def record(value, _frame):
        if not cancelled:
            cancelled.append(value)

    previous = {value: signal.signal(value, record) for value in (signal.SIGTERM, signal.SIGINT)}
    adopt_descendants()
    phase_read, phase_write = os.pipe()
    child = None
    try:
        with tempfile.TemporaryFile() as stack:
            evidence = Evidence(phase_read, stack)
            if cancelled:
                return SHELL_SIGNAL_OFFSET + cancelled[0]
            child = subprocess.Popen(
                [sys.executable, __file__, 'child', target, str(phase_write), str(stack.fileno())],
                pass_fds=(phase_write, stack.fileno()), start_new_session=True,
            )
            os.close(phase_write)
            phase_write = None
            deadline = time.monotonic() + budget
            while not cancelled and not exited(child) and time.monotonic() < deadline:
                evidence.observe(min(0.05, max(0, deadline-time.monotonic())))
            failed = bool(cancelled) or not exited(child)
            if failed:
                evidence.freeze()
            if failed and evidence.ready and not exited(child):
                os.kill(child.pid, signal.SIGUSR1)
                until = time.monotonic() + capture
                while not exited(child) and time.monotonic() < until:
                    evidence.observe(min(0.05, max(0, until-time.monotonic())))
            if failed:
                signal_group(child, signal.SIGTERM)
                until = time.monotonic() + grace
                while not exited(child) and time.monotonic() < until:
                    evidence.observe(min(0.05, max(0, until-time.monotonic())))
            # Also retire descendants left by an already completed CLI invocation.
            retire(child, grace)
            code = child.returncode
            evidence.observe(0)
            if failed or code:
                evidence.report('cancelled' if cancelled else 'timeout' if failed else 'failed')
            return SHELL_SIGNAL_OFFSET + cancelled[0] if cancelled else 1 if failed else code
    finally:
        if child is not None and child.returncode is None:
            retire(child, grace)
        os.close(phase_read)
        if phase_write is not None:
            os.close(phase_write)
        for value, handler in previous.items():
            signal.signal(value, handler)


if __name__ == '__main__':
    if sys.argv[1] == 'child':
        instrument(sys.argv[2], int(sys.argv[3]), int(sys.argv[4]))
    else:
        target, budget, grace, capture = sys.argv[1:]
        try:
            code = supervise(target, float(budget), float(grace), float(capture))
        except Exception:
            print('Python agent evidence owner failed during lifecycle', file=sys.stderr)
            code = 1
        sys.exit(code)

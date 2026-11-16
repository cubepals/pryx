# SPDX-License-Identifier: FSL-1.1-ALv2
"""A real blocked socket and a PTY child outside its parent's process group."""
import json
import os
import pty
import signal
import socket
import threading
import time
import pathlib
import sys

if len(sys.argv) > 1:
    if sys.argv[1] == 'bootstrap':
        sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[3] / 'sdk/python/e2e'))
        import agent_evidence
        agent_evidence.__file__ = __file__
        raise SystemExit(agent_evidence.supervise(__file__, 0.3, 0.1, 0.1))
    if sys.argv[1] == 'child':
        def unexpected_signal(_value, _frame):
            pathlib.Path(os.environ['FIXTURE_PIDS'] + '.unexpected').touch()
        signal.signal(signal.SIGUSR1, unexpected_signal)
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        pathlib.Path(os.environ['FIXTURE_PIDS']).write_text(json.dumps([os.getpid()]))
        while True:
            signal.pause()

from agent_evidence import own_pty_child

mode = os.environ['FIXTURE_MODE']
if mode == 'success':
    print('unchanged success')
elif mode == 'failure':
    raise SystemExit(7)
else:
    credential = os.environ['FIXTURE_SECRET']
    private_url = 'https://fixture.invalid/' + credential
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    parent = os.getpid()
    pid, master = pty.fork()
    if pid == 0:
        own_pty_child(parent)
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        signal.signal(signal.SIGHUP, signal.SIG_IGN)
        while True:
            time.sleep(1)
    with open(os.environ['FIXTURE_PIDS'], 'w', encoding='ascii') as stream:
        json.dump([os.getpid(), pid], stream)

    def blocked_socket():
        with socket.socket() as connection:
            connection.connect(('127.0.0.1', int(os.environ['FIXTURE_PORT'])))
            connection.recv(1)

    thread = threading.Thread(target=blocked_socket)
    thread.start()
    thread.join()
    os.close(master)

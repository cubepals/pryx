# SPDX-License-Identifier: FSL-1.1-ALv2
"""Pause the test-owned engine's exact peer while the installed node serves another tier.

This is CI acceptance only. A pidfd pins the same process for both stop and resume;
no workload, system engine or process selected from an untrusted output is signalled.
"""
import http.client
import json
import os
from pathlib import Path
import signal
import socket
import struct
import subprocess
import sys
import time

cli = sys.argv[1]
retained = subprocess.run([cli, 'sandbox', 'get', sys.argv[2]], check=True, capture_output=True, text=True, timeout=5)
assert json.loads(retained.stdout)['isolation'] == 'container', 'The retained outage canary must be a container'
endpoint = Path(os.environ['PRYX_TEST_ENGINE_SOCKET'])
owned = Path(os.environ['RUNNER_TEMP']) / ('pryx-engine-' + os.environ['PRYX_TEST_ENGINE'])
assert endpoint.is_relative_to(owned)
assert endpoint.stat().st_uid == os.getuid()
with socket.socket(socket.AF_UNIX) as peer:
    peer.connect(str(endpoint))
    pid, uid, _ = struct.unpack('3i', peer.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, struct.calcsize('3i')))
    assert pid > 1 and uid == os.getuid()
    descriptor = os.pidfd_open(pid)
try:
    signal.pidfd_send_signal(descriptor, signal.SIGSTOP)
    with socket.socket(socket.AF_UNIX) as probe:
        probe.settimeout(1)
        probe.connect(str(endpoint))
        connection = http.client.HTTPConnection('localhost', timeout=1)
        connection.sock = probe
        connection.request('GET', '/_ping')
        try:
            connection.getresponse()
        except TimeoutError:
            pass
        else:
            raise AssertionError('The paused engine still answered its API')
    answer = subprocess.run([cli, 'run', '--isolation', 'process', '--', 'echo', 'alive'],
                            check=True, capture_output=True, text=True, timeout=45)
    assert answer.stdout.strip() == 'alive'
    # The pre-outage copy view can remain cached until a live node observation arrives.
    # Observe withdrawal first, so the later true value proves recovery rather than stale state.
    withdrawn = time.monotonic() + 45
    while True:
        view = subprocess.run([cli, 'sandbox', 'get', sys.argv[2]], capture_output=True, text=True, timeout=5)
        if view.returncode == 0 and json.loads(view.stdout).get('guarantees', {}).get('private-pids') is False:
            break
        if time.monotonic() >= withdrawn:
            raise AssertionError('The installed node did not withdraw paused container availability')
        time.sleep(0.2)
finally:
    signal.pidfd_send_signal(descriptor, signal.SIGCONT)
    os.close(descriptor)
deadline = time.monotonic() + 45
while True:
    view = subprocess.run([cli, 'sandbox', 'get', sys.argv[2]], capture_output=True, text=True, timeout=5)
    if view.returncode == 0 and json.loads(view.stdout).get('guarantees', {}).get('private-pids') is True:
        break
    if time.monotonic() >= deadline:
        raise AssertionError('The installed node did not restore container availability')
    time.sleep(0.2)
answer = subprocess.run([cli, 'run', '--isolation', 'container', '--image', 'alpine:3.22', '--', 'echo', 'recovered'],
                        check=True, capture_output=True, text=True, timeout=45)
assert answer.stdout.strip() == 'recovered'
print('engine_outage=process_serving engine_recovery=container_serving')

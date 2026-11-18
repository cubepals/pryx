# SPDX-License-Identifier: FSL-1.1-ALv2
"""Pause the test-owned engine's listener while the installed node serves another tier.

This is CI acceptance only. Pidfds pin each verified listener holder for stop and resume;
no workload, system engine or process selected from an untrusted output is signalled.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import time
from engine_pause import paused

cli = sys.argv[1]
retained = subprocess.run([cli, 'sandbox', 'get', sys.argv[2]], check=True, capture_output=True, text=True, timeout=5)
assert json.loads(retained.stdout)['isolation'] == 'container', 'The retained outage canary must be a container'
endpoint = Path(os.environ['PRYX_TEST_ENGINE_SOCKET'])
owned = Path(os.environ['RUNNER_TEMP']) / ('pryx-engine-' + os.environ['PRYX_TEST_ENGINE'])
with paused(endpoint, owned):
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

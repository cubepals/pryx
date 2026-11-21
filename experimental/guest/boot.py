#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Time a guest boot, ELF entry and the real agent's first vsock connection.

Each sample starts a fresh monitor and uses private sockets. Guest CLOCK_BOOTTIME
and host monotonic observations are retained separately. The real-agent socket
receives no bootstrap secret or command in this pre-bootstrap experiment.
"""
import json
import os
from pathlib import Path
import re
import selectors
import socket
import subprocess
import time

STAMP = re.compile(rb'(?:^|[\n\r])pryx-lab-pid1 ([0-9a-f]{16})\r?\n')


def sample(monitor, configurator, kernel, root, directory, boot_mode, port, extra, real):
    directory.mkdir(mode=0o700)
    channel = directory / 'v'
    host = Path(f'{channel}_{port}')
    writes = directory / 'disk'
    with writes.open('xb') as disk:
        disk.truncate(128 * 1024 * 1024)
    args = [configurator, '--kernel', kernel, '--root', root, '--writes', writes,
            '--socket', channel, '--init-mode', boot_mode]
    config = json.loads(subprocess.check_output(args, timeout=10))
    before, separator, after = config['boot-source']['boot_args'].partition(' -- ')
    if not separator:
        raise ValueError('Missing guest init mode in the verified configuration')
    config['boot-source']['boot_args'] = ' '.join([before, *extra]) + separator + after
    machine = directory / 'machine.json'
    machine.write_text(json.dumps(config))
    with (directory / 'serial.log').open('xb') as log, selectors.DefaultSelector() as events, \
            socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
        listener.bind(str(host))
        listener.listen(1)
        if real:
            events.register(listener, selectors.EVENT_READ)
        started = time.monotonic_ns()
        process = subprocess.Popen([monitor, '--no-api', '--config-file', machine],
                                   stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                   stderr=subprocess.STDOUT)
        observation = {}
        output = bytearray()
        try:
            events.register(process.stdout, selectors.EVENT_READ)
            deadline = time.monotonic() + 20
            while 'pid1BootNs' not in observation or (real and 'launchToBootstrapNs' not in observation):
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError(f'Missing guest observation in {directory}')
                for event, _ in events.select(remaining):
                    seen = time.monotonic_ns() - started
                    if event.fileobj is listener:
                        connection, _ = listener.accept()
                        connection.close()
                        events.unregister(listener)
                        observation['launchToBootstrapNs'] = seen
                    else:
                        chunk = os.read(process.stdout.fileno(), 65536)
                        if not chunk:
                            raise ValueError(f'Monitor exited before proof in {directory}')
                        log.write(chunk)
                        output.extend(chunk)
                        if len(output) > 4 * 1024 * 1024:
                            raise ValueError('Guest serial exceeded 4 MiB')
                        match = STAMP.search(output)
                        if match and 'pid1BootNs' not in observation:
                            observation['pid1BootNs'] = int(match.group(1), 16)
                            observation['launchToPid1SerialNs'] = seen
            return observation
        finally:
            if process.poll() is None:
                process.kill()
            process.wait(timeout=5)
            process.stdout.close()
            host.unlink(missing_ok=True)
            channel.unlink(missing_ok=True)
            writes.unlink(missing_ok=True)

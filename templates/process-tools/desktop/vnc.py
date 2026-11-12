# SPDX-License-Identifier: FSL-1.1-ALv2
"""Give each private Unix connection to the unchanged x11vnc inetd program."""
import os
import socket
import subprocess
import sys

listener = socket.socket(socket.AF_UNIX)
listener.bind(sys.argv[1])
os.chmod(sys.argv[1], 0o600)
listener.listen()
children = []
while True:
    connection, _ = listener.accept()
    children = [child for child in children if child.poll() is None]
    with connection:
        children.append(subprocess.Popen([
            sys.argv[2], '-inetd', '-display', os.environ['DISPLAY'],
            '-auth', os.environ['XAUTHORITY'], '-nopw', '-quiet', '-shared',
            '-noxdamage', '-noshm', '-no6', '-rfbport', '0',
        ], stdin=connection, stdout=connection, stderr=subprocess.DEVNULL))

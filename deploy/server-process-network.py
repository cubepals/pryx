# SPDX-License-Identifier: FSL-1.1-ALv2
"""A process sandbox's kernel network policy through the installed server preset, as root runs it:
with public internet off it reaches the one address its rule names and nothing else, and its view
says the host firewall holds. Run after the root VM acceptance with the credential the install
wrote for the sudo caller; deletes what it makes."""
import os
from cubepals_pryx import Pryx

# A connection attempt from inside the sandbox: `reached`, or `refused` and the errno's name.
CONNECT = """import errno, socket, sys
try:
    socket.create_connection((sys.argv[1], int(sys.argv[2])), timeout=10).close()
    print('reached')
except OSError as error:
    print('refused', errno.errorcode.get(error.errno, error.errno))
"""
ALLOWED = ('1.1.1.1', '443')
OTHER = ('8.8.8.8', '443')


def connect(box, destination):
    result = box.run(['python3', '-c', CONNECT, *destination])
    assert result.code == 0, result.stderr
    return result.stdout.strip()


with Pryx(url=os.environ['PRYX_API_URL'], key=os.environ['PRYX_API_KEY']) as client:
    box = client.sandbox(
        isolation='process',
        require=['host-firewall'],
        egress={'allow': []},
        egressAddresses={'allow': [{'prefix': ALLOWED[0] + '/32', 'ports': [443, 443]}]},
    )
    try:
        assert box.isolation == 'process', box.isolation
        assert connect(box, ALLOWED) == 'reached'
        # The sandbox's connect program refused it before any packet left.
        assert connect(box, OTHER) == 'refused EPERM'
        print('root_installed_process_network=passed')
    finally:
        box.delete()

# SPDX-License-Identifier: FSL-1.1-ALv2
"""Inside one real policy cgroup, while its owner deliberately never refreshes its rules."""
import ctypes
import socket
import struct
import sys

address4, address6, tcp4, udp4, tcp6, udp6 = sys.argv[1:]
tcp4, udp4, tcp6, udp6 = map(int, (tcp4, udp4, tcp6, udp6))
libc = ctypes.CDLL(None, use_errno=True)
libc.connect.argtypes = [ctypes.c_int, ctypes.c_void_p, ctypes.c_uint]
libc.connect.restype = ctypes.c_int


def pair(family):
    loopback = "127.0.0.1" if family == socket.AF_INET else "::1"
    listener = socket.socket(family, socket.SOCK_STREAM)
    listener.bind((loopback, 0))
    listener.listen()
    client = socket.socket(family, socket.SOCK_STREAM)
    client.settimeout(2)
    client.connect(listener.getsockname())
    accepted, _ = listener.accept()
    accepted.settimeout(2)
    client.sendall(b"request")
    assert accepted.recv(7) == b"request"
    accepted.sendall(b"reply")
    assert client.recv(5) == b"reply", "an accepted TCP descriptor can answer"
    listener.close()
    return client, accepted


def disconnect(sock):
    # connect(2): AF_UNSPEC dissolves a TCP association without changing the descriptor.
    address = ctypes.create_string_buffer(struct.pack("H14s", socket.AF_UNSPEC, b""))
    assert libc.connect(sock.fileno(), address, len(address) - 1) == 0, ctypes.get_errno()


def refused(sock, address, port):
    sock.settimeout(0.25)
    try:
        sock.connect((address, port))
    except OSError:
        return
    raise AssertionError(f"a stale public rule reached newly host-local {address}:{port}")


def reuse_owned(accepted, family):
    loopback = "127.0.0.1" if family == socket.AF_INET else "::1"
    with socket.socket(family, socket.SOCK_STREAM) as listener:
        listener.bind((loopback, 0))
        listener.listen()
        accepted.connect(listener.getsockname())
        with listener.accept()[0] as peer:
            accepted.sendall(b"reused")
            assert peer.recv(6) == b"reused"
            peer.sendall(b"usable")
            assert accepted.recv(6) == b"usable"


for family in (socket.AF_INET, socket.AF_INET6):
    client, accepted = pair(family)
    client.close()
    accepted.close()

owned4 = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
owned4.bind(("0.0.0.0", 0))
owned4.listen()
owned6 = socket.socket(socket.AF_INET6, socket.SOCK_STREAM)
owned6.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 1)
owned6.bind(("::", 0))
owned6.listen()

# Preserve an ordinary IPv6 Destination Options chain to an owned UDP socket. This is one
# eight-octet header with PadN, using the kernel's IPV6_DSTOPTS socket option (RFC 3542).
options = bytes((socket.IPPROTO_UDP, 0, 1, 4, 0, 0, 0, 0))
with socket.socket(socket.AF_INET6, socket.SOCK_DGRAM) as receiver:
    receiver.bind(("::1", 0))
    receiver.settimeout(2)
    with socket.socket(socket.AF_INET6, socket.SOCK_DGRAM) as sender:
        sender.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_DSTOPTS, options)
        sender.sendto(b"owned-options", receiver.getsockname())
        assert receiver.recv(64) == b"owned-options"

print("ready", flush=True)
assert sys.stdin.readline().strip() == "go"
for family, address, listener in (
    (socket.AF_INET, address4, owned4),
    (socket.AF_INET6, "::ffff:" + address4, owned4),
    (socket.AF_INET6, address6, owned6),
):
    with socket.socket(family, socket.SOCK_STREAM) as client:
        client.settimeout(2)
        destination = (address, listener.getsockname()[1])
        try:
            client.connect(destination)
        except OSError:
            print(f"owned {family.name} listener at {destination}", file=sys.stderr)
            raise
        with listener.accept()[0] as accepted:
            accepted.settimeout(2)
            client.sendall(b"new-owned")
            assert accepted.recv(9) == b"new-owned"
            accepted.sendall(b"still-owned")
            assert client.recv(11) == b"still-owned"
for attempt in range(10):
    for family, address, tcp, udp in (
        (socket.AF_INET, address4, tcp4, udp4),
        (socket.AF_INET6, "::ffff:" + address4, tcp4, udp4),
        (socket.AF_INET6, address6, tcp6, udp6),
    ):
        with socket.socket(family, socket.SOCK_STREAM) as sock:
            refused(sock, address, tcp)
        for connected in (False, True):
            with socket.socket(family, socket.SOCK_DGRAM) as sock:
                if address == address6:
                    sock.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_DSTOPTS, options)
                try:
                    if connected:
                        sock.connect((address, udp))
                        sock.send(b"must-not-arrive")
                    else:
                        sock.sendto(b"must-not-arrive", (address, udp))
                except OSError:
                    pass
    for family, address, port in (
        (socket.AF_INET, address4, tcp4),
        (socket.AF_INET6, address6, tcp6),
    ):
        client, accepted = pair(family)
        disconnect(accepted)
        client.close()
        reuse_owned(accepted, family)
        disconnect(accepted)
        # The accepted descriptor had no cloned socket storage. A reconnect must create
        # initiated-connection storage before the packet hook can apply its response rule.
        refused(accepted, address, port)
        accepted.close()
    print(f"refused {attempt}", flush=True)
    assert sys.stdin.readline().strip() == "checked"


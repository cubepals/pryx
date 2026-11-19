# SPDX-License-Identifier: FSL-1.1-ALv2
"""Inside one real policy cgroup: where a sandbox's packets actually go, one case per mode.

Each mode prints one line of outcomes, `ok` or `refused <errno>`, for path_tests.rs to read.
"""
import ctypes
import socket
import struct
import sys
import time

# linux/in.h and linux/in6.h; Python's socket module names neither.
IP_FREEBIND = 15
IPV6_FREEBIND = 78
# RFC 791 §3.1: loose source and record route, a no-operation to pad it, and where its first
# address starts.
LSRR = 0x83
NOP = 0x01
ROUTE_POINTER = 4
# RFC 8754 §2: the segment routing header's routing type.
SEGMENT_ROUTING = 4

libc = ctypes.CDLL(None, use_errno=True)
libc.connect.argtypes = [ctypes.c_int, ctypes.c_void_p, ctypes.c_uint]
libc.connect.restype = ctypes.c_int


def outcome(action):
    try:
        action()
    except OSError as error:
        return f"refused {error.errno}"
    return "ok"


def disconnect(sock):
    # connect(2): AF_UNSPEC dissolves a TCP association without changing the descriptor.
    address = ctypes.create_string_buffer(struct.pack("H14s", socket.AF_UNSPEC, b""))
    assert libc.connect(sock.fileno(), address, len(address) - 1) == 0, ctypes.get_errno()


def broadcast(address, port):
    """A datagram to a subnet's broadcast address, which the host's own sockets also receive."""
    port = int(port)
    results = []
    for family, target in ((socket.AF_INET, address), (socket.AF_INET6, "::ffff:" + address)):
        for connected in (False, True):
            with socket.socket(family, socket.SOCK_DGRAM) as sock:
                sock.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
                if connected:
                    results.append(outcome(lambda: (sock.connect((target, port)), sock.send(b"must-not-arrive"))))
                else:
                    results.append(outcome(lambda: sock.sendto(b"must-not-arrive", (target, port))))
    return results


def twin(address):
    """A host address's NAT64 twin, from a socket that once dialled out, at a listener of its own."""
    with socket.socket(socket.AF_INET6, socket.SOCK_STREAM) as dialler:
        dialler.setblocking(False)
        try:
            dialler.connect(("::ffff:192.0.2.1", 80))
        except BlockingIOError:
            pass
        disconnect(dialler)
        with socket.socket(socket.AF_INET6, socket.SOCK_STREAM) as listener:
            listener.setsockopt(socket.IPPROTO_IPV6, IPV6_FREEBIND, 1)
            listener.bind((address, 0))
            listener.listen()
            dialler.settimeout(2)
            return [outcome(lambda: dialler.connect((address, listener.getsockname()[1])))]


def stale(address):
    """An address the rules still call this host's, now routed off it, at a listener of its own."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.setsockopt(socket.IPPROTO_IP, IP_FREEBIND, 1)
        listener.bind((address, 0))
        listener.listen()
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as client:
            client.settimeout(1)
            return [outcome(lambda: client.connect((address, listener.getsockname()[1])))]


def reuseport(port):
    """Join another owner's reuse-port group and dial it until the connect hook picks our own."""
    port = int(port)
    members = []

    def join():
        member = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        member.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEPORT, 1)
        member.bind(("127.0.0.1", port))
        member.listen()
        member.setblocking(False)
        members.append(member)

    for _ in range(4):
        join()
    moved = 0
    for _ in range(96):
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as client:
            client.settimeout(0.5)
            if outcome(lambda: client.connect(("127.0.0.1", port))) == "refused 1" and len(members) < 16:
                # The hook's own lookup found the other owner: a larger group moves its pick.
                join()
                moved += 1
        for member in members:
            try:
                while True:
                    member.accept()[0].close()
            except BlockingIOError:
                pass
    return [f"moved {moved}"]


def source_route(first4, first6, port):
    """Datagrams to allowed destinations whose source routes name another first hop."""
    port = int(port)
    lsrr = bytes((NOP, LSRR, 7, ROUTE_POINTER)) + socket.inet_aton(first4)
    srh = struct.pack("!BBBBBBH", 0, 2, SEGMENT_ROUTING, 0, 0, 0, 0) + socket.inet_pton(socket.AF_INET6, first6)
    results = []
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        sock.setsockopt(socket.IPPROTO_IP, socket.IP_OPTIONS, lsrr)
        results.append(outcome(lambda: sock.sendto(b"routed", ("192.0.2.1", port))))
    with socket.socket(socket.AF_INET6, socket.SOCK_DGRAM) as sock:
        sock.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_RTHDR, srh)
        results.append(outcome(lambda: sock.sendto(b"routed", ("2001:db8::1", port))))
    return results


def time_wait():
    """Dial a listener of its own again from the same port while the served side waits."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        source, elapsed = 0, 0.0
        for _ in range(2):
            with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as client:
                client.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                client.bind(("127.0.0.1", source))
                source = client.getsockname()[1]
                client.settimeout(3)
                began = time.monotonic()
                result = outcome(lambda: client.connect(listener.getsockname()))
                if result != "ok":
                    return [result]
                elapsed = time.monotonic() - began
                served, _ = listener.accept()
                # The served side closes first, so it is the one left in TIME-WAIT.
                served.close()
                assert client.recv(1) == b""
            # Let the client's own last ACK arrive before its port is bound again.
            time.sleep(0.1)
        return ["ok" if elapsed < 1 else f"slow {elapsed:.2f}"]


MODES = {
    "broadcast": broadcast,
    "twin": twin,
    "stale": stale,
    "reuseport": reuseport,
    "source-route": source_route,
    "time-wait": time_wait,
}
print(" ".join(MODES[sys.argv[1]](*sys.argv[2:])), flush=True)

# SPDX-License-Identifier: FSL-1.1-ALv2
"""Construct valid Ethernet/IP packets for disposable cgroup_skb program test runs."""

import ipaddress
import socket
import struct


def checksum(data):
    if len(data) % 2:
        data += b"\0"
    total = sum(word for word, in struct.iter_unpack("!H", data))
    while total >> 16:
        total = (total & 0xffff) + (total >> 16)
    return ~total & 0xffff


def packet(ip_version, protocol, payload):
    addresses = ("192.0.2.1", "192.0.2.2") if ip_version == 4 else ("2001:db8::1", "2001:db8::2")
    src, dst = [ipaddress.ip_address(address).packed for address in addresses]
    if protocol == socket.IPPROTO_TCP:
        segment = struct.pack("!HHIIHHHH", 48152, 8080, 1, 1, (5 << 12) | 0x10, 65535, 0, 0)
        field = 16
    else:
        segment = struct.pack("!HHHH", 48152, 8080, 8 + len(payload), 0)
        field = 6
    segment += payload
    length = len(segment)
    if ip_version == 4:
        pseudo = src + dst + struct.pack("!BBH", 0, protocol, length)
        header = struct.pack("!BBHHHBBH4s4s", 0x45, 0, 20 + length, 0, 0, 64, protocol, 0, src, dst)
        header = header[:10] + struct.pack("!H", checksum(header)) + header[12:]
        ether_type = 0x0800
    else:
        pseudo = src + dst + struct.pack("!I3xB", length, protocol)
        header = struct.pack("!IHBB16s16s", 6 << 28, length, protocol, 64, src, dst)
        ether_type = 0x86dd
    value = checksum(pseudo + segment)
    if protocol == socket.IPPROTO_UDP and value == 0:
        value = 0xffff
    segment = segment[:field] + struct.pack("!H", value) + segment[field + 2:]
    return bytes.fromhex("020000000002020000000001") + struct.pack("!H", ether_type) + header + segment


def packets():
    for version in [4, 6]:
        for name, protocol in [("tcp", socket.IPPROTO_TCP), ("udp", socket.IPPROTO_UDP)]:
            for size in [0, 1, 64, 1024]:
                yield f"v{version}-{name}-{size}", packet(version, protocol, bytes(i % 251 for i in range(size)))

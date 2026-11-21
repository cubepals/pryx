# SPDX-License-Identifier: FSL-1.1-ALv2
"""Offline regressions for map normalization and refusal accounting in the fixed UDP comparison."""
import copy
import ipaddress
from pathlib import Path
import struct
import unittest
from unittest.mock import patch

import socket_compare as worker


def octets(data):
    return [hex(value) for value in data]


def fixture(cookie, cpu):
    value = bytearray(200)
    value[0:2] = bytes((2, 2))
    value[8:16] = struct.pack("=HHBBH", 26404, 26404, 2, 0, 0)
    value[16:24] = struct.pack("=HHBBH", 26403, 26403, 2, 1, 0)
    rules = [{"key": octets(struct.pack("=I", 128) + ipaddress.IPv6Address(ip).packed), "value": octets(value)}
             for ip in ("::ffff:127.0.0.1", "::1")]
    counters = []
    for index, counter in enumerate(((2, 94), (2, 94), (2, 0), (0, 0), (0, 0))):
        counters.append({"key":octets(struct.pack("=I", index)), "values":[
            {"cpu":n, "value":octets(struct.pack("=QQ", *(counter if n == cpu else (0,0))))} for n in (0,1)]})
    peer = struct.pack("=Q", cookie) + ipaddress.IPv6Address("::ffff:127.0.0.1").packed + struct.pack("!H", 26403) + bytes(6)
    return {"rules":rules, "config":[{"key":octets(bytes(4)),"value":octets(bytes(96))}],
            "peers":[{"key":octets(peer),"value":octets(b"\x01")}],"counters":counters}


def observe(cookie, tables, mutated=False):
    state = bytes(23) + bytes([int(mutated)])
    reader = "\n".join(f"{cookie+n} 1 {state.hex()}" for n in range(4))
    with patch.object(worker.subprocess,"check_output",return_value=reader), \
         patch.object(worker,"map_dump",side_effect=lambda _, __, table: tables[table]):
        return worker.observe("tool",Path("/unused"),Path("/reader"),(10,11),(12,13,14,15),{},"127.0.0.1","allow-send")


class PrivateState(unittest.TestCase):
    def test_normalizes_kernel_cookie_and_retains_cpu_difference(self):
        first, raw_a = observe(8, fixture(8, 0))
        second, raw_b = observe(517, fixture(517, 1))
        self.assertEqual(first, second)
        self.assertNotEqual(raw_a, raw_b)

    def test_rejects_wrong_cookie_socket_value_and_missing_refusal(self):
        known = fixture(8, 0)
        with self.assertRaisesRegex(ValueError, "peer address"):
            observe(19, known)
        with self.assertRaisesRegex(ValueError, "live state"):
            observe(8, known, mutated=True)
        wrong = copy.deepcopy(known)
        wrong["counters"][2]["values"][0]["value"] = octets(bytes(16))
        with self.assertRaisesRegex(ValueError, "policy counters"):
            observe(8, wrong)


if __name__ == "__main__":
    unittest.main()

# SPDX-License-Identifier: FSL-1.1-ALv2
"""Regression fixtures for perf script's native symbol output and denied sampling."""
import json
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest
from unittest.mock import patch

import record


def boundaries():
    return [{"end_of": label, "observed_ns": 10**9 + i * 100_000_000,
             "workload_reported_s": 0.1 if i else None} for i, label in enumerate(record.LABELS)]


class Record(unittest.TestCase):
    def test_symbolized_fields_require_ip_dso_and_period(self):
        rows = record.parse_samples([
            "1.000000001: 10000000    55a5d67887a0 pryxd::tarball::pack (/tmp/storage_bench-x)",
            "1.050000000:       10    000000000001 [unknown] ([unknown])",
            "1.100000000: 10000000        0000abcd memcpy (/lib/aarch64-linux-gnu/libc.so.6)",
            "1.190000000: 10000000",
        ], boundaries())
        self.assertTrue(rows[0]["rankable"])
        self.assertEqual(rows[0]["sample_count"], 2)
        self.assertEqual(rows[0]["unresolved_sample_ns"], 10)
        self.assertEqual(rows[0]["symbols"][0]["name"], "pryxd::tarball::pack")
        self.assertEqual(rows[1]["symbols"][0]["dso"], "/lib/aarch64-linux-gnu/libc.so.6")
        self.assertFalse(rows[1]["rankable"])
        self.assertEqual(rows[1]["malformed_sample_count"], 1)
        self.assertFalse(rows[2]["rankable"])

    def test_permission_denial_keeps_wall_times_and_no_symbol_claim(self):
        with TemporaryDirectory() as tmp, patch.dict("os.environ", {"RUNNER_TEMP": "/fixture"}):
            directory = Path(tmp)
            with patch.object(record.subprocess, "check_output", return_value="source"), \
                    patch.object(record.subprocess, "run") as probe, \
                    patch.object(record, "phases", return_value=(0, boundaries())) as workload:
                probe.return_value.returncode = 1
                record.main("perf", "/fixture/storage_bench", tmp)
            command = workload.call_args.args[0]
            self.assertIn("/fixture/storage_bench", command)
            self.assertNotIn("sudo", command)
            self.assertEqual(probe.call_count, 1)
            report = json.loads((directory / "profile.json").read_text())
            self.assertEqual(report["status"], "sampling unavailable")
            self.assertEqual(len(report["phases"]), 5)
            self.assertTrue(all(r["wall_ns"] > 0 and not r["rankable"] for r in report["phases"]))


if __name__ == "__main__":
    unittest.main()

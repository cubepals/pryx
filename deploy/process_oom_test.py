#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Exercise the process-limit fixture's evidence rules using ordinary files."""
import json
import os
from pathlib import Path
import tempfile
import unittest

from process_oom import observe


class ObservationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        self.cgroup = root / "pryx-target"
        self.cgroup.mkdir()
        self.events = self.cgroup / "memory.events"
        self.count(3)
        self.log = root / "node.log"
        self.log.touch()

    def count(self, kills):
        self.events.write_text(f"low 0\nmax 17\noom 12\noom_kill {kills}\noom_group_kill 0\n")

    def record(self, cgroup, kills, target="pryxd::runtime::process::resources"):
        fields = {"message": "process sandbox's commands passed its memory.max",
                  "cgroup": str(cgroup), "oom_kills": kills}
        with self.log.open("a") as output:
            print(json.dumps({"level": "INFO", "target": target, "fields": fields}), file=output)

    def retire(self):
        self.events.unlink()
        self.cgroup.rmdir()
        return os.EX_TEMPFAIL

    def test_live_counter_returns_increment_and_command_status(self):
        def run():
            self.count(5)
            return os.EX_TEMPFAIL

        self.assertEqual(observe(self.cgroup, self.log, run), (os.EX_TEMPFAIL, 2))

    def test_retirement_record_returns_increment_from_original_counter(self):
        def run():
            self.record(self.cgroup, 4)
            return self.retire()

        self.assertEqual(observe(self.cgroup, self.log, run), (os.EX_TEMPFAIL, 1))

    def test_missing_retirement_record_is_rejected(self):
        with self.assertRaises(RuntimeError):
            observe(self.cgroup, self.log, self.retire)

    def test_record_from_before_command_is_rejected(self):
        self.record(self.cgroup, 8)
        with self.assertRaises(RuntimeError):
            observe(self.cgroup, self.log, self.retire)

    def test_record_for_other_cgroup_is_rejected(self):
        def run():
            self.record(self.cgroup.with_name("pryx-neighbor"), 4)
            return self.retire()

        with self.assertRaises(RuntimeError):
            observe(self.cgroup, self.log, run)

    def test_record_from_other_target_is_rejected(self):
        def run():
            self.record(self.cgroup, 4, target="another::module")
            return self.retire()

        with self.assertRaises(RuntimeError):
            observe(self.cgroup, self.log, run)

    def test_successful_command_is_rejected_even_if_counter_increases(self):
        def run():
            self.count(4)
            return 0

        with self.assertRaises(RuntimeError):
            observe(self.cgroup, self.log, run)

    def test_preexisting_live_kills_are_insufficient(self):
        with self.assertRaises(RuntimeError):
            observe(self.cgroup, self.log, lambda: os.EX_TEMPFAIL)

    def test_retirement_without_an_increment_is_rejected(self):
        def run():
            self.record(self.cgroup, 3)
            return self.retire()

        with self.assertRaises(RuntimeError):
            observe(self.cgroup, self.log, run)


if __name__ == "__main__":
    unittest.main()

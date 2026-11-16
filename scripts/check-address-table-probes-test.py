# SPDX-License-Identifier: FSL-1.1-ALv2
"""Cancellation controls through the actual probe harness and its real command runner.

Private executable shims replace systemd tools; no real unit is created. They stall admission
or status for 20 seconds and verify the admission condition is closed before cleanup begins.
"""
import ctypes
import importlib.util
import json
import os
from pathlib import Path
import select
import signal
import socket
import subprocess
import sys
import tempfile
import time
import unittest


def event(directory, name):
    with (directory / "commands").open("a") as stream:
        stream.write(json.dumps({"event": name, "pid": os.getpid()}) + "\n")


def stage(directory, value):
    with socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM) as channel:
        channel.sendto(value, str(directory / "stages"))


def tool(name, arguments):
    directory = Path(os.environ["PRYX_PROBE_FIXTURE_DIRECTORY"])
    phase = os.environ["PRYX_PROBE_FIXTURE_PHASE"]
    if name == "systemd-run":
        assert "--no-block" in arguments
        assert "--property=StandardOutput=journal" in arguments
        assert "--property=StandardError=journal" in arguments
        assert not any(value.startswith("--property=StandardOutput=file:") for value in arguments)
        lease = next(value.removeprefix("--property=ConditionPathExists=") for value in arguments
                     if value.startswith("--property=ConditionPathExists="))
        assert Path(lease).is_file()
        (directory / "lease").write_text(lease)
        event(directory, "start")
        if phase == "admission":
            stage(directory, b"R")
            time.sleep(20)
    elif name == "systemctl" and arguments[0] == "show":
        event(directory, "show")
        stage(directory, b"R")
        time.sleep(20)
    elif name == "systemctl" and arguments[0] == "stop":
        assert not Path((directory / "lease").read_text()).exists()
        event(directory, "stop started")
        stage(directory, b"T")
        time.sleep(0.2)
        event(directory, "stop completed")
    else:
        raise RuntimeError(f"Unexpected command after cancellation: {name} {arguments}")


def owner(directory, entry, phase):
    source = Path(__file__).with_name("check-address-table-probes.py")
    spec = importlib.util.spec_from_file_location("address_probes", source)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    binaries = directory / "bin"
    binaries.mkdir()
    for name in ("systemd-run", "systemctl", "journalctl"):
        script = binaries / name
        script.write_text(f"#!{sys.executable}\nimport runpy, sys\nsys.argv = [{__file__!r}, '--tool', {name!r}] + sys.argv[1:]\nrunpy.run_path({__file__!r}, run_name='__main__')\n")
        script.chmod(0o755)
    os.environ["PATH"] = str(binaries) + os.pathsep + os.environ["PATH"]
    os.environ["PRYX_PROBE_FIXTURE_DIRECTORY"] = str(directory)
    os.environ["PRYX_PROBE_FIXTURE_PHASE"] = phase
    if entry == "library":
        module.probe("/bin/echo", directory, "baseline")
    else:
        module.main(["/bin/echo", str(directory)])


class Cancellation(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # PR_SET_CHILD_SUBREAPER from linux/prctl.h: failed controls still reap their own shims.
        PR_SET_CHILD_SUBREAPER = 36
        result = ctypes.CDLL(None, use_errno=True).prctl(PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0)
        if result:
            raise OSError(ctypes.get_errno(), "Cannot own fixture descendants")

    def test_closed_admission_refuses_even_a_late_inside_start(self):
        source = Path(__file__).with_name("check-address-table-probes.py")
        with tempfile.TemporaryDirectory() as temporary:
            lease = Path(temporary) / "admission"
            lease.touch()
            runtime = Path(temporary) / "runtime"
            runtime.mkdir(mode=0o700)
            result = subprocess.run([sys.executable, source, "--inside", "/bin/echo", "baseline", lease, runtime], capture_output=True, text=True, check=False)
            self.assertEqual((result.returncode, result.stdout, result.stderr), (0, "", ""))
            self.assertEqual((runtime / "probe.json").read_text(), "probe --json\n")
            self.assertEqual((runtime / "probe.stderr").read_text(), "")
            self.assertEqual((runtime / "probe.json").stat().st_mode & 0o777, 0o600)
            lease.unlink()
            result = subprocess.run([sys.executable, source, "--inside", "/bin/echo", "baseline", lease, runtime], capture_output=True, text=True, check=False)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(result.stdout, "")
            self.assertIn("Probe admission is closed", result.stderr)
            self.assertEqual((runtime / "probe.json").read_text(), "probe --json\n")

    def test_inside_refuses_to_replace_existing_report_files(self):
        source = Path(__file__).with_name("check-address-table-probes.py")
        with tempfile.TemporaryDirectory() as temporary:
            runtime = Path(temporary)
            lease = runtime / "admission"
            lease.touch()
            report = runtime / "probe.json"
            report.write_text("retained evidence\n")
            result = subprocess.run([sys.executable, source, "--inside", "/bin/echo", "baseline", lease, runtime], capture_output=True, text=True, check=False)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(result.stdout, "")
            self.assertEqual(report.read_text(), "retained evidence\n")

    def test_stalled_commands_stop_promptly_and_preserve_repeated_owner_signals(self):
        for entry in ("library", "cli"):
            for phase in ("admission", "status"):
                for initial in (signal.SIGTERM, signal.SIGINT):
                    with self.subTest(entry=entry, phase=phase, initial=initial), tempfile.TemporaryDirectory() as temporary:
                        directory = Path(temporary)
                        with socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM) as channel:
                            channel.bind(str(directory / "stages"))
                            channel.settimeout(5)
                            with (directory / "owner.log").open("w") as output:
                                child = subprocess.Popen(
                                    [sys.executable, __file__, "--owner", str(directory), entry, phase],
                                    stdout=output, stderr=subprocess.STDOUT, start_new_session=True,
                                )
                                pidfd = os.pidfd_open(child.pid)
                                try:
                                    self.assertEqual(channel.recv(1), b"R")
                                    started = time.monotonic()
                                    signal.pidfd_send_signal(pidfd, initial)
                                    self.assertEqual(channel.recv(1), b"T")
                                    stopping = time.monotonic() - started
                                    signal.pidfd_send_signal(pidfd, signal.SIGINT if initial == signal.SIGTERM else signal.SIGTERM)
                                    self.assertTrue(select.select([pidfd], [], [], 5)[0], "owner did not finish cleanup")
                                    elapsed = time.monotonic() - started
                                    outcome = os.waitid(os.P_PID, child.pid, os.WEXITED | os.WNOWAIT)
                                    self.assertEqual((outcome.si_code, outcome.si_status), (os.CLD_KILLED, initial))
                                    self.assertLess(elapsed, 5)
                                    events = [json.loads(line)["event"] for line in (directory / "commands").read_text().splitlines()]
                                    self.assertEqual(events.count("start"), 1)
                                    self.assertEqual(events.count("show"), int(phase == "status"))
                                    self.assertEqual(events.count("stop started"), 1)
                                    self.assertEqual(events.count("stop completed"), 1)
                                    print(f"{entry}/{phase}/{initial.name}: stop {stopping:.3f}s; exit {elapsed:.3f}s", file=sys.stderr)
                                finally:
                                    # WNOWAIT keeps the group leader's PID reserved until this kill.
                                    try:
                                        os.killpg(child.pid, signal.SIGKILL)
                                    except ProcessLookupError:
                                        pass
                                    child.wait()
                                    os.close(pidfd)
                                    if (directory / "commands").exists():
                                        for pid in {json.loads(line)["pid"] for line in (directory / "commands").read_text().splitlines()}:
                                            try:
                                                os.waitpid(pid, 0)
                                            except ChildProcessError:
                                                pass
                                    if (directory / "lease").exists():
                                        Path((directory / "lease").read_text()).unlink(missing_ok=True)


if __name__ == "__main__":
    if len(sys.argv) == 5 and sys.argv[1] == "--owner":
        owner(Path(sys.argv[2]), sys.argv[3], sys.argv[4])
    elif len(sys.argv) >= 3 and sys.argv[1] == "--tool":
        tool(sys.argv[2], sys.argv[3:])
    else:
        unittest.main()

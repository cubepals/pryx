# SPDX-License-Identifier: FSL-1.1-ALv2
"""Prove unreadable host address tables refuse kernel policy in private systemd units.

Retain the real probe's report and selected unit status before retiring each unit. The inside
mode checks the masked path in the same PID that execs pryxd; it never reads sandbox state.
"""
import errno
from contextlib import contextmanager
import json
import os
from pathlib import Path
import signal
import shutil
import subprocess
import sys
import time
import uuid


def report_outputs(runtime):
    # systemd sets up StandardOutput before creating RuntimeDirectory. Open these only
    # after it has started this command, when the owned directory actually exists.
    for descriptor, name in ((sys.stdout.fileno(), "probe.json"), (sys.stderr.fileno(), "probe.stderr")):
        opened = os.open(Path(runtime) / name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        try:
            os.dup2(opened, descriptor)
        finally:
            os.close(opened)


def inside(binary, table, lease, runtime):
    if not Path(lease).is_file():
        raise RuntimeError("Probe admission is closed")
    report_outputs(runtime)
    for name in ("fib_trie", "if_inet6") if table == "baseline" else (table,):
        path = Path("/proc/net") / name
        try:
            with path.open("rb") as stream:
                stream.read(1)
        except OSError as error:
            if table == "baseline" or error.errno not in (errno.EACCES, errno.EPERM):
                raise
            print(f"Fixture verified read denial for {path}: {error.strerror}", file=sys.stderr, flush=True)
        else:
            if table != "baseline":
                raise RuntimeError(f"Fixture left {path} readable")
    os.execv(binary, [binary, "probe", "--json"])


class Cancelled(BaseException):
    pass


class Cancellation:
    def __init__(self):
        self.requested = None
        self.lease = None

    def record(self, number, _frame):
        if self.requested is None:
            self.requested = number

    def close_admission(self):
        if self.lease is not None:
            self.lease.unlink(missing_ok=True)

    def check(self):
        if self.requested is not None:
            self.close_admission()
            raise Cancelled()


def command(arguments, output, timeout=10, cancellation=None):
    if cancellation:
        cancellation.check()
    with output.open("w") as stream:
        output.chmod(0o644)
        process = subprocess.Popen(arguments, stdin=subprocess.DEVNULL, stdout=stream, stderr=subprocess.STDOUT)
        deadline = time.monotonic() + timeout
        try:
            while True:
                if cancellation:
                    cancellation.check()
                try:
                    return process.wait(timeout=0.05)
                except subprocess.TimeoutExpired:
                    if time.monotonic() >= deadline:
                        if cancellation:
                            cancellation.close_admission()
                        stream.write(f"Command exceeded {timeout}s\n")
                        return os.EX_TEMPFAIL
        finally:
            if process.returncode is None:
                process.kill()
                process.wait(timeout=1)


def show(path):
    if path.exists():
        print(f"--- {path.name} ---", file=sys.stderr)
        print(path.read_text(errors="replace")[:8192], file=sys.stderr)


@contextmanager
def owned_termination():
    cancellation = Cancellation()
    previous = {number: signal.signal(number, cancellation.record) for number in (signal.SIGTERM, signal.SIGINT)}
    try:
        yield cancellation
    except Cancelled:
        if cancellation.requested is None:
            raise
    finally:
        for number, handler in previous.items():
            signal.signal(number, handler)
        if cancellation.requested is not None:
            # Repeated TERM/INT cannot interrupt the mandatory bounded stop.
            signal.signal(cancellation.requested, signal.SIG_DFL)
            signal.raise_signal(cancellation.requested)
            raise RuntimeError("The cancellation signal was not delivered")


def probe(binary, evidence, table):
    with owned_termination() as cancellation:
        cancellation.check()
        owned_probe(binary, evidence, table, cancellation)


def unit_status(unit, status, cancellation):
    selected = "LoadState,ActiveState,SubState,Result,ExecMainCode,ExecMainStatus,ConditionResult,InaccessiblePaths,CapabilityBoundingSet,Delegate"
    return command(["systemctl", "show", unit, f"--property={selected}"], status, timeout=2, cancellation=cancellation)


def wait_finished(unit, status, cancellation):
    deadline = time.monotonic() + 25
    while True:
        observed = unit_status(unit, status, cancellation)
        cancellation.check()
        if observed:
            raise RuntimeError(f"Unit status query exited {observed}")
        fields = dict(line.split("=", 1) for line in status.read_text().splitlines() if "=" in line)
        if fields.get("ActiveState") == "active" and fields.get("SubState") == "exited":
            return fields
        if fields.get("ActiveState") not in ("inactive", "activating") or time.monotonic() >= deadline:
            raise RuntimeError(f"The probe did not finish successfully: {fields}")
        time.sleep(0.1)


def capture(runtime, evidence, table):
    for source, suffix in (("probe.json", "json"), ("probe.stderr", "stderr")):
        path = evidence / f"{table}.{suffix}"
        path.write_bytes((runtime / source).read_bytes() if (runtime / source).exists() else b"")
        path.chmod(0o644)


def owned_probe(binary, evidence, table, cancellation):
    name = f"pryx-k31-address-{uuid.uuid4().hex}-{table}"
    unit = name + ".service"
    runtime = Path("/run") / name
    status = evidence / f"{table}.status"
    run_log = evidence / f"{table}.run.log"
    # A root service without DAC bypass must reach this marker without traversing runner home.
    # /tmp's sticky bit protects the uniquely created, caller-owned marker from other users.
    cancellation.lease = Path("/tmp") / f".{name}.admission"
    cancellation.lease.touch(mode=0o600, exist_ok=False)
    arguments = [
        "systemd-run", "--no-block", "--no-ask-password", f"--unit={unit}", "--service-type=oneshot", "--remain-after-exit",
        "--property=Delegate=yes", "--property=TimeoutStartSec=20s", "--property=TimeoutStopSec=2s",
        f"--property=ConditionPathExists={cancellation.lease}",
        "--property=CapabilityBoundingSet=~CAP_DAC_OVERRIDE CAP_DAC_READ_SEARCH",
        f"--property=RuntimeDirectory={name}", "--property=RuntimeDirectoryMode=0700",
        "--property=RuntimeDirectoryPreserve=yes",
        "--property=StandardOutput=journal",
        "--property=StandardError=journal",
    ]
    if table != "baseline":
        arguments.append(f"--property=InaccessiblePaths=/proc/net/{table}")
    arguments += [sys.executable, str(Path(__file__).resolve()), "--inside", binary, table, str(cancellation.lease), str(runtime)]
    failed = True
    cleanup = 0
    try:
        result = command(arguments, run_log, timeout=5, cancellation=cancellation)
        cancellation.check()
        if result:
            raise RuntimeError(f"{table}: systemd-run exited {result}")
        fields = wait_finished(unit, status, cancellation)
        cancellation.check()
        capture(runtime, evidence, table)
        for key, expected in {"ActiveState": "active", "SubState": "exited", "Result": "success", "ExecMainStatus": "0", "ConditionResult": "yes"}.items():
            if fields.get(key) != expected:
                raise RuntimeError(f"{table}: {key} was {fields.get(key)!r}, expected {expected!r}")
        report = json.loads((evidence / f"{table}.json").read_text())
        capability = report["capabilities"]["kernel-network-policy"]
        if table == "baseline":
            valid = capability["state"] == "on"
        else:
            valid = capability["state"] == "off" and table in capability["evidence"]
        if not valid:
            raise RuntimeError(f"{table}: unexpected kernel policy capability: {capability}")
        print(f"{table}: {json.dumps(capability)}", flush=True)
        failed = False
    finally:
        # Close late admission before killing a stuck start client or stopping the unit. A
        # StartTransientUnit request received after StopUnit then fails its systemd condition.
        cancellation.close_admission()
        try:
            cleanup = command(["systemctl", "stop", unit], evidence / f"{table}.cleanup", timeout=4)
            if status.exists() and "ActiveState=failed\n" in status.read_text():
                cleanup |= command(["systemctl", "reset-failed", unit], evidence / f"{table}.reset", timeout=2)
            capture(runtime, evidence, table)
            if failed and cancellation.requested is None:
                if not status.exists():
                    observed = unit_status(unit, status, cancellation)
                    if observed:
                        with status.open("a") as stream:
                            stream.write(f"Unit status query exited {observed}\n")
                command(["journalctl", f"--unit={unit}", "--no-pager", "--lines=30", "--output=short-monotonic"], evidence / f"{table}.journal", cancellation=cancellation)
                for suffix in ("run.log", "status", "stderr", "json", "journal"):
                    show(evidence / f"{table}.{suffix}")
            if cleanup:
                show(evidence / f"{table}.cleanup")
                raise RuntimeError(f"{table}: unit cleanup failed")
        finally:
            if runtime.exists():
                shutil.rmtree(runtime)


def main(arguments):
    if len(arguments) == 5 and arguments[0] == "--inside" and arguments[2] in ("baseline", "fib_trie", "if_inet6"):
        inside(arguments[1], arguments[2], arguments[3], arguments[4])
    elif len(arguments) == 2:
        binary = str(Path(arguments[0]).resolve(strict=True))
        evidence = Path(arguments[1]).resolve()
        evidence.mkdir(mode=0o755, parents=True, exist_ok=True)
        evidence.chmod(0o755)
        for table in ("baseline", "fib_trie", "if_inet6"):
            probe(binary, evidence, table)
    else:
        raise SystemExit("Usage: check-address-table-probes.py PRYXD EVIDENCE_DIRECTORY")


if __name__ == "__main__":
    main(sys.argv[1:])

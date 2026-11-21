# SPDX-License-Identifier: FSL-1.1-ALv2
"""Exercise the documented sudo identity transition inside a disposable, prebuilt stock image."""

import json
import os
from pathlib import Path
import subprocess
import tempfile


def exercise(image, repo, cli, public_key, base):
    uid = "57321"
    home = "/home/lab-operator"
    with tempfile.TemporaryDirectory() as directory:
        programs = Path(directory)
        programs.chmod(0o755)
        os.link(cli, programs / "pryx")
        subprocess.run(["bun", "build", "--compile", "experimental/install/credential-owner.ts",
                        "--define", "PRYX_PREVIEW_PUBLIC_KEY=" + json.dumps(public_key),
                        "--outfile", programs / "fixture"], cwd=repo, check=True)
        probe = programs / "bin"
        probe.mkdir()
        probe.chmod(0o755)
        (probe / "pryxd").write_text("#!/bin/sh\necho '{\"host\":\"stock-fixture\"}'\n")
        (probe / "pryxd").chmod(0o755)
        container = subprocess.check_output([
            "docker", "run", "-d", "--rm", "--pull=never", "--user", "0:0", "--network", "host",
            "--entrypoint", "/bin/sh", "-v", f"{programs}:/fixture:ro",
            "-e", "PRYX_RELEASE_BASE_URL=" + base, "-e", "PRYX_BUNDLE_DIR=/fixture",
            image, "-c", "exec sleep 180",
        ], text=True).strip()

        def run(args, user="0", failure=None):
            env = ["-e", "HOME=" + ("/root" if user == "0" else home), "-e", "SUDO_UID=" + uid,
                   "-e", "TMPDIR=/tmp/lab-untrusted"]
            result = subprocess.run(["docker", "exec", "--user", user, *env, container, *args],
                                    capture_output=True, text=True, timeout=30)
            if failure == "killed":
                assert result.returncode == 137, result.returncode
            elif failure:
                assert result.returncode and failure in result.stderr, (result.returncode, result.stderr[-512:])
            else:
                assert not result.returncode, (args, result.stderr[-512:])
            return result.stdout

        def temporary_parent_change_time():
            return run(["python3", "-I", "-c", "import os; print(os.stat('/tmp/lab-untrusted').st_ctime_ns)"])

        def report():
            return json.loads(run(["/fixture/pryx", "doctor", "--json"], uid))

        try:
            run(["sh", "-ec", 'useradd --create-home --uid "$1" --shell /bin/sh lab-operator; '
                 'mkdir -m 0777 /tmp/lab-untrusted; chown "$1" /tmp/lab-untrusted', "sh", uid])
            before = temporary_parent_change_time()
            run(["/fixture/fixture", "killed"], failure="killed")
            run(["/fixture/fixture", "failed"], failure="stock setup failed after its credential drop")
            assert "experimental" not in report()
            run(["/fixture/fixture", "ready"])
            assert report() == {"host": "stock-fixture", "experimental": {"self-check": {
                "on": True, "reason": "explicitly selected"}}}
            assert run(["/fixture/pryx", "experimental", "run", "self-check"], uid).strip() == "self-check passed"
            assert run(["/fixture/pryx", "config", "key"], uid).strip() == "disposable-fixture"
            assert run(["stat", "-c", "%u:%a", home + "/.config/pryx/client.json"]).strip() == uid + ":600"
            run(["/fixture/fixture", "failed"], failure="stock setup failed after its credential drop")
            assert report()["experimental"]["self-check"]["on"]
            run(["/fixture/fixture", "foreground"])
            assert "experimental" not in json.loads(run(["/fixture/pryx", "doctor", "--json"]))
            run(["chmod", "0777", "/tmp"])
            unavailable = report()
            assert unavailable["host"] == "stock-fixture" and not unavailable["experimental"]["installed"]
            assert "protected system temporary directory" in unavailable["experimental"]["reason"]
            run(["/fixture/pryx", "experimental", "run", "self-check"], uid,
                failure="protected system temporary directory")
            assert "experimental" not in json.loads(run(["/fixture/pryx", "doctor", "--json"]))
            run(["chmod", "1777", "/tmp"])
            assert report()["experimental"]["self-check"]["on"]
            assert temporary_parent_change_time() == before, "A temporary entry used an unprotected TMPDIR"
            for directory, name in [("/tmp/lab-untrusted", "*"), ("/tmp", "pryx-experimental-*")]:
                assert not run(["find", directory, "-mindepth", "1", "-maxdepth", "1", "-name", name])
            print("passed two-UID privilege drop, caller credentials and selection, rollback and staging cleanup")
        finally:
            subprocess.run(["docker", "rm", "-f", container], check=True, stdout=subprocess.DEVNULL)

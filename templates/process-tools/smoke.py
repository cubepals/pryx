#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Exercise actual relocated upstream interpreters over abstract Jupyter IPC as an ordinary user.

This is bundle acceptance, independent of the agent/edge confinement tests. It proves state
and PNG results with the real kernels, and never substitutes TCP or weakens browser policy.
"""
import base64
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import uuid

from jupyter_client import BlockingKernelClient


def execute(client, code):
    request = client.execute(code, allow_stdin=False)
    result = []
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        message = client.get_iopub_msg(timeout=30)
        if message.get("parent_header", {}).get("msg_id") != request:
            continue
        if message["msg_type"] == "error":
            raise AssertionError(message["content"])
        result.append(message)
        if message["msg_type"] == "status" and message["content"]["execution_state"] == "idle":
            break
    else:
        raise AssertionError("Kernel execution exceeded its deadline")
    while True:
        reply = client.get_shell_msg(timeout=30)
        if reply.get("parent_header", {}).get("msg_id") == request:
            assert reply["content"]["status"] == "ok", reply["content"]
            return result


def kernel(bundle, language):
    with tempfile.TemporaryDirectory(prefix=f"pryx-bundle-{language}-") as private:
        private = Path(private)
        info = dict(transport="ipc", ip=f"@pryx-{uuid.uuid4()}", shell_port=1, iopub_port=2,
                    stdin_port=3, control_port=4, hb_port=5, signature_scheme="hmac-sha256",
                    key=str(uuid.uuid4()))
        connection = private / "connection.json"
        connection.write_text(json.dumps(info))
        connection.chmod(0o600)
        environment = os.environ.copy()
        environment.update(IPYTHONDIR=str(private / "ipython"), MPLCONFIGDIR=str(private / "matplotlib"),
                           MPLBACKEND="Agg", HOME=str(private), JUPYTER_RUNTIME_DIR=str(private),
                           FONTCONFIG_FILE=str(bundle / "etc/fonts/fonts.conf"))
        if language == "python":
            command = [str(bundle / "bin/python3"), "-m", "ipykernel_launcher", "-f", str(connection),
                       "--config", str(bundle / "kernels/python/ipython/profile_default/ipython_config.py")]
        else:
            command = [str(bundle / "bin/node"), str(bundle / "kernels/javascript/kernel.cjs"), str(connection)]
            environment["NODE_PATH"] = str(bundle / "kernels/javascript/node_modules")
        with (private / "kernel.log").open("w+") as log:
            process = subprocess.Popen(command, env=environment, cwd=private, stdout=log, stderr=log, start_new_session=True)
            client = BlockingKernelClient()
            client.load_connection_info(info)
            client.start_channels()
            try:
                client.wait_for_ready(timeout=30)
                execute(client, "value = 40" if language == "python" else "var value = 40")
                messages = execute(client, "value + 2")
                assert any(m["content"].get("data", {}).get("text/plain") == "42" for m in messages)
                chart = ("import matplotlib.pyplot as plt; from IPython.display import display; "
                         "plt.plot([1,2,3]); display(plt.gcf()); plt.close()") if language == "python" else (
                         "var c=require('@napi-rs/canvas').createCanvas(32,32); "
                         "c.getContext('2d').fillRect(0,0,16,16); "
                         "$$.mime({'image/png':c.toBuffer('image/png').toString('base64')})")
                messages = execute(client, chart)
                images = [m["content"].get("data", {}).get("image/png") for m in messages]
                assert any(base64.b64decode(image).startswith(b"\x89PNG\r\n\x1a\n") for image in images if image)
                print(json.dumps(dict(language=language, uid=os.getuid(), transport="ipc", abstract=True,
                                      stateRetained=True, png=True)), flush=True)
            except Exception:
                log.seek(0)
                print(log.read(), file=sys.stderr)
                raise
            finally:
                client.stop_channels()
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=5)


if __name__ == "__main__":
    assert os.getuid() != 0, "Acceptance must run as an ordinary user"
    root = Path(sys.argv[1]).resolve()
    kernel(root, "python")
    kernel(root, "javascript")

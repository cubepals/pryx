# SPDX-License-Identifier: FSL-1.1-ALv2
"""Supervise the existing X11, window manager, VNC and noVNC programs as the sandbox user.
No protocol implementation or public listener lives here; agent expose owns browser access.
"""
import os
import pathlib
import secrets
import signal
import subprocess
import sys
import time

root = pathlib.Path(os.environ["PRYX_TOOL_DIRECTORY"])
width = int(os.environ.get("PRYX_DESKTOP_WIDTH", "1280"))
height = int(os.environ.get("PRYX_DESKTOP_HEIGHT", "720"))
if not (64 <= width <= 8192 and 64 <= height <= 8192):
    raise SystemExit("Desktop size must be between 64 and 8192 pixels")
subprocess.run(["xauth", "-f", os.environ["XAUTHORITY"], "add", os.environ["DISPLAY"], ".", secrets.token_hex(16)], check=True)
(root / "tokens").touch(mode=0o600)
children = []
def start(argv):
    process = subprocess.Popen(argv, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    children.append(process)
    return process

def stop(*_, failure=False):
    for child in children:
        if child.poll() is None:
            child.terminate()
    for child in children:
        try:
            child.wait(timeout=3)
        except subprocess.TimeoutExpired:
            child.kill()
    raise SystemExit(1 if failure else 0)

signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)
start(["Xvfb", os.environ["DISPLAY"], "-screen", "0", f"{width}x{height}x24", "-nolisten", "tcp", "-auth", os.environ["XAUTHORITY"], "-noreset"])
for _ in range(300):
    if subprocess.run(["xdotool", "getmouselocation"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0:
        break
    if children[0].poll() is not None:
        raise SystemExit("Xvfb exited before readiness")
    time.sleep(0.02)
else:
    raise SystemExit("Xvfb did not become ready")
start(["jwm", "-f", "/opt/pryx/desktop/jwm.xml"])
start(["x11vnc", "-display", os.environ["DISPLAY"], "-auth", os.environ["XAUTHORITY"], "-localhost", "-listen", "127.0.0.1", "-no6", "-rfbport", os.environ["PRYX_VNC_PORT"], "-forever", "-shared", "-nopw", "-quiet"])
start([sys.executable, "/opt/pryx/desktop/web.py", "--web", "/opt/pryx/novnc", "--token-plugin", "policy.ExpiringTokenFile", "--token-source", str(root / "tokens"), "127.0.0.1:" + os.environ["PRYX_DESKTOP_PORT"]])
while True:
    if any(child.poll() is not None for child in children):
        print("Desktop service exited",file=sys.stderr)
        stop(failure=True)
    time.sleep(0.1)

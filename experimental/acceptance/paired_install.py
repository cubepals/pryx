# SPDX-License-Identifier: FSL-1.1-ALv2
"""Compare the stock bootstrap requests in fresh homes against each commit's own release."""
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading


def install(release, home):
    requests = []

    class Record(SimpleHTTPRequestHandler):
        def do_GET(self):
            requests.append(self.path)
            super().do_GET()

        def log_message(self, *_):
            pass

    home.mkdir(mode=0o700)
    handler = partial(Record, directory=release)
    with ThreadingHTTPServer(("127.0.0.1", 0), handler) as server:
        address = f"http://127.0.0.1:{server.server_port}"
        thread = threading.Thread(target=server.serve_forever)
        thread.start()
        try:
            env = {"HOME": str(home), "PATH": os.environ["PATH"], "PRYX_PRESET": "user",
                   "PRYX_RELEASE_BASE_URL": address, "PRYX_NO_UP": "1"}
            subprocess.run(["bash", "-o", "pipefail", "-c",
                            'curl -fsS "$PRYX_RELEASE_BASE_URL/install.sh" | sh'],
                           env=env, timeout=300, check=True)
            if not (home / ".local/bin/pryx").is_file() or (home / ".pryx-experimental").exists():
                raise ValueError("default bootstrap installed unexpected state")
        finally:
            server.shutdown()
            thread.join()
    return requests


if __name__ == "__main__":
    if os.getuid() == 0:
        raise SystemExit("Use an ordinary runner account")
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        baseline, candidate = (install(Path(release).resolve(), root / name)
                               for release, name in zip(sys.argv[1:], ("before", "after"), strict=True))
        if not baseline or baseline != candidate:
            raise SystemExit(f"default request sequence differs: {baseline} versus {candidate}")
        print(f"matched default bootstrap requests {baseline}")

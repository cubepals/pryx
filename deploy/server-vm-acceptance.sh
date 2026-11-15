#!/bin/bash
# SPDX-License-Identifier: FSL-1.1-ALv2
# Run deploy/vm-installed-acceptance.ts as root against the server preset this disposable runner
# installed with sudo. The node's own policy admits the pinned VM images, as an operator edits
# /etc/pryxd/pryxd.toml; the node starts again and the acceptance waits for its first heartbeat
# after that. The credential is the one the install wrote for this user. Changes nothing else.
set -euo pipefail
: "${PRYX_PYTHON:?Name the Python that has the installed SDK}"
test "$(id -u)" -ne 0
failed() {
  echo 'Root VM acceptance failed; the node and VM worker journals end:' >&2
  sudo journalctl -u pryxd -u pryxd-vm-worker --no-pager -n 400 | tail -c 30000 >&2 || true
}
trap failed ERR
sudo systemctl stop pryxd
stopped=$(date +%s)
sudo python3 - templates/vm-tools/image-inputs.json <<'PY'
import json, pathlib, re, sys, tomllib
path = pathlib.Path('/etc/pryxd/pryxd.toml')
text = path.read_text()
allowed = tomllib.loads(text)['workloads']['allowed_images']
for image in json.loads(pathlib.Path(sys.argv[1]).read_text()):
    prefix = image['reference'].split('@', 1)[0] + '@'
    if prefix not in allowed:
        allowed.append(prefix)
line = 'allowed_images = ' + json.dumps(allowed)
text, count = re.subn(r'(?m)^allowed_images = .*$', lambda _: line, text)
assert count == 1, 'One allowed_images line, as pryxd join writes it'
assert tomllib.loads(text)['workloads']['allowed_images'] == allowed
path.write_text(text)
print('allowed_images=' + ','.join(allowed))
PY
sudo systemctl start pryxd
# Control places on what the node's last heartbeat announced; wait for one from this start.
sudo python3 - "$stopped" <<'PY'
import json, sys, time, urllib.request
token = next(json.loads(line.split('=', 1)[1]) for line in open('/etc/pryx/control.env')
             if line.startswith('PRYX_OPERATOR_TOKEN='))
stopped = int(sys.argv[1])
deadline = time.time() + 180
while time.time() < deadline:
    try:
        request = urllib.request.Request('http://127.0.0.1:8081/fleet/v1/nodes',
                                         headers={'authorization': f'Bearer {token}'})
        nodes = json.load(urllib.request.urlopen(request, timeout=5))['nodes']
        age = nodes[0]['lastHeartbeatSecondsAgo'] if len(nodes) == 1 else None
        if age is not None and nodes[0]['health'] == 'healthy' and time.time() - age > stopped + 1:
            print('node_heartbeat_after_start=true')
            break
    except OSError:
        pass
    time.sleep(1)
else:
    raise SystemExit('The node sent no heartbeat after it started again')
PY
PRYX_API_KEY=$(pryx config key)
PRYX_API_URL=$(python3 -c 'import json, pathlib; print(json.loads((pathlib.Path.home() / ".config/pryx/client.json").read_text())["url"])')
PRYX_VM_INSTALLATION=server
export PRYX_API_KEY PRYX_API_URL PRYX_VM_INSTALLATION
sudo --preserve-env=PRYX_VM_INSTALLATION,PRYX_API_URL,PRYX_API_KEY,PRYX_PYTHON \
  "$(command -v bun)" deploy/vm-installed-acceptance.ts
echo 'root_installed_vm_acceptance=passed'

#!/bin/sh
# SPDX-License-Identifier: FSL-1.1-ALv2
# Run inside a fresh ordinary account. Logs with the first key stay private and are never
# copied to CI output; failure diagnostics report only the last startup failure line.
set -eu
umask 077
export PATH="$HOME/.local/bin:$PATH"
export PRYX_RELEASE_BASE_URL=http://127.0.0.1:9876
cli="$HOME/.local/bin/pryx"
config="$HOME/.config/pryx"
if [ "$(id -u)" = 0 ]; then cli=/usr/local/bin/pryx; config=/etc/pryx-local; fi
started=$(date +%s)
installer_pid=
upgrade_pid=
phase=bootstrap
cleanup() {
  result=$?
  if [ "$result" != 0 ]; then
    echo "Acceptance failed during $phase" >&2
    if command -v journalctl >/dev/null; then
      journalctl --user-unit pryx.service --no-pager -n 60 > "$HOME/unit.log" 2>&1 || true
    fi
    python3 /local-diagnostics.py
  fi
  if [ -x "$cli" ]; then "$cli" down --purge > "$HOME/cleanup.log" 2>&1 || true; fi
  if [ -n "$installer_pid" ]; then kill "$installer_pid" 2>/dev/null || true; fi
  if [ -n "$upgrade_pid" ]; then kill "$upgrade_pid" 2>/dev/null || true; fi
}
trap cleanup EXIT HUP INT TERM
(curl -fsS "$PRYX_RELEASE_BASE_URL/install.sh" | sh) > "$HOME/install.log" 2>&1 &
installer_pid=$!
ready() {
  log=$1
  parent=$2
  deadline=$(($(date +%s) + 180))
  while [ "$(date +%s)" -lt "$deadline" ]; do
    if grep -q '"apiUrl"' "$log" && [ -x "$cli" ]; then return; fi
    if ! kill -0 "$parent" 2>/dev/null; then
      echo "The installer exited before authenticated readiness during $phase" >&2
      exit 1
    fi
    sleep 1
  done
  echo 'The installer did not reach authenticated readiness; private logs retained in the test account' >&2
  exit 1
}
ready "$HOME/install.log" "$installer_pid"
python3 - "$config/installation.json" <<'PY'
import json, os, pathlib, sys
r = json.loads(pathlib.Path(sys.argv[1]).read_text())
print('user_unit='+str(r['userUnit'])+' enabled_linger='+str(r['enabledLinger']))
if os.environ.get('PRYX_TEST_ENGINE_SOCKET'):
    import tomllib
    expected = os.environ['PRYX_TEST_ENGINE_SOCKET']
    assert r['engineSocket'] == expected
    configurations = [tomllib.loads(path.read_text()) for path in pathlib.Path(r['paths']['config']).glob('*.toml')]
    assert any(value.get('docker', {}).get('socket') == expected for value in configurations)
    print('selected_engine=retained_in_service_configuration')
if os.getuid() == 0:
    # PostgreSQL 17 file layout: postmaster.pid's first line identifies the server.
    pid = int((pathlib.Path(r['paths']['state'])/'postgres/data/postmaster.pid').read_text().splitlines()[0])
    uid = pathlib.Path('/proc/'+str(pid)).stat().st_uid
    metadata = json.loads((pathlib.Path(r['bundle'])/'bundle.json').read_text())
    assert uid == metadata['serviceUid'] and uid != 0
    print('embedded_postgres_uid=nonzero')
PY
if [ "${PRYX_EXPECT_WASM:-0}" = 1 ]; then
  python3 -c 'import json,pathlib; assert json.loads((pathlib.Path.home()/".config/pryx/installation.json").read_text())["tier"] == "wasm"'
fi
phase=hello
[ "$(pryx run -- echo hello)" = hello ]
[ "$(sh -lc 'pryx run -- echo hello')" = hello ]
echo "seconds_to_hello=$(($(date +%s)-started))"
if [ "${PRYX_EXPECT_WASM:-0}" != 1 ]; then
  # Where the host delegates memory and cpu, a process sandbox past its memory.max is OOM-killed
  # inside its own cgroup and the rest survives, and its cpu.max throttles it; where it doesn't,
  # the CPU ceiling is refused naming cpu-limit. PRYX_EXPECT_PROCESS_LIMITS=on requires the first.
  phase=process-limits
  printf '{"isolation":"process","resources":{"memoryMb":320,"cpuMillis":100}}\n' > "$HOME/limited.json"
  if pryx sandbox create --spec "$HOME/limited.json" > "$HOME/limited.out" 2>&1; then
    box=$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["id"])' "$HOME/limited.out")
    python3 - "$box" <<'PY'
import json, subprocess, sys, time
deadline = time.monotonic() + 60
while True:
    view = json.loads(subprocess.run(['pryx', 'sandbox', 'get', sys.argv[1]], check=True, capture_output=True).stdout)
    if view['guarantees']['memory-limit'] and view['guarantees']['cpu-limit']:
        break
    assert time.monotonic() < deadline, view['guarantees']
    time.sleep(1)
PY
    # Its own cgroup: the one pryx-<key> with a CPU ceiling (control sizes it in whole units).
    cgroup=$(python3 -c '
import os
read = lambda root, name: open(os.path.join(root, name)).read().split()
found = [root for root, _, files in os.walk("/sys/fs/cgroup") if os.path.basename(root).startswith("pryx-")
         and "cpu.max" in files and read(root, "cpu.max")[0] != "max"]
assert len(found) == 1, found
assert read(found[0], "memory.max")[0] != "max", read(found[0], "memory.max")
print(found[0])')
    quota=$(python3 -c 'import sys; q, p = open(sys.argv[1] + "/cpu.max").read().split(); print(int(q) / int(p))' "$cgroup")
    counter() { sed -n "s/^$2 //p" "$cgroup/$1"; }
    throttled=$(counter cpu.stat nr_throttled)
    share=$(pryx sandbox exec "$box" -- python3 -c 'import time
cpu, wall = time.process_time(), time.monotonic()
while time.monotonic() - wall < 3: pass
print(round((time.process_time() - cpu) / (time.monotonic() - wall), 3))')
    # Throttled to its ceiling, with room for the period's edges, and well short of a whole CPU.
    python3 -c 'import sys; share, quota = map(float, sys.argv[1:]); assert share < min(quota * 1.5, 0.9), (share, quota)' "$share" "$quota"
    [ "$(counter cpu.stat nr_throttled)" -gt "$throttled" ]
    # The kernel can kill the supervisor too. Measure CPU while the sandbox is live, then
    # verify OOM in its own leaf and exercise the installation again after deletion.
    oom=$(python3 "$(dirname "$0")/process_oom.py" "$cgroup" "$config/node.log" "$HOME/oom.out" \
      pryx sandbox exec "$box" -- python3 -c 'b = b"x" * (1024 << 20)')
    pryx sandbox rm "$box" > "$HOME/limited-deleted.json"
    [ "$(pryx run -- echo hello)" = hello ]
    echo "process_limits=on $oom cpu_max=$quota cpu_share=$share host=survived"
  else
    grep -q 'cpu-limit' "$HOME/limited.out"
    if [ "${PRYX_EXPECT_PROCESS_LIMITS:-}" = on ]; then
      echo 'Process limits were expected on this host' >&2
      tail -n 5 "$HOME/limited.out" >&2
      exit 1
    fi
    echo 'process_limits=off cpu_ceiling=refused'
  fi
fi
if [ "${PRYX_EXPECT_BOTH_TIERS:-0}" = 1 ]; then
  phase=both-tiers
  [ "$(pryx run --isolation process -- echo hello)" = hello ]
  [ "$(pryx run --isolation container --image alpine:3.22 -- echo hello)" = hello ]
  echo 'process=hello container=hello'
fi
if [ "${PRYX_EXPECT_CONTAINER:-}" = on ]; then
  phase=explicit-container
  box=$(pryx sandbox create --isolation container --image alpine:3.22 | python3 -c 'import json,sys;r=json.load(sys.stdin);assert r["isolation"] == "container";assert r["guarantees"]["private-pids"];assert not r["guarantees"]["memory-limit"];assert not r["guarantees"]["cpu-limit"];assert not r["guarantees"]["memory-sleep"];print(r["id"])')
  [ "$(pryx run --isolation container --image alpine:3.22 -- echo hello)" = hello ]
  pryx sandbox rm "$box" > "$HOME/deleted-image.json"
  [ "$(pryx run --isolation process --network none -- echo private-process)" = private-process ]
  echo 'rootless_container=running explicit_limits=off process_network_none=running'
elif [ "${PRYX_EXPECT_CONTAINER:-}" = off ]; then
  phase=explicit-container-refusal
  if pryx sandbox create --isolation container --image alpine:3.22 > "$HOME/refused-image.log" 2>&1; then
    echo 'Unsupported container mapping created a sandbox' >&2
    exit 1
  fi
  grep -Eiq 'container|isolation' "$HOME/refused-image.log"
  [ "$(pryx run --isolation process -- echo hello)" = hello ]
  [ "$(pryx run --isolation process --network none -- echo private-process)" = private-process ]
  echo 'rootless_container=refused process=running process_network_none=running'
fi
# Check never generates credentials, rewrites configuration or starts another supervisor.
find "$config" -type f ! -name '*.log' -exec sha256sum {} \; | sort > "$HOME/before-check"
pryx up --check > "$HOME/check.json"
find "$config" -type f ! -name '*.log' -exec sha256sum {} \; | sort > "$HOME/after-check"
python3 -c 'import filecmp,sys; assert filecmp.cmp(sys.argv[1],sys.argv[2],shallow=False)' "$HOME/before-check" "$HOME/after-check"
pryx up > "$HOME/repeated.log"
if grep -q '"apiKey"' "$HOME/repeated.log"; then echo 'Repeated the first key' >&2; exit 1; fi
phase=retained-workload
key_before=$(pryx config key | sha256sum)
if [ "${PRYX_EXPECT_CONTAINER:-}" = on ]; then
  box=$(pryx sandbox create --isolation container --image alpine:3.22 --idle 5m | python3 -c 'import json,sys;r=json.load(sys.stdin);assert r["isolation"] == "container";assert r["guarantees"]["private-pids"];print(r["id"])')
else
  box=$(pryx sandbox create --idle 5m | python3 -c 'import json,sys;print(json.load(sys.stdin)["id"])')
fi
printf 'state survives upgrade\n' > "$HOME/kept"
pryx sandbox cp "$HOME/kept" "$box:kept"
phase=upgrade
pryx upgrade > "$HOME/upgrade.log" 2>&1 &
upgrade_pid=$!
ready "$HOME/upgrade.log" "$upgrade_pid"
[ "$(pryx config key | sha256sum)" = "$key_before" ]
if grep -q '"apiKey"' "$HOME/upgrade.log"; then echo 'Upgrade repeated the first key' >&2; exit 1; fi
phase=restore
pryx sandbox start "$box" > "$HOME/restarted-box.json"
pryx sandbox cp "$box:kept" "$HOME/restored"
python3 -c 'import filecmp,sys; assert filecmp.cmp(sys.argv[1],sys.argv[2],shallow=False)' "$HOME/kept" "$HOME/restored"
[ "$(pryx run -- echo hello)" = hello ]
if [ "${PRYX_EXPECT_CONTAINER:-}" = on ]; then
  phase=engine-outage
  python3 "${PRYX_ENGINE_OUTAGE_PROBE:?}" "$cli" "$box"
  pryx sandbox cp "$box:kept" "$HOME/after-engine-outage"
  python3 -c 'import filecmp,sys; assert filecmp.cmp(sys.argv[1],sys.argv[2],shallow=False)' "$HOME/kept" "$HOME/after-engine-outage"
fi
# Leave the real retained workload for purge to stop, rather than deleting it through the API.
phase=purge
pryx down --purge > "$HOME/purged.json"
if [ "$(id -u)" = 0 ]; then
  for path in /etc/pryx-local /var/lib/pryx /opt/pryx/local /opt/pryx/cache /run/pryx-local /opt/pryx/cli /usr/local/bin/pryx /opt/pryx/operation.sqlite; do test ! -e "$path"; done
  test ! -e "$HOME/.config/pryx/client.json"
else
  for path in .config/pryx .local/share/pryx .local/state/pryx .cache/pryx .cache/pryx-runtime .local/share/pryx-cli .local/bin/pryx .pryx-operation.sqlite; do test ! -e "$HOME/$path"; done
fi
echo 'upgrade_state=preserved purge=complete'

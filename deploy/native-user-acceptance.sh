#!/bin/sh
# SPDX-License-Identifier: FSL-1.1-ALv2
# Run the one-command installer as a fresh ordinary account. Credentials stay in private
# logs and the installed CLI record; public observations contain no capability or API token.
set -eu
umask 077
test "$(id -u)" -ne 0
export PATH="$HOME/.local/bin:$PATH"
export PRYX_RELEASE_BASE_URL=http://127.0.0.1:9876
cli="$HOME/.local/bin/pryx"
installer=
cleanup() {
  result=$?
  trap - EXIT HUP INT TERM
  if [ "$result" != 0 ]; then python3 deploy/local-diagnostics.py || true; fi
  if [ -x "$cli" ]; then "$cli" down --purge > "$HOME/native-cleanup.log" 2>&1 || true; fi
  if [ -n "$installer" ]; then kill "$installer" 2>/dev/null || true; fi
  exit "$result"
}
trap cleanup EXIT HUP INT TERM
(curl -fsS "$PRYX_RELEASE_BASE_URL/install.sh" | sh) > "$HOME/install.log" 2>&1 &
installer=$!
deadline=$(($(date +%s) + 180))
until grep -q '"apiUrl"' "$HOME/install.log" && [ -x "$cli" ]; do
  if ! kill -0 "$installer" 2>/dev/null || [ "$(date +%s)" -ge "$deadline" ]; then
    echo 'Native user installation did not reach authenticated readiness; private account logs retained' >&2
    exit 1
  fi
  sleep 1
done
python3 - <<'PY'
import json, pathlib
record = json.loads((pathlib.Path.home()/'.config/pryx/installation.json').read_text())
assert record['preset'] == 'user' and record['tier'] == 'process'
assert not record.get('engineSocket'), 'The ordinary tool fixture must have no engine'
descriptor = pathlib.Path(record['bundle'])/'bin/process-tools.json'
assert descriptor.is_file()
print('installed_preset=user tier=process engine=none protected_descriptor=present')
PY
"${PRYX_TEST_BUN:?}" deploy/native-tools-acceptance.ts
"$cli" down --purge > "$HOME/native-purge.log" 2>&1
test ! -e "$HOME/.config/pryx"
test ! -e "$HOME/.local/share/pryx"
echo 'native_installed_acceptance=passed purge=complete'

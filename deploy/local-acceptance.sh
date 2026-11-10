#!/bin/sh
# SPDX-License-Identifier: FSL-1.1-ALv2
# Run inside a fresh ordinary account. Logs with the first key stay private and are never
# copied to CI output; failure diagnostics report only the last startup failure line.
set -eu
umask 077
export PATH="$HOME/.local/bin:$PATH"
export PRYX_RELEASE_BASE_URL=http://127.0.0.1:9876
started=$(date +%s)
installer_pid=
upgrade_pid=
cleanup() {
  if [ -x "$HOME/.local/bin/pryx" ]; then pryx down --purge > "$HOME/cleanup.log" 2>&1 || true; fi
  if [ -n "$installer_pid" ]; then kill "$installer_pid" 2>/dev/null || true; fi
  if [ -n "$upgrade_pid" ]; then kill "$upgrade_pid" 2>/dev/null || true; fi
}
trap cleanup EXIT HUP INT TERM
(curl -fsS "$PRYX_RELEASE_BASE_URL/install.sh" | sh) > "$HOME/install.log" 2>&1 &
installer_pid=$!
ready() {
  log=$1
  deadline=$(($(date +%s) + 180))
  while [ "$(date +%s)" -lt "$deadline" ]; do
    if grep -q '"apiUrl"' "$log" && [ -x "$HOME/.local/bin/pryx" ]; then return; fi
    sleep 1
  done
  echo 'The installer did not reach authenticated readiness; private logs retained in the test account' >&2
  exit 1
}
ready "$HOME/install.log"
[ "$(pryx run -- echo hello)" = hello ]
echo "seconds_to_hello=$(($(date +%s)-started))"
# Check never generates credentials, rewrites configuration or starts another supervisor.
find "$HOME/.config/pryx" -type f -exec sha256sum {} \; | sort > "$HOME/before-check"
pryx up --check > "$HOME/check.json"
find "$HOME/.config/pryx" -type f -exec sha256sum {} \; | sort > "$HOME/after-check"
cmp "$HOME/before-check" "$HOME/after-check"
pryx up > "$HOME/repeated.log"
if grep -q '"apiKey"' "$HOME/repeated.log"; then echo 'Repeated the first key' >&2; exit 1; fi
key_before=$(pryx config key | sha256sum)
box=$(pryx sandbox create --idle 5m | python3 -c 'import json,sys;print(json.load(sys.stdin)["id"])')
printf 'state survives upgrade\n' > "$HOME/kept"
pryx sandbox cp "$HOME/kept" "$box:/kept"
pryx upgrade > "$HOME/upgrade.log" 2>&1 &
upgrade_pid=$!
ready "$HOME/upgrade.log"
[ "$(pryx config key | sha256sum)" = "$key_before" ]
if grep -q '"apiKey"' "$HOME/upgrade.log"; then echo 'Upgrade repeated the first key' >&2; exit 1; fi
pryx sandbox cp "$box:/kept" "$HOME/restored"
cmp "$HOME/kept" "$HOME/restored"
[ "$(pryx run -- echo hello)" = hello ]
# Leave the real retained workload for purge to stop, rather than deleting it through the API.
pryx down --purge > "$HOME/purged.json"
for path in .config/pryx .local/share/pryx .local/state/pryx .cache/pryx .cache/pryx-runtime/pryx .local/share/pryx-cli .local/bin/pryx .pryx-operation.sqlite; do
  test ! -e "$HOME/$path"
done
echo 'upgrade_state=preserved purge=complete'

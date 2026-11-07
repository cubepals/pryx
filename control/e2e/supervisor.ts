// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** The test process supervisor used to exercise daemon upgrades and rollback. */

/**
 * systemd's part, as the unit sets it: pryxd run from the state directory after its
 * ExecStartPre, and after each stop the unit's ExecStopPost, then a start again. ExecStopPost is
 * told what systemd tells it: SERVICE_RESULT always, EXIT_CODE and EXIT_STATUS only once the main
 * process ran. Its first run reports 0.0.1 (a debug build's PRYXD_TEST_VERSION), so the node is
 * older than the pryxd it is offered. `$2` is the test's release key, which a debug build takes
 * as PRYXD_TEST_RELEASE_KEY besides the keys compiled into it.
 */
export const SUPERVISOR = [
  'v=0.0.1',
  'while :; do',
  '  if "$0" doctor --preflight --config "$1"; then',
  '    PRYXD_TEST_RELEASE_KEY="$2" PRYXD_TEST_VERSION=$v "$0" serve --config "$1" --log-format pretty; s=$?; v=',
  '    r=success; [ $s = 0 ] || r=exit-code',
  '    SERVICE_RESULT=$r EXIT_CODE=exited EXIT_STATUS=$s "$0.prev" upgrade --exited --config "$1"',
  '  else',
  '    SERVICE_RESULT=exit-code "$0.prev" upgrade --exited --config "$1"',
  '  fi',
  '  sleep 1',
  'done',
].join('\n')

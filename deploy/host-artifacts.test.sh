#!/bin/bash
# SPDX-License-Identifier: FSL-1.1-ALv2
# Real checksums and files verify retries of missing, incomplete and corrupted artifact reads.
# The CLI fixture performs no network access and never runs an installer.
set -euo pipefail
source deploy/host-artifacts.sh
pryx_artifact_fixture=$(mktemp -d)
trap 'rm -rf -- "$pryx_artifact_fixture"' EXIT
export PRYX_ARTIFACT_FIXTURE="$pryx_artifact_fixture" GITHUB_RUN_ID=1 GITHUB_REPOSITORY=fixture/repo
mkdir "$pryx_artifact_fixture/source" "$pryx_artifact_fixture/bin" "$pryx_artifact_fixture/work"
printf alpha > "$pryx_artifact_fixture/source/part0"
printf beta > "$pryx_artifact_fixture/source/part1"
printf gamma > "$pryx_artifact_fixture/source/part2"
(cd "$pryx_artifact_fixture/source" && sha256sum part? > PARTS.sha256)
cat "$pryx_artifact_fixture"/source/part? > "$pryx_artifact_fixture/source/host-transport.tar"
(cd "$pryx_artifact_fixture/source" && sha256sum host-transport.tar > TRANSPORT.sha256)
pryx_manifest_digest=$(sha256sum "$pryx_artifact_fixture/source/PARTS.sha256" | cut -d ' ' -f1)
pryx_transport_digest=$(sha256sum "$pryx_artifact_fixture/source/host-transport.tar" | cut -d ' ' -f1)
cat > "$pryx_artifact_fixture/bin/gh" <<'PY'
#!/usr/bin/env python3
import os, pathlib, shutil, sys
root = pathlib.Path(os.environ['PRYX_ARTIFACT_FIXTURE'])
args = sys.argv
piece = args[args.index('--name')+1][-1]
target = pathlib.Path(args[args.index('--dir')+1])
counter = root/('attempt-'+piece)
attempt = int(counter.read_text())+1 if counter.exists() else 1
counter.write_text(str(attempt))
if piece == '1' and attempt < 3:
    print('Artifact not yet available', file=sys.stderr)
    sys.exit(1)
for name in ['PARTS.sha256', 'TRANSPORT.sha256']:
    shutil.copyfile(root/'source'/name, target/name)
if os.environ.get('PRYX_ARTIFACT_BAD') or (piece == '2' and attempt == 1):
    (target/'TRANSPORT.sha256').write_text('unverified transport\n')
if piece == '0' and attempt == 1:
    sys.exit(0)  # A successful CLI exit with an incomplete extraction must not pass.
shutil.copyfile(root/'source'/('part'+piece), target/('part'+piece))
if piece == '0' and attempt == 2:
    (target/('part'+piece)).write_text('corrupt part')
PY
cat > "$pryx_artifact_fixture/bin/sleep" <<'SH'
#!/bin/sh
printf '%s\n' "$1" >> "$PRYX_ARTIFACT_FIXTURE/delays"
SH
chmod 700 "$pryx_artifact_fixture/bin/gh" "$pryx_artifact_fixture/bin/sleep"
export PATH="$pryx_artifact_fixture/bin:$PATH"
download_host_parts "$pryx_artifact_fixture/bin/gh" 3 "$pryx_manifest_digest" "$pryx_transport_digest" \
  "$pryx_artifact_fixture/work" "$pryx_artifact_fixture/parts" > "$pryx_artifact_fixture/success.log" 2>&1
test "$(cat "$pryx_artifact_fixture/attempt-0")" = 3
test "$(cat "$pryx_artifact_fixture/attempt-1")" = 3
test "$(cat "$pryx_artifact_fixture/attempt-2")" = 2
test "$(wc -l < "$pryx_artifact_fixture/delays")" = 5
(cd "$pryx_artifact_fixture/parts" && sha256sum -c PARTS.sha256)
cat "$pryx_artifact_fixture"/parts/part? > "$pryx_artifact_fixture/reassembled"
printf '%s  %s\n' "$pryx_transport_digest" "$pryx_artifact_fixture/reassembled" | sha256sum -c -
mkdir "$pryx_artifact_fixture/bad-work"
if PRYX_ARTIFACT_BAD=1 download_host_parts "$pryx_artifact_fixture/bin/gh" 3 \
    "$pryx_manifest_digest" "$pryx_transport_digest" "$pryx_artifact_fixture/bad-work" \
    "$pryx_artifact_fixture/rejected" > "$pryx_artifact_fixture/failure.log" 2>&1; then
  echo 'An unverified artifact was accepted' >&2; exit 1
fi
test ! -e "$pryx_artifact_fixture/rejected"
test "$(cat "$pryx_artifact_fixture/attempt-0")" = 9
echo 'Missing, partial and corrupt reads retry; an unauthenticated transport remains refused'

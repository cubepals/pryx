#!/bin/sh
# SPDX-License-Identifier: FSL-1.1-ALv2
# Exercise the real relocated programs and PostgreSQL under the default container profile.
# Installer/preset acceptance extends this with service startup, hello, upgrade and removal.
set -eu
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
chmod 0755 "$temporary"
mkdir "$temporary/bundle"
tar -xzf /artifacts/pryx-bundle-linux-x64.tar.gz -C "$temporary/bundle"
bundle="$temporary/bundle"
"$bundle/bin/pryx" --version
"$bundle/bin/pryxd" --version
"$bundle/bin/pryx-agent" --version
"$bundle/bin/pryx-control" --version
"$bundle/bin/pryx-edge" --version
"$bundle/bin/pryx-image" --version
"$bundle/bin/setpriv" --reuid=1001 --regid=1001 --clear-groups "$bundle/bin/pryx" --version
mkdir "$temporary/bootstrap"
tar -xzf /artifacts/pryx-bootstrap-linux-x64.tar.gz -C "$temporary/bootstrap"
if [ -f "$bundle/bin/process-tools.json" ]; then
  cmp "$bundle/bin/process-tools.json" "$temporary/bootstrap/bin/process-tools.json"
  # Bun's documented standalone CLI mode reuses the shipped runtime on minimal hosts.
  BUN_BE_BUN=1 "$bundle/bin/pryx" --eval '
    const assert = require("node:assert/strict");
    const fs = require("node:fs");
    const path = process.argv.at(-1);
    const info = fs.lstatSync(path);
    assert(info.isFile() && info.size <= 4096);
    assert.equal(info.mode & 0o022, 0);
    const descriptor = JSON.parse(fs.readFileSync(path, "utf8"));
    assert(descriptor.url.startsWith("https://"));
    for (const name of ["sha256", "manifest_sha256"])
      assert(/^[a-f0-9]{64}$/.test(descriptor[name]));
    console.log("native_descriptor=retained_in_base_and_bootstrap");
  ' "$bundle/bin/process-tools.json"
fi
"$bundle/bin/setpriv" --reuid=1001 --regid=1001 --clear-groups "$temporary/bootstrap/bin/pryxd" probe --json > "$temporary/bootstrap-probe.json"
"$bundle/bin/setpriv" --reuid=1001 --regid=1001 --clear-groups "$temporary/bootstrap/bin/pryx-agent" process-probe > "$temporary/agent-probe.json"
"$bundle/bin/setpriv" --reuid=1001 --regid=1001 --clear-groups "$temporary/bootstrap/bin/pryx" --version
if "$bundle/postgres/bin/initdb" -D "$temporary/root-data" > "$temporary/root-refusal" 2>&1; then
  echo 'PostgreSQL unexpectedly initialized as uid 0' >&2
  exit 1
fi
test ! -e "$temporary/root-data"
mkdir "$temporary/postgres"
printf 'pryx:x:1001:1001:pryx:%s:/bin/sh\n' "$temporary/postgres" > "$temporary/passwd"
printf 'pryx:x:1001:\n' > "$temporary/group"
export NSS_WRAPPER_PASSWD="$temporary/passwd" NSS_WRAPPER_GROUP="$temporary/group"
chown 1001:1001 "$temporary/postgres"
# This private disposable database tests relocation and uid dropping. The supervisor's
# production initialization continues to require SCRAM authentication on every transport.
"$bundle/bin/setpriv" --reuid=1001 --regid=1001 --clear-groups \
  "$bundle/postgres/bin/initdb" -D "$temporary/postgres/data" -U pryx --auth=trust --locale=C --encoding=UTF8
"$bundle/bin/setpriv" --reuid=1001 --regid=1001 --clear-groups \
  "$bundle/postgres/bin/pg_ctl" -D "$temporary/postgres/data" -l "$temporary/postgres/server.log" \
  -o "-h '' -k '$temporary/postgres'" -w start
query=$("$bundle/bin/setpriv" --reuid=1001 --regid=1001 --clear-groups \
  "$bundle/postgres/bin/psql" -h "$temporary/postgres" -U pryx -d postgres -Atc 'SELECT 1')
test "$query" = 1
"$bundle/bin/setpriv" --reuid=1001 --regid=1001 --clear-groups \
  "$bundle/postgres/bin/pg_ctl" -D "$temporary/postgres/data" -m fast -w stop

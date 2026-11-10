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

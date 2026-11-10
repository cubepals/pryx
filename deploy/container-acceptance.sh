#!/bin/sh
# SPDX-License-Identifier: FSL-1.1-ALv2
# Test preparation runs as root; every installer and sandbox command runs as uid 1001.
set -eu
if command -v apt-get >/dev/null; then
  apt-get update -qq
  DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends curl ca-certificates python3 passwd tar
elif command -v apk >/dev/null; then
  apk add --no-cache curl ca-certificates python3 shadow tar
elif command -v dnf >/dev/null; then
  dnf install -y python3 shadow-utils tar gzip
  command -v curl >/dev/null || dnf install -y curl-minimal
else
  echo 'Test preparation needs a supported distribution package manager' >&2; exit 1
fi
useradd -m -u 1001 -s /bin/sh pryxtest
if [ "${PRYX_PREPARE_ONLY:-0}" = 1 ]; then exit 0; fi
python3 -m http.server 9876 --bind 127.0.0.1 --directory /artifacts > /tmp/artifact-server.log 2>&1 &
server=$!
trap 'kill "$server"' EXIT HUP INT TERM
su -s /bin/sh pryxtest -c 'sh /local-acceptance.sh'

#!/bin/sh
# SPDX-License-Identifier: FSL-1.1-ALv2
# Test preparation runs as root; every installer and sandbox command runs as uid 1001.
set -eu
if command -v apt-get >/dev/null; then
  pryx_apt_ca=${PRYX_TEST_APT_CA:-/etc/ssl/certs/ca-certificates.crt}
  pryx_ubuntu_sources=/etc/apt/sources.list.d/ubuntu.sources
  if sh -c '. /etc/os-release; [ "$ID" = ubuntu ]'; then
    pryx_ubuntu_mirror=${PRYX_TEST_UBUNTU_MIRROR:?The Ubuntu fixture needs its archive mirror}
    grep -q '^URIs:' "$pryx_ubuntu_sources"
    sed -i "s|^URIs:.*|URIs: $pryx_ubuntu_mirror|" "$pryx_ubuntu_sources"
  fi
  # The minimal Ubuntu image bootstraps its own CA package using the read-only runner roots.
  # TLS, Ubuntu release signatures and package digest verification all remain mandatory.
  apt-get update -qq -o Acquire::Retries=3 -o APT::Update::Error-Mode=any -o "Acquire::https::CaInfo=$pryx_apt_ca"
  DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends -o "Acquire::https::CaInfo=$pryx_apt_ca" curl ca-certificates python3 passwd tar
elif command -v apk >/dev/null; then
  apk add --no-cache curl ca-certificates python3 shadow tar
elif command -v dnf >/dev/null; then
  dnf install -y python3 shadow-utils util-linux tar gzip
  command -v curl >/dev/null || dnf install -y curl-minimal
else
  echo 'Test preparation needs a supported distribution package manager' >&2; exit 1
fi
useradd -m -u 1001 -s /bin/sh pryxtest
if [ "${PRYX_PREPARE_ONLY:-0}" = 1 ]; then exit 0; fi
python3 -m http.server 9876 --bind 127.0.0.1 --directory /artifacts > /tmp/artifact-server.log 2>&1 &
server=$!
trap 'kill "$server"' EXIT HUP INT TERM
curl --retry 5 --retry-connrefused --retry-delay 1 --max-time 10 -fsS http://127.0.0.1:9876/install.sh -o /dev/null
if [ "${PRYX_TEST_ROOT_MODE:-0}" = 1 ]; then
  sh /local-acceptance.sh
elif [ "${PRYX_UNSUPPORTED:-0}" = 1 ]; then
  su -s /bin/sh pryxtest -c 'python3 /unsupported-acceptance.py'
else
  su -s /bin/sh pryxtest -c 'sh /local-acceptance.sh'
fi

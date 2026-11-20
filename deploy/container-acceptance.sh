#!/bin/sh
# SPDX-License-Identifier: FSL-1.1-ALv2
# Test preparation runs as root; every installer and sandbox command runs as uid 1001.
set -eu
rewrite_ubuntu_sources() {
  pryx_source_root=$1
  pryx_source_mirror=$2
  pryx_sources_found=0
  for pryx_source in "$pryx_source_root/etc/apt/sources.list" \
    "$pryx_source_root"/etc/apt/sources.list.d/*.list \
    "$pryx_source_root"/etc/apt/sources.list.d/*.sources; do
    [ -f "$pryx_source" ] || continue
    case "$pryx_source" in
      *.sources)
        if grep -q '^URIs:' "$pryx_source"; then
          sed -i "s|^URIs:.*|URIs: $pryx_source_mirror|" "$pryx_source" || return 1
          pryx_sources_found=1
        fi ;;
      *)
        if grep -Eq '^deb(-src)?[[:space:]]' "$pryx_source"; then
          sed -Ei "s|^(deb(-src)?[[:space:]]+(\[[^]]*\][[:space:]]+)?)[^[:space:]]+|\1$pryx_source_mirror|" "$pryx_source" || return 1
          pryx_sources_found=1
        fi ;;
    esac
  done
  if [ "$pryx_sources_found" != 1 ]; then
    echo 'The Ubuntu fixture has no recognized signed archive sources' >&2
    return 1
  fi
}
# The minimal Ubuntu image uses the read-only runner roots until ca-certificates is installed.
fixture_apt() {
  DEBIAN_FRONTEND=noninteractive apt-get "$@" -o Acquire::ForceIPv4=true -o Acquire::Retries=3 \
    -o "Acquire::https::CaInfo=${PRYX_TEST_APT_CA:-/etc/ssl/certs/ca-certificates.crt}"
}
install_apt_prerequisites() {
  fixture_apt update -qq -o APT::Update::Error-Mode=any || return 1
  fixture_apt install -y -qq --no-install-recommends curl ca-certificates python3 passwd tar || return 1
  if [ "${PRYX_PREPARE_ONLY:-0}" = 1 ]; then
    fixture_apt install -y dbus-user-session libpam-systemd polkitd
  fi
}
install_ubuntu_prerequisites() {
  for pryx_ubuntu_mirror in ${PRYX_TEST_UBUNTU_MIRRORS:?The Ubuntu fixture needs its archive mirrors}; do
    rewrite_ubuntu_sources "$1" "$pryx_ubuntu_mirror" || return 1
    if install_apt_prerequisites; then return 0; fi
  done
  echo 'No Ubuntu mirror installed the test prerequisites' >&2
  return 1
}
if command -v apt-get >/dev/null; then
  if sh -c '. /etc/os-release; [ "$ID" = ubuntu ]'; then
    install_ubuntu_prerequisites ""
  else
    install_apt_prerequisites
  fi
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

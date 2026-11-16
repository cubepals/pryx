#!/bin/bash
# SPDX-License-Identifier: FSL-1.1-ALv2
# Installs gVisor's runsc at the release deploy/gvisor.json pins, checked against its SHA-512, and
# registers it with Docker under the name runsc, through runsc's own installer. Live engines
# reload their configuration; idle engines restart. Immutable version directories retain old
# executables and sidecars for running sandboxes. No sidecar is downloaded by installation.
set -euo pipefail
pin=${PRYX_GVISOR_PIN:-$(dirname "$0")/gvisor.json}
root=${PRYX_GVISOR_PREFIX:-/usr/local/lib/pryx/gvisor}
version=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["version"])' "$pin")
prefix="$root/$version"
[ "$(id -u)" = 0 ] || { echo 'gvisor.sh: run as root' >&2; exit 1; }
[ "$(uname -m)" = x86_64 ] || { echo 'gvisor.sh: the pinned release is x86_64' >&2; exit 1; }
url=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["url"])' "$pin")
digest=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["sha512"])' "$pin")
if [ ! -x "$prefix/runsc" ] || ! "$prefix/runsc" --version | grep -Fq "runsc version $version"; then
  work=$(mktemp -d)
  trap 'rm -rf -- "$work"' EXIT
  for wait in 2 5 10 20 0; do
    curl -fsSL "$url" -o "$work/gvisor.tar.zstd" && break
    [ "$wait" = 0 ] && { echo "gvisor.sh: cannot download $url" >&2; exit 1; }
    sleep "$wait"
  done
  printf '%s  %s\n' "$digest" "$work/gvisor.tar.zstd" | sha512sum -c --quiet -
  mkdir -m 755 "$work/unpacked"
  zstd -dc "$work/gvisor.tar.zstd" | tar -x -C "$work/unpacked" --no-same-owner
  install -d -m 755 "$(dirname "$prefix")"
  rm -rf -- "$prefix.new"
  mv "$work/unpacked" "$prefix.new"
  chown -R root:root "$prefix.new"
  chmod -R go-w "$prefix.new"
  rm -rf -- "$prefix.old"
  if [ -e "$prefix" ]; then mv "$prefix" "$prefix.old"; fi
  mv "$prefix.new" "$prefix"
  rm -rf -- "$prefix.old"
fi
"$prefix/runsc" --version
"$prefix/runsc" install --download-sidecars=NEVER --require-sidecars=ALWAYS
if [ -n "$(docker ps -q)" ]; then
  systemctl reload docker
else
  systemctl restart docker
fi
for _ in $(seq 60); do
  docker info --format '{{json .Runtimes}}' 2>/dev/null |
    python3 -c 'import json,sys; sys.exit(json.load(sys.stdin).get("runsc", {}).get("path") != sys.argv[1])' \
      "$prefix/runsc" 2>/dev/null && exit 0
  sleep 1
done
echo 'gvisor.sh: Docker does not list the pinned runsc after its reload or restart' >&2
exit 1

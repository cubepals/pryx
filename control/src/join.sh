#!/bin/sh

# SPDX-FileCopyrightText: 2026 The Cubepals Authors
#
# SPDX-License-Identifier: FSL-1.1-ALv2

# Makes this host a node of a Blockly fleet (docs/fleet-operations.md, "Adding a node"). The node
# endpoint serves this script at /fleet/v1/join.sh, with daemon.json written into it. The line
# `bun scripts/fleet.ts token` prints fetches it only once the fleet CA matches the token's hash,
# and runs it as root:
#
#   sh join.sh <bk1. token> <the fleet CA it checked> [pryxd join's options, such as --address <ip>]
#
# It installs Docker from Docker's own apt repository if it is missing, and nftables, which
# pryxd's unit needs before every start to keep sandboxs off the host and off each other's ports
# (`pryxd isolate`), gives Docker pryxd's
# daemon.json if it has none, installs the pryxd the control plane serves once its sha256
# matches (at /var/lib/pryxd/bin/pryxd, linked from /usr/local/bin), and runs `pryxd join`,
# which writes the configuration and starts the service. On a host that is already the node a
# `--node` token names, `pryxd join` enrolls it again (docs/fleet-operations.md §8).
set -eu

token=${1:?usage: join.sh <token> <fleet CA file> [--address <ip>]}
ca=${2:?usage: join.sh <token> <fleet CA file> [--address <ip>]}
shift 2
say() { printf 'join: %s\n' "$*"; }
fail() {
  printf 'join: %s\n' "$*" >&2
  exit 1
}

@ROOT_CHECK@

# The endpoint the token names: its payload is base64url JSON, with the URL as "u".
payload=$(printf '%s' "${token#bk1.}" | tr '_-' '/+')
case $((${#payload} % 4)) in
2) payload="$payload==" ;;
3) payload="$payload=" ;;
esac
url=$(printf '%s' "$payload" | base64 -d 2>/dev/null | sed -n 's/.*"u":"\([^"]*\)".*/\1/p')
[ -n "$url" ] || fail "the token names no control plane: paste the whole line fleet.ts token printed"

@PREPARE@

if ! command -v docker >/dev/null 2>&1; then
  command -v apt-get >/dev/null 2>&1 || fail "Docker is missing, and only Debian and Ubuntu are set up here: install Docker Engine, then paste the line again"
  . /etc/os-release
  codename=${VERSION_CODENAME:-}
  [ -n "$codename" ] || fail "can't tell this ${ID:-system}'s release: install Docker Engine, then paste the line again"
  say "installing Docker from Docker's apt repository"
  apt-get update -q
  apt-get install -yq ca-certificates curl
  install -m 0755 -d /etc/apt/keyrings
  curl -fsSL "https://download.docker.com/linux/$ID/gpg" -o /etc/apt/keyrings/docker.asc
  chmod a+r /etc/apt/keyrings/docker.asc
  echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/$ID $codename stable" \
    >/etc/apt/sources.list.d/docker.list
  apt-get update -q
  apt-get install -yq docker-ce docker-ce-cli containerd.io
fi

if ! command -v nft >/dev/null 2>&1; then
  command -v apt-get >/dev/null 2>&1 || fail "nft is missing, and only Debian and Ubuntu are set up here: install nftables, then paste the line again"
  say "installing nftables"
  apt-get update -q
  apt-get install -yq nftables
fi

if [ ! -e /etc/docker/daemon.json ]; then
  say "giving Docker pryxd's daemon.json"
  mkdir -p /etc/docker
  cat >/etc/docker/daemon.json <<'DAEMON_JSON'
@DAEMON_JSON@
DAEMON_JSON
  # A restart would stop whatever already runs here; a reload applies live-restore and stops nothing.
  if [ -z "$(docker ps -q 2>/dev/null)" ]; then
    systemctl restart docker
  else
    systemctl reload docker
    say "Docker has running containers, so it was reloaded, not restarted: live-restore is on now, and the rest of daemon.json applies at Docker's next restart"
  fi
fi

@DAEMON_FETCH@
# Under the state directory, which is all the service may write, so pryxd can upgrade itself.
install -d -m 0700 /var/lib/pryxd
install -D -m 0755 "$dir/pryxd" /var/lib/pryxd/bin/pryxd
ln -sfn /var/lib/pryxd/bin/pryxd /usr/local/bin/pryxd
say "installed $(/usr/local/bin/pryxd --version)"
@AGENT_INSTALL@
/usr/local/bin/pryxd join "$token" "$@"

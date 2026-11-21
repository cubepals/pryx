#!/bin/sh
# SPDX-License-Identifier: FSL-1.1-ALv2
# Verify a release bootstrap, preflight without persistent changes, then install and start.
# The ordinary-user path never invokes sudo or writes outside HOME.
set -eu
# The release's name for this machine: x86_64's files keep their plain names, others carry it.
case "$(uname -s)/$(uname -m)" in
  Linux/x86_64) arch=x64 cli=pryx ;;
  Linux/aarch64|Linux/arm64) arch=arm64 cli=pryx-linux-arm64 ;;
  *) echo 'pryx supports Linux on x86_64 and aarch64; this platform is not supported' >&2; exit 1 ;;
esac
for tool in curl sha256sum tar; do
  command -v "$tool" >/dev/null || { echo "pryx installation needs $tool" >&2; exit 1; }
done
umask 077
base=${PRYX_RELEASE_BASE_URL:-https://github.com/cubepals/pryx/releases/download/v__PRYX_VERSION__}
case "$base" in
  http://127.0.0.1:*)
    authority=${base#http://127.0.0.1:}
    port=${authority%%/*}
    case "$port" in ''|*[!0-9]*) echo 'Loopback HTTP release URL needs a numeric port' >&2; exit 1;; esac
    ;;
  https://*) ;;
  *) echo 'The release URL must use HTTPS (or loopback HTTP for build artifacts)' >&2; exit 1 ;;
esac
temporary=$(mktemp -d "$HOME/.pryx-bootstrap.XXXXXX")
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
download() {
  case "$base" in
    https://*) curl --proto '=https' --proto-redir '=https' --retry 5 --retry-all-errors --connect-timeout 15 --max-time 300 -fsSL "$base/$1" -o "$temporary/$1" ;;
    http://127.0.0.1:*) curl --proto '=http' --connect-timeout 15 --max-time 300 -fsS "$base/$1" -o "$temporary/$1" ;;
  esac
}
verify() {
  if ! awk -v name="$1" '$2 == name || $2 == "*" name { print $1 "  " name; count++ } END { if (count != 1) exit 1 }' "$temporary/SHA256SUMS" > "$temporary/asset.sha256"; then
    echo 'Release checksum manifest has no unique entry for the bootstrap' >&2; exit 1
  fi
  (cd "$temporary" && sha256sum -c asset.sha256 >/dev/null)
}
preset=${PRYX_PRESET:-}
if [ -z "$preset" ]; then
  if [ "$(id -u)" != 0 ]; then preset=user
  elif [ -f /.dockerenv ] || [ -f /run/.containerenv ] || [ -f /run/systemd/container ] || [ -n "${container:-}" ]; then preset=container
  elif [ -d /run/systemd/system ]; then preset=server
  else echo 'Root requires systemd or a container; run the command as an ordinary user' >&2; exit 1
  fi
fi
# Only an explicitly requested lab crosses the bootstrap handoff.
experimental=${PRYX_EXPERIMENTAL:-0}
lab_feature=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --experimental) experimental=1 ;;
    --feature) [ -z "$lab_feature" ] || { echo 'supply --feature only once' >&2; exit 1; }
      [ "$#" -ge 2 ] && [ -n "$2" ] || { echo '--feature needs a name' >&2; exit 1; }
      lab_feature=$2; shift ;;
  esac
  shift
done
set -- --preset "$preset"
if [ "$experimental" = 1 ]; then set -- "$@" --experimental; fi
if [ -n "$lab_feature" ]; then
  [ "$experimental" = 1 ] || { echo '--feature requires --experimental' >&2; exit 1; }
  set -- "$@" --feature "$lab_feature"
fi
if [ "$preset" = server ] && [ -n "${PRYX_DOMAIN:-}" ]; then
  set -- "$@" --domain "$PRYX_DOMAIN"
fi
download SHA256SUMS
if [ "$preset" = server ]; then
  download "$cli"
  verify "$cli"
  chmod 700 "$temporary/$cli"
  if [ "${PRYX_NO_UP:-0}" != 1 ]; then "$temporary/$cli" up "$@" --preflight; fi
  install -m 0755 "$temporary/$cli" /usr/local/bin/pryx
  destination=/usr/local/bin/pryx
else
  asset=pryx-bootstrap-linux-$arch.tar.gz
  download "$asset"
  verify "$asset"
  mkdir "$temporary/unpacked"
  # The initial bootstrap has the same HTTPS trust as this script. The CLI authenticates
  # the complete host bundle with its compiled Ed25519 key before activation.
  tar -xzf "$temporary/$asset" -C "$temporary/unpacked"
  if [ "${PRYX_NO_UP:-0}" != 1 ]; then "$temporary/unpacked/bin/pryx" up "$@" --preflight; fi
  if [ "$(id -u)" = 0 ]; then bootstrap=/opt/pryx/cli; destination=/usr/local/bin/pryx
  else bootstrap="$HOME/.local/share/pryx-cli"; destination="$HOME/.local/bin/pryx"
  fi
  "$temporary/unpacked/bin/pryx" bootstrap-install "$temporary/unpacked" "$bootstrap" "$destination"
fi
"$destination" --version
if [ "${PRYX_NO_UP:-0}" != 1 ]; then
  # sh -s may be consuming the script from stdin; services never inherit that pipe.
  "$destination" up "$@" </dev/null
elif [ "$experimental" = 1 ]; then
  set -- --experimental
  if [ -n "$lab_feature" ]; then set -- "$@" --feature "$lab_feature"; fi
  "$destination" experimental install "$@" </dev/null
fi

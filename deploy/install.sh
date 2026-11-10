#!/bin/sh
# SPDX-License-Identifier: FSL-1.1-ALv2
# Verify a release bootstrap, preflight without persistent changes, then install and start.
# The ordinary-user path never invokes sudo or writes outside HOME.
set -eu
case "$(uname -s)/$(uname -m)" in
  Linux/x86_64) ;;
  *) echo 'pryx supports Linux x86_64; this platform is not supported' >&2; exit 1 ;;
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
  elif [ -f /.dockerenv ] || [ -f /run/.containerenv ] || [ -n "${container:-}" ]; then preset=container
  elif [ -d /run/systemd/system ]; then preset=server
  else echo 'Root requires systemd or a container; run the command as an ordinary user' >&2; exit 1
  fi
fi
download SHA256SUMS
if [ "$preset" = server ]; then
  download pryx
  verify pryx
  chmod 700 "$temporary/pryx"
  if [ "${PRYX_NO_UP:-0}" != 1 ]; then "$temporary/pryx" up --preset server --preflight; fi
  install -m 0755 "$temporary/pryx" /usr/local/bin/pryx
  destination=/usr/local/bin/pryx
else
  asset=pryx-bootstrap-linux-x64.tar.gz
  download "$asset"
  verify "$asset"
  mkdir "$temporary/unpacked"
  # The initial bootstrap has the same HTTPS trust as this script. The CLI authenticates
  # the complete host bundle with its compiled Ed25519 key before activation.
  tar -xzf "$temporary/$asset" -C "$temporary/unpacked"
  if [ "${PRYX_NO_UP:-0}" != 1 ]; then "$temporary/unpacked/bin/pryx" up --preset "$preset" --preflight; fi
  if [ "$(id -u)" = 0 ]; then bootstrap=/opt/pryx/cli; destination=/usr/local/bin/pryx
  else bootstrap="$HOME/.local/share/pryx-cli"; destination="$HOME/.local/bin/pryx"
  fi
  "$temporary/unpacked/bin/pryx" bootstrap-install "$temporary/unpacked" "$bootstrap" "$destination"
fi
"$destination" --version
if [ "${PRYX_NO_UP:-0}" != 1 ]; then
  # sh -s may be consuming the script from stdin; services never inherit that pipe.
  "$destination" up --preset "$preset" </dev/null
fi

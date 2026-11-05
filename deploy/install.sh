#!/bin/sh
# SPDX-License-Identifier: FSL-1.1-ALv2
# Install the CLI from one release, verifying its digest before any execution.
set -eu
case "$(uname -s)/$(uname -m)" in
  Linux/x86_64) ;;
  *) echo 'pryx supports Linux x86_64; this platform is not supported' >&2; exit 1 ;;
esac
command -v curl >/dev/null || { echo 'pryx installation needs curl' >&2; exit 1; }
command -v sha256sum >/dev/null || { echo 'pryx installation needs sha256sum' >&2; exit 1; }
umask 077
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
base=${PRYX_RELEASE_BASE_URL:-https://github.com/cubepals/pryx/releases/download/v__PRYX_VERSION__}
case "$base" in
  http://127.0.0.1:*)
    authority=${base#http://127.0.0.1:}
    port=${authority%%/*}
    case "$port" in ''|*[!0-9]*) echo 'Loopback HTTP release URL needs a numeric port' >&2; exit 1;; esac
    ;;
esac
download() {
  case "$base" in
    https://*) curl --proto '=https' --proto-redir '=https' -fsSL "$base/$1" -o "$temporary/$1" ;;
    http://127.0.0.1:*) curl --proto '=http' -fsS "$base/$1" -o "$temporary/$1" ;;
    *) echo 'The release URL must use HTTPS (or loopback HTTP for build artifacts)' >&2; exit 1 ;;
  esac
}
download pryx
download SHA256SUMS
awk '$2 == "pryx" || $2 == "*pryx" { print $1 "  pryx"; count++ } END { if (count != 1) exit 1 }' "$temporary/SHA256SUMS" > "$temporary/cli.sha256"
(cd "$temporary" && sha256sum -c cli.sha256 >/dev/null)
destination=/usr/local/bin/pryx
if [ "$(id -u)" = 0 ] || [ -w /usr/local/bin ]; then
  install -m 0755 "$temporary/pryx" "$destination"
elif command -v sudo >/dev/null; then
  sudo install -m 0755 "$temporary/pryx" "$destination"
else
  destination="$HOME/.local/bin/pryx"
  mkdir -p "$HOME/.local/bin"
  install -m 0755 "$temporary/pryx" "$destination"
  case ":$PATH:" in *":$HOME/.local/bin:"*) ;; *) echo "Add $HOME/.local/bin to PATH before running pryx" >&2;; esac
fi
"$destination" --version

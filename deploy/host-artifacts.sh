#!/bin/bash
# SPDX-License-Identifier: FSL-1.1-ALv2
# Fetch each expected CI archive part through maintained GitHub CLI. Retry only reads;
# producing-job hashes authenticate transport independently of artifact-service listing order.
# This does not install pryx or relax the final archive checks.
set -euo pipefail

download_host_parts() {
  local cli=$1 count=$2 manifest=$3 transport=$4 work=$5 destination=$6
  local piece stage delay deadline valid
  [[ $count =~ ^[1-4]$ && $manifest =~ ^[a-f0-9]{64}$ && $transport =~ ^[a-f0-9]{64}$ ]] || return 1
  test ! -e "$destination" || return 1
  mkdir "$work/verified"
  for ((piece=0; piece<count; piece++)); do
    deadline=$((SECONDS + 120))
    valid=false
    for delay in 0 2 5 10 20 30; do
      (( SECONDS < deadline )) || break
      if ((delay)); then sleep "$delay"; fi
      (( SECONDS < deadline )) || break
      stage="$work/download-$piece-$delay"
      mkdir "$stage"
      if timeout 30s "$cli" run download "$GITHUB_RUN_ID" --repo "$GITHUB_REPOSITORY" \
          --name "host-bundle-preview-$piece" --dir "$stage" >"$work/download.log" 2>&1 &&
        (cd "$stage" &&
          printf '%s  PARTS.sha256\n' "$manifest" | sha256sum -c - &&
          printf '%s  host-transport.tar\n' "$transport" | cmp - TRANSPORT.sha256 &&
          awk -v name="part$piece" '$2 == name {print; count++} END {if(count != 1)exit 1}' PARTS.sha256 |
            sha256sum -c -) >"$work/verification.log" 2>&1; then
        cp "$stage/part$piece" "$work/verified/"
        cp "$stage/PARTS.sha256" "$stage/TRANSPORT.sha256" "$work/verified/"
        valid=true
        break
      fi
      echo "Archive part $piece is not available with its producing-job digest; bounded read retry" >&2
    done
    if ! "$valid"; then
      echo "Archive part $piece could not be authenticated before the read deadline" >&2
      # Native failure text is useful, but never expose signed download URLs or token values.
      sed -E 's#https?://[^[:space:]]+#[redacted URL]#g;s/(gh[pousr]_[A-Za-z0-9_]+|github_pat_[A-Za-z0-9_]+)/[redacted token]/g' \
        "$work/download.log" "$work/verification.log" 2>/dev/null | tail -10 >&2
      return 1
    fi
  done
  mv "$work/verified" "$destination"
}

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then
  : "${GH_TOKEN:?}" "${GITHUB_RUN_ID:?}" "${GITHUB_REPOSITORY:?}"
  : "${PRYX_EXPECTED_PARTS:?}" "${PRYX_PARTS_MANIFEST:?}" "${PRYX_TRANSPORT_DIGEST:?}"
  pryx_host_artifacts_work=$(mktemp -d "${RUNNER_TEMP:-/tmp}/pryx-host-artifacts.XXXXXX")
  trap 'rm -rf -- "$pryx_host_artifacts_work"' EXIT
  unset GH_DEBUG
  if ! curl --silent --show-error --proto '=https' --proto-redir '=https' --fail --location --retry 5 --retry-all-errors \
    --retry-delay 2 --retry-max-time 180 --connect-timeout 15 --max-time 60 \
    https://github.com/cli/cli/releases/download/v2.103.3/gh_2.103.3_linux_amd64.tar.gz \
    -o "$pryx_host_artifacts_work/gh.tar.gz"; then
    curl --silent --show-error --proto '=https' --proto-redir '=https' --fail --location \
      --retry 5 --retry-all-errors --retry-delay 3 --retry-max-time 180 --connect-timeout 15 --max-time 60 \
      -H 'Accept: application/octet-stream' https://api.github.com/repos/cli/cli/releases/assets/679195621 \
      -o "$pryx_host_artifacts_work/gh.tar.gz"
  fi
  printf '%s  %s\n' 4c806978ffce1cec70853302a56d70ac6da4f19bae7adca4e1a8ab00633dd91d \
    "$pryx_host_artifacts_work/gh.tar.gz" | sha256sum -c -
  tar -xzf "$pryx_host_artifacts_work/gh.tar.gz" -C "$pryx_host_artifacts_work"
  download_host_parts "$pryx_host_artifacts_work/gh_2.103.3_linux_amd64/bin/gh" \
    "$PRYX_EXPECTED_PARTS" "$PRYX_PARTS_MANIFEST" "$PRYX_TRANSPORT_DIGEST" \
    "$pryx_host_artifacts_work" host-parts
fi

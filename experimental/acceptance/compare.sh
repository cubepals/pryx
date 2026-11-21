#!/bin/bash
# SPDX-License-Identifier: FSL-1.1-ALv2
# Compare the two native bundle builds with identical signing and runtime inputs on this runner.
set -euo pipefail
arch=$1
access_log=$2
python3 -B -m unittest discover -s experimental/acceptance
base_commit=35c28a630404d34fcc451528369432a54abba496
base_tree=$(mktemp -d)
base_assets=$(mktemp -d)
container_id=
cleanup() {
  if [ -n "$container_id" ]; then docker rm -v "$container_id" >/dev/null; fi
  rm -rf "$base_tree" "$base_assets"
}
trap cleanup EXIT
git fetch --no-tags --depth=1 origin "$base_commit"
git archive "$base_commit" | tar -xf - -C "$base_tree"
# The existing job rewrites these descriptors to address this run's staged, verified inputs.
for input in deploy/process-tools/process-tools.json deploy/vm/vm-bundle.json; do
  git diff --exit-code "$base_commit" HEAD -- "$input"
  if [ -f "$input" ]; then cp "$input" "$base_tree/$input"; fi
done
docker build --target bundle --build-arg "PRYX_PREVIEW_PUBLIC_KEY=$(cat "$RUNNER_TEMP/preview.pub")" \
  -t pryx-lab-proof-base "$base_tree"
container_id=$(docker create pryx-lab-proof-base)
docker cp "$container_id:/bundle/." "$base_assets/"
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$base_tree/Cargo.toml" | head -1)
sed "s/__PRYX_VERSION__/$version/g" "$base_tree/deploy/install.sh" > "$base_assets/install.sh"
(cd "$base_assets" && sha256sum -- * > SHA256SUMS)
python3 experimental/acceptance/paired_install.py "$base_assets" host-artifacts
python3 experimental/acceptance/default_install.py host-artifacts "$base_assets" "$access_log" "$arch"

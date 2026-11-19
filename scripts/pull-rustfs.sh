#!/bin/sh
# SPDX-License-Identifier: FSL-1.1-ALv2
# Pulls CI's RustFS test image and names it ghcr.io/rustfs/rustfs:1.0.0, as the tests run it.
# RustFS publishes the same image index on GHCR and Docker Hub; Docker Hub's copy comes through
# Google's mirror, as CI's other images do. Each attempt tries GHCR, then that copy, both by the
# digest the Dockerfile pins, which Docker checks; waits between attempts are the pulls' own.
set -eu

alias=ghcr.io/rustfs/rustfs:1.0.0
digest=$(sed -n "s|^FROM $alias@\(sha256:[0-9a-f]\{64\}\) .*|\1|p" "$(dirname "$0")/../Dockerfile")
[ -n "$digest" ] || { echo "The Dockerfile pins no $alias digest" >&2; exit 1; }
for wait in 2 5 10 20 40 0; do
  for source in ghcr.io/rustfs/rustfs mirror.gcr.io/rustfs/rustfs; do
    if docker pull -q "$source@$digest"; then
      docker tag "$source@$digest" "$alias"
      exit 0
    fi
  done
  [ "$wait" = 0 ] && exit 1
  sleep "$wait"
done

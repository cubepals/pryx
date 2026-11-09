# SPDX-License-Identifier: FSL-1.1-ALv2
# Exercise the documented rclone fallback with the same audited dependency set.
ARG NATIVE_IMAGE=ghcr.io/cubepals/pryx-tools:latest
FROM ${NATIVE_IMAGE}
RUN set -eu; \
    while preferred="$(command -v mount-s3)"; do rm "$preferred"; done; \
    command -v rclone

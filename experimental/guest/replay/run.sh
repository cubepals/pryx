#!/bin/sh
# SPDX-License-Identifier: FSL-1.1-ALv2
# Invoke only the capture replay. The Rust test and its caller own every temporary file.
set -eu
umask 077
test "$#" -eq 3
export PRYX_LAB_CAPTURE_SOURCE="$1"
export PRYX_LAB_CAPTURE_SCRATCH="$2"
export PRYX_LAB_CAPTURE_REPORT="$3"
test ! -e "$3"
exec "$(dirname "$0")/replay-tests" --exact capture_replay::captured_image

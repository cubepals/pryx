#!/bin/sh
# SPDX-License-Identifier: FSL-1.1-ALv2
# Read only the source and the separately captured fork. Publish aggregate CSV only.
set -eu
umask 077
test "$#" -eq 3
export PRYX_LAB_FORK_SOURCE="$1"
export PRYX_LAB_FORK_MEMORY="$2"
export PRYX_LAB_FORK_REPORT="$3"
test ! -e "$3"
exec "$(dirname "$0")/replay-tests" --exact fork_replay::captured_fork_lookup

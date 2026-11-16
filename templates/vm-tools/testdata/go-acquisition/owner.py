#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Own one real Go acquisition while the check signals only this Python caller.

The fixture records its child identity for pidfd-based lifetime checks. It neither
implements a Go command nor changes the process's inherited authentication settings.
"""
import argparse
import json
import os
from pathlib import Path
import select
import signal
import sys


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--source', type=Path, required=True)
parser.add_argument('--go', type=Path, required=True)
parser.add_argument('--root', type=Path, required=True)
parser.add_argument('--pid-file', type=Path, required=True)
parser.add_argument('--custom-handler', type=Path)
parser.add_argument('--archive', action='store_true')
parser.add_argument('--auto-reap', action='store_true')
args = parser.parse_args()
sys.path.insert(0, str(args.source))
import go_acquisition


original = go_acquisition.subprocess.Popen
child_fd = None


def recorded(*values, **options):
    global child_fd
    process = original(*values, **options)
    if child_fd is not None:
        os.close(child_fd)
    child_fd = os.pidfd_open(process.pid)
    args.pid_file.write_text(str(process.pid))
    return process


def previous(value, _frame):
    args.custom_handler.write_text(json.dumps({
        'restored': signal.getsignal(value) is previous,
        'childExited': bool(select.select([child_fd], [], [], 0)[0]),
    }))


go_acquisition.subprocess.Popen = recorded
signal.signal(signal.SIGTERM, signal.SIG_DFL)
signal.signal(signal.SIGINT, signal.default_int_handler)
if args.auto_reap:
    signal.signal(signal.SIGCHLD, signal.SIG_IGN)
if args.custom_handler is not None:
    signal.signal(signal.SIGTERM, previous)
try:
    if args.archive:
        go_acquisition.module_archive(args.go, args.root, 'example.org/retry@v1.0.0',
                                      budget=30, initial_wait=10)
    else:
        go_acquisition.module_list(args.go, args.root, ['./...'], budget=30, initial_wait=10)
finally:
    if child_fd is not None:
        os.close(child_fd)

# SPDX-License-Identifier: FSL-1.1-ALv2
"""Proportional resident bytes of explicit process roots and their descendants.

Run as the host administrator in CI so smaps_rollup includes every runtime UID. PSS avoids
counting shared executable pages once per process. Roots and missing processes are reported.
"""
import json
import pathlib
import sys

roots = {int(value) for value in sys.argv[1:]}
parents = {}
for directory in pathlib.Path('/proc').iterdir():
    if not directory.name.isdigit():
        continue
    try:
        text = (directory / 'status').read_text()
        parents[int(directory.name)] = int(next(line.split()[1] for line in text.splitlines() if line.startswith('PPid:')))
    except (FileNotFoundError, ProcessLookupError):
        continue
selected = set(roots)
while True:
    children = {pid for pid, parent in parents.items() if parent in selected}
    if children <= selected:
        break
    selected |= children
memory = 0
missing = []
for pid in sorted(selected):
    try:
        lines = pathlib.Path(f'/proc/{pid}/smaps_rollup').read_text().splitlines()
        memory += int(next(line.split()[1] for line in lines if line.startswith('Pss:'))) * 1024
    except (FileNotFoundError, ProcessLookupError):
        missing.append(pid)
print(json.dumps({'pssBytes': memory, 'roots': sorted(roots), 'processes': len(selected), 'missing': missing}))

# Measurement summaries

Each record identifies the measurement, decisive aggregates, source revision when known, and
CI run when recorded. A local result or missing provenance never establishes a CI pass. Raw
logs, serial output, inventories and individual samples belong in CI artifacts, not this tree.
Historical runs may have expired artifacts; the summaries preserve their decisions and failures.

Only `k23/micromamba-builder.json` and `k23/python-standalone-review.json` remain full
machine-readable reviews: `templates/process-tools/build.py` and its artifact descriptor read
them for acquisition and shipped notice provenance. Do not reduce or remove these build inputs.

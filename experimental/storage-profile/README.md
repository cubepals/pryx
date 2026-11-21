# Storage profile

Run the existing release-mode storage benchmark on the two native Linux runners with the Measure workflow's `storage_profile` dispatch input. The timing boundaries come from the test's own progress output. Linux perf samples only user-mode CPU time in the test process tree. The fixture setup and synthetic boot contention are excluded. Off-CPU I/O and kernel work still contribute to wall time. Sampling uncertainty and the small delay between a phase ending and its output reaching the reader limit the attribution precision.

The output records each operation's duration, raw samples, top symbols, native CPU, commit and filesystem. Use the measured profile to propose at most two further candidates. Register the candidate and its threshold before comparing an implementation.

## Parts (`experimental/storage-profile/`)

- `record.py` timestamps phase observations and attributes sampled CPU time.
- `run.sh` builds the existing benchmark, identifies its executable and invokes the collector.
- `README.md` describes the evidence and how to collect it.

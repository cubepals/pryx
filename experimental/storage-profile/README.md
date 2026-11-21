# Storage profile

Run the existing release-mode storage benchmark on the two native Linux runners with the Measure workflow's `storage_profile` dispatch input. The timing boundaries come from the test's own progress output. Linux perf samples only user-mode CPU time in the test process tree. The fixture setup and synthetic boot contention are excluded. Off-CPU I/O and kernel work still contribute to wall time. Sampling uncertainty and the small delay between a phase ending and its output reaching the reader limit the attribution precision.

The output records the benchmark's own elapsed seconds (rounded to two decimals), each observed phase interval, sample counts and periods, resolved symbols and DSOs, native CPU, commit and actual fixture filesystem. The collector probes the requested user CPU-clock event before starting the benchmark. When sampling is unavailable it preserves the diagnostic and runs the same workload once for wall times. An empty, unresolved or malformed phase cannot establish a hotspot. CPU-period fractions measure sampled on-CPU demand against elapsed time, with uncertainty from finite sampling. Use the measured profile to propose at most two further candidates. Register the candidate and its threshold before comparing an implementation.

## Parts (`experimental/storage-profile/`)

- `record.py` timestamps phase observations and attributes sampled CPU time.
- `record_test.py` checks symbolized records and the unavailable-sampling result.
- `run.sh` builds the existing benchmark, identifies its executable and invokes the collector.
- `README.md` describes the evidence and how to collect it.

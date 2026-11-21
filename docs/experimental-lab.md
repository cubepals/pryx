# Experimental lab review

The [lab integration branch](https://github.com/cubepals/pryx/tree/k60-experimental) collects opt-in experiments for the owner's review. A worker branch starts at integration, records its hypothesis before timing, and retains both the baseline and the experimental implementation. An experiment enters integration only after independent review and native proofs.

## Review an experiment

1. Inspect the worker commit, its actual baseline and its feature-off behavior. The default installer must make zero requests for lab artifacts. Only the stock command-line and installer hand-off may change the shipped bytes.
2. Read the raw measurements from the same commit on both native architectures. Check the scalar reference, guarded differential tests and the relevant end-to-end workload. A simulator or throughput model cannot establish a native performance gain.
3. For the scaffold, run the existing Any host workflow on the worker ref with its lab input. For a kernel, use its registered manual proof. Each proof waits for the preceding one and yields runner priority to ordinary builder work.
4. The owner reviews before merge. Graduation also requires an end-to-end gain on the benchmark harness on both architectures, a scalar fallback and the owner's explicit decision.

An experiment can finish with a negative result. Its code stays isolated and the default behavior follows the established release.

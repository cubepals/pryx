# Real guest page replay

The x86 KVM memory acceptance job captures one 256 MiB process guest after a confirmed sleep. Its normal authenticated wake is an independent prerequisite. This supplemental run reports the observed zero-page fraction. The registered synthetic workload retains its own results and verdict.

Pass a checkout of the exact A1 commit and a new private target directory to `bash build.sh CHECKOUT TARGET`. The resulting `TARGET/replay SOURCE PRIVATE_SCRATCH AGGREGATE_CSV` reads a user-owned, private 256 MiB regular file. Use an existing owner-only scratch directory and a new CSV path outside it. The caller owns cleanup of the capture, monitor state and all staged files. Only the named aggregate CSV is eligible for upload.

The unmodified A1 writer and verifier run five fixed methods for one untimed warm-up round followed by 15 measured rounds in alternating order. Each method syncs its output and verifies every restored byte. The dense writer, maintained fill detector, widest supported assembly fill detector, BLAKE3, and native four-lane fingerprint use the same source image. The CSV gives the actual selected backend and physical file allocation.

| Parts | Purpose |
| --- | --- |
| `README.md` | Inputs, scope and result schema |
| `build.sh` | Verifies the A1 source commit, stages the module and builds its release-mode test entry point |
| `capture.rs` | Reads the stabilized capture, reuses the A1 writer and verifies the materialized bytes |
| `run.sh` | Supplies the three paths and selects the exact capture-only entry point |

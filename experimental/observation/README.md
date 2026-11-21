# Process-tier syscall observation

A manual `measure.yml` dispatch with `observe_syscalls=true` starts native x86-64 and AArch64 jobs serially. The build copies the stock Cargo manifests and lockfile into a temporary workspace and declares the observation feature and example there. It compiles the existing agent policy source with the server feature. A separate Cargo workspace boundary keeps these lab files out of stock source packages. Each method runs inside a disposable unprivileged child after the same Landlock, network-off seccomp, resource and child restrictions are installed. A previously readable file outside the allowed tree must return EACCES. An invalid bind must change from the host's EBADF to the policy's EPERM. Failure of either check invalidates the sample.

The fixture runs 128 warm-up and 4,096 timed raw getpid calls. Every return must equal the child's known PID. Its harmless, unused first argument identifies fixture calls in the observer. The control, ptrace, seccomp notification and cooperative User Dispatch workloads all enter the same fixed assembly site. User Dispatch covers only that function. Each signal handler obtains the child's actual PID, and every sample requires one signal from that exact function and one correct result per call. The reported delta includes the cooperative wrapper, kernel signal delivery, a real getpid in the handler and count validation. The kernel must support inclusive mode under the installed floor.

The link-time cohort uses a second static build of this consenting child. The linker wraps only its named fixture function. The x86-64 and AArch64 entry points increment a child-private count and tail-branch directly into the same marked getpid assembly as the raw control. The timed loop checks the real returned PID each time. The child checks 128 warm-up events before timing and 4,096 more events after timing. Both executable digests accompany the paired samples. This measures the counting cost at a cooperative call site. General binary rewriting, transparent coverage of other call sites, and syscall mediation remain unmeasured.

The prepared-ELF comparison uses the same cooperative getpid site. Two independent static links create an inactive input and active oracle. Both read and write the child-private counter. Their only permitted difference is the immediate on one four-byte add in the source-defined `.pryx_observe_count` section. On x86-64 the instruction adds zero or one to `r10d`. On AArch64 it adds zero or one to `w10`. The build extracts the named sections, verifies the expected instruction bytes, changes the bit before execution and requires the entire resulting file to equal the oracle. Both prepared links omit the linker build ID for the full-file comparison. The raw and ordinary link-time controls retain their original build flags.

Each of 15 triples runs raw, inactive and transformed executables on the same CPU, reversing the order on alternating triples. All three require the real PID for 128 warm-up calls and 4,096 measured calls. The inactive counter must stay at zero. The transformed counter must report all 4,224 calls. The primary median and p95 compare transformed against raw. Separate paired deltas measure transformed against inactive, and inactive against raw. The inactive control includes the cost of loading and storing the counter. The runner checks both measured ELF digests before and after this cohort. The results JSON includes the raw timings, verified instruction encodings and independent oracle digest. This experiment covers activation of one counter in this deliberately prepared executable.

The parent traces only its own consenting child. It observes entry and exit stops, verifies the returned PID, and never reads process memory or writes a register. The listener receives only the fixed marked operation, checks its own child's identity, and allows the kernel to complete that argument-free call. The listener descriptor is transferred before the production policy closes network access. Installed filters and Landlock are always present during the timed region. No other program or target can be selected.

The runner pins both processes to one CPU and alternates the raw/control order across 15 pairs for each method. Results give nanoseconds per completed call from the child's monotonic clock. This single-CPU scheduling choice is part of the measured cost. Missing host support produces an unavailable reason; no method can earn a verdict from incomplete counts. The local p50 budget is 100 ns additional per call; both native architectures must pass the same method for a lab-level timing verdict. This fixture makes no arbitrary-workload coverage claim.

Parts (`observation/`):

- `README.md` describes the fixed workload and timing protocol.
- `Cargo.toml` establishes a separate package boundary for these lab sources.
- `build.sh` creates the private Cargo build, links the static fixtures and verifies the prepared ELF against the oracle.
- `floor.rs` applies the production restrictions in the child.
- `workload.c` owns the child, safety checks, timer and validated native return values.
- `link-count.S` provides the private counter for the link-time and prepared executables, then enters the same fixed assembly body.
- `getpid.S` issues only the fixed getpid syscall on each supported architecture.
- `mark.h` names the unused argument that identifies this fixture.
- `observation.h` defines the fixed sample sizes and private C interfaces.
- `dispatch.c` measures signal delivery at the one fixed getpid function.
- `trace.c` counts the child's marked syscall entries and validated exits.
- `notification.c` owns the restricted listener and checks each notification.
- `cohort.py` runs the paired and three-way comparisons, checks digests and counts, and calculates timing differences.

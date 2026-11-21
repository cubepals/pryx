# Process-tier syscall observation

A manual `measure.yml` dispatch with `observe_syscalls=true` starts native x86-64 and AArch64 jobs serially. The process floor comes from the ordinary pryx agent. Each method runs inside a disposable unprivileged child after the same Landlock, network-off seccomp, resource and child restrictions are installed. A previously readable file outside the allowed tree must return EACCES. An invalid bind must change from the host's EBADF to the policy's EPERM. Failure of either check invalidates the sample.

The fixture runs 128 warm-up and 4,096 timed raw getpid calls. Every return must equal the child's known PID. Its harmless, unused first argument identifies fixture calls in the observer. The control, ptrace and seccomp-user-notification workloads use the same compiled assembly.

The parent traces only its own consenting child. It observes entry and exit stops, verifies the returned PID, and never reads process memory or writes a register. The listener receives only the fixed marked operation, checks its own child's identity, and allows the kernel to complete that argument-free call. The listener descriptor is transferred before the production policy closes network access. Installed filters and Landlock are always present during the timed region. No other program or target can be selected.

The runner pins both processes to one CPU and alternates the raw/control order across 15 pairs for each method. Results give nanoseconds per completed call from the child's monotonic clock. This single-CPU scheduling choice is part of the measured cost. Missing host support produces an unavailable reason; no method can earn a verdict from incomplete counts. The local p50 budget is 100 ns additional per call; both native architectures must pass the same method for a lab-level timing verdict. This fixture makes no arbitrary-workload coverage claim.

Parts (`observation/`):

- `README.md` describes the fixed workload and timing protocol.
- `build.sh` builds the optional policy library and native static fixture.
- `floor.rs` applies the production restrictions in the child.
- `workload.c` owns the child, safety checks, timer and validated native return values.
- `getpid.S` issues only the fixed getpid syscall on each supported architecture.
- `mark.h` names the unused argument that identifies this fixture.
- `observation.h` defines the fixed sample sizes and private C interfaces.
- `trace.c` counts the child's marked syscall entries and validated exits.
- `notification.c` owns the restricted listener and checks each notification.
- `cohort.py` runs paired controls, checks counts and calculates timing differences.

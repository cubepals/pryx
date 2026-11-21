<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

# Guest boot

The current VM guest agent is Rust. The SDK configurator, network helper and
minimal filesystem proof include Go programs. The measurements below use the
real Rust agent from the VM workflow's static bootstrap.

The existing **VM experiments** dispatch accepts two independently named inputs.
Set `lab_guest=true` to build the two freestanding inits and run fifteen cold
rounds. Each round rotates the order of six cases across three init executables
and the current and quiet kernel command lines. Each monitor gets one vCPU,
256 MiB, its read-only EROFS root, a fresh throwaway writes device and vsock.
CPU caches and monitor binaries are warm across rounds. The C and assembly
guest images have only one executable. Their init blocks until the host stops
the fixture.

The first user instructions take the Linux boot-clock timestamp and write its
hexadecimal nanoseconds to the serial console. All three ELF files use the same
entry stamp. The instrumented production agent continues through its original
entry and makes its normal private bootstrap connection. For this experiment
the connection ends before configuration or credentials are sent.

| Column | Origin and endpoint |
| --- | --- |
| Boot to PID 1 | Guest CLOCK_BOOTTIME to the timestamp taken on ELF entry, after a PID check |
| Launch to serial | Host monotonic time from monitor spawn to reading the PID 1 marker |
| Launch to bootstrap | Same host clock to accepting the real agent’s first vsock connection |

Read the historical 816–832 ms in [the VM stage record](../../docs/decisions.md)
as monitor startup to bootstrap. It includes part of guest userspace startup.
The per-guest kernel clock and the host clock have different origins. The
small fixture has no authenticated service, so it cannot supply an agent-ready
time. Host serial delivery includes console transit.

The dispatch also attempts a matched kernel build within fifteen minutes total.
It uses the recorded source and published config, first with the original
serial timestamp option, then without it. The recorded upstream source, vendor
build and local toolchain can differ. Only the two local siblings form a config
comparison. The build status, raw rows, logs and table live in the
`experimental-guest-boots` artifact. A failed or timed-out build keeps the
first cohort and supplies no configuration verdict. The pinned production
monitor, configurator and kernel currently support x86-64. Both tiny programs
build for AArch64; the ARM guest timing needs native KVM and a validated ARM
VM input set.

After the page benchmarks, set `lab_agent_baseline=true` in a separate dispatch
to measure the unchanged agent. This collects fifteen fresh, disconnected,
one-image boots. Each sample times monitor launch through the existing
authenticated health check and reads VmRSS in KiB for PID 1 and the serving
process via authenticated guest exec immediately afterwards. The per-process
values can count shared pages twice. Every sample also completes the existing
signed shutdown and replay check. The existing idle health probe cadence enters
its timings. Raw rows are in the `experimental-agent-baseline` artifact. A
handshake alone provides no evidence of authenticated readiness.

Parts (`guest/`):

- `README.md` describes the two dispatch inputs and measurement units.
- `stamp.S` timestamps the first user instructions on each architecture.
- `init.S` and `init.c` are the freestanding assembly and C guest programs.
- `build.sh` builds the native ELF probes and checks their program headers.
- `boot.py` owns a fresh monitor, its clocks, bounded serial and private sockets.
- `cohort.py` rotates the cases, writes raw rows and computes p50 and p95.
- `kernel_pair.py` prepares the verified, controlled kernel-config pair.
- `agent_baseline.rs` collects authenticated service readiness and guest RSS.
- `prove.sh` runs the opt-in experiment with the existing workflow inputs.
- `test_boot.py` checks marker framing, summaries and threshold boundaries.

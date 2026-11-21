<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

# Guest boot

The current VM guest agent is Rust. The SDK configurator, network helper and
minimal filesystem proof include Go programs. The measurements below use the
real Rust agent from the VM workflow's static bootstrap.

The existing **VM experiments** dispatch accepts independently named inputs.
Selecting both `lab_agent_baseline` and `lab_guest` stages the expensive VM
inputs once. The stock A3 baseline runs and uploads first. E1 then uses separate
images and fresh monitors on the same host.
Set `lab_guest=true` to build the two freestanding inits and run fifteen cold
rounds. The six cases combine three init executables with the current and quiet
kernel command lines. Odd rounds use the recorded order and even rounds reverse
the entire sequence. Each event gives its one-based round and position, case,
and direction. Host frequency fields are CPU 0 sysfs values at cohort start
in kHz, with null for unavailable values. Each monitor gets one vCPU,
256 MiB, its read-only EROFS root, a fresh throwaway writes device and vsock.
CPU caches and monitor binaries are warm across rounds. The C and assembly
guest images have only one executable. Their init blocks until the host stops
the fixture.

The first user instructions take the Linux boot-clock timestamp and write its
hexadecimal nanoseconds to the serial console. The assembly and production ELF files share an
entry stamp. The freestanding C implementation takes the same clock reading
after the same PID check with a native syscall. The instrumented production agent continues through its original
entry and makes its normal private bootstrap connection. For this experiment
the connection ends before configuration or credentials are sent.

| Column | Origin and endpoint |
| --- | --- |
| Boot to PID 1 | Guest CLOCK_BOOTTIME to the timestamp taken on ELF entry, after a PID check |
| Launch to serial | Host monotonic time from monitor spawn to reading the PID 1 marker |
| Launch to bootstrap | Same host clock to accepting the real agent’s first vsock connection |
| Serial to bootstrap | Elapsed time between the two host deliveries in each real-agent boot, aggregated after subtraction |
| Unordered/same-poll pairs | Bootstrap arrives before serial, or both become readable in one poll; these rows retain their raw delta and leave the phase percentiles |

Read the historical 816–832 ms in [the VM stage record](../../docs/decisions.md)
as the wait after host forwarding was set up until bootstrap delivery completed.
It includes guest userspace startup and uses a later endpoint than this
experiment's bootstrap-connection acceptance.
The per-guest kernel clock and the host clock have different origins. The
small fixture has no authenticated service, so it cannot supply an agent-ready
time. Host serial delivery includes console transit.

The dispatch also attempts a matched kernel build within fifteen minutes total.
The original runtime artifacts and mandatory boot samples upload first. The
55-minute input job reserves two minutes for final upload and two minutes for
paired boots. If there is too little time for the full build budget, its status
records why it was skipped.
It uses the recorded source and published config, first with the original
serial timestamp option, then without it. The recorded upstream source, vendor
build and local toolchain can differ. Only the two local siblings form a config
comparison. The raw PID 1 rows, logs and table upload as `experimental-guest-boots`
before the kernel build starts. Its build status, log, configs and completed
kernel comparison upload separately as `experimental-guest-kernel-pair`. A failed or timed-out build keeps the
first cohort and supplies no configuration verdict. The pinned production
monitor, configurator and kernel currently support x86-64. Both tiny programs
build for AArch64; the ARM guest timing needs native KVM and a validated ARM
VM input set. The ARM step of the same dispatch runs each static executable
as an ordinary native process, checks its executable identity and blocking
syscall, opens the KVM device and requests its API revision. The
`experimental-arm-guest-prerequisites` artifact records the real revision or
errno. Selecting `lab_guest_arm=true` on its own runs only this short ARM check.
An ELF run and a VM boot are separate observations.

After the page benchmarks, set `lab_agent_baseline=true`
to measure the unchanged agent. This collects fifteen fresh, disconnected,
one-image boots. Each sample times monitor launch through the existing
authenticated health check. The host launches the production private forwarder,
waits for bootstrap delivery and invokes the live runtime's readiness loop.
The route-scoped signed request goes through the reserved stream-port HTTP/2
CONNECT forwarder to the unchanged guest service. The first authenticated
diagnostic exec uses that same forwarding address and reads VmRSS in KiB for PID 1 and the serving
process immediately afterwards. The report retains host-clock times for monitor plan sent, forwarder listening, bootstrap delivery,
authenticated health and completion of the first RSS exec. The last observation
bounds the delay between readiness and the guest memory read. An invalid bearer
must get HTTP 401 on that same health route. The per-process
values can count shared pages twice. The combined process RSS row uses the sum
from each individual boot before computing the median and tail. Every sample also completes the existing
signed shutdown and replay check. The baseline uses the production 10 ms health retry interval. The `experimental-agent-baseline` artifact contains phase and RSS
percentiles, raw rows and host and binary identities. Its conditional timing
bound holds the observed bootstrap-delivery times fixed and sets all later work
to zero. A
handshake alone provides no evidence of authenticated readiness.

Selecting `lab_memory_replay=true` adds one separate x86 test after the stock
sleep, wake and fork tests on the existing memory runner. It measures one
disconnected 256 MiB Alpine guest with a running sleep process. The regular
authenticated pause saves the RAM, retires its monitor and commits the image
before the test opens a read-only descriptor. It completes the normal signed
wake and reads a tmpfs marker through the authenticated agent. Only then does
it copy and compare every source byte in a private directory.

`experimental-real-snapshot-writers` retains one capture interval, the wake
interval, the full pause-through-marker interval and the separate synced
file-write trials. It also retains zero-page fraction inputs, disk allocation,
source revisions and byte-equality results. The input memory, individual page
hashes, boot state and writer output files stay in temporary private storage.
The captured guest is one observed distribution and supplements the registered
synthetic A1 cohorts. The writer process has a three-minute limit. A combined
dispatch runs this test after the accepted A3 baseline.

Parts (`guest/`):

- `README.md` describes the dispatch inputs and measurement units.
- `stamp.S` timestamps the first user instructions on each architecture.
- `init.S` and `init.c` are the freestanding assembly and C guest programs.
- `build.sh` builds the native ELF probes and checks their program headers.
- `facts.py` checks input digests and records the compiler, filesystem, CPU policies and page settings.
- `boot.py` owns a fresh monitor, its clocks, bounded serial and private sockets.
- `cohort.py` reverses the full case order on alternating rounds, writes raw rows and computes p50 and p95.
- `native.py` checks both native ELF programs and records the host prerequisites.
- `kvm_api.c` reads the host KVM API revision from the ordinary-user device.
- `kernel_pair.py` prepares the verified, controlled kernel-config pair.
- `kernel_proof.sh` bounds that optional build and releases the source and object tree.
- `agent_baseline.rs` collects authenticated service readiness and guest RSS.
- `framed_health.rs` invokes the production forwarding and readiness APIs for the external acceptance fixture.
- `baseline_summary.py` verifies accepted samples and computes phase and RSS percentiles.
- `prove.sh` runs the opt-in experiment with the existing workflow inputs.
- `fork_replay.rs` captures a diverged, authenticated fork for private page matching.
- `snapshot_replay.rs` captures the real RAM after a successful pause and checks the restored service.
- `replay/` contains the pinned writer adapter and protected synthetic correctness fixture.
- `test_boot.py` and `test_agent_baseline.py` check markers, summaries, accepted samples and thresholds.

# Process policy cost

This opt-in experiment collects the ordinary guest's existing seccomp and network filter
baselines before choosing a new decision order. Each operation runs only on a lab worker. Dispatch `engines.yml` on the experiment branch
with `policy_cost=true` to collect both architectures serially. The ordinary workflow
dispatch still runs the engine suite. A lab-only run does not satisfy its landing proof.

## Seccomp

From the checkout root, on either native architecture:

```sh
cargo build --release --locked --manifest-path experimental/policy/Cargo.toml --features experimental
python3 experimental/policy/seccomp.py experimental/policy/target/release/pryx-policy-cost > seccomp.json
```

The baseline and candidate samples use the same single CPU in alternating order.
The candidate checks the two measured syscall paths after the original architecture
check and retains the complete original decision body. Each shortcut's action is derived
from the original bytecode using only the architecture and syscall number. Encountering
an argument-dependent action leaves that path in the original body. The ABI guard still
wraps every install. A reference interpreter compares complete 32-bit actions for native and foreign
audit values, rule numbers and argument patterns on each native runner. `pryx-policy-cost verify` checks every filter variant without installing it. The process cohort inherits
the runner's seccomp policy, then installs the supervisor, child and network-off seccomp stack
in its production order. The host cohort inherits just the runner's policy. Raw getpid takes
the allowed path. Bind on an invalid descriptor normally returns EBADF and the network-off
filter returns EPERM. Its paired delta includes the cost of the two different return paths.
The kernel may use its constant-ALLOW cache for getpid. On kernels built with
CONFIG_SECCOMP_CACHE_DEBUG, the workflow also captures the kernel's per-syscall cache view
when readable. The sample includes any kernel shortcut.
The benchmark makes no change to the host's enforcement or sysctls.

## Cgroup programs

On the disposable privileged runner configured by the existing process network policy test:

```sh
sudo python3 experimental/policy/network.py "$RUNNER_TEMP/network-policy" --verify --skb
```

The script creates fresh pins under a mounted bpffs, requests kernel verification and runs
IPv4/IPv6 TCP and UDP packets through the ingress and egress hooks with the kernel test
runner's default loopback interface and empty maps. All
packet traffic stays within the kernel's test harness. This cohort omits real socket
ownership and populated rules; the existing process network suite covers those cases.
The standalone command without options reads only the shipped object's disassembly.

The kernel test result gives the verdict and average runtime for a repeated packet.
LLVM disassembly lists static instructions. The verifier's processed-instruction statistic
counts abstract exploration. Measuring dynamically executed eBPF instructions per hook
requires a separate tracing or interpreter comparison on the chosen workload. Retired
native instructions from a host PMU use a different unit. Keep the raw verifier log and
kernel JSON, including unavailable counters. Register every candidate before benchmarking.

### Count executed original BPF instructions

This second test-run-only cohort covers the two skb hooks. It restricts its input to the exact
stock object and the fixed IPv4/IPv6 corpus. It opens new private maps for each pass and never
attaches either object. The original program supplies the verifier result and stock outputs.
Each diagnostic copy preserves the original classifier and counters and accumulates a unique path number
for this acyclic program. A conditional's taken-edge paths occupy the first range of ranks,
followed by its fall-through paths. The kernel writes the rank to the test packet's scratch
words at exit. The collector decodes the full path from this single invocation and emits every original instruction PC. A two-slot 64-bit immediate counts as one
instruction. Calls to map-lookup and socket-storage-get are counted separately. Kernel helper
internals, verifier visits and JIT instructions are different quantities.

Every event must match stock return, output packet, all other skb output context and complete
per-CPU counter map after each diagnostic invocation. A failure produces no count file. Regular
socket-address and socket-creation hooks need real contexts in a separate workload.

From the existing native A2 worker, use its already verified bpftool and LLVM 18 package:

```sh
python3 -B experimental/policy/trace_test.py
sudo env GITHUB_REPOSITORY="$GITHUB_REPOSITORY" GITHUB_ACTIONS="$GITHUB_ACTIONS" \
  python3 -B experimental/policy/run_coverage.py bpf/network.bpf.o "$RUNNER_TEMP/policy/trace" --bpftool "$bpftool"
```

## Parts (`experimental/policy/`)

- `README.md` describes scope, commands and interpretation.
- `Cargo.toml` declares the standalone package and off-by-default feature.
- `Cargo.lock` pins the dependencies used by the agent's maintained filter compiler.
- `decision.rs` builds the two-key candidate and differentially checks the reference bytecode.
- `latency.rs` installs the production seccomp stack and checks the two syscall results.
- `seccomp.py` pairs fresh processes and records distributions and raw samples.
- `network.py` collects the compiled BPF and kernel verifier evidence.
- `corpus.py` generates valid Ethernet frames for the isolated kernel packet test.
- `layout.c` obtains offsets, structure size and helper identifiers from the runner's UAPI.
- `counted_object.py` builds the disposable annotated copy of the pinned fixture.
- `run_coverage.py` cross-checks kernel outcomes and reconstructs executed paths.
- `trace_test.py` tests relocation and path ordering in isolation.

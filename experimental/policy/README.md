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

### Time the seven socket hooks

The disposable CI worker opens a fresh network namespace containing only loopback. It loads the digest-checked stock object with its own maps and attaches all nine programs to a new cgroup. Only the fixed UDP child enters that cgroup. For IPv4 and IPv6 it binds both sockets to loopback, sends to a peer owned by its cgroup, verifies the returned payload and source, and connects to the same peer. The two exact loopback prefixes use the stock OWNED policy.

This test records the result of a direct, empty-context test-run probe per socket hook. Kernel errno 524 identifies a missing test-run operation on the checked kernels. Other results retain their raw errno. Linux runtime statistics measure the attached programs in the real socket context. The accounting descriptor enables kernel-wide BPF timing until it closes. Each reported duration uses the difference in `run_time_ns` and `run_cnt` of the same private program ID. A batch must observe four socket creations per round, one call to each of the six address hooks per round and traffic in both packet directions. The journal retains completed kernel snapshots after a partial failure. These counters include helpers and timing instrumentation. Original instructions per socket invocation remain unmeasured.

```sh
sudo env GITHUB_ACTIONS="$GITHUB_ACTIONS" GITHUB_REPOSITORY="$GITHUB_REPOSITORY" \
  GITHUB_EVENT_NAME="$GITHUB_EVENT_NAME" python3 -B experimental/policy/socket_hooks.py \
  "$RUNNER_TEMP/policy/socket" --bpftool "$bpftool"
```

### Compare a named egress candidate

A manual `engines.yml` run selects the isolated packet and real-socket experiment with `policy_experiment=local-tcp-ack-first`. The default choice is `stock`. The experiment reproduces the shipped ELF with its own release recipe, then builds the one registered source edit in an empty directory. Its candidate ELF must have SHA-256 `1c7f2419490f4f270046fc056cdca0971834be7b2d5d8a31421fa17165164993`. The eight other program sections and their map relocation targets must match stock exactly.

The packet test exercises both native address families, with explicit local and second-device contexts, positive local TCP ACKs, SYNs, other TCP flags and UDP. The kernel must return the expected interface, verdict, bytes, context and per-CPU counter state. The timing cohort uses the original 16 packets on both hooks in 31 alternating stock/candidate pairs with 1,000 repeats per event. Each worker also compares fourteen real private UDP checkpoints across two families, including peer addresses, delivery, EPERM, per-CPU state and live socket-storage values. The full unchanged process-network integration target uses a disposable checkout with the candidate embedded. Only the CI adapter uses the pinned stock test namespace and test services.

The seven socket paths have a separate positive-UDP-only diagnostic. It appends its own map to a private copy, preserves the original non-diagnostic maps and kernel actions, and measures whether every invocation followed its complete registered path. Unsupported tracing or any mismatch leaves those counts unavailable. This diagnostic runs after the packet timing cohort.

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
- `socket_hooks.py` times stock programs on real private UDP sockets and journals their kernel counters.
- `socket_uapi.c` obtains the native syscall, command, attachment and structure layout constants for those programs.
- `candidate_image.py` builds and identifies the precisely scoped compiler inputs and ELF pair.
- `packet_candidate.py` compares the named packet candidate and measures the registered pairs.
- `socket_compare.py` checks real socket actions and map effects across the two private images.
- `socket_compare_test.py` exercises the socket-state comparison and failure cases offline.
- `socket_state.c` reads socket storage from the original live descriptors in the test workload.
- `candidate_tests.py` runs the protected revision's unchanged policy integration target.
- `socket_witness.py` cross-checks the fixed socket path witnesses with a separate stock workload.
- `socket_witness_object.py` constructs the fixed map-bearing copy for the seven original paths.
- `socket_witness.bpf.c` declares the dedicated diagnostic map.
- `test_socket_witness.py` checks original branch edges, relocation targets and return preservation.

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

The paired samples use the same single CPU in alternating order. The process cohort inherits
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
IPv4/IPv6 TCP and UDP packets through the ingress and egress hooks with empty maps. All
packet traffic stays within the kernel's test harness. This cohort omits real socket
ownership and populated rules; the existing process network suite covers those cases.
The standalone command without options reads only the shipped object's disassembly.

The kernel test result gives the verdict and average runtime for a repeated packet.
LLVM disassembly lists static instructions. The verifier's processed-instruction statistic
counts abstract exploration. Measuring dynamically executed eBPF instructions per hook
requires a separate tracing or interpreter comparison on the chosen workload. Retired
native instructions from a host PMU use a different unit. Keep the raw verifier log and
kernel JSON, including unavailable counters. Register every candidate before benchmarking.

## Parts (`experimental/policy/`)

- `README.md` describes scope, commands and interpretation.
- `Cargo.toml` declares the standalone package and off-by-default feature.
- `Cargo.lock` pins the dependencies used by the agent's maintained filter compiler.
- `latency.rs` installs the production seccomp stack and checks the two syscall results.
- `seccomp.py` pairs fresh processes and records distributions and raw samples.
- `network.py` collects the compiled BPF and kernel verifier evidence.
- `corpus.py` generates valid Ethernet frames for the isolated kernel packet test.

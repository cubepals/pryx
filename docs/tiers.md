<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

# Sandbox tiers

A node with `runtime = "auto"` serves every backend its measured host capabilities permit.
Container, process and Wasm workloads can coexist under one capacity and reservation ledger.
Each workload keeps its chosen tier for its lifetime and reports that tier's guarantees.
A named unavailable tier is refused; it is never substituted.

For an unnamed request, control chooses container, then process, then Wasm among healthy nodes
that meet `require` and accept the supplied source: images need containers, image-free requests
use process or Wasm, and package requests need Wasm. Snapshots and templates retain their saved
tier. VM joins that order between container and process when available in K25.
`PRYX_DEFAULT_ISOLATION=process` fixes one tier; a comma-separated list changes the priority.
Local installations retain this preference across user-manager restarts and upgrades.
The image and new local installations use auto mode. Without an engine, the usual first choice
remains **process**, with Wasm available alongside it. `PRYX_DEFAULT_TIER` remains a legacy
single-backend override. Explicit `runtime` settings and older installation records retain their
single backend. Both doctors list enabled tiers and reasons for those that are off.
Before switching an older configuration to auto, start it once with its previous explicit
runtime to save existing workloads' tiers. Auto can identify existing compute and container
images; it refuses a resting legacy record whose process/Wasm identity cannot be established.

The complete measurements below retain their original contexts, samples and budgets.

## Rootless engine measurements

The [ordinary-user engine run](https://github.com/cubepals/pryx/actions/runs/43071213897)
measures source `197a372781c654311205064e3f482852b2e01907` on an x86_64 Ubuntu 24.04 hosted
runner. Podman is the pinned Ubuntu `4.9.3+ds1-1ubuntu0.2` package, with vfs storage and no
delegated controllers. Each fresh workload reserves 256 MiB for admission and explicitly accepts
missing memory enforcement; CPU, PID and memory-sleep limits are off. The same job proves
nonzero identity, readonly root, zero kernel capabilities, PID and network separation,
HTTP ingress, snapshot/fork retention, stop/wake and bounded exec-tree cancellation.
A separate private-network case proves published ingress, no default route and denied external
egress after an internet-mode canary first proves the target is reachable.

These are **direct node-manager measurements**. They include neither the public API nor the
edge, and do not replace the public-edge cohorts and budgets below. All 30 serial samples
use a locally available image and a new workload identity. Create includes ensure and start;
exec runs `true` after writing the service's data file outside the exec timer; network is one
HTTP request through the engine's published loopback port, returning that file. Image pulling,
strict deletion and installer startup are outside these timers. Percentiles use nearest rank.

| Engine and host context | Completed samples | Create ms p50 / p90 | Exec ms p50 / p90 | HTTP round trip ms p50 / p90 |
| --- | ---: | ---: | ---: | ---: |
| Rootless Podman, ordinary runner user | 30 | 2247.29 / 2472.42 | 75.68 / 265.73 | 3.07 / 3.20 |
| Standard rootless Docker 29.9.2 mapping | 0 | Unavailable | Unavailable | Unavailable |

The standard Docker mapping cannot expose daemon-owned data to the required nonzero sandbox
user: default and host namespaces deny the write, and keep-id is unsupported. It refuses
`data-owner-mapping` before data or compute, while other tiers remain available. Its positive
lifecycle and timings are parked; a custom mapped local daemon is not evidence for that host.
Rootless Docker also refuses `network-policy` for `network: "none"`: its internal bridges cannot
provide ingress through the node user. Podman uses native published ingress for internal bridges.
The [Podman samples](checks/measurements/k22/podman-197a372.json),
[Docker refusal](checks/measurements/k22/docker-197a372.json) and
[artifact provenance](checks/measurements/k22/provenance-197a372.json) are retained byte for byte after
checking their GitHub archive digests. A failed collection retains its partial sample count
and cannot report completion.

Rootless memory/CPU guarantees require both engine support and an enforced limit observed on
that sandbox copy. Nodes announcing `observed-resource-limits` include the copy's guarantees
in heartbeats; the public API uses the report matching its node and placement epoch. Missing
observations keep these guarantees off. A controller enabled later therefore leaves an older
unlimited copy's limits off until a configured replacement is observed. Placement still uses
the node's available capabilities, and legacy/rootful reporting retains its existing contract.

## Comparison method

`control/bench/tiers.ts one-box|image output-prefix` runs all three tiers from the same release
source and build profile on the same runner. Container and process are the reference cohorts
and wasm is the added candidate. Historical K12 measurements remain separate; they are not
substituted for a current reference. Both existing CI jobs upload raw samples on failure too.

Each requested latency and incremental memory cohort has 30 samples; nearest-rank p50 and
p90 are computed from every successful sample, with partial counts retained on failure.
Concurrent admission records all 30 attempted waves, including capacity refusals;
only waves that admit all 50 enter the full-50 latency and memory percentiles.
Cold creation uses a new sandbox identity, no template and a locally available runtime image.
It includes API admission and running acknowledgement. `true` and `echo hi` use the real agent
through the edge. Cold and 50-way cohorts use an explicit common 256 MiB / 100 CPU-millis unit;
Hot and package cohorts use two units, 512 MiB / 200 CPU-millis. These measurement
configurations are not a capacity claim or replacement for the product's normal unit defaults. A declared 2× CPU admission
overcommit lets fifty idle 100-CPU-millis sandboxes fit the four-core runner; the reports record it. Node memory capacity uses the actual host with its normal reservation;
fifty idle sandboxes request 12,800 MiB on the observed 16 GiB-class runner.
Enforcement differs:
container uses cgroups, process uses native limits and priority, and wasm uses virtual heap/process
bounds in a shared worker without a per-VM CPU-rate quota. Equal requests do not imply identical
host enforcement.

Each shell emits a validated readiness marker before its memory sample. The node baseline is
refreshed after each deletion, while its initial fixed overhead is recorded separately.

Memory is proportional resident memory from Linux `smaps_rollup`, in bytes, including the
node's process tree and container workload trees. Reports distinguish fixed node memory
from incremental idle, shell and 50-sandbox memory. The ordinary-image tree includes embedded
services, so its fixed overhead is not a per-VM claim. Shared executable pages count once
proportionally; retained allocator pages and host noise remain in the samples, including
negative incremental readings. Absolute idle and running-shell context PSS are retained too:
Wasm disposal can release a prior shell's heap during the next sample, making its paired
increment negative. Such a delta is not a negative per-VM footprint. Absolute context PSS
includes the fixed node/services and retained allocator memory, rather than attributing every
resident page to the one active VM. The 50-way latency ends when every creation reports running;
its memory read and deletion are outside that latency timer.
Every wave issues all 50 requests without retry or downsizing. After all responses settle,
`concurrentCapacity` records the requested count, confirmed running count, refused count,
elapsed time, baseline and current context PSS, paired PSS delta, and each refusal's code,
HTTP status, message and count, **before deleting any successful creation**. A structured SDK
`no_room` response produces `capacity_refused`; the tier's final status is `capacity_limited`
if any wave is refused. This is a successful measurement. Later waves and tiers still run.
Partial waves do not count as full-50 latency successes. All other creation errors, invalid
running results, missing memory readings and failed cleanup remain fatal. The public API also
uses `no_room` for unavailable admission, so a refusal is an observed admission result, not proof
that physical RAM alone set the limit or that the fitted count is a universal maximum.
After the checkpoint, teardown deletes every admitted sandbox once, in sequence, outside the
creation timer and memory reading. All fifty creation requests remain concurrent. Teardown
does not add a second fifty-way mutation load; every delete is attempted and any failure is
fatal, without retry. Raw failures retain nested SDK phase/status/code/public-message evidence
while omitting request details, private error text and stacks.
The existing one-box and ordinary-image jobs upload these raw records and append every wave's
running/refused counts, memory and refusal to their GitHub job summaries, including on failure.
The one-box collector reuses its node across all thirty 50-way batches. Its temporary listener
pool is derived below the host kernel's ephemeral range so outgoing connections cannot race
listener allocation. It has 2,048 ports and retains the normal ten-minute quarantine, allowing
all 1,500 concurrent-cohort creations plus scalar cohorts without immediate port reuse.
The ordinary-image collector prepares a fresh context outside each 50-way
timer on all three tiers: the product's unchanged ten-minute port quarantine and thousand-port
pool cannot immediately reuse ports for 1,500 creations. Image startup and baseline memory
sampling are outside this creation timer; each sample still creates fifty sandboxes at once.
New image state likewise chooses a thousand-port pool below the kernel's outgoing range;
stored explicit and legacy implicit pools remain stable across restarts.
The ordinary-image comparison supplies an explicit bounded edge profile: 64 simultaneous
greetings per address, a burst of 128 connections and 64 new connections per second. This
admits the fifty-client experiment without disabling any guard. Default/forced-Wasm goal
cohorts retain the normal edge limits. Raw reports identify the comparison profile.

### Capacity evidence before refusal reporting

For head `795abeedc9d599f296e29f00e47609812c1c8683`, the
[PR run, attempt 1](https://github.com/cubepals/pryx/actions/runs/42227512991/attempts/1) measured merge source
`0b15b8e7b4f9ad711488c5c157e2dfd0f2f391ec`: it completed 16 full-50 Wasm
waves, then aborted on wave 17 with HTTP 503, `no_room`, "No node can complete this request".
The old collector did not retain that wave's admitted count or PSS: **both are unavailable**.
The wave number is not an admitted-sandbox count. Its preceding approximately 680 MB readings
are total context PSS, rather than a footprint for each sandbox.

The [push run of that head](https://github.com/cubepals/pryx/actions/runs/42227505210)
did fit all 50 in all 30 waves on each tier. Its
[raw artifact](https://github.com/cubepals/pryx/actions/runs/42227505210/artifacts/13513821935)
records the following; memory includes the context and fixed services. This successful run
does not fill in the missing observations from the failed run.
Both raw cohorts are retained under [measurements/k14](checks/measurements/k14/).

| One-box tier | Requested per wave | Running per completed wave | Full waves | Refusals observed | Context PSS MiB, p50 / p90 |
| --- | ---: | ---: | ---: | --- | ---: |
| Container | 50 | 50 | 30/30 | None | 53.00 / 53.15 |
| Process | 50 | 50 | 30/30 | None | 55.01 / 55.49 |
| Wasm | 50 | 50 | 30/30 | None | 668.63 / 673.10 |

The new collector keeps this full-50 path wherever it fits and records partial or zero-admission
waves with their own measured memory and refusal instead of terminating the comparison.

### Measured capacity refusal with the new collector

A single real Wasm wave on the local workspace at source
`9ebc6da8a14b095d64ffa2f73398838cf487a6d8` validates the refusal path end to end:

| Context / tier | Waves | Requested | Confirmed running | Refused | Baseline PSS MiB | PSS before cleanup MiB | Delta MiB | Refusal |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| Local host / Wasm | 1 | 50 | 31 | 19 | 80.84 | 150.37 | 69.53 | 19 × HTTP 503 `no_room`: "No node can complete this request" |

All 50 responses settled in **128,992.20 ms**. The outcome is `capacity_refused`, the final
report status is `capacity_limited`, cleanup completed and the collector exited **0**. The
31 running sandboxes were still present when memory and refusal evidence were checkpointed.
This partial wave contributes **zero full-50 latency samples**. Its admitted count describes
this observation, rather than establishing a physical memory ceiling or a runner-independent
maximum. This is one local validation wave, separate from the CI jobs' thirty-wave comparisons.

The [raw report](checks/measurements/k14/local-capacity-9ebc6da/tiers-one-box-wasm.json) records the
host, source, binary hash, unit profile, CPU admission overcommit, exact bytes and refusal.
The [driver and invocation](checks/pr8-capacity-local-command.txt) use the benchmark collector,
real SDK/API/Postgres/node/sidecar and unchanged `tier-memory.py`. The workspace has no `sudo`
and cannot install it; all measured Wasm processes belong to the current UID, so the driver
reads that owned process tree directly and refuses missing PSS. This fresh node has no prior
scalar/package workload or retained allocator history; its PSS is separate from the historical
reused-node CI memory above. Neither set of totals measures memory per VM. Raw output and the
rendered [capacity summary](checks/pr8-capacity-local-summary.md) retain the counts and refusal.

### Failed teardown remains a failure

The [7934065 push run](https://github.com/cubepals/pryx/actions/runs/42254735382) records a separate
ordinary-image failure after the first Wasm wave had admitted all 50 with zero admission
refusals. Its creation time was 66,869.40 ms and pre-cleanup context PSS was 554.99 MiB.
The simultaneous delete fan-out then returned HTTP 503 `no_room`; the tier remained
`incomplete` and the job failed. This public error does not identify the underlying host or
database cause. The [raw report](checks/measurements/k14/42254735382/tiers-image-wasm.json) and
[log excerpt](checks/pr8-capacity-image-cleanup-before.txt) retain that failed observation;
the one recorded wave is not presented as a completed thirty-wave comparison.
Container, process, normal-default and forced-Wasm hello cohorts from that run retain their
complete samples beside it. The teardown follow-up keeps every delete and its fatal assertion,
serializes the unmeasured deletion phase on every tier/context, and verifies that the fifty
creation requests still overlap. The actual admission-refusal observation above stays separate.

The corresponding [PR run](https://github.com/cubepals/pryx/actions/runs/42254747830)
instead failed in the container fork cohort after 22 timings: Docker's start response exceeded
the unchanged 20-second deadline. Its [raw reports](checks/measurements/k14/42254747830/) retain the
incomplete container result and complete process, Wasm and hello cohorts; the
[failure log](checks/pr8-container-fork-before.txt) does not establish that container's final state.
The runtime now inspects once after a start timeout and succeeds only if Docker confirms
`Running`, without repeating start. A real Docker transport-fault regression withholds the
response after Docker accepts the mutation: it fails before the fix and passes after it,
confirming a single start request and the actual running container. Other states and errors
remain failures; this changes neither capacity classification nor benchmark budgets.

Snapshots and forks use exactly 100,000,000 bytes of repeated byte 42, validated before capture
and after fork. This compressible fixture measures the same workload on each tier; it is not
an incompressible-data throughput claim. Snapshot timing ends at the API acknowledgement of
local capture; background object-store upload is not a remote-durability timing.
Each package installation uses a fresh sandbox prepared outside its timer. Native pip installs
`peppercorn==0.6` into a new target and verifies its import path, version and behavior. The wasm
micropip shim lacks `--target`, so it installs into a fresh VM after an assertion that the package
is absent; creation, that assertion, import verification and deletion are outside the install timer.
The sidecar resolves npm's CLI from the operator's host PATH; the image includes that userland.
Npm installs `is-number` at version 7.0.0 with scripts disabled and verifies its behavior. Commands and exact
package versions are recorded in the collector source. An unavailable tool
has an explicit n=0 reason, never an invented timing or a passing test.

The no-engine process and wasm image cohorts use ordinary `docker run` without privileged mode,
added capabilities, security options, a Docker socket or host mounts. A container sandbox
requires an engine; its reference runs on the same runner's host node and is explicitly marked
as that different topology. A container tier inside an engine-free image is unavailable.

`control/bench/tiers-default.ts` measures the actual image without a tier override, and a
separate invocation measures the forced wasm candidate. Both use normal product resource
defaults and record 30 SDK create-to-hello samples plus 30 compiled `pryx run -- echo hello`
samples separately. It also records create acknowledgement and first-byte components;
whole-command timings include waiting for exit, and CLI totals include cleanup.
The 100 ms create-to-output target is assessed separately below; it is missed on both
candidates. These are fresh sandboxes, without a pre-created VM or warmed command.

## Capabilities

| Capability | Container | Process | wasm |
| --- | --- | --- | --- |
| Arbitrary image | Yes | No | No; explicit 422 image problem |
| Native Linux binaries | Image architecture | Host architecture and policy | No; compatible WASM software |
| Python | Install in image | Bundled CPython | Embedded Pyodide |
| Node | Install in image | Bundled Node | Embedded V8 compatibility runtime |
| Shell | Static agent fallback | Static agent shell | Verified agentOS coreutils shell |
| Isolation | Shared host kernel, namespaces and cgroups | Landlock, seccomp, process groups; dedicated UIDs when root; shared host network, mutually trusted Internet workloads | Virtual kernel, WebAssembly and V8; shared unprivileged sidecar |
| Kernel per sandbox | No | No | No native kernel; a virtual kernel per VM |
| CPU enforcement | Per-sandbox cgroup quota and weight | Admission reservation and best-effort nice; hard CPU limits/weights refused | Admission reservation; hard CPU limits/weights refused |
| Sleep retains files | Yes | Yes | SQLite root and explicit persistent data mount |
| Application ingress | TCP/UDP according to host policy | Host TCP according to policy | HTTP over TCP; no UDP, raw TCP, TLS passthrough or app WebSockets |
| Sleep retains processes and terminals | Frozen container | Suspended process group | No; VM is disposed and recreated |

agentOS 0.4.7 has no per-VM suspend operation in its published client. Suspending its shared
sidecar would pause unrelated sandboxes, so it cannot implement sleep that retains processes.
The additive `memory-sleep` guarantee is false on this tier; data sleep still works. The collector
records that unsupported alternative with n=0 and measures disposal plus recreation with
filesystem validation instead.

## Complete comparison

The tables pin the complete [raw reports from CI 42166712195](checks/measurements/k14/42166712195/),
source `becd30a2051e59ecde599a22f3ff5e868bcd59e3`. Every tier completes every requested cohort,
including thirty verified pip installs, thirty verified npm installs and thirty batches of
fifty creations. Recomputing nearest-rank percentiles from the raw samples matches every
reported value. Later PID-identity and port-ownership hardening have failing-before regressions and full Rust
verification; the final branch workflow repeats collection on that code. The table remains
pinned to its measured source rather than relabeling old samples with a new commit.

| Runner | CPU model | Visible cores | RAM bytes | Kernel | Bun |
| --- | --- | ---: | ---: | --- | --- |
| one-box | AMD EPYC 9V74 80-Core Processor | 4 | 16,760,348,672 | 7.0.0-1017-azure | 1.4.4 |
| image | AMD EPYC 7763 64-Core Processor | 4 | 16,760,344,576 | 7.0.0-1017-azure | 1.4.4 |

Each context compares all three tiers sequentially on its own single runner. The two contexts
use separate runners; cross-context differences are not attributed solely to the deployment.
All six use the release daemon SHA-256
`e55ae6475d085d1ba74bbad7ba20b2631bda9a0a6bdec764875f5095cd99b9e2`.
The one-box release image is
`sha256:2ed5e4ad69a1c34d3fcd13951a329cac025e9171d6536dee4c10e2d4e0de3b93`;
the ordinary image is
`sha256:20353f09155fb93e1875d5078be2de057f287c5854aa74573016d71429dfba24`.
Both contain the same checksum-pinned agentOS 0.4.7 distribution described in
[agentos.md](agentos.md). Resource requests, enforcement differences, context resets,
port quarantine and the bounded comparison gateway profile are specified above and in each
raw report.

### One-box services

All cells are p50 / p90; every requested cohort has n=30.

| Latency (ms) | Container | Process | Wasm |
| --- | ---: | ---: | ---: |
| Cold create to running | 700.59 / 982.53 | 803.10 / 833.25 | 1,922.30 / 1,986.85 |
| Agent true | 1.44 / 1.59 | 1.78 / 1.85 | 434.69 / 446.06 |
| Echo first byte | 1.43 / 1.68 | 1.63 / 1.81 | 421.10 / 429.65 |
| Sleep then wake | 417.39 / 1,000.66 | 887.07 / 992.57 | 994.51 / 1,034.36 |
| Snapshot 100 MB | 665.22 / 1,232.55 | 568.88 / 988.59 | 686.02 / 923.79 |
| Fork 100 MB | 772.08 / 1,730.24 | 941.49 / 955.35 | 1,087.66 / 1,136.90 |
| pip install | 1,494.94 / 1,520.58 | 573.22 / 584.50 | 3,659.82 / 4,041.77 |
| npm install | 2,464.77 / 2,515.95 | 416.03 / 439.78 | 4,971.42 / 5,194.99 |
| 50 concurrent to running | 5,085.35 / 6,110.38 | 2,646.52 / 3,082.58 | 68,308.21 / 85,753.49 |

| Memory (MiB) | Container | Process | Wasm |
| --- | ---: | ---: | ---: |
| Idle context PSS | 18.30 / 18.38 | 18.80 / 18.80 | 334.44 / 368.13 |
| Idle paired PSS delta | 5.21 / 5.25 | 6.19 / 6.20 | -39.60 / -39.28 |
| Running-shell context PSS | 21.14 / 21.24 | 19.29 / 19.29 | 426.45 / 464.48 |
| Running-shell paired PSS delta | 8.07 / 8.11 | 6.68 / 6.69 | 49.90 / 64.04 |
| 50-sandbox context PSS | 52.79 / 52.87 | 54.18 / 54.28 | 679.56 / 683.75 |
| 50-sandbox paired PSS delta | 37.29 / 37.42 | 39.84 / 39.91 | 2.44 / 5.57 |

Fixed context PSS (n=1, MiB): container 11.56, process 10.77, wasm 64.84.

### Ordinary image

All cells are p50 / p90; every requested cohort has n=30.

| Latency (ms) | Container | Process | Wasm |
| --- | ---: | ---: | ---: |
| Cold create to running | 629.19 / 653.16 | 777.98 / 786.75 | 1,791.04 / 1,817.37 |
| Agent true | 1.96 / 2.06 | 2.70 / 2.84 | 466.69 / 472.86 |
| Echo first byte | 1.90 / 2.38 | 2.55 / 2.95 | 452.86 / 462.80 |
| Sleep then wake | 600.61 / 1,026.34 | 819.61 / 1,025.14 | 980.40 / 1,029.71 |
| Snapshot 100 MB | 388.85 / 440.72 | 241.74 / 248.19 | 461.73 / 471.22 |
| Fork 100 MB | 738.72 / 771.98 | 961.51 / 985.50 | 1,000.84 / 1,044.90 |
| pip install | 2,055.52 / 2,172.08 | 564.06 / 694.86 | 4,969.76 / 5,127.07 |
| npm install | 3,261.74 / 3,316.12 | 569.64 / 584.98 | 6,565.43 / 6,825.03 |
| 50 concurrent to running | 7,116.82 / 7,146.71 | 1,915.59 / 2,905.34 | 68,769.87 / 70,949.82 |

| Memory (MiB) | Container | Process | Wasm |
| --- | ---: | ---: | ---: |
| Idle context PSS | 20.14 / 20.16 | 333.51 / 339.76 | 654.07 / 694.60 |
| Idle paired PSS delta | 5.20 / 5.21 | 6.80 / 9.36 | -40.11 / -37.91 |
| Running-shell context PSS | 22.99 / 23.07 | 334.08 / 339.83 | 742.80 / 798.99 |
| Running-shell paired PSS delta | 8.03 / 8.13 | 7.35 / 8.95 | 50.93 / 68.91 |
| 50-sandbox context PSS | 53.34 / 53.40 | 429.88 / 433.31 | 548.85 / 555.47 |
| 50-sandbox paired PSS delta | 39.92 / 39.98 | 118.86 / 123.59 | 182.72 / 195.10 |

Fixed context PSS (n=1, MiB): container 13.43, process 307.57, wasm 365.40.

## Measured default and create-to-output target

Keep **process** as the image default when no Docker socket is usable. In the ordinary-image
comparison its cold-create p50 is **777.98 ms**, versus **1,791.04 ms** for Wasm; hot echo first
byte is **2.55 ms** versus **452.86 ms**; fifty creations finish in **1,915.59 ms** versus
**68,769.87 ms**. Its idle context PSS is **333.51 MiB** versus **654.07 MiB**, and running-shell
context PSS is **334.08 MiB** versus **742.80 MiB**. Wasm wins none of those selection measures.
These memory totals include services; the paired deltas and fixed overhead are reported above.
Process also runs native host binaries. Wasm offers its virtual-kernel boundary and compatible
Python/JavaScript software as an explicit option; a shared worker remains a shared fault domain.
The result does not establish a universal winner for arbitrary applications or host policies.
A Docker host still defaults to container, preserving arbitrary-image support.

The following separate cohorts use the ordinary image, normal **512 MiB / 250 CPU-millis**
resource defaults and normal gateway limits. There is no tier override for process; Wasm is
explicitly forced to verify the brief's requested hello path. Every command prints exactly
`hello` plus newline, with zero exit status and no stderr; cleanup is checked. All cells are
p50 / p90 milliseconds, n=30 for every cell.

| Path | Implicit process default | Forced Wasm |
| --- | ---: | ---: |
| SDK create acknowledgement | 983.06 / 987.05 | 2,048.53 / 2,078.72 |
| SDK first echo after create | 7.67 / 8.14 | 907.84 / 921.21 |
| SDK create to first byte | 990.38 / 994.28 | 2,960.64 / 2,983.88 |
| SDK create to completed echo | 990.50 / 994.42 | 2,976.27 / 3,002.76 |
| Compiled CLI to first byte | 968.53 / 984.71 | 2,947.15 / 2,976.99 |
| Compiled CLI total including cleanup | 998.71 / 1,016.82 | 3,000.34 / 3,033.40 |

**The 100 ms goal is missed.** Forced-Wasm SDK create-to-first-byte p50 is **2,960.64 ms**;
compiled `pryx run -- echo hello` first-byte p50 is **2,947.15 ms**. The normal process default
is **990.38 ms** through the SDK and **968.53 ms** through the compiled CLI. These measurements
include fresh creation and API running acknowledgement. The observed Wasm components are
**2,048.53 ms** to create acknowledgement and **907.84 ms** from acknowledgement to first echo;
process is **983.06 ms** and **7.67 ms**, respectively. Percentiles of components do not add
exactly to the percentile of their sum. The pinned sidecar's VM/root initialization and first
command through its process/output bridge are on this measured path; the millisecond boot
claim does not describe end-to-end pryx creation. These component timings locate the cost,
without claiming an isolated profiler attribution. No pre-created sandbox, missing assertion
or relaxed product limit is used to claim the target passed.

## Paired budget checks

The same image runner also repeats the existing before/after collectors. All samples and
source/binary identities are retained beside the tier reports. K9 compares its pinned before
and after products; K11 compares its pinned pre-agent baseline with this source; K12 compares
same-source container and process/image deployments. K12's preceding daemon-only image has
no one-image API, so image startup has an explicit n=0 before reason rather than an invented
reference. Every numeric cell below has n=30. Values are milliseconds at the stated percentile;
the limits apply to the after product.

| Budget | Percentile | Before | After | Required after |
| --- | --- | ---: | ---: | ---: |
| K9 public exec | p50 | 45.13 | 46.80 | < 50 |
| K9 template create | p50 | 186.62 | 181.93 | < 1,000 |
| K9 suspended wake | p50 | 27.90 | 27.56 | < 1,000 |
| K11 agent run | p90 | 103.88 | 2.24 | < 20 |
| K11 first byte | p90 | 101.59 | 2.11 | < 10 |
| K11 expose/200 | p90 | 107.28 | 5.71 | < 200 |
| K12 process run | p90 | 2.20 | 2.82 | < 20 |
| K12 first byte | p90 | 2.12 | 2.93 | < 10 |
| K12 expose/200 | p90 | 7.57 | 60.60 | < 200 |
| K12 image startup | p90 | Unavailable, n=0 | 6,475.04 | < 10,000 |

All specified K9/K11/K12 after budgets pass in this complete checkpoint. K14's distinct cold
create-to-output goal above remains a documented miss. Historical partial and failed cohorts
remain under [measurements/k14](checks/measurements/k14/); they are not substituted for these complete
samples. The final workflow must pass its tests and repeat the required comparisons before
the pull request is opened; its exact final commit and run are linked in the pull request.

## K16 policy and convenience tools

Hostname egress allow/deny is available only on container nodes that announce
`egress-hostnames` after root firewall installation. Each such sandbox has a dedicated bridge
and HTTP/TLS gateways; no sandbox proxy setting can bypass the firewall. Process and wasm
return an explicit tier refusal. `network:none` remains the all-tiers network boundary.
Workdir uses the `workload-options` feature; user selection is restricted to the operator
container user. Metrics use cached tier counters and preserve null where unavailable.
These policy fields are refused when the destination no longer announces the feature,
including on a wake whose specification is unchanged. They are never silently dropped.
The default Internet network remains compatible with old nodes.
Git requires an image/userland installation on native tiers. The verified optional wasm Git
package is staged with the distribution and loaded only on request. Stateful shells use pipes
on every tier; [K16 measurements](checks/k16.md) retain the before/after raw samples.

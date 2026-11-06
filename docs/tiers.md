<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

# Sandbox tiers

The image contains container, process and wasm runtimes. A usable Docker socket selects
container; `PRYX_DEFAULT_TIER=container|process|wasm` is an operator override. The no-engine
selection remains process while the K14 comparison is collected. This document does not yet
claim a measured final selection.

## Comparison method

`control/bench/tiers.ts one-box|image output-prefix` runs all three tiers from the same release
source and build profile on the same runner. Container and process are the reference cohorts
and wasm is the added candidate. Historical K12 measurements remain separate; they are not
substituted for a current reference. Both existing CI jobs upload raw samples on failure too.

Each requested latency and incremental memory cohort has 30 samples; nearest-rank p50 and
p90 are computed from every successful sample, with partial counts retained on failure.
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
The 100 ms create-to-output target will be assessed from these measurements, with any miss
and its observed causes recorded here.

## Capabilities

| Capability | Container | Process | wasm |
| --- | --- | --- | --- |
| Arbitrary image | Yes | No | No; explicit 422 image problem |
| Native Linux binaries | Image architecture | Host architecture and policy | No; compatible WASM software |
| Python | Install in image | Bundled CPython | Embedded Pyodide |
| Node | Install in image | Bundled Node | Embedded V8 compatibility runtime |
| Shell | Static agent fallback | Static agent shell | Verified agentOS coreutils shell |
| Isolation | Shared host kernel, namespaces and cgroups | Landlock, seccomp, process groups; dedicated UIDs when root | Virtual kernel, WebAssembly and V8; shared unprivileged sidecar |
| Kernel per sandbox | No | No | No native kernel; a virtual kernel per VM |
| Sleep retains files | Yes | Yes | SQLite root and explicit persistent data mount |
| Application ingress | TCP/UDP according to host policy | Host TCP according to policy | HTTP over TCP; no UDP, raw TCP, TLS passthrough or app WebSockets |
| Sleep retains processes and terminals | Frozen container | Suspended process group | No; VM is disposed and recreated |

agentOS 0.4.7 has no per-VM suspend operation in its published client. Suspending its shared
sidecar would pause unrelated sandboxes, so it cannot implement pryx sleep. The collector
records that unsupported alternative with n=0 and measures disposal plus recreation with
filesystem validation instead.

## Evidence and selection

The full K14 CI comparison and final default selection are pending. No budget is claimed to
have passed until its raw report is present and its cohort is complete.

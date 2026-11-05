<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

# One-box speed

## K11 TCP_NODELAY cohort

Run [42015142557](https://github.com/cubepals/pryx/actions/runs/42015142557) completed the
paired collector on one two-vCPU AMD EPYC 7763 runner, then failed sleep/wake acceptance.
It is not a green workflow. Every available path has n=30; missing baseline paths remain n=0.
The static agent digest and daemon digest are recorded in the
[before](checks/agent-speed-nodelay-before.json) and
[after](checks/agent-speed-nodelay-after.json) reports. The before product is b91ee23 and
the after product is 008a865. No sample, including the individual slower outliers, is dropped.

| Path | Before p50/p90 ms | After p50/p90 ms | Budget |
| --- | --- | --- | --- |
| Run through edge | 103.15 / 177.71 | 2.32 / 2.65 | < 20 ms: met |
| First output byte | 98.00 / 170.02 | 2.30 / 2.90 | < 10 ms: met |
| Start, expose, HTTP 200 | 108.19 / 181.08 | 5.02 / 7.62 | < 200 ms: met |
| Agent from node loopback | unavailable | 1.26 / 1.48 | measured |
| Terminal round trip | unavailable | 21.19 / 21.81 | measured |
| Write 100 MB | unavailable | 750.92 / 898.94 | measured |
| Read 100 MB | unavailable | 2256.98 / 2289.35 | measured |

This differs from the initial cohort's CPU and execution conditions, so its loopback and
terminal rows do not isolate a causal TCP_NODELAY improvement. The read row is slower than
the initial cohort and has no specified budget. Final acceptance and a final cohort remain open.

## K11 initial agent cohort

Run [42007702494](https://github.com/cubepals/pryx/actions/runs/42007702494) completed paired
agent measurements, then failed sleep/wake and scratch-image acceptance. These are initial
measurements, not a claim of completed acceptance. Both cohorts used the same two-vCPU AMD
EPYC 9V45 host. Raw samples and daemon identities are retained in
[before](checks/agent-speed-initial-before.json) and [after](checks/agent-speed-initial-after.json).
The baseline is b91ee23 and the after product is 83c7e2d, using the static release agent.

| Path | Before p50/p90 ms | After p50/p90 ms | Budget |
| --- | --- | --- | --- |
| Run through edge | 60.98 / 92.04 | 1.49 / 1.82 | < 20 ms: met |
| First output byte | 57.70 / 89.36 | 1.45 / 2.11 | < 10 ms: met |
| Start, expose, HTTP 200 | 61.98 / 96.50 | 3.34 / 4.41 | < 200 ms: met |
| Agent from node loopback | unavailable | 41.64 / 42.07 | measured |
| Terminal round trip | unavailable | 41.01 / 41.14 | measured |
| Write 100 MB | unavailable | 965.87 / 995.25 | measured |
| Read 100 MB | unavailable | 699.98 / 722.92 | measured |

Every available cell has n=30. Missing agent-only baseline paths have n=0, since those
transports did not exist. The baseline expose comparison uses a port declared at creation,
so it excludes dynamic reservation. Both data directions validate 100,000,000 bytes. A later
cohort must measure TCP_NODELAY and the acceptance corrections before final reporting.

K9's collector runs real control, edge and pryxd together, with Postgres and S3 on the same
CI host. No local Docker/e2e measurement is claimed. The paired comparison is from
[run 41975259790](https://github.com/cubepals/pryx/actions/runs/41975259790): before product
fe77e673364580efe372a4f685837f219ae7a0a5 and after product
7cd80e9bff7be320d10993aeadb8e73f3f6f65c8, using the same collector and host. Both measurement
steps passed; the later Rivet/celld integration step failed, so the workflow is not green.
[Before](checks/speed-before.json) and [after](checks/speed-after.json) retain every raw sample,
daemon digest and advertised feature list. Only the after daemon advertises exec-stream.

```sh
DATABASE_URL=... S3_TEST_ENDPOINT=http://127.0.0.1:9000 PRYXD_SUDO=1 \
  PRYXD_BIN=$PWD/target/debug/pryxd bun control/bench/speed.ts before speed-before.json
```

Each path collects 30 samples and reports nearest-rank p50/p90, retaining every raw duration.
The collector records the commit, kernel, CPU, Bun version, image, exact data size and node-call
timing. It validates that create/start reach running and that suspended wake actually resumes.
Cleanup, initial setup and template warming are excluded in the corrected common collector.
The [initial baseline](checks/speed-initial-before.json) included reference clearing/verification
in its cold timer; the paired rows below exclude it. Snapshot/fork use 100,000,000 zero
bytes, so data compressibility and the host filesystem's copy behavior matter.

Image creation is measured both from an already cached image and with its local image
references removed before each sample. Shared Docker layers and OS page caches can remain;
the latter is an image-reference cold start, not a claim that every physical cache is empty.
Template creation begins only after its node copy is ready. No failed/censored sample is
silently dropped. The common collector loads the pinned product SDK/control/edge for the before phase and
exercises the current handle's `box.run('true')` for the after phase. It records both the
product commit and the collector commit; neither phase patches the measured product code.

| Path | Before n | Before p50/p90 | After n | After p50/p90 | Budget |
| --- | ---: | --- | ---: | --- | --- |
| Public API exec | 30 | 30.82 / 33.42 ms | 30 | 46.04 / 46.42 ms | p50 < 50 ms: met |
| Control-to-node exec | 30 | 29.77 / 33.30 ms | 30 | 32.70 / 36.23 ms | measured |
| Create from cached image | 30 | 141.72 / 156.11 ms | 30 | 141.25 / 150.07 ms | measured |
| Create with cold image reference | 30 | 1417.34 / 2372.72 ms | 30 | 1380.38 / 2295.81 ms | measured |
| Create from warm template | 30 | 170.26 / 181.22 ms | 30 | 154.89 / 162.15 ms | p50 < 1 s: met |
| Wake from suspended | 30 | 18.71 / 24.08 ms | 30 | 21.38 / 24.89 ms | p50 < 1 s: met |
| Start from stopped | 30 | 109.03 / 117.65 ms | 30 | 114.63 / 121.67 ms | measured |
| Snapshot 100 MB | 30 | 1009.33 / 1246.25 ms | 30 | 1035.35 / 1060.81 ms | measured |
| Fork that snapshot | 30 | 1260.32 / 1280.95 ms | 30 | 1010.23 / 1025.15 ms | measured |

Baseline machine: Linux x86_64, Azure kernel 7.0.0-1017-azure, two AMD EPYC 9V45 vCPUs,
Bun 1.4.4. Public exec (46.04 ms), warm-template creation (154.89 ms) and suspended wake
(21.38 ms) meet their budgets in this cohort. Public exec remains slower than its paired
buffered baseline, so this comparison does not show an exec latency improvement. The original
streaming cohort missed the exec budget at 57.36 ms; its [before](checks/speed-stream-first-before.json)
and [after](checks/speed-stream-first-after.json) reports remain available. The final change
awaits Docker process exit under the command deadline with four-millisecond inspection steps;
runtime, transport and stream overhead remain for K11 to remove. Node-call timings are in the raw
report. The after implementation combines durable-operation insert/read and claim/activity
updates into one database round trip each. The baseline already had authenticated keep-alive
connections and used the node start response without waiting for a heartbeat; these are retained.

Run 41957179796's after samples are discarded: its shared Cargo target reused the baseline
executable, and the live-output assertion exposed buffered fallback. Both builds now clean the
daemon package and the collector asserts advertised features and records the executable digest.
An expired publishing credential delayed the signed fix. The valid paired run now proves live
TypeScript/CLI output and the four-command quickstart. Run 41969685155 then passed Python's
async/sync handles and both literal SDK README examples after the same-second trash fix.
Run 41975259790 also passes the project-capacity fixture, all 20 control end-to-end tests and
all ten isolated-node tests. Full workflow acceptance remains open on the Rivet/celld integration step.

## K10 project admission

These are local control-overhead samples with real Postgres and the established in-memory node,
on the same development host. They do not measure Docker execution or host capacity. Each
sample creates from the same explicit 128 MiB API input (K10 rounds it to one configured unit),
waits for running, then deletes outside the timer. One initial warmup is excluded; all 30 timed
observations are retained. The pinned before source is 36470599034e8a7f1e7f944b809f10267f3331d4.
The after source was uncommitted during collection; its source digest is in the report.

| Path | Before n | Before p50/p90 | After n | After p50/p90 |
| --- | ---: | --- | ---: | --- |
| Public create, Postgres and node double | 30 | 47.19 / 89.51 ms | 30 | 47.53 / 99.84 ms |

[Before](checks/projects-before.json) and [after](checks/projects-after.json) use the same
collector and adjacent runs. The observed median increase is 0.35 ms; p90 increases 10.33 ms.
This shared development host is not a controlled container-runtime benchmark. K10 specifies no
latency budget. Initial exploratory samples are retained separately: the
[initial before](checks/projects-initial-before.json), [first admission implementation](checks/projects-admission-first.json),
and [optimized trial](checks/projects-optimized-trial.json). A duplicate project transaction on
newly reserved creates was removed; an existing intent with a released reservation still
reacquires quota before starting.

```sh
git worktree add --detach /tmp/pryx-k10-before 36470599034e8a7f1e7f944b809f10267f3331d4
(cd /tmp/pryx-k10-before && bun install --frozen-lockfile)
DATABASE_URL=... PRYX_PROJECT_SOURCE=/tmp/pryx-k10-before \
  bun control/bench/project-overhead.ts before docs/checks/projects-before.json
DATABASE_URL=... bun control/bench/project-overhead.ts after docs/checks/projects-after.json
```

The installed one-box workflow runs `sudo "$(command -v bun)" control/bench/projects-one-box.ts`.
It records actual node unit capacity, then creates five stopped sandboxes for each of ten
projects and runs every one, rotating active work within a quota no larger than the measured
capacity. A sixth admission must return the full 403 maxUnits body, then the group is cleaned.
It does not relax CPU or memory admission. The [first capacity observation](checks/projects-capacity.json)
is 7,932 MiB/two cores: four free units after 2,048 MiB and 1,000 CPU millis of host reservations.
The first five-active-sandbox fixture hit physical capacity before its project limit. The
[corrected exercise](checks/projects-one-box.json) passed in CI run 41975259790: all 50 sandboxes
ran, with four active at once and ten exact maxUnits refusals. Capacity n=2; completed
50-sandbox exercise n=1. A physical 16 GiB host is
unavailable here; the pure capacity tests cover 31 free units for a 16 GiB/8-core reference with
512 MiB/250-millis host reservations, and seven for the same memory with two cores. Those are
arithmetic checks, not physical host measurements.

## Process tier and one-image startup

The K12 collector is `control/bench/process-speed.ts`. CI records paired n=30 container and
process cohorts on the same runner and source commit, with CPU, kernel, image ID, static-agent
hash, dataset size, raw samples and completion status. This compares deployment tiers; it does
not attribute the difference to a single code optimization. The earlier daemon-only image has
no all-in-one API, so its image-start baseline is explicitly unavailable (n=0).

The unchanged agent p90 budgets are run through edge under 20 ms, first byte under 10 ms and
start/expose/HTTP 200 under 200 ms. Image startup must answer the API in under 10 seconds.
CLI echo, signal sleep/wake, and snapshot/fork of 100 MB are measured in both cohorts. Cleanup
is outside the operation timer. No failing sample is discarded. The ordinary image is also
checked with the default Docker seccomp profile, no extra capabilities or engine socket, and
one shared UID under `--user`. The collector requires actual Landlock filesystem support.

These Docker cohorts have not run locally. Their CI reports remain pending; local fail-closed
kernel tests do not establish successful process isolation or any speed result.

Run 42035048409 supplies the initial real one-image cohort on a two-core AMD EPYC 9V74
runner: n=30, p50 6,691.32 ms and p90 6,977.85 ms, within the 10,000 ms startup budget.
The ordinary-container probes report default seccomp, no engine socket, no writable cgroup
and no creating of user/mount/PID/network namespaces; changing UID and the required Landlock
filesystem/network probes succeed. This cohort then fails sandbox admission on available disk,
before process-operation samples, so it is not successful process acceptance. Raw startup samples,
image/agent digests and the failure are retained in
[the initial process report](checks/process-speed-initial-after.json).

### K11 private-ingress cohort

Run [42040665809](https://github.com/cubepals/pryx/actions/runs/42040665809) measures the pinned
K11 baseline and d89960e on the same 2-CPU EPYC 7763 runner, n=30 each. This run passes both
TypeScript and Python agent acceptance, including network none, and the separate two-node
recovery and lease watchdog suites. Its process-tier admission and edge connection test still
fail; this is not a claim that the complete workflow passed. Raw
[before](checks/agent-speed-relay-before.json) and [after](checks/agent-speed-relay-after.json) samples
retain all observations.

| operation | before p50 / p90 ms | after p50 / p90 ms | budget p90 |
| --- | --- | --- | --- |
| run through edge | 103.64 / 177.56 | 2.29 / 2.66 | 20 ms |
| first output byte | 99.06 / 170.16 | 2.43 / 3.43 | 10 ms |
| start, expose, HTTP 200 | 108.48 / 185.97 | 5.29 / 11.08 | 200 ms |
| node loopback | unavailable on baseline | 1.92 / 2.94 | measured |
| terminal round trip | unavailable on baseline | 21.33 / 23.33 | measured |
| write 100 MB | unavailable on baseline | 3040.83 / 3105.22 | measured |
| read 100 MB | unavailable on baseline | 5634.01 / 5973.70 | measured |

All three stated latency budgets pass. Large-file throughput remains slower than the initial
cohort on a different runner; it is retained without a causal speedup claim.

### K12 first process compute cohort

Run [42055145022](https://github.com/cubepals/pryx/actions/runs/42055145022), source bbadbde,
reaches actual Landlock-restricted process compute in the ordinary image. Its paired
[container](checks/process-speed-compute-before.json) and
[process](checks/process-speed-compute-after.json) reports retain all samples. Both use the same
CI host and source. The process cohort stops at its first Python exposed-HTTP check, so later
operations are absent and the cohort/workflow is not complete.

| operation | container p50 / p90 ms | process p50 / p90 ms | result |
| --- | --- | --- | --- |
| image API startup | prior daemon-only image has no API, n=0 | 6604.52 / 6670.35 | n=30, under 10 s |
| run through edge | 2.25 / 2.99 | 2.44 / 2.69 | n=30, under 20 ms |
| first output byte | 2.32 / 2.92 | 2.37 / 3.05 | n=30, under 10 ms |
| start, expose, HTTP 200 | 4.60 / 34.32 | n=0, failed | not accepted |

The public/agent end-to-end step, including both reference MCP transports and sandbox MCP URL,
passes in this run. This does not substitute for the remaining process-image proof.

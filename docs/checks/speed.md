<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

# One-box speed

## K11 TCP_NODELAY cohort

Run [42015142557](https://github.com/cubepals/pryx/actions/runs/42015142557) completed the
paired collector on one two-vCPU AMD EPYC 7763 runner, then failed sleep/wake acceptance.
It is not a green workflow. Every available path has n=30; missing baseline paths remain n=0.
The static agent digest and daemon digest are recorded in the
[before](agent-speed-nodelay-before.json) and
[after](agent-speed-nodelay-after.json) reports. The before product is b91ee23 and
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
the initial cohort and has no specified budget. At collection time final acceptance remained open; the completed final cohort is recorded below.

## K11 initial agent cohort

Run [42007702494](https://github.com/cubepals/pryx/actions/runs/42007702494) completed paired
agent measurements, then failed sleep/wake and scratch-image acceptance. These are initial
measurements, not a claim of completed acceptance. Both cohorts used the same two-vCPU AMD
EPYC 9V45 host. Raw samples and daemon identities are retained in
[before](agent-speed-initial-before.json) and [after](agent-speed-initial-after.json).
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
[Before](speed-before.json) and [after](speed-after.json) retain every raw sample,
daemon digest and advertised feature list. Only the after daemon advertises exec-stream.

```sh
DATABASE_URL=... S3_TEST_ENDPOINT=http://127.0.0.1:9000 PRYXD_SUDO=1 \
  PRYXD_BIN=$PWD/target/debug/pryxd bun .benchmark-collector/control/bench/speed.ts before speed-before.json
```

Historical collectors are checked out verbatim at c1c9fa13578087d7e657b2d048a7619bef696619
in `.benchmark-collector`; measured source checkouts remain separately pinned in CI.
This keeps historical SDK execution exact without retaining those SDK surfaces in current code.

Each path collects 30 samples and reports nearest-rank p50/p90, retaining every raw duration.
The collector records the commit, kernel, CPU, Bun version, image, exact data size and node-call
timing. It validates that create/start reach running and that suspended wake actually resumes.
Cleanup, initial setup and template warming are excluded in the corrected common collector.
The [initial baseline](speed-initial-before.json) included reference clearing/verification
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
streaming cohort missed the exec budget at 57.36 ms; its [before](speed-stream-first-before.json)
and [after](speed-stream-first-after.json) reports remain available. The final change
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

[Before](projects-before.json) and [after](projects-after.json) use the same
collector and adjacent runs. The observed median increase is 0.35 ms; p90 increases 10.33 ms.
This shared development host is not a controlled container-runtime benchmark. K10 specifies no
latency budget. Initial exploratory samples are retained separately: the
[initial before](projects-initial-before.json), [first admission implementation](projects-admission-first.json),
and [optimized trial](projects-optimized-trial.json). A duplicate project transaction on
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
It does not relax CPU or memory admission. The [first capacity observation](projects-capacity.json)
is 7,932 MiB/two cores: four free units after 2,048 MiB and 1,000 CPU millis of host reservations.
The first five-active-sandbox fixture hit physical capacity before its project limit. The
[corrected exercise](projects-one-box.json) passed in CI run 41975259790: all 50 sandboxes
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

These Docker cohorts have not run locally. The complete guarded-process CI cohort is recorded
below; local fail-closed kernel tests do not establish successful process isolation or speed.

Run 42035048409 supplies the initial real one-image cohort on a two-core AMD EPYC 9V74
runner: n=30, p50 6,691.32 ms and p90 6,977.85 ms, within the 10,000 ms startup budget.
The ordinary-container probes report default seccomp, no engine socket, no writable cgroup
and no creating of user/mount/PID/network namespaces; changing UID and the required Landlock
filesystem/network probes succeed. This cohort then fails sandbox admission on available disk,
before process-operation samples, so it is not successful process acceptance. Raw startup samples,
image/agent digests and the failure are retained in
[the initial process report](process-speed-initial-after.json).

### K11 private-ingress cohort

Run [42040665809](https://github.com/cubepals/pryx/actions/runs/42040665809) measures the pinned
K11 baseline and d89960e on the same 2-CPU EPYC 7763 runner, n=30 each. This run passes both
TypeScript and Python agent acceptance, including network none, and the separate two-node
recovery and lease watchdog suites. Its process-tier admission and edge connection test still
fail; this is not a claim that the complete workflow passed. Raw
[before](agent-speed-relay-before.json) and [after](agent-speed-relay-after.json) samples
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
[container](process-speed-compute-before.json) and
[process](process-speed-compute-after.json) reports retain all samples. Both use the same
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

### K12 complete operation cohort and CLI refusal

Run [42066544946](https://github.com/cubepals/pryx/actions/runs/42066544946), source 52b03d3,
completes all 30 operation samples on the same two-core EPYC 9V74 host for both tiers.
The [container](process-speed-http-before.json) and
[process](process-speed-http-after.json) reports retain every sample and the failure:
CLI image-refusal acceptance fails after measurement, so this is not completed acceptance.
The one-box job separately passes the literal quickstart, SDK examples and image host installer,
including repeat installation. Image-start baseline is unavailable on the prior daemon-only image.

| operation | container p50 / p90 ms | process p50 / p90 ms | budget p90 |
| --- | --- | --- | --- |
| image API startup | unavailable, n=0 | 6845.24 / 7180.20 | 10,000 ms |
| run through edge | 1.77 / 2.47 | 1.98 / 2.23 | 20 ms |
| first output byte | 1.72 / 2.01 | 2.03 / 2.53 | 10 ms |
| start, expose, HTTP 200 | 3.75 / 4.70 | 52.80 / 54.85 | 200 ms |
| snapshot 100 MB | 1446.74 / 2005.11 | 831.14 / 1004.25 | measured |
| fork 100 MB | 1777.60 / 2696.74 | 1002.13 / 1950.96 | measured |
| sleep | 32.89 / 103.87 | 8.20 / 16.04 | measured |
| wake | 886.20 / 989.63 | 11.53 / 15.53 | measured |
| CLI echo | 1015.73 / 1924.59 | 1000.40 / 1126.74 | measured |

The three agent latency budgets and image-start budget pass. CLI refusal was reproduced
locally: Hono does not catch a manually constructed non-Error ZodError. Parsing the rejected
image produces the proper Error-derived validation failure and preserves its structured field
problem; final CI must validate this correction and the remaining isolation assertions.

### K12 complete guarded-process acceptance

Run [42074622463](https://github.com/cubepals/pryx/actions/runs/42074622463), source f8cd96c,
passes the ordinary-image proof with default Docker isolation, required Landlock enforcement,
UID pool and shared UID operation, cross-sandbox filesystem denial, network none, stopped-file
ownership, Node, CLI image refusal, and credentials printed once across restart. The guest-only
supervisor guard and SCRAM on both Postgres transports are included. This workflow separately
fails project rotation on a host port conflict; its full workflow is not green.

The paired [container](process-speed-guard-before.json) and
[process](process-speed-guard-after.json) reports both complete every operation, n=30
on the same host and source, and retain image/agent digests and all kernel capability facts.

| operation | container p50 / p90 ms | process p50 / p90 ms | budget p90 |
| --- | --- | --- | --- |
| image API startup | unavailable, n=0 | 6713.14 / 6775.73 | 10,000 ms |
| run through edge | 2.28 / 2.55 | 3.04 / 3.36 | 20 ms |
| first output byte | 2.37 / 2.89 | 3.07 / 4.06 | 10 ms |
| start, expose, HTTP 200 | 5.32 / 7.05 | 67.01 / 69.85 | 200 ms |
| snapshot 100 MB | 1198.24 / 1330.37 | 485.07 / 498.29 | measured |
| fork 100 MB | 1739.09 / 1766.25 | 965.36 / 990.85 | measured |
| sleep | 34.79 / 39.07 | 9.55 / 13.16 | measured |
| wake | 948.37 / 1004.62 | 12.43 / 15.60 | measured |
| CLI echo | 1006.44 / 1043.73 | 992.00 / 1044.73 | measured |

All four stated process/image budgets pass with the supervisor guard. The additional command
launcher costs about 1 ms compared with the earlier unguarded process cohort on another run;
that is an observation, not a controlled causal estimate. The same-host container comparison
retains its 1,004.62 ms wake p90 rather than rounding it below a second. The process wake p90 is
15.60 ms. Earlier failed cohorts remain above and in raw reports.

## Native celld comparison

[CI 42077009774](https://github.com/cubepals/pryx/actions/runs/42077009774) completes n=30 for
both engines using the same patched celld 0.6.3 executable and host. Backend-start p50/p90 is
142/154 ms locally and 215/233 ms through pryx; start through first HTTP response is
1,881.78/1,889.72 ms locally and 1,274.87/1,284.34 ms through pryx. Warm HTTP is
5.50/5.72 ms locally and 5.74/7.24 ms through pryx. These are observed costs, with no additional
speed threshold in K13. [docker-api.md](docker-api.md) records the method, limits and complete
raw samples, including the retained failed initial cohort and unshipped socket experiment.

### Project rotation with reserved host ports

[CI 42082995409](https://github.com/cubepals/pryx/actions/runs/42082995409), source 66230f5,
passes the corrected one-box installer, both SDK README examples, all ten project groups and
the image host installer including repeat installation. The
[complete report](projects-reserved-one-box.json) records 50 acknowledged creations,
50 validated command runs, four active at once and ten exact quota refusals. The kernel reports
outgoing ephemeral range 32768–60999 and reserved workload range 42000–42999. Counts and partial
failures are now retained by the collector. This remains an observed 8 GiB-class runner, not a
physical 16 GiB measurement. Every assigned test job has now passed; the final aggregate did not start because of the
account-level Actions runner-allocation restriction described in decisions.md. The workflow
concluded with failure, not success.


## Final K9–K13 measurement cohort

[CI 42082995409](https://github.com/cubepals/pryx/actions/runs/42082995409) completes every
assigned product/test/build job on 66230f5. Its required aggregate has no runner and executes
no steps because of an account-level Actions restriction; the overall workflow fails. The
[CI status report](k9-k13-ci.json) preserves this distinction. The release-image job is
skipped by its pre-existing pull-request-only condition, not by a change to acceptance.

These paired samples use one two-vCPU AMD EPYC 9V45 runner, kernel 7.0.0-1017-azure and Bun
1.4.4. Every available operation has n=30, with every sample and source identity retained.
Comparisons below keep their own baseline definitions; they are not interchangeable.

### Frozen K9 baseline and after source

The common collector on 66230f5 measures fe77e67 before and 7cd80e9 after, with the agent
disabled, preserving the historical Docker-exec path. Raw [before](final-speed-before.json)
and [after](final-speed-after.json) reports include node-call timings and cold-image
semantics: references are removed, while shared layers and OS caches can remain.

| Operation | Before p50 / p90 ms | After p50 / p90 ms | Budget |
| --- | --- | --- | --- |
| Public API exec | 39.00 / 41.15 | 47.50 / 52.39 | p50 < 50 ms: met |
| Control-to-node exec | 36.57 / 39.06 | 39.20 / 44.82 | measured |
| Cached-image create | 175.92 / 212.72 | 165.84 / 177.60 | measured |
| Cold-reference create | 189.76 / 215.61 | 180.23 / 199.40 | measured |
| Warm-template create | 188.30 / 206.09 | 186.38 / 202.77 | p50 < 1,000 ms: met |
| Suspended wake | 24.07 / 27.45 | 27.28 / 31.28 | p50 < 1,000 ms: met |
| Stopped start | 124.61 / 132.50 | 138.87 / 166.48 | measured |
| Snapshot 100 MB | 1044.69 / 1282.62 | 1080.91 / 1132.87 | measured |
| Fork 100 MB snapshot | 1300.97 / 1359.17 | 1069.84 / 1201.65 | measured |

K9 specifies median budgets: 47.50 ms public exec, 186.38 ms template create and 27.28 ms wake
all meet them. Exec p90 is 52.39 ms; K9 does not specify a p90 threshold. The public median
remains slower than the buffered baseline, so this does not show a K9 exec latency improvement.
The earlier 57.36 ms median failure remains recorded above. The current product's agent path
is measured separately below.

A [local transport diagnostic](legacy-node-tls-probe.json) retains 30 real loopback
mTLS/Bun HTTPS requests per mode, including cold requests, over a fake exec backend. It does
not reproduce a sustained 40 ms transport delay and is not a Docker/public-control budget
measurement. Production transport behavior was left unchanged and the temporary probe removed.

### K11 agent baseline and current product

The pinned b91ee23 baseline and current 66230f5 use the same host. Raw
[before](final-agent-speed-before.json) and [after](final-agent-speed-after.json)
reports preserve unavailable agent-only baseline paths as n=0, rather than inventing samples.

| Operation | Before p50 / p90 ms | After p50 / p90 ms | Budget |
| --- | --- | --- | --- |
| Run through edge | 62.91 / 94.32 | 1.57 / 2.04 | p90 < 20 ms: met |
| First output byte | 85.67 / 91.01 | 1.47 / 2.06 | p90 < 10 ms: met |
| Start, expose, HTTP 200 | 79.21 / 97.79 | 3.43 / 4.17 | p90 < 200 ms: met |
| Node loopback run | unavailable, n=0 | 0.88 / 1.43 | measured |
| Terminal round trip | unavailable, n=0 | 20.66 / 20.84 | measured |
| Write 100 MB | unavailable, n=0 | 2420.89 / 2505.91 | measured |
| Read 100 MB | unavailable, n=0 | 2792.86 / 2921.69 | measured |

### K12 container and process tiers

The [container](final-process-speed-before.json) and
[process](final-process-speed-after.json) cohorts use the same 66230f5 source and host,
including the supervisor guard and descriptor-copy refusal. Both complete every operation.
The prior daemon-only image has no all-in-one API startup baseline, explicitly n=0.

| Operation | Before p50 / p90 ms | After p50 / p90 ms | Budget |
| --- | --- | --- | --- |
| Image API startup | unavailable, n=0 | 6425.27 / 6507.19 | p90 < 10,000 ms: met |
| Run through edge | 1.61 / 2.02 | 2.18 / 2.50 | p90 < 20 ms: met |
| First output byte | 1.60 / 2.05 | 2.09 / 2.54 | p90 < 10 ms: met |
| Start, expose, HTTP 200 | 3.34 / 6.79 | 44.28 / 47.74 | p90 < 200 ms: met |
| Snapshot 100 MB | 1254.61 / 1344.53 | 802.42 / 866.96 | measured |
| Fork 100 MB snapshot | 1785.43 / 1818.78 | 985.23 / 1964.65 | measured |
| Sleep | 30.42 / 32.41 | 6.60 / 8.69 | measured |
| Wake | 930.05 / 986.18 | 8.36 / 11.08 | measured |
| CLI echo | 999.13 / 1039.28 | 994.40 / 1039.11 | measured |

All four process/image thresholds pass. Process exposure is slower than the same-source
container comparison and is retained without a speedup claim. Capability, mandatory filesystem
and network enforcement facts, shared UID probes and image/agent digests remain in the raw data.

### K13 native celld repeat

The [complete native report](final-celld-native-measurements.json) uses the same patched
celld 0.6.3 binary/profile and unchanged Worker/container example for both engines, n=30 each.
It repeats the earlier cohort without replacing it.

| Operation | Docker p50 / p90 ms | Native pryx p50 / p90 ms |
| --- | --- | --- |
| Backend start | 157.00 / 172.00 | 236.00 / 259.00 |
| Start through first HTTP response | 1569.30 / 1585.11 | 955.87 / 965.10 |
| Warm HTTP response | 4.28 / 5.12 | 4.26 / 5.06 |

Native backend start costs more; the lower start-through-first-response includes celld readiness
and polling, and does not isolate a transport optimization. K13 sets no numeric speed threshold.
The [unmodified-release socket experiment](final-docker-api-spike.json) explicitly fails
unsupported BuildKit operations and remains unshipped, without advertising a docker-api feature.
See [docker-api.md](docker-api.md) for the native ownership limits and exact experiment scope.

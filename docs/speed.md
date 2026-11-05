<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

# One-box speed

K9's collector runs real control, edge and pryxd together, with Postgres and S3 on the same
CI host. No local Docker/e2e measurement is claimed. The paired comparison is from
[run 41966866167](https://github.com/cubepals/pryx/actions/runs/41966866167): before product
fe77e673364580efe372a4f685837f219ae7a0a5 and after product
6218d87b0422bf827a93356f11f98cd618c7916a, using the same collector and host. Both measurement
steps passed; later one-box and fleet-runtime tests failed, so the workflow is not green.
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
| Public API exec | 30 | 55.36 / 58.01 ms | 30 | 57.36 / 63.17 ms | p50 < 50 ms: missed |
| Control-to-node exec | 30 | 53.40 / 58.17 ms | 30 | 55.74 / 60.35 ms | measured |
| Create from cached image | 30 | 204.72 / 223.36 ms | 30 | 195.86 / 209.23 ms | measured |
| Create with cold image reference | 30 | 1679.18 / 1768.75 ms | 30 | 1702.56 / 1788.75 ms | measured |
| Create from warm template | 30 | 245.00 / 261.27 ms | 30 | 227.67 / 233.25 ms | p50 < 1 s: met |
| Wake from suspended | 30 | 32.61 / 37.29 ms | 30 | 35.56 / 40.46 ms | p50 < 1 s: met |
| Start from stopped | 30 | 176.36 / 184.57 ms | 30 | 186.40 / 205.29 ms | measured |
| Snapshot 100 MB | 30 | 588.92 / 832.93 ms | 30 | 602.52 / 644.37 ms | measured |
| Fork that snapshot | 30 | 835.62 / 848.90 ms | 30 | 837.50 / 1103.21 ms | measured |

Baseline machine: Linux x86_64, Azure kernel 7.0.0-1017-azure, two AMD EPYC 7763 vCPUs,
Bun 1.4.4. The after public exec p50 (57.36 ms) misses its 50 ms target; warm-template creation
(227.67 ms) and suspended wake (35.56 ms) meet their 1 s targets. After control-to-node exec is
55.74 ms: most latency remains below the public API, in the Docker execution path. Reaching the
exec budget needs that runtime round trip removed, which K11's in-sandbox agent targets.
This comparison does not show an exec latency improvement. Node-call timings are in the raw
report. The after implementation combines durable-operation insert/read and claim/activity
updates into one database round trip each. The baseline already had authenticated keep-alive
connections and used the node start response without waiting for a heartbeat; these are retained.

Run 41957179796's after samples are discarded: its shared Cargo target reused the baseline
executable, and the live-output assertion exposed buffered fallback. Both builds now clean the
daemon package and the collector asserts advertised features and records the executable digest.
An expired publishing credential delayed the signed fix. The valid paired run now proves live
TypeScript/CLI output and the four-command quickstart. Run 41969685155 then passed Python's
async/sync handles and both literal SDK README examples after the same-second trash fix.
Full workflow acceptance remains open: the project-capacity fixture and a foreign-archive
fleet test still need their closing CI results.

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
The first five-active-sandbox fixture hit physical capacity before its project limit; the
rotation correction still needs CI. Capacity n=1; completed 50-sandbox exercise n=0. A physical 16 GiB host is
unavailable here; the pure capacity tests cover 31 free units for a 16 GiB/8-core reference with
512 MiB/250-millis host reservations, and seven for the same memory with two cores. Those are
arithmetic checks, not physical host measurements.

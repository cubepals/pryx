<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

# One-box speed

K9's collector runs real control, edge and pryxd together, with Postgres and S3 on the same
CI host. No local Docker/e2e measurement is claimed. The complete baseline is from
[run 41950545724](https://github.com/cubepals/pryx/actions/runs/41950545724), commit
fe77e673364580efe372a4f685837f219ae7a0a5. The measurement step passed even though the later
isolated-node test failed. [Raw samples](checks/speed-before.json) retain all 30 observations
per path. After measurements remain pending and are not reported as passed.

```sh
DATABASE_URL=... S3_TEST_ENDPOINT=http://127.0.0.1:9000 PRYXD_SUDO=1 \
  PRYXD_BIN=$PWD/target/debug/pryxd bun control/bench/speed.ts before speed-before.json
```

Each path collects 30 samples and reports nearest-rank p50/p90, retaining every raw duration.
The collector records the commit, kernel, CPU, Bun version, image, exact data size and node-call
timing. It validates that create/start reach running and that suspended wake actually resumes.
Cleanup, initial setup and template warming are excluded in the corrected common collector.
The initial cold-image row below includes reference clearing/verification; it will be replaced
by the corrected same-host comparison and is not presented as pure create-to-running latency. Snapshot/fork use 100,000,000 zero
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
| Public API exec | 30 | 52.10 / 59.32 ms | 0 | pending | p50 < 50 ms |
| Control-to-node exec | 30 | 48.34 / 52.86 ms | 0 | pending | measured |
| Create from cached image | 30 | 185.27 / 191.72 ms | 0 | pending | measured |
| Create with cold image reference | 30 | 683.58 / 799.89 ms | 0 | pending | measured |
| Create from warm template | 30 | 208.59 / 218.77 ms | 0 | pending | p50 < 1 s |
| Wake from suspended | 30 | 30.79 / 36.68 ms | 0 | pending | p50 < 1 s |
| Start from stopped | 30 | 165.29 / 177.02 ms | 0 | pending | measured |
| Snapshot 100 MB | 30 | 569.58 / 779.88 ms | 0 | pending | measured |
| Fork that snapshot | 30 | 829.12 / 850.71 ms | 0 | pending | measured |

Baseline machine: Linux x86_64, Azure kernel 7.0.0-1017-azure, two AMD EPYC 7763 vCPUs,
Bun 1.4.4. The public exec p50 (52.10 ms) missed its 50 ms target; warm-template creation
(208.59 ms) and suspended wake (30.79 ms) met their 1 s targets. Control-to-node exec was
48.34 ms: most latency was below the public API. Node-call timings are included in the raw
report. The after implementation combines durable-operation insert/read and claim/activity
updates into one database round trip each. The baseline already had authenticated keep-alive
connections and used the node start response without waiting for a heartbeat; these are retained.

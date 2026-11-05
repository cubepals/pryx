<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

# One-box speed

K9's collector runs real control, edge and pryxd together, with Postgres and S3 on the same
CI host. No local Docker/e2e measurement is claimed. The initial baseline is pending its first
CI run; values will be populated only from completed raw samples.

```sh
DATABASE_URL=... S3_TEST_ENDPOINT=http://127.0.0.1:9000 PRYXD_SUDO=1 \
  PRYXD_BIN=$PWD/target/debug/pryxd bun control/bench/speed.ts before speed-before.json
```

Each path collects 30 samples and reports nearest-rank p50/p90, retaining every raw duration.
The collector records the commit, kernel, CPU, Bun version, image, exact data size and node-call
timing. It validates that create/start reach running and that suspended wake actually resumes.
Cleanup, initial setup and template warming are excluded. Snapshot/fork use 100,000,000 zero
bytes, so data compressibility and the host filesystem's copy behavior matter.

Image creation is measured both from an already cached image and with its local image
references removed before each sample. Shared Docker layers and OS page caches can remain;
the latter is an image-reference cold start, not a claim that every physical cache is empty.
Template creation begins only after its node copy is ready. No failed/censored sample is
silently dropped. The collector currently uses the existing thin SDK; the after phase will
exercise the new handle's `box.run('true')` explicitly.

| Path | Before n | Before p50/p90 | After n | After p50/p90 | Budget |
| --- | ---: | --- | ---: | --- | --- |
| Public API exec | 0 | pending | 0 | pending | p50 < 50 ms |
| Control-to-node exec | 0 | pending | 0 | pending | measured |
| Create from cached image | 0 | pending | 0 | pending | measured |
| Create with cold image reference | 0 | pending | 0 | pending | measured |
| Create from warm template | 0 | pending | 0 | pending | p50 < 1 s |
| Wake from suspended | 0 | pending | 0 | pending | p50 < 1 s |
| Start from stopped | 0 | pending | 0 | pending | measured |
| Snapshot 100 MB | 0 | pending | 0 | pending | measured |
| Fork that snapshot | 0 | pending | 0 | pending | measured |

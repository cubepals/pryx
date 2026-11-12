# Interpreter and tool measurements

Historical daemon, shell, tier and one-box results remain in [checks/speed.md](checks/speed.md).
K17 samples were collected on this Linux x86_64 workspace through authenticated agent HTTP on
loopback. Each cohort has 30 validated samples. Both sides use CPython 3.12.14 and the same
pinned scientific wheels. The baseline launches a fresh Python process with NumPy, pandas and
pyplot imported; the old release has no kernel endpoint, so there is no invented pre-K17
persistent-kernel number. Baseline process launch is measured directly at its unchanged
supervisor, without HTTP overhead, which favors that baseline. CPU and database checks also
ran on the host; these observations are not a dedicated-runner capacity claim.

| Operation | Baseline p50 / p90 ms | K17 p50 / p90 ms |
| --- | --- | --- |
| Trivial `1+1` | 1237.4 / 2428.3 (fresh process) | 13.1 / 30.7 |
| PNG chart | 1943.6 / 2733.0 (fresh process) | 435.8 / 559.0 |
| First execution after reusing `python()` | Unavailable | 16.5 / 32.7 |
| Restart and Jupyter-ready handshake | Unavailable | 3530.3 / 5345.8 |

Both direct-agent p90 budgets hold: trivial execution below 50 ms, chart below 1 second.
[Raw samples](checks/measurements/k17-loopback.json) retain all observations.
[The collector](../agent/examples/kernel-bench.rs) executes and validates every sample.
The [initial collector run](checks/measurements/k17-loopback-before-http-tuning.json) retained a
78.3 ms trivial p90 because its HTTP fixture lacked the TCP_NODELAY setting already used in the
real bootstrap. The corrected collector uses that same setting; no sample is discarded.

Warm-template **first run after `python()`** measurements are a separate deployment cohort in
[the end-to-end suite](../control/e2e/interpreter.e2e.test.ts). CI uploads all 30 first-run,
loopback, chart and restart observations as `interpreter-measurements` and prints p50/p90 in the
job summary. Local loopback context reuse above is not labelled as template creation. The first CI report at `4938c93` is preserved in
[its raw artifact](checks/measurements/k17-ci-interpreter-4938c93.json): warm first
run 6.1 / 6.8 ms, trivial loopback 5.1 / 81.6 ms, chart 687.6 / 693.5 ms, restart
6492.6 / 6588.2 ms (p50 / p90, n=30 each). The trivial p90 fails the 50 ms budget.
That loopback sandbox accidentally used the default single unit, while the clone
cohort requested four. The corrected loopback allocation is measured in the deployment cohort below;
this first report remains evidence of the failed budget, not a passing result.

Both the CI loopback budget cohort and first-run clone cohort request four default units (one CPU core, 2 GiB). These resource settings are explicit in the collector so the budgets are not attributed to an unspecified CPU quota.

## Native tools baseline

Before K18, the agent at commit `8bfb636` does not support desktop screenshots,
browser screenshots, SSH leases or mounts. The authenticated baseline collector
sent 30 requests to each new route and received 404 on every request. The
[raw before report](checks/measurements/k18-before.json) preserves every response
and elapsed time; those refusal latencies are not successful tool measurements.

The native deployment suite collects 30 warm-template create-to-first-PNG samples
for desktop and browser at four default units (one CPU core, 2 GiB). Browser time
also includes starting its local deterministic page server. CI uploads the raw
`native-tools-measurements` report and prints p50/p90 in the job summary.
CI run [42461834578](https://github.com/cubepals/pryx/actions/runs/42461834578)
measured commit `625ec23` through the real control and edge:

| Operation | Before K18 | n | K18 p50 ms | K18 p90 ms |
| --- | --- | --- | --- | --- |
| Desktop warm create to first PNG | Route unavailable (404) | 30 | 751.7 | 774.9 |
| Browser warm create to rendered-page PNG | Route unavailable (404) | 30 | 1859.6 | 2000.3 |

[Every native sample](checks/measurements/k18-ci-native-625ec23.json) is retained.
These are end-to-end observations, not local X11 or standalone Chromium smoke timings.
The browser samples include page-server startup and rendering. The brief sets no absolute
native-image latency budget. The same deployment case passes desktop input/noVNC, raw SSH
wake and revocation, real Mountpoint and rclone read/write, read-only refusal, browser CDP,
ordinary-command chroot denial, and both SDKs. All
[111 comparison links](checks/measurements/k18-ci-compare-625ec23.json) resolve.
[Acceptance provenance](checks/measurements/k18-ci-acceptance-625ec23.json) records the
source commit, artifact IDs and passing case.

## Interpreter deployment cohort

CI run [42461834578](https://github.com/cubepals/pryx/actions/runs/42461834578)
measured commit `625ec23` with the documented four units for the loopback sandbox
as well as the clone cohort. Both execution budgets pass in this report:

| Operation | n | p50 ms | p90 ms |
| --- | --- | --- | --- |
| First run after `python()` on a warm-template clone | 30 | 5.8 | 6.2 |
| Trivial `1+1`, loopback | 30 | 5.1 | 5.7 |
| PNG chart, loopback | 30 | 156.4 | 162.0 |
| Restart and ready handshake, loopback | 30 | 1621.3 | 1651.6 |

[Every sample](checks/measurements/k17-ci-interpreter-625ec23.json) is retained.
The [earlier corrected report](checks/measurements/k17-ci-interpreter-fc13ecc.json)
also passes and remains available separately.
The first-run timer starts after `python()` opens the warmed kernel; it does not
measure sandbox creation. These CI results are separate from the workspace's
paired before/after measurements above. The earlier single-unit budget failure
remains recorded; changing the CPU allocation is not presented as a code speedup.

## K23 startup comparison

Source `95560de` in green normal CI run
[43219637483](https://github.com/cubepals/pryx/actions/runs/43219637483)
and green installed bundle job
[131190931529](https://github.com/cubepals/pryx/actions/runs/43219637379/job/131190931529)
retains thirty samples per tool through real control and edge. Each timer starts
before sandbox creation and ends at the first validated PNG, including routing
and kernel startup. Browser timing includes starting its own HTTP page server.
Templates and the process bundle are cached before timing; each fresh process
supervisor still verifies the bundle's complete inventory.

| Create to first PNG | n per tier | Container p50 ms | Container p90 ms | Process p50 ms | Process p90 ms |
| --- | --- | --- | --- | --- | --- |
| Python kernel and chart | 30 | 1424.0 | 1490.6 | 11954.5 | 12045.2 |
| JavaScript kernel and chart | 30 | 773.9 | 823.8 | 7962.1 | 8006.7 |
| Browser rendered page | 30 | 1676.7 | 1778.3 | 8946.1 | 9036.7 |

Both request four default units with different resource guarantees: containers
enforce one CPU core and 2 GiB, while process CPU/memory guarantees are off. The tiers run on separate CI runners. The ordinary process account has
UID 1002, x86_64, host glibc 2.39 and Landlock ABI 8, with no engine access.

The [container kernel samples](checks/measurements/k23/container-interpreter-startup-95560de.json),
[container browser samples](checks/measurements/k23/container-native-95560de.json),
[container provenance](checks/measurements/k23/container-provenance-95560de.json),
[process samples](checks/measurements/k23/installed-process-startup-95560de.json)
and [installed provenance](checks/measurements/k23/installed-provenance-95560de.json)
retain every observation and verified producer digest. The sixty-sample container
kernel case takes 96.7 seconds. The unchanged
[interpreter budget report](checks/measurements/k23/container-interpreter-budgets-95560de.json)
still passes: trivial p90 3.0 ms and chart p90 82.2 ms. Those budget timers keep
their original clock points and are separate from startup.

Signed ordinary-user and default-Docker native acceptance, purge and Alpine
relocation pass in the installed bundle job. Later host-matrix jobs fail while
downloading Ubuntu test prerequisites, so the whole Any-host workflow remains
unfinished. The [progress record](any-host-progress.md) tracks that correction.
Earlier d1a0a63 container and 3b20de4 process observations remain retained with
their original source, hashes and workflow qualifications.

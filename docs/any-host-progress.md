# Any-host progress

Continue on `any-host`. Follow `PLAN-ANY-HOST.md`, `AGENTS.md`, `PLAN.md`,
`docs/any-host.md`, `docs/decisions.md` and the current milestone brief.
Only recorded CI evidence establishes completion; a local pass alone does not.

| Milestone | Status | Finishing commit | CI proof | Ran | Did not run | Parked and why | Left |
| --- | --- | --- | --- | --- | --- | --- | --- |
| K19 | done | `b535d24b95ab2046aa6178fdc1d2683640ce2544` | [42777018303](https://github.com/cubepals/pryx/actions/runs/42777018303), green including aggregate | Real hosted KVM, full Rust and Docker suites, CLI doctor, default-profile image probes, non-root API refusal, full end-to-end attacks, one-box/Action/Python, Wasm, two-node, upstream integrations and recovery; local checks below | Live paid-provider provisioning and publication; neither is authorized | None | None |
| K20 | done | `54bd364` | Normal CI 42832474233 and Any host 42832474296, both green | All 13 host jobs: 8 user distro contexts, root container/PG UID, fallback, refusal, sudo, relocation; full normal CI including real Docker/KVM, one-box/Action, Wasm, 2-node, Rivet/celld and recovery | Production main/release prohibited; unchanged credentialed cloud/bootstrap cases skipped | None | Complete; proceed to K21 |
| K21 | done | `de3ef9dc13e955e96c2f7c1e530f800069ff50c0` | Normal CI [42885151433](https://github.com/cubepals/pryx/actions/runs/42885151433), Any host [42885151332](https://github.com/cubepals/pryx/actions/runs/42885151332), Measure [42885520235](https://github.com/cubepals/pryx/actions/runs/42885520235), all green | All normal jobs and aggregate; all thirteen host jobs; both complete measurement jobs, including historical budgets and three-tier collectors | Production main/release prohibited; unchanged credentialed cases | None | None |
| K22 | done except | `7963826182e3f8e9037aea17d4716df15086bd81` | Normal push [43075452804](https://github.com/cubepals/pryx/actions/runs/43075452804) and PR [43075462394](https://github.com/cubepals/pryx/actions/runs/43075462394), Any host [43075452782](https://github.com/cubepals/pryx/actions/runs/43075452782), Rootless engines [43075452793](https://github.com/cubepals/pryx/actions/runs/43075452793) and [43075462656](https://github.com/cubepals/pryx/actions/runs/43075462656), all green | Full normal CI including actual rootful Docker, Wasm, two-node, integrations and recovery; all 15 host jobs; Podman lifecycle/attacks, install/repeat/upgrade/outage/recovery/purge; controller refusals, copy-bound guarantees and 30 hosted engine samples | Main/release prohibited; unchanged credentialed cases; supplementary older-source Measure 43063939164 comparisons still running, preceding budget and corrected-refusal stages passed | Standard rootless Docker data-owner mapping with dependent positive lifecycle/timings; rootless Docker internal ingress/network-policy. Three approaches each recorded; both remain named refusals until their post-K27 second try | Supported scope complete; proceed to K23 and monitor supplementary comparisons |
| K23 | in progress | — | Not yet claimed | Brief, ownership, policy and existing licence review; begin real shared-UID IPC behavior proof | Tool bundle, native process kernels/browser, SDK acceptance and thirty-sample CI cohorts | None newly parked | Complete PLAN-K23.md; preserve both K22 Docker exceptions until after K27 |
| K24 | not started | — | — | — | All | — | Complete brief after K23 |
| K25 | not started | — | — | — | All | — | Complete brief after K24 |
| K26 | not started | — | — | — | All | — | Complete brief after K25 |
| K27 | not started | — | — | — | All | — | Complete brief after K26, then revisit every parked piece |

## CI cost

K19 adds KVM permission setup and one real guest probe to the existing Rust job
(estimated under one minute per pull request), a frozen Bun install and real CLI
doctor invocation in that job (estimated under one minute), an ordinary-image probe to existing
Compose acceptance (estimated under one minute), and failure-only Docker/kernel
diagnostics (under one minute on failure). No new required job is added.
The real shared-uid image case adds up to one further minute to the existing
Compose acceptance step; it starts one non-root image and uses its public API.

## Evidence and handoff

- Started from `origin/main`; the original environment checkout predates this plan.
- K19 is complete at the tested implementation commit above. Later documentation checkpoints cite that source without claiming their own future CI results.
- Bootstrap CI: https://github.com/cubepals/pryx/actions/runs/42768042942 at
  `03e4affe3f1adfda46d70b126ad5177935716168`. Core and control checks pass;
  Docker integration fails the existing OOM-reporting case: Docker reports exit 137
  with OOMKilled false. This was before any implementation change. Failure evidence
  is retained in that run; kernel/daemon evidence is added to diagnose the next run.
- K19 implementation in progress: field refusals and unchanged-spec wake regression,
  runtime-specific doctor and common data ownership, measured host probes, additive
  guarantee names/views/requirements and both generated SDKs. The early control
  suite passed 368 tests with real Postgres; the early Rust suite passed with the
  verified agentOS distribution. These precede the latest source edits and do not
  establish completion. Hosted KVM and ordinary-image acceptance have not run yet.
- Current control check: 372 pass, one existing compiled-assets skip, 8,098 assertions
  against real local Postgres. Python: 23 tests, one existing credential skip. Schema/SDK
  generation, structure, workflow/ShellCheck and whitespace checks pass.
- Local Docker OOM acceptance fails with the workload still running. Its engine reports
  `rootless`, `memory=false`, `swap=false`, `cgroup=none`; this cannot establish a hard
  memory limit. Keep the original assertion and require real rootful CI evidence.
- One overlapping local Rust run fails the existing Wasm compact command with exit 137
  instead of zero. The cause is unknown; retain it separately from the earlier complete
  passing run. The final current-source Rust run passes 420 tests, with 16 existing
  ignored cases; both strict Clippy modes and fmt pass. No test was skipped or loosened.
- Implementation commit `38e4efc8deedfcb6f1605343f1ad610665cc0ee3`, CI
  https://github.com/cubepals/pryx/actions/runs/42772099448: control workspace,
  dependency and upstream contract jobs pass. Core stops at the Rust structure check:
  `main` is 102 lines against its 100-line limit, so its KVM and Docker cases did not run.
  The separate move-only commit `37b83aa78607504815409450e9513fd7dd4f6bdb`
  extracts the unchanged command; CI is
  https://github.com/cubepals/pryx/actions/runs/42772998503. The exact pinned CI Rust
  structure checker now passes locally. Do not use the expanded TypeScript wrapper
  with `--rust`: CI invokes that wrapper without the Rust flag and the original pinned
  checker with it, which have different source scopes.
- Follow-up socket correction: inaccessible engine paths remain unknown and a user-session
  socket must accept a connection. Real socket/symlink-loop regression and full Rust suite
  pass (421 tests, 16 existing ignored), preserving all previous acceptance assertions.
  The bootstrap run completed without cancellation; its end-to-end and
  one-box jobs pass, and its aggregate fails only because of the recorded Docker OOM case.
- CI at `61115513b4bac6b8ab962035bb5e21d903ef4d54`, run
  https://github.com/cubepals/pryx/actions/runs/42773196810, stops at the existing
  comment-attribution grep because a progress-note sentence matches it. Reword the
  sentence without changing the gate. The move commit's run already passes real KVM
  permission setup, the complete Rust suite and the real CLI doctor check; remaining
  Docker/image acceptance is still required.
- `c64b1d1d133ebb6387d1b25db3d28df389bda662`, CI
  https://github.com/cubepals/pryx/actions/runs/42773837666, fixes only that progress-note
  wording. Its control, dependency and upstream contract jobs pass; core and deployment
  acceptance are running. Runs 42772099448 and 42773196810 were canceled after their
  completed failures were retained. Their interrupted deployment work is not a pass.
- Run 42772998503 at the move commit passes the real KVM guest, CLI doctor and all Docker
  integration cases, including the existing hard-memory/OOM assertion. This proves those
  checks on that source; it does not make a later head or interrupted deployment green.
- Conservative firewall follow-up requires a rootful engine before the loaded host table
  can establish host-firewall. The rootless user network does not traverse that bridge.
  All 421 Rust tests pass with the verified distribution (16 existing ignored), strict
  all-target Clippy and the exact Rust structure check pass. Rootless-engine CI belongs
  to K22; no local hard-limit pass is claimed for the cgroup-less engine.
- Source `d6a8420d7700316365f032024d90082c8d8fa0b2`, CI
  https://github.com/cubepals/pryx/actions/runs/42774643808: core is green, including
  actual KVM HLT exits, CLI doctor and every Docker case. Control, dependency and
  upstream contract jobs are green; full deployment completion is still pending.
- Add a real shared-uid process-node requirement/refusal check to existing Compose
  acceptance. The new script passes lint, typecheck and structure; the full control
  suite passes 372 tests with the existing compiled-assets skip and 8,098 assertions.
  Final generated-type checks also pass. The actual non-root image case has not run
  locally because this workspace cannot enforce the required Landlock policy; require
  its hosted CI result. No complete milestone result is inferred from these local checks.

- K19 final proof: run 42777018303 at b535d24 passes every required job and
  aggregate check 129717858784. Core 129697829486 proves the KVM guest, probe and
  full Docker suite; end-to-end 129697829488 proves ordinary-image facts, the real
  non-root refusal and the declared boundary attacks; one-box 129697829727 proves
  the installer, Action, SDKs and repeated image installation. Wasm, isolated nodes,
  public Rivet/celld and store-loss recovery also pass. Earlier runs 42773837666
  and 42774643808 finish green too. Preserve all earlier failures above.
- K20 begins only after that current-source aggregate success. The production
  release/main steps stay configured but unexecuted: this task forbids main pushes,
  version changes, tags and release-key changes. Preview acceptance must use an
  independently compiled fixture trust key and never the release secret.

- K20 foundation: the pure entrypoint move passes the complete local Bun check
  (372 tests, one existing skip, 8,098 assertions) and structure. The subsequent
  signed-archive/path/layout foundation passes 378 tests, one existing skip and
  8,123 assertions, plus typecheck, structure, actionlint, standalone ShellCheck
  and Python compilation. Six new tests exercise real tar files and Ed25519 keys,
  forged-checksum rejection, signed-link refusal, transport policy and home escape.
  The installer dispatch, user service lifecycle, upgrade/removal, one-line bootstrap,
  release signing and full distribution matrix are still unfinished.
- The real PostgreSQL 17.11.0 archive matches its published SHA-256 and runs locally.
  Direct loader invocation also runs a real Bun-compiled CLI successfully; Bun's
  process.execPath then names the loader, so bundle-aware discovery must use the
  explicit bundle directory. No new binary patcher is needed.
- Local bundle build attempt 1 fails Python TLS verification against the managed
  proxy. Attempt 2 mounts the provided combined CA only during build commands and
  preserves verification, then fails because the local VFS storage fills this
  workspace while copying the image. Reclaim only this task's unused build cache;
  keep its running test database. Neither attempt is a successful bundle build.
- The new Any host workflow starts on any-host pushes before manual dispatch is
  available on main, and weekly thereafter. Its first job builds the real bundle,
  runs its programs and a real non-root PostgreSQL in Debian and default-profile
  Alpine, and uploads the artifacts. Its result is pending. The full booted and
  ordinary-container one-command matrix remains to be added.

K20 CI cost so far: the separate weekly/any-host workflow has one free hosted job
with a 60-minute ceiling; the initial estimate is 15–30 minutes for compilation
and under two minutes for the two runtime smoke cells. It adds zero minutes to
a pull request's ordinary gate. Record actual build/archive measurements in its
job summary before claiming the final release cost.

- Bundle foundation source fb748708d6539cde91c2e75cab6d8d6f5220ce77, Any host
  run https://github.com/cubepals/pryx/actions/runs/42786925009, job 129730388593:
  real image and PostgreSQL download/build finish, then the mandatory licence
  inventory fails because dpkg-query's libc diversion diagnostic is treated as
  a package name. Keep the failed build and fix owner-record parsing using Debian
  package-name syntax; the notices requirement remains unchanged. Local Python
  compilation and actual installed-loader package/copyright lookup pass after
  the correction. No archive or relocation smoke pass is claimed yet.
- The corrected bundle at 580f4426 passes Any host run 42788562263 and the
  complete existing CI run 42788562041. Real Debian and default-profile Alpine
  run the relocated programs, ordinary-user CLI and PostgreSQL initialization,
  query and shutdown; uid-zero initialization refuses before creating a cluster.
  This is bundle smoke proof, not the unfinished one-command installation matrix.
- The factory-only Rust move f3945f44 passes 421 local tests (16 existing ignored),
  strict all-target Clippy, fmt and the exact Rust structure gate. Its Any host run
  42796115098 is green; existing CI 42796115176 is still running.
- Local lifecycle foundation passes the complete Bun check: 382 tests, one existing
  skip, 8,135 assertions, plus structure, typecheck and actionlint. A competing-process
  test first exposed SQLite connection collection releasing an exclusive lock. A
  strong connection registry fixes it; the test now forces collection before proving
  contention and proves reacquisition after owner death. Preserve that failed run.
  An earlier invocation without DATABASE_URL also failed database tests; repeating
  with the actual local PostgreSQL and its configured URL produces the result above.
- The 601,694,541-byte combined preview artifact exceeds the GitHub connector's
  512-MiB download limit; native artifact redirects also return 403. Split transport
  into independently checksummed 256-MiB parts, retaining the original archive bytes
  and both runtime smokes. This adds no PR gate; actual elapsed cost remains to be
  measured. Installer dispatch and lifecycle commands remain unfinished.
- Factory-move source f3945f44 now has full green existing CI 42796115176.
  Lifecycle foundation fb0f6ca3 has green Any host 42801855499; its actual bundle
  build takes 790 seconds and produces a 531-MiB runtime plus 58-MiB bootstrap.
  The split preview artifacts download through the connector; the first transported
  ZIP is also streamed locally and matches GitHub's artifact SHA-256. Preserve the
  earlier oversize and native redirect failures.
- K20 installer dispatch is connected: user/container bundle setup, optional user
  manager/linger, foreground supervision, authenticated upgrades, owned-process stop
  and backend compute cleanup before purge. install.sh preflights then starts up,
  never escalating an ordinary account; PRYX_NO_UP installs only the CLI. Production
  release packaging/signing is configured without accessing a release secret or
  publishing anything. Preview builds compile an isolated public fixture key.
- The README and install guide use one-command installation. Domainless servers
  reuse edge path aliases on loopback; server placement retains enforced namespace
  and CPU/memory requirements without relaxing daemon unit hardening. Existing
  domain-based one-box assertions remain and now run the sudo one-command bootstrap.
- Local dispatch implementation passes 382 Bun tests, one existing skip and 8,135
  assertions, complete Rust tests (421 passed, 16 existing ignored), strict all-target
  Clippy, fmt, both structure gates, actual CLI compilation/version, documentation,
  release generation, Python compilation, actionlint and standalone ShellCheck.
  These do not establish distribution acceptance. The new four-cell ordinary-user
  Docker matrix checks the real one command, hello, read-only check, repeat key
  suppression, state/credential retention across upgrade, and purge. Its result is
  pending. The booted-system/user-unit matrix and remaining hardening are unfinished.

K20 added CI cost at this checkpoint: four separate free hosted matrix cells have
20-minute ceilings, estimated 3–8 minutes each after the shared bundle build. They
run weekly and on any-host pushes, adding zero minutes to a pull request's ordinary
gate. Existing one-box changes replace its two installer commands with one sudo
bootstrap and add read-only preflight/probe work, estimated under one minute.
Production release packaging reuses its already built signed image; its measured
archive sizes and actual additional packaging time will be reported by the release
job when publication is authorized. That job has not run in this task.
- Added the four booted-system cells using the pinned Ubuntu Incus package. Package
  metadata from both noble and noble-updates confirms the exact version before use.
  Systemd distributions run the installer as a fresh ordinary user in a PAM login
  session; Alpine uses its own foreground path. Each cell records its image fingerprint,
  supervision mode and seconds to hello, and repeats upgrade/state/purge acceptance.
  No booted cell has run yet. Failure diagnostics redact credentials before printing
  bounded logs; a real Python fixture verifies password/encoded-value suppression.
  Actionlint, standalone ShellCheck, Python compilation and whitespace checks pass.
- Booted CI costs four additional free hosted jobs with 25-minute ceilings, estimated
  5–12 minutes per cell after the shared bundle build, weekly and on any-host pushes.
  These add zero minutes to a pull request's ordinary gate. Current implementation
  source 0b6b0e18 is under existing CI 42804826909 and Any host 42804827105;
  neither current run is claimed green before completion.
- Local signed-fixture acceptance reaches real staging, authenticates a separately
  compiled preview key, and verifies the real staged Wasm distribution with doctor.
  Its initial temporary doctor configuration lacked api and then standalone node_id;
  both real failures are retained and the complete ephemeral TLS configuration passes.
- The actual local Tini 0.19.0 rejects numeric -p 15. Correct both launch paths to its
  documented SIGTERM name. The next run initializes and starts PostgreSQL 17.11 and
  RustFS, then fails node enrollment because the local fixture replaced the release's
  static daemon with a GNU build run through the loader: current_exe names the loader,
  so Wasm's launcher copies the wrong executable. This is a fixture-format failure,
  not a pass or a production static-daemon conclusion. Rebuild the real pinned musl
  format before repeating it; do not change launcher hardening to hide the failure.
- The incomplete local installation purges successfully before the next attempt.
  Both complete Bun checks pass after the fallback/Tini corrections: 382 tests,
  one existing skip, 8,135 assertions. Typecheck, structure, actionlint and whitespace
  checks pass. A real occupied listener verifies default-port fallback and an explicit
  privileged-port refusal. No full local install/hello pass is claimed yet.
- Any host 42804827105 builds and relocates the signed preview bundle successfully.
  Its Fedora job 129799556332 reports success, but its actual log ends with
  `/container-acceptance.sh: line 20: su: command not found`. The surrounding tee
  masked Docker's failure because the workflow used the default unspecified shell.
  This is not installation proof. Select explicit bash (GitHub adds pipefail) for
  every Any host run step and install util-linux in Fedora's test preparation.
  Preserve the original log; require every real acceptance command to complete.
- The other three first container jobs also report success while their actual logs
  end in readiness timeout: Debian 129799556266, Alpine 129799556353 and Ubuntu
  129799557321. Run 42804827105 is therefore not installer acceptance despite its
  aggregate green UI. The later explicit-bash correction propagates these failures;
  the earlier logs are retained. Redacted failure excerpts in later cells identify
  the actual installer fault rather than hiding it behind timeout alone.
- Add one real unsupported Alpine cell with denied Landlock syscalls and an unchanged
  home snapshot, preserving all four default-profile cells. Add a fresh domainless
  sudo-server cell that proves alias hello and repeated credential stability; the
  existing domain-based one-box suite remains intact. Neither new cell has run yet.
  Their free hosted ceilings are 20 and 25 minutes, estimated 1–3 and 4–10 minutes
  after the shared bundle build; they add no minutes to ordinary pull-request CI.
  Actionlint/ShellCheck, Python compilation and whitespace validation pass.
- The current Rust source builds and runs in the exact pinned Alpine/musl compiler
  image and file reports static PIE. This corrects the local GNU fixture's executable
  format; the fresh signed static-daemon install is now running. It has not reached
  hello yet. Repair of the previous fixture's bad launcher link and explicit host-owned
  launcher override were test cleanup only, never a production launcher-policy change.
- The fresh signed static-daemon bundle now passes actual local user startup,
  `pryx run -- echo hello`, read-only check and repeated-key suppression. It selects
  Wasm automatically because this workspace cannot enforce process confinement.
  This is a modified preview fixture built from current source, not the CI release
  bundle or default-process acceptance. Its first absolute-root file write refuses
  permission; the relative agent path used by existing Wasm acceptance succeeds.
- Keep all file-byte assertions across upgrades. A deliberate down stops compute;
  after restart, Wasm needs control's fresh ensure/wake before agent file access.
  Test that explicit wake, retaining its bytes, and finish actual purge. The stable
  launcher change avoids path mismatches across immutable bundle replacement while
  preserving the existing launcher checks. Current complete Bun validation passes
  382 tests, one existing skip, 8,135 assertions, plus structure/typecheck/actionlint.
- Read every completed first booted failure in Any host 42805352411: Debian
  129801314044 and Fedora 129801314232 reach ordinary-user preflight then report a
  probe failure without its underlying stderr. Alpine 129801314158 starts with only
  link-local IPv6 and its package indexes fail DNS lookup. Preserve these logs; add
  bounded probe diagnostics and wait for actual IPv4 acquisition before setup.
  The first Debian/Alpine Docker logs in that run instead fail the fixture HTTP
  server race, now guarded by an actual curl readiness check. Fedora still lacks
  su in that old source. None of these UI-green Docker jobs proves installation.
- Add one GNU-host denied-Landlock cell for automatic Wasm fallback, estimated
  3–8 free hosted minutes after the shared build (20-minute ceiling), with no PR gate
  cost. The four ordinary default-profile Docker cells remain unchanged in coverage.
- Actual local static-bundle stop/restart, explicit control wake, file-byte recovery and
  purge all pass; nine managed paths are removed. The later full curl/sh bootstrap
  test then exposes the packaging root cause behind the CI probe failures: GNU ldd
  returns zero for static PIE, so bundle.py wrapped static pryxd and pryx-agent and
  bootstrap copying omitted their .real payloads. Directly inspecting the ELF shows
  DYNAMIC but no INTERP. This also means the earlier loader/current_exe failure was
  not confined to the manually substituted GNU fixture; the original packer could
  wrap the production static PIE too. Correct that earlier incomplete attribution.
- GNU readelf 2.44 now identifies PT_INTERP explicitly. A real current static daemon
  remains ELF with no wrapper and successfully probes after packaging; a real dynamic
  printf retains its loader wrapper and runs from the relocated closure. Bootstrap
  copying includes every actual payload, and both runtime smoke cells now execute
  its real unprivileged daemon/agent probes and CLI. Python compilation, standalone
  ShellCheck, actionlint and whitespace checks pass. Full installation after this
  root-cause correction is still pending.
- Prevent the release's second checksum-generation step from hashing its own old
  SHA256SUMS by removing that staging manifest before expanding the asset glob.
  No release is published and no release key is accessed.
- Every completed failure through runs 42805352411, 42807401129 and the completed
  cells of 42807721973 has been read. Old sources repeat the now-identified missing
  bootstrap payload, missing Fedora su, fixture-server race and early Alpine DNS
  failures. The explicit-bash run correctly fails its cells instead of hiding those
  failures. Preserve those results; no whole-job replay or test weakening is used.
- Source 8725e3a1 passes the complete normal CI run 42812398328, including the
  preserved one-box, Docker, KVM and end-to-end suites. Any host 42812398279 passes
  bundle/bootstrap relocation, actual unchanged-host refusal, and domainless sudo
  installation. Its four default Docker cells reach hello (Fedora 28s, Debian 40s,
  Ubuntu 34s, Alpine 56s); denied-Landlock GNU fallback reaches hello in 36s.
  These are partial results: Fedora then lacks cmp, the other foreground cells
  expose stale-session readiness across upgrade, and the systemd distributions
  fail user-unit startup without enough diagnostics. K20 remains in progress.
- Fix restart readiness to require the replacement daemon's session and API endpoint
  after the unchanged registry fencing hold. Add a real-PostgreSQL regression for
  that hold and endpoint transition. Add failure-phase and redacted user-journal
  diagnostics; preserve every upgrade byte comparison through Python filecmp.
- Exercise future-login PATH integration, retained deployment policy and root-container
  PostgreSQL ownership in the existing acceptance suite; add the dedicated root
  Docker cell (estimated 2–5 free hosted minutes after build, 20-minute ceiling,
  no ordinary PR cost). Retain all existing default-profile and booted cells.
- Read all remaining failures in runs 42807721973, 42808509316 and 42810213406:
  the older bootstrap probes lack their .real payload, both old domainless jobs
  report release-binary version mismatch from that same wrapped static executable,
  Alpine's old booted cell lacks DHCP/DNS, and unsupported-host tests see the probe
  failure instead of the expected Landlock refusal. The new source's successful
  refusal/domainless cells verify the packaging correction; no rerun hides them.
- Local signed curl/sh installation, same-version immutable-bundle upgrade, retained
  key, explicit control wake, exact file-byte recovery and purge pass with the static
  preview fixture. These are local checks, not hosted matrix acceptance.
- Current local validation passes all 388 Bun tests, one existing skip, 8,159
  assertions, plus typecheck, generated contracts/docs, TypeScript structure,
  actionlint/ShellCheck and whitespace checks. The real registry restart regression
  passes against PostgreSQL; no source is declared hosted-green before its next run.
- Local root-container acceptance exposes initdb permission denial. Inspecting actual
  installed paths shows the inherited private umask reduced intended public release
  and service traversal directories to 0700. Explicitly set the managed root data/
  release parents to 0755 and runtime parent to 0711; private paths remain 0700.
  Full Bun validation passes 388 tests, one existing skip, 8,159 assertions.
- The second local root test also identifies a fixture error: its earlier unprivileged
  extraction applied umask 077 to PostgreSQL wrappers and library directories. This
  differs from the original signed artifact's modes. Restore those modes directly
  from the original archive before retesting; no production archive check is relaxed.
  Hosted root-container acceptance at e976866 has not finished.
- Native systemd-analyze 257.13 reproduces the booted user-unit failure independently:
  the generated quoted WorkingDirectory value is rejected as non-absolute and the
  unit has a fatal setting error. Remove that redundant setting, retaining systemd's
  user-service default directory; the corrected generated unit passes real --user
  verification. Full validation and hosted startup remain pending for this correction.
- With original artifact modes restored, the local root-container test reaches
  authenticated readiness and hello in 126s, proves its actual PostgreSQL process UID
  matches the bundle's nonzero service UID, and runs the future-login PATH check.
  Upgrade and purge are still running; this rootless-engine workspace is local
  evidence only and uses Wasm because its process confinement probe is unavailable.
- The corrected user unit passes native systemd verification and complete local Bun
  validation: 388 tests, one existing skip, 8,159 assertions. No service-unit
  hardening was removed. Its real booted-system acceptance must still pass in CI.
- Local root-container acceptance now completes the entire signed one-command flow:
  hello, new-login PATH, read-only check, repeated-key suppression, authenticated
  upgrade, retained key and exact file bytes, then complete purge. Its printed
  PostgreSQL ownership assertion passes. The modified fixture uses current compiled
  CLI/supervisor plus the preserved static daemon and original artifact modes; it
  remains local rootless-engine/Wasm evidence, not default-profile hosted proof.
- Any host 42820324687 at e976866 passes all four complete default-profile Docker
  cells: Fedora 129851728789 (30s to hello), Alpine 129851728878 (52s), Debian
  129851728888 (41s), Ubuntu 129851729183 (36s). Each also verifies repeat, readonly
  check, future-login PATH, upgrade, retained key/file bytes and purge. Booted Alpine
  129851728721 passes the same flow (47s); denied-Landlock GNU automatic Wasm
  129851728829 passes (43s). Domainless sudo and unsupported-host refusal pass too.
- Read all four failures in that run: booted Fedora 129851728629, Debian
  129851728696 and Ubuntu 129851728831 show systemd's fatal quoted WorkingDirectory
  setting; root Docker 129851728974 shows the independently reproduced initdb path
  permission denial. These are corrected in 5447657 and b1037a7 respectively for
  permissions and the unit setting. Those revisions still await their hosted proof.
- Read the three completed systemd failures in Any host 42822045954 at 5447657:
  Debian 129857118959, Fedora 129857118868 and Ubuntu 129857118909 all identify the
  same invalid quoted WorkingDirectory setting. No additional failure is concealed
  or rerun. The root cell in that run is still executing; b1037a7's matrix is still
  building. Normal CI 42820324770 has passed its K11 Wasm agent suite and is testing
  Rivet/celld; it is not yet a complete green run.
- Hosted root Docker 129857119053 at 5447657 passes the full upgrade/purge flow,
  reaches hello in 37s and verifies the actual nonzero PostgreSQL UID. Normal CI
  42820324770 at e976866 is completely green, including Wasm, Rivet/celld, recovery,
  one-box/Action, actual KVM and the real Docker suite.
- Normal CI 42822883955 at b1037a7 fails Docker job 129854093639 in the existing OOM
  assertion. All other thirteen Docker cases pass. Kernel diagnostics show the
  64 MiB cgroup killed its supervisor shell, while Docker retains SIGKILL exit but
  OOMKilled=false. Preserve this mismatch; no job replay or OOM assertion weakening.
- Replace the fixture's repeated post-allocation sleep forks with a blocking FIFO
  acknowledgement, retaining every memory/input/deadline and OOM/exit assertion.
  Failure-only diagnostics now include bounded engine/cgroup details, OOM event
  IDs without labels, and 240 kernel lines. This removes avoidable post-OOM fork
  pressure; whether it resolves the reporting race still requires hosted proof.
- A real hardened local container verifies the blocking FIFO keeps the chosen child
  exit and native ps shows no children while the supervisor waits. The first attempt
  to enumerate children through an optional proc file could not read that file and
  did not prove the process-count assertion; native ps supplies that actual check.
  The local rootless engine cannot enforce memory limits, so this is no OOM proof.
- The FIFO candidate passes all 421 local Rust tests (16 existing ignored cases),
  strict all-target Clippy, fmt, the unchanged Rust structure ratchet, actionlint/
  ShellCheck and whitespace validation. Existing complete Bun validation remains
  388 passing tests, one existing skip and 8,159 assertions; no TypeScript changed
  in this candidate. The real hard-memory Docker case remains a CI requirement.
- Any host 42822883979 at b1037a7 is fully green across all thirteen jobs. Booted
  Fedora 129861287860 reaches hello in 40s, Debian 129861287918 in 45s, Ubuntu
  129861288077 in 43s; each reports user_unit=True and enabled_linger=True and
  completes repeat, check, retained-key/file upgrade and purge. Alpine, all default
  Docker cells, root Docker, GNU fallback, unchanged unsupported-host refusal,
  domainless sudo and relocation also pass. K20 still awaits complete normal CI.
- The FIFO candidate a83699a passes real Docker job 129866676186 in CI 42826944648:
  the hard-memory/OOM assertion succeeds, all fourteen integration cases pass, and
  none is ignored (the separate lease case remains in its existing suite). This is
  actual hosted enforcement evidence; the full workflow is still running.
- A four-process barrier exposes a separate local-summary race: all four commands
  print the initial key before the old marker write. The regression fails with four
  disclosures. Claim the marker atomically before output; the same regression now
  proves exactly one disclosure, four successful processes and mode 0600. Probe
  presentation is a fixture; the concurrent processes, filesystem claim and output
  collection are real, and credential values never appear in assertions or logs.
- The atomic-summary correction passes the complete Bun check: 389 tests, one
  existing skip, 8,162 assertions, generated contracts/docs/release checks, typecheck
  and unchanged TypeScript structure. All implementation changes remain on any-host;
  no production release, key, tag, version or repository setting was changed.
- Any host 42826946692 exposes an artifact-service availability race before eight
  cells can run: the action lists all three IDs then cannot fetch some IDs, or
  reports success after downloading only part 0. Root hashes reject missing parts.
  Read every failure: jobs 129876514110, 129876514137, 129876514208, 129876514227,
  129876514257, 129876514280, 129876514337 and 129876514368. Do not count those cells
  as installer failures or passes, and do not replay whole jobs to hide the outage.
- Fetch exact expected artifact names through pinned GitHub CLI's maintained REST
  client, with bounded read backoff and fresh staging per attempt. Producing-job
  outputs pin the part count, manifest digest and transport digest; all original
  reconstruction, archive and installation checks remain. Only artifacts and their
  source hashes are retried, never installation or another mutating operation.
- Native fixture checks reject a successful but incomplete download, retry missing
  artifacts and corrupted bytes, and never publish a bad transport after the retry
  budget. The official GitHub CLI 2.103.3 archive is downloaded, its owner-published
  SHA-256 verified, and its real executable reports the pinned version. REST artifact
  acquisition with the workflow token still requires hosted proof.
- ShellCheck, actionlint and whitespace checks pass for the new helper and workflow.
  It adds an approximately one-second fixture check and 5–15 seconds per successful
  matrix cell for the pinned CLI/read setup, with bounded outage backoff (a 120s
  per-part deadline plus one in-flight 30s read). It adds no ordinary PR minutes or
  required job; standard free hosted job ceilings remain 20/25 minutes.
- Read b1037a7's aggregate failure 129875204433: it reports the already recorded
  Docker OOM job failure; its other jobs complete. The corrected FIFO source passes
  that same real Docker assertion without a test, limit or deadline reduction.
- Any host 42832474296 is completely green at 54bd364 with the authenticated
  per-part reader: all thirteen jobs pass, including all eight ordinary-user distro
  contexts, the root-container PostgreSQL UID/port/upgrade case, GNU Wasm fallback,
  unchanged unsupported-host refusal, domainless sudo and bundle relocation. No
  download checksum, signature, retained-byte assertion or install action was skipped.
- At that same source, the normal CI Rust/Docker, contract, workspace and one-box/
  Action jobs pass. Its end-to-end job 129885926157 remains in progress. Keep K20
  in progress until the complete current-source workflow is green.
- Size/timing evidence from the fully successful b1037a7 bundle job 129852581747:
  approximately 531 MiB runtime and 71 MiB bootstrap (about 602 MiB additional release
  assets), 632s for the full preview build, and 126.4s for the packaging stage itself.
  The earlier 58 MiB bootstrap omitted a static executable payload and is not the
  final size. Production release execution remains prohibited; its actual packaging
  and publication duration has not been measured. The configured pack step reuses
  the built release image, with an approximately 2–3 minute packaging estimate.
- Final documentation review corrects the node one-command example to use sudo env
  and state its existing root/systemd fleet-join prerequisite. The verified bootstrap
  and Rust join already require root; the example now matches that actual behavior.
- K20 is complete at implementation source 54bd364: normal CI 42832474233 and
  Any host 42832474296 both finish green. All required host contexts, normal
  compatibility/security/recovery checks and the corrected real Docker OOM assertion
  pass together. No parked piece remains. The unchanged credentialed cloud and
  release-signed remote-bootstrap cases are skipped without configured credentials;
  production main/release execution remains forbidden. This documentation checkpoint
  records the validated source rather than claiming its own future CI result.

- K20 documentation checkpoint eb16949: all 13 Any host jobs in 42840518448 pass; normal CI 42840518632 core job 129914265051 fails the preserved Docker OOM exit assertion (OOM true, exit 1 instead of SIGKILL). Kernel evidence identifies the allocating tail as the victim. Investigate the remaining FIFO producer SIGCHLD/read interruption; no full-job rerun or relaxed assertion. K21 contract work is in progress while this cause is corrected.
- K21 first contract step: optional tier selection, named single-tier refusal and generated node reads are implemented; legacy spec/record digest fixtures still pass. Full local Rust: 423 pass, 16 existing ignored. Full Bun: 389 pass, one existing skip, 8,162 assertions. Strict Clippy, Rust/TypeScript structure and formatting pass. A real Alpine FIFO transport probe preserves allocator SIGKILL status 137 after producer reaping; this is not hard-memory proof on the local cgroup-less engine. Router and mixed-tier CI acceptance remain open.
- K21 first source 10abf12: normal CI 42845960324 core, control and unchanged upstream contract jobs pass, including all real Docker cases with producer reaping. One-box/end-to-end and Any host 42845960303 are pending. Checkpoint 42840518632 additionally fails five fleet end-to-end checks: the first archive upload has no API completion and hits the fixture’s 30-second timeout; the four following checks expect the stopped workload running. Aggregate 129938349277 fails those and the recorded OOM case. Logs for both failed jobs and aggregate were read; export root cause remains under investigation, with no timeout increase or mutation replay.
- K21 routing step: auto backend construction, persisted record/recovery-label tier, immutable ensure routing, selected-tier agent/data/validation/guarantees, container-only leases, tier-set placement, shared capacity and feature-gated spec writes are implemented. Current full native checks: 427 Rust pass, 16 existing ignored; 392 Bun pass, one existing skip, 8,184 assertions; strict Clippy, fmt and both structure checks pass. Early local failures corrected the test’s generic error-envelope assumption, mismatched legacy feature fixtures and tier-named refusal expectations; the outage regression also exposed the old successful-info fallback, which now refuses. Automatic public order, new-config defaults, doctors and real mixed-image acceptance remain.
- K21 move-only dependency correction: 17b3212 extracts unchanged bounded child/C-library evidence; complete local Rust (427 pass, 16 existing ignored), strict Clippy and both structure checks pass. The preceding auto implementation exposed a probe/runtime dependency cycle and the fleet class size ratchet. Keep host evidence independent and selection in NodeChoice; revalidate the restored auto implementation before claiming it passed.
- A local full fleet attempt cannot reproduce the checkpoint archive timeout: its first sleep requires a real freezer, but this engine has cgroup=none and correctly stops instead; dependent tests then fail, and root-only join cases refuse this unprivileged caller. The result is 4 pass, 11 fail, not archive or deployment acceptance. Hosted source 10abf12 has since passed the full native end-to-end step without any job rerun; preserve the earlier failure and collect more bounded diagnostics if it recurs.
- K21 configured-doctor validation now uses one applicable distribution check: a real configured CLI report completes in 0.35 seconds and marks container/Wasm off for a process configuration; the real daemon doctor lists all three and exits 1 for this host’s denied Landlock, preserving its refusal. An overlapping local check then hits the unchanged five-second real Wasm HTTP readiness deadline (guest process still running, no listener) and the unchanged 30-second long-lifecycle HTTP test deadline. Preserve both logs; remove the unused task S3 service and serialize resource-heavy checks as CI does. No timeout or assertion is changed, and overlap remains a candidate explanation rather than a proven product cause.
- K21 automatic selection/doctor step is locally complete: 427 Rust pass, 16 existing ignored; 396 Bun pass, one existing skip, 8,202 assertions; strict Clippy, fmt and both structure checks pass in the serialized run. Both formerly timed-out tests pass with unchanged deadlines. New local installs select auto only when the bundle probe supports it; older records/bundles keep their explicit tier. The CLI uses an owned discovered configuration when the daemon supports the additive probe flag. Real mixed-tier image/doctor acceptance and final current-source CI remain open. Routing source f98a5de has full green CI 42849302425 and all 13 Any host jobs in 42849303232; move source 17b3212 has green Any host 42854309881 and pending normal CI 42854309801.
- K21 CI cost: add one mixed-tier image startup, four live sandboxes and doctor checks to the existing Compose acceptance step, estimated up to two extra minutes per pull request. Keep all existing tier collectors, thirty-sample cohorts, budgets and default-profile assertions. No new required job is added. The existing native CLI doctor check additionally asserts all three tier entries.
- K21 final acceptance step adds actual ordinary-image coexistence, exact guarantees, unnamed selection, unavailable named-tier refusal and configured/unconfigured doctors. Local default preferences now survive worker restarts and upgrades. Single-backend changes retain saved routes; legacy auto migration discovers existing compute and refuses an ambiguous image-free resting record. Full local Rust: 429 pass, 16 existing ignored; full Bun: 397 pass, one existing compiled-assets skip, 8,208 assertions. Real image acceptance and final-source hosted CI remain unrun. Move source 17b3212 has complete green normal CI 42854309801; auto source 6df6f09 has all thirteen Any host jobs green in 42860958825 and green core, contract, workspace, native end-to-end and one-box steps in pending normal run 42860958833.
- Auto source 6df6f09 now has full green normal CI 42860958833 and all thirteen Any host jobs green in 42860958825. Acceptance source e299fd2's Compose step fails waiting for the mixed image to start; its later native-template stage and one-box work were canceled after the failure to release logs, so they are interrupted, not passes. Job 130004831055's complete logs were read: Compose/restart and shared-UID cases pass, mixed startup has no stdout and reaches its unchanged sixty-second deadline. Its Docker log helper discarded stderr. The fixture's host-created root is runner-owned, contradicting credentials()'s explicit root-image ownership guard; correct that setup and collect both log streams without relaxing the product guard. Subsequent source 9d66a1e passes hosted core/Docker/KVM, contract and workspace jobs; full mixed-image proof remains pending.
- Source 9d66a1e's Compose step also reaches the mixed-image startup deadline before any sandbox operation. Complete job 130006743187 logs and both aggregate failures 130014663927 / 130017186080 were read; each aggregate records the deliberately interrupted deployment jobs. The directory-ownership correction is pending fresh CI. Add failure-only private logs through the existing redactor with explicit fixture scope; a real Python invocation preserves startup evidence, removes fixture passwords and encoded secret strings, and prints only the selected directory's log. No startup deadline, state ownership guard, sandbox assertion or existing test is reduced.
- Any host 42866109085 at e299fd2 passes all thirteen jobs. Run 42866655193 at 9d66a1e passes twelve; Ubuntu Docker job 130017303576 fails archive acquisition before installation after four thirty-second part-0 reads. Its full log was read and shows no native download error before termination. Extend only the bounded network read to sixty seconds and its retry window to three minutes; add native timeout diagnostics. All producing-job hashes and final transport/archive checks stay mandatory. This can add up to three minutes of slow-read waiting per host job, with no new normal pull-request job or healthy-read delay. Current root-ownership/diagnostic source f01083e is in CI 42870016454 and Any host 42870016512; neither establishes final completion yet.
- Bounded-read source ed9c3b4 passes the existing real-file corruption/incomplete-read harness, ShellCheck and actionlint; hosted CI is 42870683207 and Any host 42870683673. A local attempt to read the earlier real GitHub artifacts instead receives HTTP 403 after the artifact redirect on every bounded retry. Its redacted log is /tmp/pryx-k21-real-reader.log; it does not reproduce the hosted thirty-second stalls or prove an artifact/installer pass. No signed URL or credential is emitted.
- The root-owned fixture correction succeeds: f01083e end-to-end job 130018519923 passes Compose, including real container/process/Wasm coexistence, exact guarantees, file bytes, named refusal, unnamed selection and both doctors under default capabilities/security. Its full end-to-end job is still running. One-box jobs 130018519725 (f01083e) and 130020908853 (ed9c3b4) fail the first CLI hello flow with ECONNRESET; both complete logs were read. Their PostgreSQL initialization failures recover before this request, and daemon create/start/token/delete calls succeed. The failing fetch phase is not established. Add opt-in CI phase tracing with no request data or mutation replay; the actual reset remains under investigation. Trace cost is below one second per pull request, with no new job, gate or dependency.
- Any host 42870016512 (f01083e) and 42870683673 (ed9c3b4) each finish green across all thirteen jobs. The latest source also passes its Compose/mixed-tier step in job 130020909029 and its core, contract and workspace jobs. Its full end-to-end work is pending and its quickstart failure prevents completion. The new trace's real tests preserve request count and response/error identity, and exclude credentials, request data, arbitrary method values and arbitrary error-code values from its fixed metadata fields.
- Quickstart investigation: thirty separate-process HTTP readiness/command/output flows pass through the real front/proxy with both a protocol peer and the real Rust agent; these loopback probes do not establish Docker confinement. A new real-Postgres/socket regression delays the public route by one second under the edge's unchanged default limits. Existing production polling causes sixty address-rate refusals and the separate command client fails with ECONNRESET (/tmp/pryx-k21-readiness-measured-before.log). Back off only health GET retries within the same five-second deadline; add bounded-label edge metrics to existing failure diagnostics. Require the regression, full checks and new CI before claiming the quickstart cause resolved.
- The cold-route regression passes after backoff: one authenticated command POST reaches the peer and no address-rate refusal occurs. Full current TypeScript checks pass 400 tests with one existing compiled-assets skip and 8,220 assertions; structure, actionlint and ShellCheck pass. The tracing source e8f236b's one-box quickstart passes and proceeds to image installation, so the earlier reset remains timing-dependent rather than an always-failing transport. The regression runs without tracing; final hosted proof of the backoff source remains required. The new socket regression costs about two seconds in the existing workspace job; failure-only metrics are bounded to five seconds with no new job.
- Proof audit corrects the collector assumption: the full thirty-sample three-tier and historical budget cohorts are in the separate scheduled/manual Measure workflow, not normal CI. Earlier green end-to-end jobs prove functional suites, interpreter samples and recovery, not the complete tier budgets. Explicitly dispatch Measure on the final source. Its pre-K20 four-command bootstrap selects the new user preset against server-only preview assets; use the already-tested one-command root quickstart instead, retaining all existing assertions. The unchanged two hosted measurement jobs have 180/120-minute bounds and add no normal pull-request cost. K21 remains in progress until their real results and final normal/host CI are recorded.
- Measurement-preset source e14e2fa is in normal CI 42879955471, Any host 42879956369 and explicit Measure 42880161549. During proof review, three new dispatch regressions fail against existing code: inspect/list errors leave the first discovered route cached, and listing a same-named process object overrides a saved container route. Each then starts the wrong backend once (/tmp/pryx-k21-discovery-before.log). Stage discovered bindings and validate the entire result before publishing, preserving authoritative saved-tier bindings. Fresh complete Rust checks and final-source hosted proof are required after this correction.
- Atomic discovery passes all three regressions and the complete native suite: 432 pass, 16 existing ignored; strict Clippy, fmt and both structure checks pass. Record binding retains its direct in-place path; discovery stages only the current result's name/ID pairs. Replace the superseded measurement run with a complete run of this corrected source; interrupted older cohorts cannot establish budget proof. The existing 400-pass TypeScript suite remains the latest complete check of unchanged TypeScript sources.

- K21 final proof audit: all three workflows above completed successfully at de3ef9d. Measure jobs 130072712944 and 130072713325 report every collector, three-tier comparison and measurement-save step successful. Normal CI includes mixed-image acceptance, real Docker/KVM, upstream contract and aggregate; all thirteen Any host jobs pass. Archived log reads returned no content here, so this audit uses GitHub run/job/step results and makes no additional numeric measurement claim. Resume with K22.

- K22 discovery checkpoint: share ordered local engine selection between probe, configured runtime and doctor, retain explicit endpoint selection and negotiate the compatible API through Bollard. Preserve every existing engine capability name. Local final Rust suite: 437 pass, 16 existing ignored; fmt and strict all-target Clippy pass. The final Rust structure result, read after this checkpoint was pushed, fails doctor::run at 102 lines against its 100-line limit; the earlier structure result predates endpoint handling. A move-only extraction and fresh checks are required. The six-request Unix transport fixture proves version alone is insufficient and no later engine is contacted after selection; the actual local rootless engine also answers the enhanced probe. No rootless/Podman CI lifecycle, hard-limit refusal, standard UID-map inference or new timing acceptance is claimed by this checkpoint. Added tests use existing CI and add well under one minute per pull request, with no new job or required gate.

- The move-only c7e9eb6 extraction resolves doctor::run's structure failure without changing behavior. Fresh complete Rust validation passes 437 tests with 16 existing ignored, strict all-target Clippy and the exact Rust structure checker both pass, and fmt/whitespace checks pass. The correction to the preceding premature structure note is retained above. Current-source hosted proof is pending; K22 remains in progress.

- Read both failed b08513f core job logs (130344119271 in 42966253310 and 130344116367 in 42966259730): each stops solely at doctor::run's 102/100 structure finding, before hosted Rust/KVM/Docker execution. Corrected discovery source 510fc48 core 130346091054 passes in CI 42966889091, together with workspace, contract and dependency jobs; complete deployment and Any host 42966889057 proof remains pending.

- Discovery source 510fc48 finishes completely green in normal CI 42966889091 and all thirteen Any host jobs in 42966889057. Source b08513f's earlier structure failure is retained; no failed job was replayed.
- K22 policy checkpoint: feature-gated allowUnenforcedMemory explicitly permits missing enforcement while retaining memoryMb admission accounting. Capable engines retain hard limits regardless of the flag. Explicit CPU/PID limits refuse without their controllers; boot/resume recheck enforcement. Rootless hostname egress, native FUSE and store leases stay refused. Podman is explicitly refused before compute because its compatible network handler ignores the required bridge isolation option; it is unfinished, not parked or accepted.
- Current local policy validation: complete Rust 439 pass, 17 ignored (16 existing plus the new engine case assigned to the existing Docker CI step); strict all-target and ordinary Clippy, fmt, Rust/TypeScript structure and whitespace checks pass. The targeted new case passes against actual rootless Docker 29.9.2 with cgroup=none, in 2.76s: controller refusals leave no data/compute, opted-in execution writes retained bytes and reports enforcement off. It also checks the rootful branch when run by existing Docker CI; that branch has not yet run on this source. The compatible HTTP peer proves unsupported bridge policy refuses both resource validation and create before mutations; it is not real Podman proof.
- Full current Bun check: 401 pass, one existing compiled-assets skip, 8,230 assertions, real local PostgreSQL 17.11. Python: 23 tests with one existing credential skip; generation/check pass for node/public APIs and both SDKs. No complete rootless engine lifecycle, standard UID-map ownership, delegated-controller attack or rootless timing claim is made. This step adds one short real case to the existing Docker job and one short workspace test, estimated under one extra minute per pull request, with no new job or gate. Measure proof remains required for final K22 source.

- Policy source 64c517f finishes green in normal CI 42973067945 and all thirteen Any host jobs in 42973067832, including the new real rootful enforcement branch. Move-only source 6b462f6 splits network operations; complete local Rust, strict Clippy and structure pass. Its hosted core, contract, workspace and dependency jobs pass in pending CI 42976541144; deployment and host-matrix results remain pending.
- K22 sleep checkpoint: control reads the current copy guarantee and stops directly when memory sleep is false or unknown, with no mutation on stale epoch or read failure. Legacy node behavior is preserved. The node rejects an unsupported freeze before saving suspension or pausing. Complete local Rust: 439 pass, 18 ignored (the additional real engine sleep case runs in the existing Docker CI step); strict Clippy, fmt and Rust/TypeScript structure pass. Complete Bun: 406 pass, one existing skip, 8,241 assertions against real PostgreSQL. The targeted actual rootless Docker stop/wake test passes in 3.77s with retained data; the rootful freeze/resume branch remains hosted proof. An initial TypeScript check caught the local view alias missing the existing optional guarantees field; the corrected alias and complete rerun pass. No new job; estimated CI cost under one minute per pull request.

- Network move 6b462f6 finishes green in normal CI 42976541144 and all thirteen Any host jobs in 42976541933. Sleep source ab6b8e2 has all thirteen Any host jobs green in 42978725930 and green hosted core/Docker, workspace, contract and dependency jobs in pending normal CI 42978725972; deployment completion remains pending.
- K22 outage checkpoint: independent serving health and complete-per-tier inventories preserve unavailable records and routes, report their state unknown and guarantees off, and keep healthy tiers available. Raw all-backend info/list remain strict. Health recovery alone cannot advertise a stale tier; a consistent inventory is required. Per-tier outage generations keep recovered crashes out of watched-failure restart policy. Four new regressions cover continued process/Wasm service, all-tier failure and single-tier recovery, byte-identical records after startup with an unreadable engine, and crash attribution. Existing ambiguous identity, unresolved legacy and atomic discovery tests remain unchanged and pass.
- Complete current local outage checks: 443 Rust pass, 18 ignored; strict all-target Clippy, fmt, Rust structure, OpenAPI freshness and TypeScript structure pass. The unchanged TypeScript sources retain their latest full 406-pass check. An early new regression used the wrong public test aliases; corrected without changing production visibility or existing assertions. The unresolved legacy regression also caught and restored its existing startup refusal before this final run. This is not yet real engine-outage CI proof. Estimated addition under one second in the existing native job; no new job or gate.

- Sleep source ab6b8e2 finishes fully green in normal CI 42978725972 and all thirteen Any host jobs in 42978725930. Outage source 1c56ae9 has green PR CI 42981562718 and all thirteen Any host jobs in 42981556121. Push CI 42981556160 fails only one-box job 130392960667 and its aggregate 130412369251; both full failure logs were read. The host probe exits 137 with no diagnostic output under its existing thirty-second caller deadline. The log does not identify its phase or prove OOM. All other jobs, including full native end-to-end, pass. Preserve that failure and add phase and bounded kernel/memory diagnostics; do not replay the job or increase its deadline.
- Move-only 70bbca7 extracts existing engine preflight checks before adding the native adapter. Complete local Rust passes 443 tests with 18 ignored, strict Clippy and structure pass. An initial local run fails four tests on ENOSPC and one Wasm execution; retaining two dependency variants filled the 32-GiB workspace. Removing 9.1 GB of obsolete test executables permits the complete unchanged suite to pass. Its normal CI 42987105772 and Any host 42987105647 are pending.
- K22 native adapter checkpoint: generated Podman owner models and OCI mounts over existing Reqwest Unix transport handle native Netavark bridges and create; each workload gets a separate bridge with its retained isolation option checked. Native create disables implicit writable temporary filesystems, keeps nonzero IDs/capability drop/no-new-privileges/read-only root, bounded logs and available hard limits, and uses keep-id to map the daemon-owned data into its workload UID. Explicit configured data ownership remains authoritative. Custom native-tool profiles, hostname egress and FUSE remain refused on this adapter. Native controller metadata and its systemd manager are prerequisites for reporting rootless limits on; without controllers, accepted requests disable cgroup creation and leave limits off. Fresh create also rechecks explicitly requested CPU weight and PID limits.
- The latest full Rust SDK attempt fails unchanged cargo-deny on unmaintained paste. Replace its transport with maintained Reqwest and retain only its generated owner types; no advisory waiver, source rename or policy change. Final current local checks pass: 446 Rust tests, 18 ignored; strict all-target and ordinary Clippy; Rust structure; complete advisory/license/source/bans checks; OpenAPI freshness; actionlint and shell/Python syntax. The two actual local rootless Docker cases pass in 5.30s with inferred ownership, controller refusals, stop/wake and retained bytes. The ownership diagnostic independently measures default/host writes succeeding under this machine's custom UID map and keep-id rejected. These are not standard rootless Docker or Podman acceptance.
- Opt-in phase tracing preserves JSON stdout and reports only fixed phase names and monotonic timestamps. A real local debug probe completes and parses all three tier entries; distribution verification takes 25.27s. That identifies a real expensive phase but does not establish the hosted release-probe failure cause. The thirty-second deadline remains unchanged and no distribution hash is skipped.
- Add the authorized Rootless engines workflow on branch pushes, manual dispatch and relevant pull requests: two ordinary-user Ubuntu jobs with pinned Docker/RootlessKit and Podman packages, explicit digest validation, two actual resource/sleep cases and three recorded data-mapping strategies. Estimated extra PR cost is 10–15 runner-minutes per job (20–30 total, 10–15 wall minutes), with a 35-minute cold-build bound per job; real durations are unmeasured. Startup's AppArmor attachment permits user namespaces only for the test engine's exact RootlessKit executable; workload profiles are unchanged. Full engine suites, adversarial isolation, lifecycle/port/snapshot/fork measurements, supported user-preset enablement, standard mappings, exact freezer/low-port reporting and final Measure remain open. No piece is parked yet.

- Source 70bbca7 finishes green in normal CI 42987105772 and all thirteen Any host jobs in 42987105647. Native source 0c67ac9 finishes green in normal CI 42990994662 and all thirteen Any host jobs in 42990994683, including the previously failing one-box probe. This does not erase its earlier SIGKILL or establish a root cause.
- Read all four failed native engine jobs in full: push Docker 130423553364 and Podman 130423552907 in 42990994771, PR Docker 130423586596 and Podman 130423586995 in 42991004688. Docker refuses the shared isolated bridge because bridge netfilter is absent inside its user namespace; do not set its suggested ignore flag. Podman cases fail discovery and report the missing fallback Docker socket, although the native CLI works; the failed phase remains unknown. These are not positive engine acceptance.
- The actual ordinary-user Docker daemon maps its original host UID 1001 to guest zero and guest 1 onward to host 165536 onward. Three different data strategies fail: default and host user namespaces cannot write the daemon-owned mode-0700 data as workload UID/GID 1000; keep-id is rejected. Park standard rootless Docker ownership and its dependent positive lifecycle/timing acceptance, with data-owner-mapping refused before compute; never run the workload as zero or change host data ownership to subordinate IDs. Revisit after K27. The same three-mode Podman measurement finds default and host denied but keep-id succeeds, with the written file owned by the runner. Podman remains active work, not parked.
- Move-only d5c66a3 separates Docker construction tests before the ownership proof. Full local checks pass 446 Rust tests with 18 ignored, strict all-target Clippy, fmt and exact Rust structure. Its hosted workflows are pending.
- Add bounded metadata route timings, fixed opt-in request phases and an explicit selected-socket fixture with an independent discovery assertion. The typed protocol peer reaches the exact native route, but this is not actual Podman proof. These additions cost at most twenty seconds in each existing engine job, with no new job or required gate. The named ownership refusal and the new diagnostics require fresh local and hosted verification before acceptance.

- Read all four failed move-source engine logs: push Docker 130443732635 and Podman 130443732436 in 42997078509, PR Docker 130443761493 and Podman 130443761965 in 42997087723. Docker repeats the bridge-netfilter failure and all three failed ownership strategies. PR Podman repeats the discovery fallback, but push Podman reaches native create and fails its literal cap-drop token assertion; its second case fails reconciliation without the underlying error printed. Neither proves confinement or retained bytes. Preserve Docker's token assertion, add actual zero inheritable/permitted/effective/bounding/ambient capability and no-new-privileges assertions, print reconciliation's error, and give these distinct deployments their own owned bridges. No production hardening is relaxed.

- Current ownership/diagnostic source passes the full Rust suite: 449 pass, 19 ignored. Final real local engine checks pass all three discovery/controller/sleep cases in 5.82s, including zero kernel capability sets, no-new-privileges, retained data and fixture-owned network cleanup. Strict all-target Clippy, exact Rust structure, complete cargo-deny, actionlint, fmt, shell/Python syntax and whitespace checks pass. A test-only TOML table-order mistake initially fails configuration parsing; corrected before this final real-engine run. The earlier private internal-field assertion fails to compile and is replaced by the explicit fixture identity check, without broadening production visibility. Hosted standard-map refusal and Podman positive proof are still pending.

- Ownership source 2db9d0d passes both hosted Docker refusal jobs (130448282410 in push 42998443354 and 130448308914 in PR 42998451113). Both Podman jobs (130448281871, 130448308493) pass real controller/refusal, zero-kernel-capability, data-write and stop/wake retention cases, but fail the independent discovery assertion. All failure logs were read. Bounded metadata timings show 559–734 ms per read; four sequential reads exceed two seconds. Preserve the deadline and request count, overlap independent read-only requests, and reproduce the old failure in the exact-route peer with the actual 560-ms delay. Before: timeout after 2.01 s; after: success in 1.18 s. Real hosted discovery on the new source is still required.
- Read failed normal-CI core 130448333921 and workspace 130448333854 in 42998443283. Both stop at the mixed-source structure check's three-module cycle docker/ownership/engine. The preceding Rust-only structure pass did not check that graph; its recorded pass is limited to that check. Move the shared unchanged timeout to runtime and preserve the engine reexport. Require both structure checks before the next push. No check or baseline changes.
- New real lifecycle acceptance passes locally in 7.92 s before the additional PID-namespace and strict deletion assertions: image, published HTTP, readonly root/nonzero identity, local snapshot restore, checksum-verified archive fork with independent bytes, neighbour bridge denial and cold stop/wake. The pinned base image lacks httpd; an initial listener attempt exits immediately and exec correctly refuses NotRunning. Use its maintained BusyBox nc listener without adding image packages. This adds one case to the existing core and engine suites, estimated under one minute per job, no new job or gate. Full current checks and actual Podman lifecycle proof remain pending.

- The same cycle fails PR core 130448375646 and workspace 130448375738; both logs were read. Sharing the deadline alone is insufficient: the child's public reexport and the transport peer's dependency on discovery also create back edges. Keep the diagnostic constant in its Docker owner, preserve the engine timeout reexport from runtime, and test Docker's bounded info directly in its own transport peer. Original and expanded mixed-source structure now both pass, with no baseline changes.
- PR contract 130448375752 in 42998451075 times out after 180 seconds in upstream adoption from an archive; subsequent restart/re-enrollment/lease/operator/destruction cases fail after its endpoint is closed. The complete job log was read; it does not identify the initial stalled phase. Push contract 130448333842 passes. Retain upstream fixture logs through its existing keep-state environment switch (process/container cleanup still runs), then use the existing bounded credential-redacting diagnostic program and bounded S3 logs on failure. Keep the same tests and deadlines. Estimated failure-only cost under ten seconds, no new job or gate.
- Final local real-engine lifecycle source passes all four cases in 44.50 s, including separate PID namespaces, private-network denial, actual file bytes after cold wake and strict deletion. A preceding run gets an empty HTTP body only after wake; the minimal test listener had closed without reading the request. Read request headers and send the exact Content-Length, while additionally asserting the retained file through exec. This fixes the fixture protocol rather than accepting missing bytes. Allow up to two extra runner-minutes in each existing core/engine job for the new lifecycle case; actual hosted lifecycle durations remain unmeasured.

- Current concurrency/cycle/lifecycle source passes the complete local Rust suite: 449 pass, 20 ignored. All four actual local engine cases pass in 44.50 s. Strict all-target Clippy, original source structure, expanded mixed-source structure, exact Rust structure, fmt, actionlint and whitespace checks pass. The delayed native metadata peer runs in the full suite. This is final local evidence for this checkpoint, not hosted discovery/lifecycle proof. The dependency graph and APIs add no crate, schema or SDK change.

- Source e83f760 completes both rootless workflows green: push 43001598656 (Docker 130458798174, Podman 130458798694) and PR 43001607289 (Docker 130458827418, Podman 130458826791). Podman 4.9.3's four real cases take 48.65 s in PR, including bounded discovery (about 1.44 s), zero capability sets, named limits, inferred runner-owned data, HTTP, readonly root/nonzero identity, snapshot/checksummed fork with independent bytes, PID separation, neighbour denial, stop/wake and strict deletion. Docker's unsupported standard map is explicitly refused before data/compute in all applicable cases. Both contracts and workspaces pass on this source; normal deployment and host-matrix completion remain pending.
- Read failed e83f760 core jobs 130458852812 and 130458883692 in full: both stop at actionlint's ShellCheck SC2174 on the newly added diagnostic mkdir options. The local actionlint executable ran without a discoverable ShellCheck binary, so its pass did not include that integration. Restore the already-recorded separate ShellCheck 0.11.0 from the owner-digested archive (SHA-256 8c3be12b05d5c177a04c29e3c78ce89ac86f1595681cab149b65b97c4e227198), use install -d for the mode-0700 diagnostic directory and run actionlint with ShellCheck actually on PATH. Both that complete workflow check and the engine setup's standalone ShellCheck pass. No warning or check is suppressed.
- K22 control defaults in progress: persist independent original memory/CPU intent before unit rounding; keep reservations and capable/legacy wire bytes, and use the existing announced acceptance field only for an omitted memory ceiling. Explicit memory, CPU, units and required guarantees retain enforcement/refusal. A named memory ceiling does not invent an enforced CPU quota on a node without that controller. Snapshot forks inherit known intent; legacy snapshots keep their original ceilings without inference from their numbers. Five real-Postgres/wire regressions pass, plus existing focused checks. The first full workspace run fails five existing sleep regressions because the added placement memory filter misclassified legacy specs with no intent metadata. Preserve those tests and legacy routing/writes; known new requests alone get the new placement filter. A fresh complete workspace run is required after that correction.

- Final resource-intent source passes all workspace checks against real PostgreSQL: 411 pass, one existing compiled-assets skip, 8,276 assertions; lint, typecheck, OpenAPI/SDK freshness, docs/release checks and original/expanded structure pass. All twenty targeted intent/sleep regressions pass unchanged after limiting the new memory-placement filter to requests with known original intent. Complete actionlint with ShellCheck and whitespace checks pass. Existing Rust sources retain e83f760's full 449-pass validation; this control checkpoint adds no Rust/schema/dependency change. The five new workspace regressions add about two seconds, with no new job or gate. Hosted proof of this checkpoint remains required.

- Before enabling user engines, a new actual no-cgroup exec regression fails locally: the existing cgroup-only timeout path reports timed_out=true/killed=false. Docker's maintained top API also fails because its runc path requires a cgroup. A separate real pidfd/shim-identity experiment successfully kills the exec and its child, keeps the original workload running and rejects a forged-ID host process. Podman's native hpid listing is available; its v4.9 forced exec-removal path is unsuitable because it waits the workload stop timeout and signals only the exec root. Continue with generation-bound process handles and engine-owned membership; do not enable this incomplete path yet.
- The code-only termination split first fails compilation because three unit-test call sites still name the moved helper; those references are corrected. Its full suite then catches an existing store-lease fixture's outside/inside boot-clock samples crossing a millisecond, failing the unchanged upper-bound assertion. Give the same exchange body an explicit anchor for that real-HTTP fixture, with production still reading its clock before networking. The complete current lease-fixture correction passes 449 Rust tests, 20 ignored, strict Clippy, original/expanded/Rust structure, fmt and whitespace. Its hosted proof is pending. The termination split and exec regression are retained in task stashes for the next steps.

- Move-only 6780cf1 retains rootful termination byte for byte in its own module; fresh complete validation passes 449 Rust tests with 20 ignored, strict Clippy and all three structure checks. Both rootless workflows finish green (push 43010230777 and PR 43010239503); normal/host workflows are still running. No rootful behavior changes in this split.
- The new rootless exec cancellation source passes the full local Rust suite: 453 pass, 21 ignored; strict all-target Clippy and all three structure checks pass. All five actual mapped-rootless Docker cases pass in 16.58 s, including both open-output and closed-output timeout scripts, no surviving sleep child, unchanged engine start time, and continued workload execution. Four real kernel regressions prove dead pidfds, forged task-ID arguments, cancellation cleanup and actual local peer identity handling. The original failing timed_out=true/killed=false evidence remains above. This adds about four seconds to the existing real-engine/core suites and no job or gate. Hosted Podman membership and rootful compatibility still require fresh CI; user-preset enablement is a separate checkpoint.

- User-preset enablement now follows the measured local engine on an ordinary host; recognized container contexts keep local engines disabled because a mounted socket alone proves neither host bind paths nor published-loopback reachability. Process/Wasm fallback remains available, with no engine installation or new user option. The two added Any host cells reuse the same authenticated preview archive and complete bootstrap/hello/repeat/read-only check/retained-workload upgrade/purge, additionally asserting Podman image execution with truthful missing limits or standard Docker mapping refusal and continued process service. They run on any-host pushes/manual/schedule, adding zero ordinary PR minutes and an estimated 5–10 runner-minutes each to Any host (25-minute bound each); actual timings and hosted acceptance are pending.
- Final local user-preset source passes all workspace checks with real PostgreSQL: 411 tests pass, one existing compiled-assets skip, 8,276 assertions, plus typecheck, generated contracts/docs/release checks and lint. Original and expanded source graphs pass, complete actionlint with ShellCheck and both changed shell scripts pass. No source-level container marker fixture is claimed as real host acceptance; the new hosted cells supply that proof.

- Resource-intent source b340588 is now fully green in both normal CI runs 43004264993 and 43004274291, both rootless workflows 43004265090/43004274292, and Any host 43004265088. The later lease-fixture source also passes Any host 43009175533. This closes the earlier pending status for these exact sources, not the new cancellation/preset work.
- Read both failed cancellation-source Podman logs, push 130494128080 and PR 130494153691, and the preset checkpoint repeat 130496652475. All four compute cases refuse before exec: Podman 4.9.3 Top explicitly rejects NoCgroups before invoking psgo, in both native and compatible routes. The previous source-based assumption about hpid without cgroups was incorrect. Keep the observed failure and replace that dependency with the workload init's pinned kernel PID-namespace descriptor. Direct namespace inspection is denied locally where the mapped Docker daemon has a different host UID; retain its already tested distinct-UID shim proof as Docker-only fallback. Neither path trusts command output from the workload or changes rootful termination. Fresh hosted proof is required.
- Add a CI-only 30-sample direct-node-manager collector around real image ensure/start, true exec and published-loopback HTTP; record every completed sample before strict deletion and retain partial JSON on failure. These are not the historical public-edge cohorts. The first local attempt fails because the collector supplies the wrong sandbox deletion confirmation; correct the fixture to the actual identity, retaining the production confirmation check. The next mapped-Docker run completes all 30, with create p50/p90 542.90/691.15 ms, exec 58.79/78.16 ms and HTTP 5.88/7.49 ms, but those values precede the namespace correction and are not hosted acceptance. Extra CPU-weight/PID-limit refusals now assert the real engine creates neither data nor compute. An initial test-only compile fails on private fields and the Arc receiver; use serialized public errors and the actual receiver without changing visibility.
- Repeated local engine runs exhaust Docker's bridge address pools because the new fixture removes its public bridge but leaves its owned internal bridge. Remove both fixture-owned bridges in teardown. Inspect and remove 27 empty local bridges only after their generated test name, managed deployment label and empty endpoint set agree; no host/user network or policy is pruned. The source retains strict lifecycle cleanup.

- Final namespace/doctor/collector source passes the complete local Rust suite: 453 pass, 22 ignored; strict all-target Clippy and all three source structure checks pass. All six actual mapped-rootless Docker cases pass in 45.16 s, including the real doctor, additional CPU-weight/PID refusals, timeout containment, lifecycle and all 30 serial measurements. Current local direct-node p50/p90: create 560.90/624.27 ms, exec 61.00/80.58 ms, HTTP 5.76/9.25 ms; these are this custom mapping, not hosted standard Docker acceptance. Final workspace validation passes 411 tests, one existing compiled-assets skip and 8,276 assertions, including generated-contract/docs/release/type/lint checks. Final actionlint with ShellCheck, changed-shell checks, Python syntax and whitespace pass. The doctor fixture initially writes an internal discover field and omits required standalone identity material; correct its configuration, retain the named assertions, and verify it on the actual engine.
- The user-preset fixture now adds actual engine outage/recovery acceptance for supported Podman: verify the selected test-owned Unix peer and pin its pidfd, pause it, require its ping to time out, then execute a new process workload through the installed public service. Always resume that exact descriptor; poll only read-only retained-workload availability before one new container mutation and verify the original retained bytes. This has not run locally because no installed host bundle is active here. It adds an estimated under one minute normally (bounded by three 45-second operation/read windows) to the existing new Podman host cell, and no ordinary PR job. The 30-sample collector adds an estimated 3–5 runner-minutes to the existing Podman engine job; unsupported standard Docker records refusal rather than timings. Both additions still need hosted proof.

- Namespace source 03729e9 passes the complete push Rootless engines run 43015556681: Docker 130505604597 asserts the parked mapping refusal; Podman 130505604317 passes all six actual cases in 208.73 s, including bounded discovery, real doctor, kernel capabilities, resource refusals, PID-namespace timeout containment, lifecycle/attacks and all 30 samples. Both raw artifacts are downloaded through the GitHub tool and their owner archive SHA-256 values verified before preserving JSON and provenance under docs/checks/measurements/k22. Hosted direct-node Podman p50/p90 in ms: create 4382.40/4623.39, exec 46.81/177.34, HTTP 3.14/3.20. Docker has zero timing samples and a named refusal; none are invented. PR, normal CI, actual user-preset/outage and Measure are still pending. Measure 43015602845 was dispatched on this exact source.
- A new regression reproduces a rootless guarantee upgrade without enforcement: change current controller metadata after an unlimited copy is already observed, then view incorrectly promises memory-limit. Store private per-copy configured memory/CPU observations from the owner's inspect model and intersect those with rootless engine capability. The same regression now passes, confirms configured enforcement can turn on, and preserves legacy rootful reporting. This regression models the metadata transition; it does not claim real controller delegation changed in the test. A separate actual mapped no-cgroup Docker case allocates 16 MiB beyond its 256 MiB reservation through the pinned image's maintained BusyBox awk, exits successfully and reports the allocated length, proving the off guarantee against the real workload. Full current verification and hosted acceptance of this addition remain pending.
- Final copy-guarantee source passes the full local Rust suite: 454 pass, 22 ignored; all six actual mapped-engine cases pass in 17.20 s, including the real memory overrun and all prior confinement/retention/timeout assertions. Strict all-target Clippy and all three structure checks pass. docs:check, fmt and whitespace pass. The unchanged TypeScript source retains its prior full 411-pass validation. Current rootless/host/normal CI and measured budgets still require hosted completion.

- Internal-network audit finds a separate mapped-rootless Docker gap. Three real approaches fail: a host relay cannot connect to the container's private address; explicitly requested engine port publishing produces null bindings on an internal bridge; the already reviewed util-linux 2.41.5 nsenter cannot enter the selected daemon's user/net namespaces (Operation not permitted). The scoped canary container/network are strictly removed. Park Docker private internal ingress, including supported custom mappings, with network-policy absent and named pre-data/pre-compute refusal. Preserve internet-mode isolation and rootful behavior; revisit after K27. Podman already uses native published ingress for its internal bridges, so keep it active and add a real positive test with a reachable internet canary, no default route, blocked canary egress and successful private HTTP ingress. Local Docker proves the named refusal; actual Podman positive proof is still pending. The new case adds an estimated under one minute to each existing engine/core job and no job or gate.

- Copy-limit source d186479 passes hosted Rootless engines 43017830459: Podman 130513238558 passes all six real cases in 216.32 s, including the allocation beyond the unenforced reservation; Docker 130513239010 passes named mapping refusals. Both namespace-source engine workflows 43015556681 and 43015564369 are green. Current private-ingress and scoped-placement additions still need hosted proof.
- The private-ingress candidate passes local full Rust (454 pass, 23 ignored), all seven mapped-engine cases in 14.99 s, strict Clippy and all source structure checks before the subsequent tier-announcement change. A mixed-node review finds that withdrawing Docker's global network-policy would also hide a supporting process backend or borrow its capability for Docker. Two new placement/real-Postgres regressions fail before correction: cross-tier feature leakage and an unnamed network-none request choosing container and failing source validation. Add individually announced tier qualification, supporting-tier default selection, and matching write/wake guards, preserving legacy behavior. Targeted placement/registry tests pass after correction; a wire regression also requires scoped policy serialization when only the tier advertises it and refusal before an unchanged-spec wake after loss. Both new user-preset cells exercise process network none. Schema metadata is additive and generated clients are updated; the first full workspace check correctly rejects stale generated fingerprints before regeneration. Full final checks are pending.
- Final scoped-policy source passes complete local Rust: 454 pass, 23 ignored; all seven real mapped-Docker cases pass in 18.63 s, including the named private-ingress refusal. Strict all-target Clippy and all three structure checks pass. Complete workspace validation passes 414 tests with one existing compiled-assets skip and 8,287 assertions, after regenerating both schema fingerprints; three new placement/registry/wire regressions pass. Typecheck and focused guard tests are repeated after switching the new HTTP assertions to the owner status constants; docs, complete actionlint/ShellCheck, fmt and whitespace pass. New hosted Podman private ingress/egress, process network-none presets, outage/recovery and final rootful budgets remain pending. No existing rootful Docker test body is changed.

- Read both scoped-policy Podman failures, jobs 130527497633 and 130527532012: six real cases pass, while the new private-network case rejects ownership before HTTP. Native Podman already creates its own fully owned per-workload bridge; the shared Docker internal-bridge call supplied workload labels against a deployment-owned placeholder. Remove that redundant call only for native Podman, retaining every ownership/isolation check, and add bounded boolean refusal diagnostics. Actual positive Podman private ingress still requires fresh hosted proof.
- Read complete user-preset failures 130510187084 and 130516259277: installation and initial process hello pass, then the systemd-managed node probes /var/run/docker.sock because the selected DOCKER_HOST was not retained. Save the measured absolute endpoint in the private installation record and generated configuration, restore it through clean service environments, and keep it across repeats/upgrades. A real Unix listener plus a fresh cleared-environment Bun child fails before the fix and passes after it. Acceptance now asserts both the retained record and actual daemon TOML; legacy records remain valid.
- Endpoint/private-bridge correction passes complete local Rust: 454 pass, 23 ignored; all seven real mapped-rootless Docker cases pass in 15.03 s. Strict all-target Clippy and all three structure checks pass. Full workspace checks pass 415 tests, one existing compiled-assets skip, 8,293 assertions, including type/schema/docs/release/lint checks. Complete actionlint with ShellCheck, changed-shell checks and whitespace pass. The new endpoint regression adds under one second to the existing workspace job; no new job or gate. Podman private ingress and installed-service outage/recovery are still unproved.
- Normal CI 43015558733 at namespace source 03729e9 is fully green. Read the separate b84df08 PR failure 130501629703 and its aggregate 130529739294. The authenticated celld-native-measurements artifact 13854584730 matches archive SHA-256 88cf804da36ed70541c054a4247b512f8c03ade37ada7f9faa37b69955d05755: sixteen pryx samples succeed, then HTTP 500 reports the container is not listening on port 8080. The unchanged 20.1-second upstream port verification expires while the container is still running; the cause is under investigation, not declared resolved by the later green run. No failed CI job is replayed, and no assertion or deadline is reduced.

- Retained-endpoint/private-bridge source 2d7e2a7 passes the complete Rootless engines run 43027222801. Podman job 130544875376 passes all seven actual cases in 211.39 s, including private HTTP ingress, no default route, external canary denial, memory overrun, timeout containment and all thirty measurements. Docker 130544875801 passes its named mapping refusal. Both raw owner artifacts are downloaded and archive SHA-256 checked, then preserved byte for byte with separate provenance. Latest direct-node Podman p50/p90 in ms: create 4217.50/4361.13, exec 47.99/127.72, HTTP 3.20/3.31. Keep the preceding cohort rather than overwriting its evidence. Actual installed-service endpoint/outage acceptance and final normal/Measure proof remain pending.
- Both normal CI runs at d186479 now pass completely, 43017822971 and 43017830454; namespace-source PR CI 43015564021 also completes green alongside push 43015558733. The isolated b84df08 celld port failure remains recorded without an established cause; these later unchanged integration suites pass. Read the complete older d186479 Any host failure 130536130090 in 43017822769: it again loses the selected endpoint across the user service and probes the absent system socket, the boundary corrected in 2d7e2a7. Its remaining host jobs pass; this old failure is not replayed or counted as current proof.
- Read scoped-policy Any host job 130544882688 in 43022085140 after it fails: its user service again selects the absent system Docker socket, so the Podman image case fails after the initial hello. This is the same endpoint-persistence defect fixed in 2d7e2a7, not a new private-network failure. Its Docker preset passes; remaining host jobs are still running. Fresh retained-endpoint host acceptance remains pending.
- Both scoped-policy normal CI runs now finish green, 43022085098 and 43022095611. Retained-endpoint Any host 43027222863 finishes with fourteen host jobs passing, including Docker user-preset 130555004055, and Podman 130555003924 failing. Read its complete log: selected_engine=retained_in_service_configuration succeeds, and the installed service announces container,process,wasm. The next plain hello fails Choose exactly one of image, snapshot or template because open-tier selection chooses container without an image. Image, repeat/upgrade and outage phases do not run in that Podman job. Filter automatic preferences by source rather than changing the command or weakening the source requirement. The new real-Postgres regression first fails with 422, then passes along with image/package selection, template persistence and retained explicit/fixed refusals. Complete final validation is pending.
- Source selection also needs a legacy snapshot’s stored image/packages when its isolation field is absent. The initial local correction fails that added regression by selecting process for a container image. Include inherited source fields in eligibility, retaining recorded snapshot/template tiers. All ten focused source/resource-intent tests then pass with 63 assertions; all three structure checks pass. Keep a real named container as the Podman upgrade/outage canary and assert its identity before pausing the engine; image-free process requests continue to exercise ordinary automatic selection.
- Final source-aware correction passes complete workspace validation: 416 tests, one existing compiled-assets skip, 8,303 assertions in 115.75 s, including lint/type/schema/docs/release checks. All three structure checks pass with cargo on PATH; the first structure invocation omitted cargo and is not counted as a pass. ShellCheck, complete actionlint, Python syntax and whitespace pass. Rust runtime sources are unchanged from the seven-case green engine/core proof at 2d7e2a7. The new Postgres regression adds under one second in the existing workspace job and the canary identity read adds a bounded five-second read to the existing host case, with no new job or gate. Fresh Podman full install/upgrade/outage/purge acceptance and complete current-source CI/Measure remain pending.

- Namespace source 03729e9 now has complete green Measure 43015602845: both one-box 130505762962 and image/control 130505763341 jobs pass every step, including historical budgets, all three-tier/capacity comparisons and default/Wasm image startup. This proves that exact source. Dispatch current-source Measure 43034003118 on 9d1ed90 after the source-selection correction, alongside normal CI 43033907246 / 43033915411, Any host 43033907235 and Rootless engines 43033907350 / 43033915390; these fresh runs remain pending.
- Read both 9d1ed90 Podman setup failures, push 130567650314 and PR 130567679376: Ubuntu archive/azure host DNS fails for six minutes of apt retries, then all six pinned-package curl attempts fail resolution. No engine or runtime test runs there; both Docker cells pass. Use a separate HTTPS Ubuntu mirror for invocation-only apt sources with the original suites/components/signing keys and strict update error handling. Add the same-version same-digest Podman package fallback after bounded longer-delay retries. The real mirrored package download matches e5c1c37e387ed14c352a744a75fbb79fb2f82573ca7bf36886e3b7333fc9ef1a. No package was installed locally and hosted corrected setup remains pending.
- A new actual-HTTP acquisition regression fails with the old fetch function on its missing primary, then passes with fallback: missing/corrupt primary retrieves the exact expected bytes; corrupt primary and mirror both refuse. It exercises the actual shell function without running engine setup or executing downloads. All three structure checks, ShellCheck and complete actionlint pass. This adds about 0.2 seconds in the existing workspace job and no job or gate; workflow timeout bounds and every runtime assertion remain unchanged. Final full workspace validation is pending.
- Final mirror-correction workspace validation passes 417 tests, one existing compiled-assets skip and 8,309 assertions in 135.12 s; type/schema/docs/release/lint checks pass. All three structure checks, ShellCheck, complete actionlint and whitespace pass. The package mirror itself supplies the pinned digest; corrected apt setup and the real engine/user-preset suites still require hosted proof. No Rust runtime or control behavior changes in this checkpoint.
- Read complete 9d1ed90 Any host failures 130577927090 and 130577927116: authenticated release transport and pinned Ubuntu image acquisition pass, then Ubuntu archive/security DNS fails and APT cannot locate the test prerequisites. No pryx installer or sandbox runs in these two cells. Apply the same manifest-owned HTTPS Ubuntu mirror only to disposable container/booted fixture sources. Mount the runner’s public CA bundle read-only to bootstrap the minimal image’s own CA package, preserving TLS, native Ubuntu signatures/digests, default profiles and every original assertion. No product installer option, job, required gate or timeout changes.
- The actual pinned Ubuntu 24.04 image completes the corrected prerequisite-only path locally through rootless Docker: APT validates metadata and installs curl, certificates, Python and the existing test utilities; the owned container removes itself. This is package-preparation proof, not local installer/outage acceptance. ShellCheck, complete actionlint and whitespace pass. The unchanged TypeScript/Rust product retains its latest complete 417-pass workspace and prior full Rust/engine checks; full corrected hosted acceptance remains pending. Healthy-path cost is one read-only CA bind and mirror metadata acquisition in the existing cells, with no added job.

- Mirror source 8113e0d passes complete Rootless engines 43037020040: Docker 130578422379 and Podman 130578422799 both succeed. The corrected signed apt metadata/package setup reaches every real runtime test, including the thirty-sample collector and private-network case. The latest container-fixture source is b1c4704; its Any host 43038188074, normal CI 43038187899 / 43038200113 and Rootless engines 43038188334 / 43038199922 are pending. Product sources still match 9d1ed90, whose Measure 43034003118 is running.
- Scoped-policy source 34f29ce also finishes fully green Measure 43022241363: image/control 130528028875 and one-box 130528029134 pass every budget, collector and summary step. This retains separate exact-source proof alongside the earlier green namespace-source run. Current product-source Measure 43034003118 remains running.

- Read complete Podman user-preset failures 130590167365 (8113e0d) and 130593418337 (b1c4704). Setup and retained endpoint pass, and plain hello succeeds in 56/61 seconds; explicit image create then receives node 422 before compute. The container agent trust guard rejects root-owned ancestors for a non-root node, so / and /home make its otherwise owned static agent unavailable. A real-filesystem regression initially fails on an incomplete test config; after adding the required API section it reproduces None for the trusted caller-owned executable. Accept only non-writable root-owned ancestors, retaining the file owner and all writable/symlink checks. The regression then passes; hosted image/upgrade/outage/purge still needs proof.
- Read complete older 8113e0d host failures 130590167590 / 130590167807: Ubuntu test prerequisites again fail DNS before installing anything, the fixture issue corrected in b1c4704. Booted Alpine 130590167494 and Debian 130590167499 instead fail at host Incus dependency acquisition before any distribution boots. Use invocation-only signed mirror sources for that host setup too, keeping its pinned Incus version. Remaining older host jobs are pending. Both normal CI runs at source-aware 9d1ed90 now pass fully, 43033907246 and 43033915411.
- The first full ancestor-correction Rust run passes the new trust regression but stops at the existing real Wasm terminal/listener case: its unchanged three-second deadline expires with a running guest and no HTTP listener. Preserve /tmp/k22-agent-ancestor-full.log; the cause is not established. Run the entire suite serially with the same deadlines/assertions and still require default hosted CI; do not count the failed run as a pass or claim a Wasm repair. Complete final validation remains pending.
- Read the latest b1c4704 booted Debian/Alpine failures 130593418434 and 130593418451: both again stop at Incus host package acquisition on Ubuntu DNS before boot. The pending invocation-only host mirror correction targets this exact setup; no guest assertion changes. The first serial local Rust run passes the Wasm listener case and advances through lifecycle tests, then the execution environment restart interrupts its process at the probe suite. Preserve that partial log separately and restart the entire check; it is not a complete pass.
- Read the remaining b1c4704 booted Ubuntu/Fedora failures 130593418574 and 130593418606: the same Incus host package DNS failure prevents boot. Both Ubuntu container cells, 130593418650 and Landlock-denied 130593418647, pass full acceptance on the HTTPS mirror/CA fixture, proving that prior setup correction. The full restarted serial Rust suite passes 455 tests with 23 ignored; strict all-target Clippy and all three structure checks pass. Default hosted CI of the trust correction is still required.
- An additional local engine invocation omits PRYX_TEST_ENGINE after restart: six real cases pass, while doctor correctly rejects the fixture’s mistaken rootful expectation for this actual rootless engine. Set the explicit Docker rootless fixture mode and selected socket, retain the failed log, and repeat all seven cases without changing assertions. Final tagged engine validation is pending.
- Final ancestor/Incus correction passes the complete serial Rust suite: 455 pass, 23 ignored; strict all-target Clippy, all three structure checks, fmt, docs, ShellCheck, complete actionlint and whitespace pass. All seven actual mapped-rootless Docker cases pass in 19.63 s with the correct explicit engine fixture. Keep the earlier parallel Wasm failure and interrupted serial run separate, and require default hosted tests of this source. The unchanged TypeScript product retains its complete 417-pass workspace result. The real metadata regression adds under one second in the existing Rust job; Incus mirror selection adds no job, gate or healthy-path delay. Hosted full Podman image/upgrade/outage/purge, booted distro acceptance and final CI remain pending.
- Ancestor/Incus source 319fdaa completes both Rootless engines runs green, 43047285575 and 43047295596. Any host 43047285188 passes fourteen of fifteen jobs, including all four booted distributions, both Ubuntu containers, standard Docker refusal and every original host preset. Read the complete failed Podman job 130626268320: selected socket persists, plain hello succeeds in 70 seconds and actual image execution/off-limit assertions pass, then the retained-workload create refuses placement after repeated up. Its public message hides the reason; add bounded read-only private placement diagnostics before fixing any admission or availability behavior. Upgrade/outage/recovery/purge on this Podman preset is still unproved.
- Public copy-bound reporting checkpoint: the real-Postgres controller-gain regression fails before the correction with memory/CPU incorrectly true, then passes with the matching epoch observation required. It also covers individual limits, controller loss, missing/stale reports and legacy semantics. The existing node regression now verifies the heartbeat map and unchanged rootful omitted bytes. First compilation catches a test-only NodeId constructor typo; first workspace validation catches a nullable report field spread into a non-null view fixture; both are corrected without loosening assertions. The mixed structure checker catches 601 class lines; reusing the already selected tier keeps the class within its unchanged limit. Final full default Rust suite passes 455 tests, 23 ignored; full workspace check passes 418 tests, one existing compiled-assets skip, 8,325 assertions. Strict all-target Clippy, fmt, all three structure checks, schema/SDK generation, docs, ShellCheck/actionlint and whitespace pass; hosted source proof remains pending. The additional regression costs under one second in the existing workspace job; no job or required gate is added.

- After pushing copy-report source 5da54b4, an additional correctly tagged local mapped-Docker run passes six cases and fails lifecycle HTTP with Reqwest IncompleteMessage (tests/docker/engine_lifecycle.rs:113); preserve /tmp/k22-copy-engine.log and do not claim seven passes for this invocation. A 200-request actual-engine canary using the exact netcat responder succeeds, and the backtrace-enabled lifecycle attempt also passes unchanged in 6.20 seconds; neither establishes the original cause. No assertion, deadline or HTTP retry changes. Hosted engine proof remains required. Separately verify the new bounded diagnostic against real local PostgreSQL and assert that the persisted password and node name never appear in its output; its disposable event table and files are removed.
- Copy-report source 5da54b4 completes Rootless engines 43052744144 green: Podman 130633985244 passes all seven actual cases in 198.45 seconds, Docker 130633985663 verifies the named pre-compute mapping refusal. Read the positive Podman log and download artifacts 13871036654 / 13871325547; verify GitHub archive SHA-256 before preserving the exact JSON in docs/checks/measurements/k22. Podman completes thirty samples: create p50/p90 3900.08/3987.13 ms, exec 44.21/47.90 ms, HTTP 2.88/3.03 ms. Docker has zero samples with data-owner-mapping refused. Update the documented cohort without claiming a causal improvement over the prior runner. Normal CI, host install/recovery and final Measure remain pending.
- Copy-report Any host 43052744200 again passes fourteen of fifteen jobs; read failed Podman 130645971620 completely. This time actual image creation, retained upgrade bytes, process hello while the verified engine peer is paused, container recovery and retained bytes all pass (engine_outage=process_serving engine_recovery=container_serving). Purge then refuses Not Found and preserves installation state. The earlier 319fdaa placement refusal does not recur, but its cause is not established and no placement repair is claimed. Purge has already stopped embedded PostgreSQL, so the failure diagnostic correctly reports its query unavailable.
- Read Podman v4.9.3 upstream at 8d2b55ddde1bc81f43d018dfc1ac027c06b26a7f outside the repository: no compatible update route exists, native update only accepts cgroup resources, and explicit no/empty restart policies are disabled. A transport regression fails before the correction on an unsupported update, then verifies disabled observations without updates and rejects active/unknown policy. Add that check to the real existing engine timeout case too. Cleanup retains its complete ownership validation and state-on-error behavior, with operation-phase context. The same hosted log shows an unused shared internal bridge failing isolation validation after recovery; omit that unused native placeholder operation while retaining every per-workload bridge check and attack. Full default Rust passes 456 tests with 23 ignored; native/mixed structure and docs checks pass. Final Clippy/Rust structure, tagged local engine and hosted purge proof remain pending. Estimated added CI cost is under two seconds in the existing engine job, with no new job or required gate.
- Restart/placeholder correction passes strict all-target Clippy, all three structure checks, fmt/docs/whitespace and all seven actual mapped-rootless Docker cases in 18.92 seconds, including the added real restart-policy check. Final review of local cleanup finds a separate data-safety gap: auto construction omits an unavailable backend, then unbound inspection can treat its recorded compute as absent. The first new fixture depends on unavailable local Landlock support and fails for that setup reason; isolate the actual ownership validator with the existing router/fake backend and real on-disk records/data instead. It meaningfully reproduces Ok([]) for an unavailable recorded container before the correction. Require and bind the persisted tier before inspection, reject unresolved legacy auto records, and retain the full-set ownership check before any compute mutation. Final complete Rust/Clippy/structure validation of this additional guard and hosted purge proof remain pending.
- Follow the renewed user instruction: both Docker pieces stay parked until the mandatory revisit after K27; make no further implementation attempts on them during K22. Finish Podman/supported-engine cleanup, obtain green final-source CI, record done except and proceed directly to K23. Apply the same three-distinct-attempt parking rule to each remaining failure. Fetch origin any-host and checkout any-host again successfully, retaining the current authorized work.
- The next parallel full Rust attempt fails the newly added positive cleanup fixture because FakeRuntime mutation methods accept names while cleanup deliberately uses immutable IDs; keep that unit at the ownership-validation boundary and assert the verified observed ID is bound before mutations. Also retain the unchanged real Wasm listener failure: the three-second deadline expires with the guest running and no port 3000 listener; its cause is not established. Preserve /tmp/k22-purge-complete-full.log. No existing assertion or deadline changes; run the complete suite serially with the corrected new fixture and still require the default hosted suite. The prior complete CLI-tier-guard suite passed 457 tests before the added observed-ID regression; final 458-test validation remains pending.
- Both normal copy-report CIs finish green, 43052744156 and 43052751503. The complete final serial Rust suite passes 458 tests, 23 ignored, with all original assertions/deadlines. Strict all-target Clippy and Rust structure pass. The corrected ownership-only observed-ID test verifies that validation binds the immutable identity without mutation; actual engine mutation coverage remains in the engine/host suites.
- Read older Measure 43034003118 failure 130567995824: its only failed step is the existing process-speed CLI image refusal assertion, which receives the new generic eligibility message instead of the established process image problem. One-box 130567994915 passes. A real-Postgres regression reproduces 503 before the correction; restore the Rust-owned process/Wasm validation error when no healthy eligible image tier exists, with no compute request, while preserving container network/guarantee refusal semantics. The focused six-case suite then passes and the original benchmark assertion is unchanged. Final full workspace/source validation and hosted proof are pending.
- Final supported-engine cleanup/refusal checkpoint passes complete serial Rust: 458 pass, 23 ignored; complete workspace check: 419 pass, one existing compiled-assets skip, 8,331 assertions in 113.50 seconds. Strict all-target Clippy, all three structure checks, fmt, docs, full ShellCheck/actionlint and whitespace pass. Keep the failed parallel Wasm/test-fixture log separate; require default hosted tests. Seven actual mapped-rootless Docker cases passed after the restart/placeholder correction in 18.92 seconds before the later cleanup-only guard; no further attempt on either parked Docker piece is made. New regressions add under one second to existing unit/workspace checks and under five seconds to the existing engine case, with no new job or gate. Push this supported-engine correction, require final Podman purge and CI proof, then record done except for the two named Docker pieces and begin K23 immediately.
- Cleanup checkpoint 87400f3 completes both Rootless engines workflows green (43063680938 and 43063691928); Podman 130672781111 passes all seven real cases in 161.20 seconds, including the new inspected restart-policy check and thirty samples. Any host 43063680778 passes thirteen of fifteen: read both complete failed logs. Podman 130682803543 refuses retained placement because isolation tier container no longer matches after repeated setup; it does not reach purge on this invocation. Booted Ubuntu 130682803523 assumes a nonexistent Deb822 source file, then its outer shell masks preparation failure and reaches systemd-run without the test user (217/USER). Normal CI and fresh Measure remain pending.
- First targeted attempt on the intermittent supported Podman tier loss: retain only successfully verified stable Podman negotiation/version metadata, with live compatible/native controller and policy reads on every health check. Engine version/transport/native changes invalidate it and the client before re-probing. A delayed real-Unix-socket regression fails before this change because repeat health performs four version requests, then passes with two, fresh controller changes, fail-closed version mismatch and successful renegotiation. This does not establish the hosted contention hypothesis. Preserve the two-second deadline and rootful path; add fixed tier/timeout-only failure diagnostics. The first compile uses a nonexistent Result convenience method and is corrected to the owner's existing is_ok_and API before the regression passes.
- First fix of the booted Ubuntu source-format failure: prepare both supported APT file formats in disposable guests, preserving signature/key/suite/component fields, and make booted preparation fail immediately. The actual previous preparation block rejects a valid legacy source fixture before the correction. The real shell function now passes legacy/signed-by/deb-src/Deb822 fixtures and refuses an empty source set; ShellCheck and complete actionlint pass. No installer dependency or new job/gate is added. The extra metadata regression adds approximately three seconds to the existing core suite and the source-format check under one second to the existing workspaces suite.
- Read the older Measure 43047495042 control failure 130615244629: it is the same existing CLI image-refusal assertion already corrected by 87400f3, with one-box passing. No rerun or benchmark assertion change; preserve its failed cohort and await the fresh corrected-source Measure 43063939164.
- Final metadata/source-format candidate passes complete serial Rust (458 pass, 23 ignored), full workspace check against real PostgreSQL (420 pass, one existing compiled-assets skip, 8,337 assertions), strict all-target Clippy, all three structure checks, fmt/docs/whitespace, changed-shell ShellCheck and complete actionlint. Hosted proof of this first targeted contention attempt and Ubuntu correction remains required. Both parked Docker pieces remain untouched.
- Metadata source 197a372 completes both Rootless engines workflows green: push 43071213897 and PR 43071225373. Push Podman 130699258860 passes all seven real cases in 136.67 seconds; PR Podman 130699301178 passes in 104.59 seconds on merge df101ff. Keep the push cohort as the documented source: download artifacts 13878187625 (Podman) and 13879050653 (Docker), verify owner archive SHA-256, and retain the exact JSON/provenance. Thirty Podman samples give create p50/p90 2247.29/2472.42 ms, exec 75.68/265.73 ms, HTTP 3.07/3.20 ms. This is a separate runner cohort, not a causal comparison or public-edge measurement; standard Docker still records zero samples and the named mapping refusal.
- Source 197a372 also passes hosted core 130699803135 under default test scheduling, including all twenty-one selected actual rootful Docker cases, contract 130699803119, workspaces 130699803118 and dependency 130699803943. Earlier normal sources 319fdaa and 87400f3 finish both push/PR runs green. Read the older 5da54b4 Measure control failure 130634575137: it is the same already corrected CLI image-refusal assertion. Fresh corrected-source Measure 43063939164 has no failed steps and has passed ordinary-process image acceptance and every preceding budget stage; complete three-tier comparisons remain pending.
- Read current host Podman 130710417282 completely: APT fails during engine prerequisites, before any Podman/pryx command, after a partially successful kernel.org index read falls back to an unreachable IPv6 address. The metadata correction's supported-preset attempt is therefore still unexercised here. Use IPv4 plus a second signed archive mirror on strict index failure, in invocation-only host sources and disposable guest prerequisites. Verify the actual MIT Podman pool file against the unchanged e5c1c37e387ed14c352a744a75fbb79fb2f82573ca7bf36886e3b7333fc9ef1a SHA-256. Both existing acquisition/source-format regressions pass (two tests, twelve assertions), full ShellCheck/actionlint and docs/whitespace pass. No Rust/TypeScript implementation changes after their green source checks, no added job/gate, and extra acquisition work only on failure. Do not rerun the failed job; push this cause-specific infrastructure correction. Remaining host cells and final normal/Measure completion are still pending.
- K22 closes as done except on 7963826182e3f8e9037aea17d4716df15086bd81: both complete normal CIs are green (push 43075452804 and PR 43075462394), including their aggregates, rootful Docker, full end-to-end attacks, Wasm agent suite, two isolated nodes, real Rivet/celld integrations and store-cut/watchdog/recovery. Both Rootless engines runs 43075452793/43075462656 are green. Any host 43075452782 passes all fifteen jobs. Read supported Podman 130721500313: selected socket persists, hello takes sixty seconds, image execution reports missing limits, process network-none runs, engine outage keeps process service and resumes container service, retained upgrade/outage bytes survive, and purge completes. Booted Ubuntu now completes its original flow too. No K22 test, assertion, hardening or deadline was skipped or loosened.
- Only two K22 pieces remain parked: standard rootless Docker data-owner mapping and its dependent positive lifecycle/timings (default/host write denial, unsupported keep-id); rootless Docker internal ingress/network-policy (failed private-IP relay, null native published binding, denied maintained namespace entry). Both retain named pre-compute refusals. Make no further attempt before K27; give each its mandatory second try afterward. Supported Podman and rootful engines are complete. Keep raw hosted source-197 engine samples/provenance separate from public-edge figures. The older supplementary Measure 43063939164 is still running its complete three-tier comparisons; its preceding historical budgets and corrected CLI image-refusal stage already pass. Continue monitoring it during K23 and read any later failure; do not label the unfinished comparisons as green or add an unrequested completion gate beyond K22's proved engine cohorts.
- Start K23 immediately after K22's final required proof. Re-read AGENTS.md, PLAN.md's execution rules, this progress record, the relevant K17–K22 decisions, PLAN-K23.md, the browser/tool design and native/interpreter contracts. First prove actual same-UID Unix-socket privacy before choosing pathname or scoped abstract IPC; a supported API or private directory alone is not proof. Native process tools remain refused until their confinement and verified bundle paths work. No new tool binary has been executed. Read-only builder review finds micromamba 2.10.0's published checksum sidecar inconsistent with the GitHub REST asset digest: owner asset 679631394 reports 9a97976244929bbae28fb1fae504e74a3e7e6d1932870e3a03321f01b7fbd07b, which matches the downloaded archive; the sidecar's own owner digest also matches its differing content. Keep that provenance explicit and complete its packaged-library licence audit before use; do not blindly accept mismatched bytes. Builder choice is not yet made.

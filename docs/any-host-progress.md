# Any-host progress

Continue on `any-host`. Follow `PLAN-ANY-HOST.md`, `AGENTS.md`, `PLAN.md`,
`docs/any-host.md`, `docs/decisions.md` and the current milestone brief.
Only recorded CI evidence establishes completion; a local pass alone does not.
Measurement files now contain summaries only (except the two process-tool build inputs named
in checks/measurements/README.md). Historical references to retained raw logs, dumps, samples
and cancellation inventories mean their CI artifacts; no raw output is kept in this checkout.

| Milestone | Status | Finishing commit | CI proof | Ran | Did not run | Parked and why | Left |
| --- | --- | --- | --- | --- | --- | --- | --- |
| K19 | done | `b535d24b95ab2046aa6178fdc1d2683640ce2544` | [42777018303](https://github.com/cubepals/pryx/actions/runs/42777018303), green including aggregate | Real hosted KVM, full Rust and Docker suites, CLI doctor, default-profile image probes, non-root API refusal, full end-to-end attacks, one-box/Action/Python, Wasm, two-node, upstream integrations and recovery; local checks below | Live paid-provider provisioning and publication; neither is authorized | None | None |
| K20 | done | `54bd364` | Normal CI 42832474233 and Any host 42832474296, both green | All 13 host jobs: 8 user distro contexts, root container/PG UID, fallback, refusal, sudo, relocation; full normal CI including real Docker/KVM, one-box/Action, Wasm, 2-node, Rivet/celld and recovery | Production main/release prohibited; unchanged credentialed cloud/bootstrap cases skipped | None | Complete; proceed to K21 |
| K21 | done | `de3ef9dc13e955e96c2f7c1e530f800069ff50c0` | Normal CI [42885151433](https://github.com/cubepals/pryx/actions/runs/42885151433), Any host [42885151332](https://github.com/cubepals/pryx/actions/runs/42885151332), Measure [42885520235](https://github.com/cubepals/pryx/actions/runs/42885520235), all green | All normal jobs and aggregate; all thirteen host jobs; both complete measurement jobs, including historical budgets and three-tier collectors | Production main/release prohibited; unchanged credentialed cases | None | None |
| K22 | done except | `7963826182e3f8e9037aea17d4716df15086bd81` | Normal push [43075452804](https://github.com/cubepals/pryx/actions/runs/43075452804) and PR [43075462394](https://github.com/cubepals/pryx/actions/runs/43075462394), Any host [43075452782](https://github.com/cubepals/pryx/actions/runs/43075452782), Rootless engines [43075452793](https://github.com/cubepals/pryx/actions/runs/43075452793) and [43075462656](https://github.com/cubepals/pryx/actions/runs/43075462656), all green; supplementary Measure [43063939164](https://github.com/cubepals/pryx/actions/runs/43063939164) on source 87400f3 is also green | Full normal CI including actual rootful Docker, Wasm, two-node, integrations and recovery; all 15 host jobs; Podman lifecycle/attacks, install/repeat/upgrade/outage/recovery/purge; controller refusals, copy-bound guarantees and 30 hosted engine samples | Main/release prohibited; unchanged credentialed cases | Standard rootless Docker data-owner mapping with dependent positive lifecycle/timings; rootless Docker internal ingress/network-policy. Three approaches each recorded; both remain named refusals until their post-K27 second try | Supported scope complete; K23 started |
| K23 | done | `5945724a4ac8fbc28610292d701ffa7e9bffc040` | Normal push [43233699289](https://github.com/cubepals/pryx/actions/runs/43233699289) and PR [43233712859](https://github.com/cubepals/pryx/actions/runs/43233712859), Any host [43233699231](https://github.com/cubepals/pryx/actions/runs/43233699231), Rootless engines [43233699487](https://github.com/cubepals/pryx/actions/runs/43233699487) and [43233712730](https://github.com/cubepals/pryx/actions/runs/43233712730), CodeQL [43233712847](https://github.com/cubepals/pryx/actions/runs/43233712847), all green | Signed fresh ordinary-user installation without engine; actual pinned upstream Python/Node paths and host glibc; both SDKs, state/charts/errors/interrupt/restart, MCP/CLI; browser screenshot/forwarded CDP and guarantee; thirty process and container samples per tool; default-Docker kernels and namespace/required-browser refusals; neighbour isolation, purge, Alpine relocation; full normal CI and all sixteen host jobs | Main/release prohibited; unchanged credentialed cases | None; corrected relocatable bundle is complete, with no binary rewriting or private tool glibc | Complete; begin K24 immediately |
| K24 | done | `53e83505f823cf4b92aca2b4a952cf084c636f32` | Normal PR [43421825320](https://github.com/cubepals/pryx/actions/runs/43421825320) and all sixteen Any-host jobs [43421813961](https://github.com/cubepals/pryx/actions/runs/43421813961), green | Signed user install, real SDK/CLI desktop and HTTP SSH/wake, noVNC/expiry, Chromium sandbox and neighbour privacy, OpenSSH PTY/keys/leases, configured doctor/placement, 30 samples per tool, unchanged container budgets, full normal and host matrix | Main/release prohibited; unchanged credentialed cases | None | Complete; begin K25 |
| K25 | in progress | — | Kernel [43442622901](https://github.com/cubepals/pryx/actions/runs/43442622901) and thirty-per-path comparison [43450896770](https://github.com/cubepals/pryx/actions/runs/43450896770), green; repeated pasta stall retained | Verified published inputs, thirty ordinary-user KVM EROFS/overlay/ext4/vsock boots, disk reboot, both network cohorts and management denials | Production VM bundle/backend/lifecycle/confinement/timings; no local KVM | None | Both spikes have recorded decisions; build the VM tier and retain the unresolved alternative-path MAC issue |
| K26 | not started | — | — | — | All | — | Complete brief after K25 |
| K27 | not started | — | — | — | All | — | Complete brief after K26, then revisit every parked piece | Raw output belongs to the cited CI artifacts; the repository keeps only its summary.

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
- Full current Bun check: 401 pass, one existing compiled-assets skip, 8,230 assertions, real local PostgreSQL 17.11. Python: 23 tests with one existing credential skip; generation/check pass for node/public APIs and both SDKs. No complete rootless engine lifecycle, standard UID-map ownership, delegated-controller attack or rootless timing claim is made. This step adds one short real case to the existing Docker job and one short workspace test, estimated under one extra minute per pull request, with no new job or gate. Measure proof remains required for final K22 source. Raw output belongs to the cited CI artifacts; the repository keeps only its summary.

- Policy source 64c517f finishes green in normal CI 42973067945 and all thirteen Any host jobs in 42973067832, including the new real rootful enforcement branch. Move-only source 6b462f6 splits network operations; complete local Rust, strict Clippy and structure pass. Its hosted core, contract, workspace and dependency jobs pass in pending CI 42976541144; deployment and host-matrix results remain pending.
- K22 sleep checkpoint: control reads the current copy guarantee and stops directly when memory sleep is false or unknown, with no mutation on stale epoch or read failure. Legacy node behavior is preserved. The node rejects an unsupported freeze before saving suspension or pausing. Complete local Rust: 439 pass, 18 ignored (the additional real engine sleep case runs in the existing Docker CI step); strict Clippy, fmt and Rust/TypeScript structure pass. Complete Bun: 406 pass, one existing skip, 8,241 assertions against real PostgreSQL. The targeted actual rootless Docker stop/wake test passes in 3.77s with retained data; the rootful freeze/resume branch remains hosted proof. An initial TypeScript check caught the local view alias missing the existing optional guarantees field; the corrected alias and complete rerun pass. No new job; estimated CI cost under one minute per pull request.

- Network move 6b462f6 finishes green in normal CI 42976541144 and all thirteen Any host jobs in 42976541933. Sleep source ab6b8e2 has all thirteen Any host jobs green in 42978725930 and green hosted core/Docker, workspace, contract and dependency jobs in pending normal CI 42978725972; deployment completion remains pending.
- K22 outage checkpoint: independent serving health and complete-per-tier inventories preserve unavailable records and routes, report their state unknown and guarantees off, and keep healthy tiers available. Raw all-backend info/list remain strict. Health recovery alone cannot advertise a stale tier; a consistent inventory is required. Per-tier outage generations keep recovered crashes out of watched-failure restart policy. Four new regressions cover continued process/Wasm service, all-tier failure and single-tier recovery, byte-identical records after startup with an unreadable engine, and crash attribution. Existing ambiguous identity, unresolved legacy and atomic discovery tests remain unchanged and pass.
- Complete current local outage checks: 443 Rust pass, 18 ignored; strict all-target Clippy, fmt, Rust structure, OpenAPI freshness and TypeScript structure pass. The unchanged TypeScript sources retain their latest full 406-pass check. An early new regression used the wrong public test aliases; corrected without changing production visibility or existing assertions. The unresolved legacy regression also caught and restored its existing startup refusal before this final run. This is not yet real engine-outage CI proof. Estimated addition under one second in the existing native job; no new job or gate. Raw output belongs to the cited CI artifacts; the repository keeps only its summary.

- Sleep source ab6b8e2 finishes fully green in normal CI 42978725972 and all thirteen Any host jobs in 42978725930. Outage source 1c56ae9 has green PR CI 42981562718 and all thirteen Any host jobs in 42981556121. Push CI 42981556160 fails only one-box job 130392960667 and its aggregate 130412369251; both full failure logs were read. The host probe exits 137 with no diagnostic output under its existing thirty-second caller deadline. The log does not identify its phase or prove OOM. All other jobs, including full native end-to-end, pass. Preserve that failure and add phase and bounded kernel/memory diagnostics; do not replay the job or increase its deadline.
- Move-only 70bbca7 extracts existing engine preflight checks before adding the native adapter. Complete local Rust passes 443 tests with 18 ignored, strict Clippy and structure pass. An initial local run fails four tests on ENOSPC and one Wasm execution; retaining two dependency variants filled the 32-GiB workspace. Removing 9.1 GB of obsolete test executables permits the complete unchanged suite to pass. Its normal CI 42987105772 and Any host 42987105647 are pending.
- K22 native adapter checkpoint: generated Podman owner models and OCI mounts over existing Reqwest Unix transport handle native Netavark bridges and create; each workload gets a separate bridge with its retained isolation option checked. Native create disables implicit writable temporary filesystems, keeps nonzero IDs/capability drop/no-new-privileges/read-only root, bounded logs and available hard limits, and uses keep-id to map the daemon-owned data into its workload UID. Explicit configured data ownership remains authoritative. Custom native-tool profiles, hostname egress and FUSE remain refused on this adapter. Native controller metadata and its systemd manager are prerequisites for reporting rootless limits on; without controllers, accepted requests disable cgroup creation and leave limits off. Fresh create also rechecks explicitly requested CPU weight and PID limits.
- The latest full Rust SDK attempt fails unchanged cargo-deny on unmaintained paste. Replace its transport with maintained Reqwest and retain only its generated owner types; no advisory waiver, source rename or policy change. Final current local checks pass: 446 Rust tests, 18 ignored; strict all-target and ordinary Clippy; Rust structure; complete advisory/license/source/bans checks; OpenAPI freshness; actionlint and shell/Python syntax. The two actual local rootless Docker cases pass in 5.30s with inferred ownership, controller refusals, stop/wake and retained bytes. The ownership diagnostic independently measures default/host writes succeeding under this machine's custom UID map and keep-id rejected. These are not standard rootless Docker or Podman acceptance.
- Opt-in phase tracing preserves JSON stdout and reports only fixed phase names and monotonic timestamps. A real local debug probe completes and parses all three tier entries; distribution verification takes 25.27s. That identifies a real expensive phase but does not establish the hosted release-probe failure cause. The thirty-second deadline remains unchanged and no distribution hash is skipped.
- Add the authorized Rootless engines workflow on branch pushes, manual dispatch and relevant pull requests: two ordinary-user Ubuntu jobs with pinned Docker/RootlessKit and Podman packages, explicit digest validation, two actual resource/sleep cases and three recorded data-mapping strategies. Estimated extra PR cost is 10–15 runner-minutes per job (20–30 total, 10–15 wall minutes), with a 35-minute cold-build bound per job; real durations are unmeasured. Startup's AppArmor attachment permits user namespaces only for the test engine's exact RootlessKit executable; workload profiles are unchanged. Full engine suites, adversarial isolation, lifecycle/port/snapshot/fork measurements, supported user-preset enablement, standard mappings, exact freezer/low-port reporting and final Measure remain open. No piece is parked yet. Raw output belongs to the cited CI artifacts; the repository keeps only its summary.

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
- The code-only termination split first fails compilation because three unit-test call sites still name the moved helper; those references are corrected. Its full suite then catches an existing store-lease fixture's outside/inside boot-clock samples crossing a millisecond, failing the unchanged upper-bound assertion. Give the same exchange body an explicit anchor for that real-HTTP fixture, with production still reading its clock before networking. The complete current lease-fixture correction passes 449 Rust tests, 20 ignored, strict Clippy, original/expanded/Rust structure, fmt and whitespace. Its hosted proof is pending. The termination split and exec regression are retained in task stashes for the next steps. Raw output belongs to the cited CI artifacts; the repository keeps only its summary.

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

- Namespace source 03729e9 passes the complete push Rootless engines run 43015556681: Docker 130505604597 asserts the parked mapping refusal; Podman 130505604317 passes all six actual cases in 208.73 s, including bounded discovery, real doctor, kernel capabilities, resource refusals, PID-namespace timeout containment, lifecycle/attacks and all 30 samples. Both artifact summarys are downloaded through the GitHub tool and their owner archive SHA-256 values verified before preserving JSON and provenance under docs/checks/measurements/k22. Hosted direct-node Podman p50/p90 in ms: create 4382.40/4623.39, exec 46.81/177.34, HTTP 3.14/3.20. Docker has zero timing samples and a named refusal; none are invented. PR, normal CI, actual user-preset/outage and Measure are still pending. Measure 43015602845 was dispatched on this exact source.
- A new regression reproduces a rootless guarantee upgrade without enforcement: change current controller metadata after an unlimited copy is already observed, then view incorrectly promises memory-limit. Store private per-copy configured memory/CPU observations from the owner's inspect model and intersect those with rootless engine capability. The same regression now passes, confirms configured enforcement can turn on, and preserves legacy rootful reporting. This regression models the metadata transition; it does not claim real controller delegation changed in the test. A separate actual mapped no-cgroup Docker case allocates 16 MiB beyond its 256 MiB reservation through the pinned image's maintained BusyBox awk, exits successfully and reports the allocated length, proving the off guarantee against the real workload. Full current verification and hosted acceptance of this addition remain pending.
- Final copy-guarantee source passes the full local Rust suite: 454 pass, 22 ignored; all six actual mapped-engine cases pass in 17.20 s, including the real memory overrun and all prior confinement/retention/timeout assertions. Strict all-target Clippy and all three structure checks pass. docs:check, fmt and whitespace pass. The unchanged TypeScript source retains its prior full 411-pass validation. Current rootless/host/normal CI and measured budgets still require hosted completion.

- Internal-network audit finds a separate mapped-rootless Docker gap. Three real approaches fail: a host relay cannot connect to the container's private address; explicitly requested engine port publishing produces null bindings on an internal bridge; the already reviewed util-linux 2.41.5 nsenter cannot enter the selected daemon's user/net namespaces (Operation not permitted). The scoped canary container/network are strictly removed. Park Docker private internal ingress, including supported custom mappings, with network-policy absent and named pre-data/pre-compute refusal. Preserve internet-mode isolation and rootful behavior; revisit after K27. Podman already uses native published ingress for its internal bridges, so keep it active and add a real positive test with a reachable internet canary, no default route, blocked canary egress and successful private HTTP ingress. Local Docker proves the named refusal; actual Podman positive proof is still pending. The new case adds an estimated under one minute to each existing engine/core job and no job or gate.

- Copy-limit source d186479 passes hosted Rootless engines 43017830459: Podman 130513238558 passes all six real cases in 216.32 s, including the allocation beyond the unenforced reservation; Docker 130513239010 passes named mapping refusals. Both namespace-source engine workflows 43015556681 and 43015564369 are green. Current private-ingress and scoped-placement additions still need hosted proof.
- The private-ingress candidate passes local full Rust (454 pass, 23 ignored), all seven mapped-engine cases in 14.99 s, strict Clippy and all source structure checks before the subsequent tier-announcement change. A mixed-node review finds that withdrawing Docker's global network-policy would also hide a supporting process backend or borrow its capability for Docker. Two new placement/real-Postgres regressions fail before correction: cross-tier feature leakage and an unnamed network-none request choosing container and failing source validation. Add individually announced tier qualification, supporting-tier default selection, and matching write/wake guards, preserving legacy behavior. Targeted placement/registry tests pass after correction; a wire regression also requires scoped policy serialization when only the tier advertises it and refusal before an unchanged-spec wake after loss. Both new user-preset cells exercise process network none. Schema metadata is additive and generated clients are updated; the first full workspace check correctly rejects stale generated fingerprints before regeneration. Full final checks are pending.
- Final scoped-policy source passes complete local Rust: 454 pass, 23 ignored; all seven real mapped-Docker cases pass in 18.63 s, including the named private-ingress refusal. Strict all-target Clippy and all three structure checks pass. Complete workspace validation passes 414 tests with one existing compiled-assets skip and 8,287 assertions, after regenerating both schema fingerprints; three new placement/registry/wire regressions pass. Typecheck and focused guard tests are repeated after switching the new HTTP assertions to the owner status constants; docs, complete actionlint/ShellCheck, fmt and whitespace pass. New hosted Podman private ingress/egress, process network-none presets, outage/recovery and final rootful budgets remain pending. No existing rootful Docker test body is changed. Raw output belongs to the cited CI artifacts; the repository keeps only its summary.

- Read both scoped-policy Podman failures, jobs 130527497633 and 130527532012: six real cases pass, while the new private-network case rejects ownership before HTTP. Native Podman already creates its own fully owned per-workload bridge; the shared Docker internal-bridge call supplied workload labels against a deployment-owned placeholder. Remove that redundant call only for native Podman, retaining every ownership/isolation check, and add bounded boolean refusal diagnostics. Actual positive Podman private ingress still requires fresh hosted proof.
- Read complete user-preset failures 130510187084 and 130516259277: installation and initial process hello pass, then the systemd-managed node probes /var/run/docker.sock because the selected DOCKER_HOST was not retained. Save the measured absolute endpoint in the private installation record and generated configuration, restore it through clean service environments, and keep it across repeats/upgrades. A real Unix listener plus a fresh cleared-environment Bun child fails before the fix and passes after it. Acceptance now asserts both the retained record and actual daemon TOML; legacy records remain valid.
- Endpoint/private-bridge correction passes complete local Rust: 454 pass, 23 ignored; all seven real mapped-rootless Docker cases pass in 15.03 s. Strict all-target Clippy and all three structure checks pass. Full workspace checks pass 415 tests, one existing compiled-assets skip, 8,293 assertions, including type/schema/docs/release/lint checks. Complete actionlint with ShellCheck, changed-shell checks and whitespace pass. The new endpoint regression adds under one second to the existing workspace job; no new job or gate. Podman private ingress and installed-service outage/recovery are still unproved.
- Normal CI 43015558733 at namespace source 03729e9 is fully green. Read the separate b84df08 PR failure 130501629703 and its aggregate 130529739294. The authenticated celld-native-measurements artifact 13854584730 matches archive SHA-256 88cf804da36ed70541c054a4247b512f8c03ade37ada7f9faa37b69955d05755: sixteen pryx samples succeed, then HTTP 500 reports the container is not listening on port 8080. The unchanged 20.1-second upstream port verification expires while the container is still running; the cause is under investigation, not declared resolved by the later green run. No failed CI job is replayed, and no assertion or deadline is reduced.

- Retained-endpoint/private-bridge source 2d7e2a7 passes the complete Rootless engines run 43027222801. Podman job 130544875376 passes all seven actual cases in 211.39 s, including private HTTP ingress, no default route, external canary denial, memory overrun, timeout containment and all thirty measurements. Docker 130544875801 passes its named mapping refusal. Both raw owner artifacts are downloaded and archive SHA-256 checked, then summarized with separate provenance; raw bytes remain in CI artifacts. Latest direct-node Podman p50/p90 in ms: create 4217.50/4361.13, exec 47.99/127.72, HTTP 3.20/3.31. Keep the preceding cohort rather than overwriting its evidence. Actual installed-service endpoint/outage acceptance and final normal/Measure proof remain pending.
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
- The supplementary K22 Measure 43063939164 now finishes fully green: one-box 130673811694 and control 130673812057, including complete three-tier comparisons, default-image/SDK/CLI paths and every historical budget. Read both completed logs (no error markers); this is source 87400f3, not a fabricated final-source run. Its rootful path is unchanged by the subsequent Podman-only version reuse and fixture-only acquisition correction. Update the table's earlier pending note; both K22 exceptions remain parked exactly as recorded.
- First K23 implementation checkpoint adds a design test, not native-tool availability. Two separate agent children run as the same real UID with the existing complete process policy. A live owner socket connects; a neighbour's ordinary file read must fail; the report records whether its pathname-socket connect is blocked or reachable. This distinguishes filesystem confinement from an assumed socket boundary. Local Landlock is unavailable, and both children correctly refuse before any positive claim. Full serial cargo test passes 459 tests, 23 ignored; strict all-target Clippy, all three structure checks, fmt and full actionlint pass. The first Rust structure command misses Cargo on PATH; retain that setup failure, correct PATH and obtain its passing result. No Rust/TypeScript runtime or protocol behavior changes, no old assertion/deadline changes, and the two existing non-Rust graph checks pass independently. The one new test plus diagnostic artifact adds an estimated under one minute to the existing core job, with no new job/test system/required gate. Hosted pathname privacy is not yet claimed.
- Complete the candidate build-tool review before execution: all nineteen micromamba 2.10.0 archive notices are read in bounded groups, including the complete Kerberos per-file terms, OpenSSL/SSLeay, Apache-2.0, CC0 and libev's BSD option. The programme remains an external build tool, never linked into pryx or shipped in the tool bundle. docs/checks/measurements/k23/micromamba-builder.json preserves the two matching owner payload digests, verified archive, inconsistent checksum-file evidence and every notice path/hash. No package install hook or new tool executable has run yet. Next obtain hosted IPC evidence while resolving the runtime bundle's pinned packages and licences.
- First K23 hosted design result: core 130749125637 on push 43085610454 passes, and its process-ipc-design artifact 13886387316 has verified archive SHA-256 549a726553317e0637a1d1f0dfe1df67f2439f42d2a954f0789995430483621b. Under enforced filesystem Landlock, owner and neighbour UID 1001 both connect to the pathname socket while the neighbour cannot read its ordinary file. Preserve raw JSON/provenance under k23 and reject this first transport approach for shared-UID tools. This is a failed privacy design, not positive native acceptance. Proceed to a distinct scoped-abstract-socket probe using existing Landlock 0.4.7; leave tools refused until actual enforcement and readiness are complete.
- Cancel only duplicate ongoing normal/host runs 43083565870, 43083574524 and 43083565706 on documentation-only checkpoint 154a1b2, after verifying its diff contains only the two Markdown state files and those runs have no failed jobs. The complete finishing-source 7963826 proofs remain green, and the new 31596de code checkpoint runs every original suite plus the IPC observation. No workflow, test, gate or assertion is disabled; completed documentation rootless/CodeQL results are retained. This frees runner capacity for the current code and does not claim a cancelled run passed.
- Add the next primitive probe with the existing Landlock owner: disposable independent abstract-socket domains, random address, actual owner traffic and a permission-denied neighbour result. No caller is confined and no native feature is enabled. A separate real process-probe child with RLIMIT_NPROC exhausted proves worker-creation failure returns JSON and withholds the capability. Targeted tests pass twice; the final full serial Rust run passes 460 tests, 23 existing ignored. Retain the earlier 459-test run as preceding the resource guard. Cargo package-scoped cleanup frees build space; the first docs-check command names a nonexistent script and is corrected to the repository's passing docs:check. Hosted abstract isolation remains pending; local Landlock is unavailable and proves only refusal. New regression cost is under one second in the existing core test suite, with no job or gate.
- The scoped-primitive checkpoint passes strict all-target Clippy, all three independent structure checks, fmt, docs:check, full actionlint/ShellCheck and whitespace. Source 31596de now has green Any host 43085610050, both Rootless engines runs 43085610421/43085619907 and CodeQL 43085619954; normal push/PR still have their end-to-end jobs running without a reported failure. Do not claim them finished or infer tool readiness from the new optional process-probe field.
- Push signed scoped-probe checkpoint 973b42c. Source 31596de's normal push 43085610454 finishes green; its PR remains in progress. Freeze 97 individually verified conda-forge archives and 141 packaged notice hashes for the staged process bundle, including an old glibc 2.28 private sysroot. Verify explicit SHA-256 fragment parsing against all original solver rows before download-only extraction; no package is linked and no install hook or new runtime has executed. Review external conda-pack/patchelf, runtime licence families and mixed glibc terms; preserve corrected PCRE2/SLJIT/font notices from their exact verified source archives. Official headless-shell zip is pinned and its complete notice matches K18 byte for byte. Retain source/publication obligations explicitly; neither bundle publication nor native process acceptance is claimed.
- Source 31596de's PR 43085620019 also finishes green. Hosted scoped-primitive artifact 13889677031 from 973b42c/core 130771942896 verifies SHA-256 dc965fd72a0f23a24f253dd23d4d99dc65467190216d6cd06b89e502673d7e41: real owner traffic succeeds and an independently scoped neighbour is refused. Preserve raw JSON and provenance; it predates actual supervisor installation and is not native-tool acceptance. Install the stronger scope without relaxing any old restriction; optional authenticated health reports only installed scope. Add two-supervisor positive/negative observations and a separate unscoped-caller observation; local unavailable policy still proves only refusal. The first integration-test module declaration needs its explicit nested path and is corrected without changing assertions.
- After signed input checkpoint 399e7e0, install only the verified locked archives/wheels/Node dependencies and retain every hashed notice. Python scientific/kernel imports and Node zeromq/canvas imports pass; headless shell reports the pinned version. Private glibc 2.28 starts both runtimes and Python imports pass. The pinned conda-pack package builds a 776-MiB exploratory archive, but its CLI/distribution reports 0.0.0 rather than the package identity 0.9.2; keep that discrepancy explicit. Safe extraction passes. Three distinct relocation approaches fail: direct loader gives Node the loader executable path; patchelf crashes rewriting itself; detached patchelf crashes rewriting its active private libc. Park **safe process-tool bundle relocation**, its verified lazy acquisition/publication and dependent native kernel/browser/SDK/thirty-sample acceptance. No fourth attempt before K27. Keep source-prefix success separate from confinement or relocation acceptance, and preserve exact evidence and untested next approach in bundle-relocation-attempts.json. Continue independent K23 policy/refusal and supported-container work. Both K22 Docker pieces remain untouched.
- Installed-supervisor scope checkpoint passes full serial Rust again: 460 pass, 23 existing ignored, including real Wasm and all unchanged network-none assertions. Targeted private-IPC/process-policy cases pass, along with strict all-target Clippy, all three independent structure checks, fmt, docs:check, full actionlint/ShellCheck and whitespace. Native tool/kernel refusals now name the requested capability, verified bundle and private transport, rather than suggesting that an API version enables them. The two-supervisor observation adds an estimated under one second to the existing core job; no new job, test system or required gate. Hosted enforcement and legacy-caller observations are pending on this source. Current 973b42c/399e7e0 runs have no failed jobs; their longer normal/host suites remain in progress. No TypeScript source changes require another workspace test at this checkpoint; complete the brief's full workspace check before K23 closes.
- Explicit user correction supersedes the bundle parking entry: **un-park K23's main deliverable now**. Complete a pinned python-build-standalone/official Node/headless-shell bundle, launch with its own libraries and host glibc, report the host version and a precise too-old refusal, and prove ordinary-user browser plus Python/Node kernels in CI before K23 closes. Never rewrite binaries or ship a private glibc for these tools. Historical failed experiments remain evidence only; their proposed post-K27 rewriting attempt is cancelled. Apply three-attempt parking only to side pieces of all milestones, never the main deliverable; update the execution plan accordingly. The two explicitly parked K22 Docker side pieces remain parked until their required post-K27 revisit.
- Independent K23 container-browser checkpoint: trusted bootstrap policy defaults strict, ignores forged guest policy paths and forces strict mode on process agents. The actual configured executable must announce the optional argument. Default container browsers retain their outer hardening and report inner browser-sandbox false; required creates refuse. Rust-owned optional request/feature metadata and persisted specs preserve legacy absence/digests; observed epoch-bound copy guarantees intersect host capability. Add actual bootstrap, metadata-deadline, model, real-PostgreSQL and wrapper/feature tests. Full local Rust passes 466 tests, 23 existing ignored; full workspace check passes 424 tests, one existing compiled-assets skip, 8,350 assertions. Strict Clippy, all three structure checks, fmt, actionlint with ShellCheck, docs and whitespace pass. Launcher ShellCheck initially reports an ambiguous empty CDPATH assignment; make the empty value explicit and obtain a clean pass.
- Add a real default-policy rootful Docker browser screenshot/CDP/SDK test to the existing end-to-end suite; all original thirty-sample strict-policy tests remain unchanged. Hosted execution is pending. Estimated additional normal CI time is under one minute (five-minute failure deadline), no new job or gate. Upstream input correction is committed with pinned Python/Node/Chrome archives, library-only inputs and small host-glibc launchers; initial unconfined version/import/self-child checks pass locally, but relocation, native confinement, scientific kernels and ordinary-user acceptance remain active work.
- Read complete failed source-973 push job 130771942859 and aggregate 130793517286: all end-to-end suites pass; native evidence upload exhausts five attempts on HTML artifact-API responses. The aggregate faithfully fails. Add one failure-only upload recovery with an independently named artifact and mandatory delivery; normal successful-path cost is zero, worst-case outage adds the action's bounded retry window. Do not rerun the failed job or describe its artifact delivery as passed. The source-235 queued jobs still have no reported result.
- Complete the corrected upstream bundle assembler locally from verified archives, without Docker, private glibc, package-link hooks or binary rewriting. Extract each library/font package afresh, verify every notice, retain helper recipes and complete runtime licences, select the actual upstream Node addons, and audit all 499 shipped ELF objects. The measured complete-dependency floor is glibc 2.34 (debugpy); Node itself requires 2.28 and CPython 2.17. The host probe reports the real C library plus a separate process-tool-runtime prerequisite, with explicit architecture/too-old/non-glibc reasons; this does not advertise ready tools.
- Final fresh assembly passes after physical relocation to a longer path with spaces as UID 1000: both real kernels retain a value and return a PNG over abstract Jupyter IPC; Chromium screenshots a real HTTP page, CDP works through the abstract Unix proxy, access expires, and actual renderer status proves a nested PID namespace, no-new-privileges and an additional seccomp filter. Preserve exact local observations, source hashes and inventory digest in upstream-local-acceptance.json; it explicitly lists remaining installer, policy, edge/SDK, neighbour, lazy-fetch, timing and CI proof. Earlier diagnostics catch missing inline plotting configuration, jp-kernel's heartbeat bind bypassing the first IPC adapter, asynchronous Puppeteer defaultArgs, an initial-WebSocket-message race and unavailable optional proc children files. Correct their causes; preserve all positive/negative assertions. No main-deliverable failure is parked.
- Add the brief-authorized process-tools job to the existing Any host workflow: verified assembly, physical relocation, both real kernel/chart sessions and mandatory-sandbox browser/CDP/expiry/renderer checks as the ordinary runner user, with no Docker invocation. Estimated new branch/scheduled CI cost is four to eight minutes, twenty-five-minute failure bound, no new required gate or test system. Production publication, signed installer metadata and native process integration remain unfinished; retain observation artifacts only at this checkpoint.
- Complete local source checks pass: 467 Rust tests and 23 existing ignored, 424 workspace tests and one existing compiled-assets skip with 8,350 assertions, strict all-target Clippy, all three structure graphs, fmt, full actionlint/ShellCheck, docs and whitespace. Preserve the earlier default-symbol ENOSPC build failure; Cargo profile cleanup and symbol-free local rebuild resolve storage without changing tests/debug assertions. Final bundle assembly, both kernel checks and the browser's real renderer assertions pass on the final assembler/adapter source.
- Source 399e7e0 normal push 43092785172 completes green. Its PR and the source-973 PR/host workflows fail only evidence delivery during the observed artifact outage; retain those failed runs and use the cause-specific bounded recovery in subsequent pushes. Source f804977 passes both Rootless engines runs 43104895679/43104903127 and CodeQL 43104903069; normal suites and remaining host cells are pending. Read its supported Podman host failure 130815085323 completely: the outage fixture mistakes a cached pre-outage positive guarantee for recovery before a later live metadata timeout. Add a negative withdrawal observation before resuming the exact peer; keep the original single recovery mutation, forty-five-second deadlines and assertions. Hosted proof of this stronger fixture remains pending. The two parked Docker exceptions receive no new attempt.
- First corrected-upstream CI job 130820732451 on 9ba7026/run 43109289475 passes verified fresh assembly, the host-glibc audit, physical relocation, official Node self-child, and both real stateful abstract-IPC kernel/chart sessions as UID 1001 without Docker. It then fails Chromium's initial debugging-pipe attachment before any browser assertion. Download artifact 13895038972, verify archive SHA-256 fd6b36a35444ad0e58bbe6600bf5f9003d8c6b958c9388c97372604bfb630319 and retain exact kernel JSON/provenance; its complete inventory reports 34,489 entries and glibc 2.34. This is positive hosted bundle/kernel evidence and a browser failure, never full K23 acceptance. Add bounded startup-only Chromium stderr, which the launcher previously retained internally, to reveal the real refusal; complete local pipe/screenshot/CDP-expiry/renderer assertions still pass with this diagnostic.
- Before adding native channel selection, move kernel startup mechanically into runtime/start.rs on signed 9cdbe17. Full serial Rust passes 467 tests with 23 existing ignored; strict all-target Clippy, Rust structure, fmt and whitespace pass. No behavior, protocol field, assertion or deadline changes in this Move commit. Native cache/policy work remains an uncompiled local draft until the next integration checkpoint.
- Read diagnostic job 130824528637 on 1582e7d/run 43110388187: assembly/relocation and both real IPC kernels pass again; Chromium explicitly reports no usable sandbox under Ubuntu's AppArmor unprivileged-userns restriction. Configure that prerequisite only on the disposable positive-test runner before running the same ordinary-user browser test. Product setup remains unchanged, all mandatory browser/namespace/seccomp/expiry assertions remain, and default-profile negative cases stay required. No browser success is claimed until the corrected hosted job passes; this main deliverable is never parked.

- K23 upstream positive: source 54b9e62, run 43111625306, job 130828840530,
  artifact 13896672084 with ZIP SHA-256
  7b778122d3bc0498f135381b0ccbe3b7c548f1922ee832e818c16d148eb129bf.
  Retained measurement summaries and provenance in checks/measurements/k23 establish
  the relocated upstream bundle only. Both real kernels retain state and return
  charts, and Chromium uses a debugging pipe with a real sandboxed renderer,
  authenticated Unix proxy and expiring CDP access. Installed Pryx, edge, SDK,
  neighbour and thirty-sample acceptance remain required. The static host-bundle
  job fails on its missing compatibility.json build input; this checkpoint fixes
  that selective Docker COPY without changing the probe or fetching tools.
- K23 integration foundation: closed pre-filesystem-policy acquisition worker,
  archive/inventory digest and bounds checks, complete immutable cache publication,
  private kernel scratch and abstract transports, and nonce-bound observations
  signed with the existing sandbox key. This enables no node native policy until
  its protected descriptor, legacy-peer verification and actual confinement are
  wired. The full local Rust suite passes 471 tests with 23 existing ignored cases;
  full Bun passes 425 tests, one existing compiled-assets skip and 8,355 assertions.
  The template missing-guarantee regression keeps the original hosted browser
  refusal assertion intact. Current-source hosted acceptance remains pending.

- Recovery checkpoint: the preceding machine's uncommitted process-launch/browser
  integration is lost. Continue from bd34a7e and the committed cache/kernel/signed
  observation foundation; no lost local check establishes current-source proof.
  Source bd34a7e has green normal CI 43116738327 and 43116730410, Any host
  43116731031, both Rootless runs 43116738176/43116730821 and CodeQL 43116738090.
  K23 remains in progress; K22's two Docker exceptions remain untouched.
- First rebuilt step adds unchanged-payload archive packaging, a pinned acquisition
  descriptor and real archive/inventory regression checks. Both Python tests,
  Python compilation, actionlint, documentation and whitespace checks pass locally.
  The existing Any-host tool job now packages before smoke execution and retains
  the descriptor; new packaging adds an estimated two to four branch/scheduled CI
  minutes, with no new job or ordinary PR gate. Hosted packaging proof is pending.
  Source retention/publication, protected installation metadata, process adoption,
  browser confinement, installed edge/SDK acceptance and thirty samples remain.
- Persist each building step with a signed commit and push, at least every thirty
  minutes, as required by the resumed task. Never accumulate a whole integration
  milestone only in the execution workspace.
- Rebuilt process launch now reads a protected bounded descriptor beside the actual
  agent, gates optional fields on its parser version, isolates cache/download/scratch
  paths from persistent data, serializes launches, and verifies fresh signed peer
  observations against journal keys and live PID start ticks. A monotonic scope
  guard prevents later unscoped launches beside eligible native tools; boot-bound
  history prevents unverifiable legacy orphans from being mistaken for absence.
  Ordinary compute remains available when only these native prerequisites fail.
  Full local Rust passes 473 tests with 23 existing ignored cases, including the
  real Wasm lifecycle; strict Clippy, all three structure checks, fmt and whitespace
  pass. Hosted current-source launch/kernel acceptance remains pending; no ready
  process browser or completed ordinary-user installation is claimed.
- Packager source fe0437a has green Any-host job 131044569877 in run 43177305638.
  Downloaded artifact 13924229695 matches ZIP SHA-256
  c136f8532c68ff6abb3a64ddaef928f7b1952b770d476dfc9ac205fd3d6efa69.
  Retain exact descriptor/kernel/browser observations and provenance under
  checks/measurements/k23/packaged-*. The pinned archive has 38,894 entries and
  1,890,465,977 expanded bytes; UID 1001 retains state and PNGs in both kernels,
  and browser/CDP/expiry/renderer sandbox observations pass after relocation.
  This proves packaging and upstream tools, not installed-product integration.
- Rebuilt browser integration now launches the fixed broker through the reviewed
  bubblewrap helper in private user/PID/mount/proc namespaces, applies its full
  browser filesystem/syscall variant there, clears guest loaders/options and
  preserves mandatory Chromium sandboxing/debugging pipe. Its logical browser
  forward uses the scoped Unix session; ordinary command and owned-TCP policies
  remain unchanged. The private route/header has one JSON owner, included in the
  static Docker build and copied unchanged into the bundle.
- Add two explicit artifact-dependent real-agent CI cases: both kernels retain
  state and produce PNGs under full policy, owner socket connections work while
  an independent same-UID neighbour is denied, browser screenshot and forwarded
  CDP work, and a namespace-policy refusal occurs without acquiring the payload.
  These new cases are invoked with --ignored in the existing Any-host tool job;
  they have not yet run on the rebuilt source. The local host probe reports
  filesystem/network policy unavailable, so no local native-positive claim is made.
  Existing ordinary tests stay intact. Full local Rust passes 473 tests, with
  23 existing ignored plus these two explicit CI cases; strict Clippy, all three
  structure checks, fmt, actionlint, Node syntax and documentation checks pass.
  The job's estimated additional cost is five to ten compile minutes plus one to
  three runtime minutes, with a forty-five-minute failure bound and no new gate.
  Installed signed metadata, corresponding-source publication, actual ordinary-user
  edge/SDK acceptance, default-Docker native cases and thirty samples remain active.

- Native-policy source 8fd54f1 fails Any-host job 131062896379 in run
  43182878919 before health: the standalone test executable has not installed
  reqwest's TLS provider. Raw assembly, packaging, both kernels and the mandatory
  browser checks pass in that same job. Initialize Ring in the fixture, matching
  product bootstrap; keep every native acceptance assertion and deadline. Local
  agent tests, strict all-target Clippy, structure checks and formatting pass;
  full serial Rust passes 473 tests with 25 ignored (the two new native cases
  run explicitly in CI). Corrected hosted native acceptance remains pending.

- Corresponding-source checkpoint assembles 25 verified upstream source archives
  plus complete selected library/helper recipes, patches and notices into a
  separate asset. Local archive SHA-256
  3db0346f1a312a8dc5f3d57cb435262446c86cd86d709b58c9a0e11a3ea00906,
  402,508,316 bytes, 898 entries; runtime and source inventories both verify.
  Source coverage includes every selected GPL/LGPL/MPL/Sleepycat package. Runtime
  notices pin the source descriptor and include Pryx's own launcher/adapter licence.
  Source delivery uses a separate uncompressed CI artifact so small tool evidence
  remains downloadable. Estimated one to three additional branch/scheduled minutes,
  no new job or ordinary PR gate. Production publication wiring remains active.

- Installation metadata checkpoint wires generated process-tools.json into the
  release image and both host/bootstrap archives; signed full-host contents bind
  its acquisition digest. The release workflow assembles and publishes runtime
  and corresponding-source assets through its existing immutable-release path.
  Any-host host packaging uses the producing tool job's exact descriptor and
  relocation checks its retention, size and private write permissions. A source-only
  image without generated metadata preserves the named missing-artifact refusal.
  This adds an estimated five to twelve release-build minutes and serializes the
  existing Any-host bundle after native acceptance; no ordinary PR job or gate.
  No release/tag/key/settings change was performed. Installed edge/SDK acceptance
  and hosted proof of this metadata step remain pending.

- Host CA checkpoint supports conventional host trust bundles in the closed
  acquisition worker, bounded to four MiB with regular-file validation. HTTPS,
  redirect and certificate validation remain mandatory; guest-selected trust
  environment is ignored. The disposable HTTPS fixture serves with its CA and
  is refused without it locally. Full serial Rust passes 473 tests, 25 ignored,
  including real Wasm; strict Clippy and all three structure checks pass.
  Hosted full-policy acceptance now uses this verified HTTPS fixture after testing
  untrusted-certificate refusal, with its CA installed only on the disposable runner.
  Actionlint, Python compilation, docs and whitespace checks pass.
- Corrected TLS-fixture source 44ba713 reaches actual native acquisition, then
  Any-host job 131070176323/run 43185094598 fails with kernel_died during the
  first kernel startup. Its no-fetch namespace-refusal case passes. Capture the
  existing supervised output next; keep the original assertions/deadlines.
  Remaining work in this failed run and earlier failed 43182878919 was canceled
  to free runners; interrupted jobs establish no pass.
- Read older fe0437a Podman failure 131050333291/run 43177305638: its exact-peer
  outage check never observes withdrawal within forty-five seconds. Preserve the
  failure, all assertions and current supported-engine coverage. Later 64f222f
  Any-host 43179552328 is fully green; repeated current-source proof remains
  required. No parked rootless Docker piece was retried. Raw output belongs to the cited CI artifacts; the repository keeps only its summary.

- Native startup diagnostic checkpoint preserves the failing kernel assertion and
  emits bounded supervised process exit/output evidence only after a failed open.
  Complete agent tests, strict agent all-target Clippy, Rust structure, formatting
  and whitespace pass. The latest complete unchanged-product Rust suite remains
  473 passing/25 ignored at 907cfca. Corrected full-policy kernel/browser proof is
  still pending; neither the raw bundle successes nor this diagnostic completes K23. Raw output belongs to the cited CI artifacts; the repository keeps only its summary.

- Installed acceptance checkpoint adds a signed-preview user install with no
  engine socket, existing Python browser/interpreter SDK checks in both context
  modes, real TypeScript state/chart/browser/CDP calls, guarantee assertions and
  thirty raw create-to-first-image samples for each native tool. The same signed
  bundle also runs inside the actual image under the default Docker profile:
  both kernels must work and the process browser must name missing user namespaces.
  CI setup alone prepares accounts, fixture trust and the positive host namespace
  prerequisite; product installation remains one command without sudo or Docker.
  These tests are wired but unrun because full-policy kernel startup still fails.
  Estimated extra ten to twenty branch/scheduled bundle-job minutes, no new job,
  ordinary PR cost or required gate.
- Local full Bun check passes with real digest-pinned PostgreSQL 17.11: 425 tests,
  one existing compiled-assets skip, 8,355 assertions; typecheck, lint, docs,
  structure checks, actionlint, shell syntax and whitespace pass. This establishes
  source validity, never installed native-tool acceptance on this restricted host.
- Hosted source assembly fdbfc72 job 131070682839 and 7823f2f job 131071983927
  both publish source SHA-256
  458be764b28ee804917fd12ab28043ec905da1fd70b720886d449cf1be1b9db6,
  inventory SHA-256
  37c76dbc43cb37414b07ca8b84773cc045c0afb0ea2c2db280d3f3fb248afca6,
  402,508,427 bytes and 898 entries. Contents match the local inventoried sources;
  archive file modes follow each build host. Both then reproduce kernel_died.
  Source 907cfca job 131076501577 proves verified HTTPS acquisition and reproduces
  that same native startup failure; its namespace refusal passes. Read all three
  failed job logs. Main K23 work remains active.

- Read diagnostic source 0beebee job 131077812690/run 43187370614:
  the Python kernel aborts with Operation not permitted at libzmq thread.cpp:315.
  Read upstream 4.3.5 source: pthread_setschedparam reasserts inherited scheduling,
  conflicting with the guest's blanket scheduler denial. Enforce SCHED_OTHER as
  a no-op rather than allowing a scheduling change; all other scheduler operations,
  priority changes and supervisor signal protections retain their denials.
  New actual foreign-process test observes SCHED_BATCH unchanged after the request.
  Both genuine pinned Python and Node ZeroMQ contexts now start threads and bind
  abstract sockets under the guest syscall filter locally. This isolates the fix;
  full filesystem/browser acceptance still belongs to hosted CI.
- Scheduler checkpoint full serial Rust passes 474 tests with 25 ignored, including
  real Wasm; strict all-target Clippy, Rust structure, formatting and whitespace
  pass. The unchanged TypeScript sources retain the complete 425-pass check at
  a2ee1e9. Its queued Any-host 43189216316 was canceled before execution because
  it retained this already-diagnosed kernel failure; that run establishes no pass.
- Next integration work includes process browser placement/copy guarantees and
  strict requirement validation: current reporting still qualifies browser-sandbox
  only on containers. Keep the installed positive assertion intact. Then obtain
  genuine signed ordinary-user edge/SDK samples and default-Docker native proof;
  K23 remains in progress, with no parked main deliverable.

- Process browser guarantee checkpoint reuses native launch readiness without
  creating cache/scratch or changing boot/scope history during placement probes.
  Actual executable metadata, protected descriptor, host prerequisites and fresh
  signed managed peers qualify the host. The private launch journal qualifies its
  copy; unknown/legacy launch state overwrites any carried browser label to false.
  Control requires both observations. Strict process browser requests validate
  before creation and on each launch/resume; Wasm retains its refusal.
  Installed acceptance now also requires browser-sandbox on the positive create
  and asserts required placement refusal in the default Docker profile.
- Local full serial Rust passes 476 tests, 25 ignored, including real Wasm; strict
  all-target Clippy, Rust structure and formatting pass. Full Bun with real
  PostgreSQL passes 426 tests, one existing compiled-assets skip and 8,359
  assertions. Current-source hosted native and installed acceptance remain pending.
- Cleared superseded incomplete checkpoint CI from the runner queue; none of their
  interrupted checks establish a pass. Preserve the older 44ba713 normal-CI
  43185102836 failure in job 131070287116: metadata's actual-executable test
  unexpectedly returns false for its positive fixture under parallel tests.
  Its complete log was read; it has no cause diagnostic. Keep its two-second
  bound and all assertions while instrumenting spawn/read/exit/parse/timeout.

- Scheduler source a1b3d34 job 131089697366/run 43190915817 now proves both
  Python and JavaScript retain state and return PNGs under the complete process
  policy as UID 1001. Download artifact 13931796583 and verify ZIP SHA-256
  bd14ab2f9c2c682915482fa4828cfff5152e54e31db5e9eff2be537cdcf053ba;
  retain exact observations/provenance in full-policy-kernels-*. The job then
  stops at the owner socket probe: its process POST omitted the mandatory
  idempotency key. Correct that fixture request, keep all socket/browser
  assertions, and capture bounded exited-process output on a failed socket check.
  Neither neighbour/browser nor signed installation acceptance has passed yet.
- Keep the metadata test's original assertions, timeout and output bound.
  Add stage-specific test-only spawn/read/exit/parse/timeout diagnostics while
  preserving product refusal behavior. Its configured parallel unit run passes;
  the first local parallel command lacked the agentOS asset environment and
  failed only that real-Wasm fixture, then was corrected without changing code.
  Full configured serial Rust passes 476 tests with 25 ignored, including real
  Wasm. The corrected keyed-fixture agent tests and strict Clippy pass; final
  Rust structure, formatting and whitespace checks pass. Unchanged TypeScript retains 426 passing
  tests, one existing skip and 8,359 assertions at 48bc51f.

- Native-policy source ae7b5a8 has green Any-host job 131101705318 in run
  43194460133: both real kernels retain state/PNGs, owner abstract connections
  succeed, a separate same-UID agent cannot connect, screenshot and forwarded CDP
  work, and the namespace refusal causes no download. Download artifact
  13933605902 and verify ZIP SHA-256
  b4071e267aba352629e9415553871481cb6037f88c369c98a65487e40e17966b;
  retain exact full-policy-*.json observations, descriptor and provenance.
  Signed ordinary-user bundle job 131107999803 is now running. This is the
  complete agent-policy proof, not installed-product or thirty-sample completion.
- Reproduce ordinary Bun rmSync failing EACCES on the sealed native cache.
  After stopping owned compute, local-stop now prepares directory permissions
  using the existing descriptor-relative walk/chmod while keeping files and link
  targets untouched. Its real regression removes a sealed tree, retains an
  outside target, refuses a linked root and handles a FIFO without opening it.
  No running guest cache grant is relaxed. The hosted fixture also preserves
  completed measurement summaries on a later teardown failure.
- Current purge checkpoint passes full serial Rust: 477 tests, 25 ignored,
  including real Wasm; strict all-target Clippy, Rust structure, formatting,
  actionlint, shell syntax, docs and whitespace pass. Unchanged TypeScript
  retains 426 passing tests/8,359 assertions and its existing compiled-assets skip.
  Actual installed native acceptance, source-final CI and K23 completion remain
  pending; the two parked Docker pieces remain untouched.

- Runtime-origin acceptance now checks actual Python/Node executable paths below
  the pinned cache and exact upstream versions, Chrome's CDP version, and the
  daemon's host-glibc report against host getconf. Retain architecture, libc and
  Landlock ABI with measurement summaries. Add TypeScript error/interrupt/restart checks,
  MCP run_python returning PNG, and the installed CLI against the existing box.
  These remain mandatory hosted assertions; no installed-product pass is claimed.
- Complete local Bun with real PostgreSQL passes 426 tests, one existing
  compiled-assets skip and 8,359 assertions; lint/typecheck, all TypeScript
  structure checks and whitespace pass. Unchanged Rust retains 477 passing
  tests and 25 ignored at 84efee5. Signed-preview CI remains in progress.

- Read ae7b5a8 installed-bundle failure 131107999803/run 43194460133 in full:
  image construction and preview signing succeed, but the fresh account exits
  before readiness and cannot read local-diagnostics.py through the runner's
  private checkout ancestry (EACCES). This establishes no installed native pass.
  Compile the real acceptance client, copy only public test inputs to a traversable
  /tmp fixture, and run the fresh account from that directory; share the same
  client/inputs with the default Docker image. Runner home/checkout permissions
  remain private. Retain safe HTTP release-server and SDK-install diagnostics.
- The standalone client compiles locally (505 modules, about two seconds); shell
  syntax, actionlint, docs and whitespace pass. Product source is unchanged from
  the 477-pass Rust and 426-pass Bun checkpoints. The exact initial installer
  refusal remains to be exposed by the now-readable diagnostics if it persists.

- Startup comparison checkpoint adds thirty real container create-to-first-PNG
  samples per language, with untimed template preparation matching the prepared
  native bundle. Process browser timing now includes its own page-server startup,
  matching the existing container browser cohort. Historical interpreter execution
  and restart budgets remain unchanged. Estimated one to three additional minutes
  in the existing normal-PR end-to-end job; no new job or required gate.
- Full local Bun check passes: 426 tests, one existing compiled-assets skip,
  8,359 assertions with real PostgreSQL. Both TypeScript structure checks,
  standalone-client compilation, actionlint and whitespace pass. Real container
  cohorts and corrected installed-product CI remain pending. Canceled superseded
  Any-host runs 43197116631 and 43198781057 after their native-policy jobs passed;
  their bundles still used the known private-checkout fixture and establish no
  installed-product pass. Keep corrected source 4eb6257 run 43199623015 active.

- Process tool documentation now describes implemented lazy acquisition, host
  glibc/library selection, signed peer isolation, browser namespace/guarantee
  policy and the current network-none refusal. It explicitly distinguishes green
  complete-policy proof from pending signed-install acceptance. Docs and whitespace
  checks pass. Source 4eb6257 native-policy job 131119291210 is now green;
  its corrected signed bundle job 131124391571 has started.
- Cleared six superseded normal CI runs after inspecting all current job results:
  43198789805, 43198780660, 43197128635, 43197116701, 43199632253 and
  43199622999. None had a new failed job; interrupted checks establish no pass.
  Keep d1a0a63's final-comparison CI and 4eb6257's corrected install proof active.

- Read corrected-fixture installed job 131124391571/run 43199623015:
  signing and image build pass, then preflight refuses the already-created
  /home/pryxtools/.cache as unsafe. This is now an explicit refusal, not the prior
  unreadable diagnostic. No native SDK or timing pass is claimed. Artifact
  13936222933 retains the refusal and SDK/server logs, ZIP SHA-256
  c9e3929d75964c2ad8e35f9909aef516ab6b28a5c1d78cb9ac74e04f0e24bbd9.
  Move Python SDK dependency installation after product readiness in both fixtures,
  disable pip caching and retain its log. All product path checks stay unchanged.
- Shell syntax, Python compilation, actionlint and whitespace pass. Product code
  retains local Rust 477/Bun 426 checks. Hosted d1a0a63 job 131124170594 is green,
  including the parallel metadata test, real Wasm and all 21 Docker integration
  cases; both rootless runs and CodeQL are green. Normal full end-to-end CI is
  still running. Cancel superseded Any-host 43200870211 and 43201263882, whose
  bundles retain the known SDK-before-install fixture failure; neither is a pass.
  Draft PR 21 now records the current scope and additional CI minutes.

- Read normal source 462216b job 131127004972/run 43201264196:
  the original metadata positive fails with the new diagnostic `spawn: Text file
  busy (os error 26)`. Locally reproduce ETXTBSY with an executable writer retained
  only by a forked peer; an immutable executable remains usable with a retained
  response-data writer. Replace only the generated executable fixture with a
  checked-in script and private response files. Preserve every response/exit
  assertion and the two-second bound; a blocking FIFO proves the stalled probe.
  Focused real-executable cases pass, including the observed deadline. Correct
  the initial local compile's rustix mkfifo/mkfifoat API mismatch; that failed
  compilation establishes no check pass. Full Rust validation remains pending.
- Cancel inspected superseded normal runs 43201264196 and 43201271250 after
  retaining that failure; interrupted end-to-end jobs establish no pass. Keep
  both d1a0a63 normal runs for the new container cohorts, and 919f7fe Any-host
  43204171378 for the corrected fresh-account install. Main K23 work stays active.
- Immutable-fixture checkpoint passes full serial Rust: 477 tests, 25 ignored,
  including real Wasm; configured parallel root-library validation also passes all
  254 cases. Strict all-target Clippy, Rust structure, formatting, shell syntax,
  actionlint, docs and whitespace pass. Unchanged TypeScript retains 426 passing
  tests and its existing skip. Normal d1a0a63 end-to-end step passes all 33 enabled
  cases (15 existing skips) with 931 assertions. Its new sixty-sample kernel
  startup case takes 115.1 seconds, within the estimated extra one to three
  ordinary-PR minutes. Artifact digests are verified; raw retention follows.
- Retain the exact d1a0a63 container reports under checks/measurements/k23 with
  source/job/ZIP and raw-file digests. Verify all thirty-sample quantiles against
  the raw arrays: startup p90 Python 1,903.7 ms, JavaScript 825.4 ms and browser
  1,787.4 ms. Existing trivial/chart p90 budgets remain 3.9/116.1 ms. Record the
  container half in speed.md without inventing process measurements or claiming
  the still-running job's later checks passed. Docs/whitespace validation follows.
- Normal d1a0a63 run 43200870546 is now fully green, including Wasm, two-node,
  real maintained integrations and store-cut/watchdog recovery. Source c0a4192's
  hosted PR Rust/Docker job 131142771911 is also green, including the immutable
  metadata fixture. The earlier recorded docs/whitespace checks passed.
- Read signed-install source 919f7fe job 131137974179/run 43204171378 in full:
  fresh ordinary-user readiness succeeds with no engine; genuine Python 3.12.16
  and Node 24.22.1 run through both SDK modes with state/charts/errors/interrupt/
  restart and actual pinned cache executables. MCP run_python and installed CLI
  pass. Browser screenshot, forwarded exact-version CDP and both Python browser
  modes succeed, then the public browser-sandbox guarantee asserts false.
  Verify artifact 13939307648 ZIP SHA-256
  a54c637d2d4fd2d357003b71cf888ffa08ee33f122b65145931ebe69f926c525;
  retain exact installed-kernels-919f7fe.md observations and qualified provenance.
  No ninety-sample, default-Docker or milestone completion is claimed.
- Diagnose the heartbeat's container-only condition dropping process copy maps.
  A real manager/heartbeat regression fails with None before the fix and passes
  afterward, retaining false/unknown/true, epoch and host-withdrawal assertions.
  Keep resource-limit maps container-only. Full Rust validation is in progress;
  installed guarantee assertion stays unchanged. Cancel superseded Any-host
  43206227281 because its bundle retains that known reporting defect.
- Clear inspected superseded runs 43204181426, 43204171456, 43206050578,
  43206038287, 43206038315, 43206050257 and 43206050117 from the queue;
  none had a new failed job when canceled, and interrupted checks prove no pass.
- Read older d1a0a63 PR end-to-end job 131124251340/run 43200882590:
  container tool/interpreter cohorts and all Wasm cases pass, but two-node
  sleeping/full-host recovery asserts running memory 1,984 MB and receives zero
  immediately after waitRunning. Other two-node cases and later integration/
  recovery steps pass; the aggregate correctly fails. Keep the resource assertion
  and investigate source observations/start accounting; no passing PR claim or
  unsupported timing explanation is substituted for this failure.
- Heartbeat projection checkpoint passes complete serial Rust: 478 tests,
  25 ignored, including real Wasm; strict all-target Clippy, Rust structure,
  formatting, actionlint, docs and whitespace pass. The new manager-to-heartbeat
  regression is red before and green after the change. TypeScript product source
  is unchanged from the 426-pass local check and green hosted workspaces. Next:
  diagnose the separate resource-ledger failure while installed CI proves the
  corrected guarantee and collects the outstanding native cohorts.
- Reproduce the two-node accounting failure against real Postgres: after an
  acknowledged first start, a delayed pre-start stopped heartbeat reduces held
  memory from 1,024 MB to zero. Provisioning bypassed the admission answer path;
  the registry could update existing observations but could not establish the
  first copy from a successful answer. Route that start through admission with
  the same wake timeout and atomically seed only the exact assigned node/epoch.
  Existing observations keep their epoch/timestamp guards. A newer genuine stop
  still releases memory. The real two-node assertion and RPC counts/deadlines
  stay unchanged; this is an accounting repair rather than a test timing change.
- The new flow is red before and green after. First-answer guards reject another
  node, another epoch and an unassigned workload; delayed older reports cannot
  override the answer, and an actual later stop remains authoritative. Focused
  flow/registry checks pass. Full Bun with real PostgreSQL passes 428 tests,
  one existing compiled-assets skip and 8,370 assertions; typecheck, lint, both
  structure graphs and whitespace pass. Rust retains 478 passing tests/25 ignored
  at 3b20de4. Hosted two-node validation and corrected installed native cohorts
  remain pending; no main deliverable is parked.
- Read source 15643ad PR Rust/Docker job 131160344053/run 43211582753:
  ordinary Rust passes, but the Docker OOM acknowledgement exits 127 instead of
  zero. Kernel logs show the allocator killed first, then the producer triggers
  a second OOM killing PID 1. Docker's journal confirms runc cannot open that
  exited init's /proc/PID/ns/ipc for the acknowledgement. Preserve this failure;
  the other twenty Docker cases pass, and the push's Docker job 131160297855 is
  green. No PR/overall completion is claimed.
- The fixture now gives both allocation children the existing preferred victim
  score and signals readiness after both waits finish, before its blocking FIFO
  read. Only then does the host issue its one acknowledgement exec. The original
  64-MiB hard limit, 400-MiB allocation, thirty-second deadline, zero-exit
  acknowledgement and final OOM/SIGKILL assertions stay unchanged. No process
  receives capabilities or a different engine policy. Local complete Rust passes
  478 tests with 25 ignored; strict all-target Clippy, Rust structure, formatting
  and whitespace pass. Real Docker validation of the coordination change belongs
  to CI and remains pending. Unchanged Bun retains 428 passing tests/8,370
  assertions and the existing skip at 15643ad.
- Cleared inspected superseded normal runs 43206227502, 43206239992,
  43210142839 and 43210152796; no new failed job was present when canceled.
  Keep source 3b20de4 Any-host 43210142686 for the first corrected installed
  guarantee/cohort proof and source 15643ad Any-host 43211573586 for accounting
  integration. Interrupted checks establish no pass; both parked Docker pieces
  remain untouched.
- Installed native acceptance on 3b20de4 completes in Any-host 43210142686,
  job 131159201658: fresh ordinary UID 1002 with no engine, both real pinned
  kernels through TypeScript and both Python SDK modes, state/PNG/error/interrupt/
  restart, MCP/installed CLI, browser screenshot/forwarded CDP/no-token refusal
  and positive copy guarantee. All ninety process samples and purge pass; the
  actual default-Docker image then passes both kernels/SDKs and namespace-based
  browser/strict-placement refusals, followed by purge. Artifact 13943183731's
  ZIP SHA-256 is c40d0458e6636d1717ba0d7ce8a635c3701b4cbbe08451c2e53b75d1162a2df9;
  exact samples, safe acceptance logs, hashes and qualifications are retained in
  docs/checks/measurements/k23/installed-provenance-3b20de4.json. Host glibc 2.39
  equals getconf. Paired startup p50/p90 and unequal-limit conditions are now in
  docs/speed.md. This completes the installed native evidence, not the whole job:
  later Alpine relocation fails because its descriptor parser invokes absent
  host Python, so subsequent distribution checks remain unproved on this source.
- Replace that smoke parser with Bun's documented BUN_BE_BUN standalone CLI mode
  on the already shipped executable. The real compiled CLI parses the retained
  descriptor locally; size/mode/regular-file, HTTPS, digest and bootstrap equality
  checks remain required. No host interpreter dependency or runtime-byte rewrite
  is introduced. Shell syntax, actionlint and whitespace pass; hosted minimal-host
  proof and complete final CI remain required before K23 is recorded done.
- Source fe030cf core job 131170162631 is green, including the immutable metadata
  fixture, process browser heartbeat regression and all twenty-one actual Docker
  tests. The revised OOM fixture passes with the original acknowledgement and
  limits. Superseded PR run 43211582753 was canceled after its OOM failure was
  read and corrected; its interrupted jobs establish no pass. Both parked K22
  rootless Docker pieces remain untouched until the post-K27 second try.
- Source 15643ad normal CI 43211573607 finishes with one failing new
  container startup cohort (127 assertions before no_room), not a kernel/tool
  failure. Completed job 131160297911's full log and retained e2e.xml were read;
  artifact 13944485683's ZIP SHA-256 is
  f04ea33d25f76f1500d3bae1ca3a644b25930e5c788ac3b1136b121cf34bdcc8.
  PostgreSQL reports the actual cause at 12:09:02: answered() reserves its new
  fleet_observations row then waits for the node foreign-key KEY SHARE lock,
  while heartbeat holds that node FOR UPDATE and waits for the same observation.
  All ten actual two-node tests pass (43 assertions), as do subsequent Rivet/
  celld and both store-cut/watchdog recovery cases; the run remains failed.
- Reproduce the exact first-insert/node-foreign-key deadlock against local real
  Postgres before changing production code. answered() now holds the node's
  KEY SHARE lock in the same transaction before its observation upsert, matching
  heartbeat's node-before-observation order. There is no lifecycle replay or
  deadlock retry. The concurrency regression first fails with deadlock.c/40P01,
  then passes with the fix; existing old-report/assigned-epoch guards also pass.
  Complete workspace lint/typecheck/OpenAPI/docs/release checks pass; all 429
  Bun tests pass with the one existing compiled-assets skip and 8,372 assertions.
  Both structure graphs and whitespace pass. Unchanged Rust retains 478 passing
  tests and 25 existing ignored cases; fresh hosted CI is still required. K23
  remains active, not parked.
- Source 95560de has complete green normal push CI 43219637483, including
  every Rust/Docker case, the 429-test workspace check, contract, one-box/Action,
  all 33 first-stage end-to-end cases, Wasm, ten two-node cases, Rivet/celld and
  store-cut/watchdog recovery. PR 43219647421 remains in its later integration
  phases at retention. Rootless workflows 43219637263/43219647485 and CodeQL
  43219647640 are green; both parked K22 pieces retain their existing refusals.
- On that same source, Any-host 43219637379's process-policy job 131187563160
  and complete bundle job 131190931529 are green. Signed ordinary UID 1002,
  engine none, both SDKs/kernels/MCP/CLI, exact upstream executables and host
  glibc, browser screenshot/CDP/guarantee, all ninety process samples and purge
  pass. The actual default Docker image then passes both kernels and mandatory
  namespace/browser placement refusals, followed by purge. Both base-userland
  and Alpine relocation retain and validate the native descriptor through the
  shipped Bun runtime; the former missing-Python smoke failure is fixed.
  Raw observations, safe logs and verified ZIP/file hashes are retained under
  installed-provenance-95560de.json, container-provenance-95560de.json and
  full-policy-provenance-95560de.json; docs/speed.md now pairs this source's
  thirty-sample cohorts with their different resource guarantees stated.
- Later host-matrix jobs 131211854724 (booted Debian), 131211854587 (booted
  Fedora) and 131211854645 (Ubuntu Docker) fail before product acceptance.
  Every complete failure log was read: the primary Ubuntu mirror serves a
  successful index update, then its package downloads time out. The boot hosts
  never install Incus or boot those distributions; the Ubuntu container never
  finishes its Python/curl prerequisites. Preserve those failures, without
  assigning them to native tools or claiming the full workflow green.
- Extend each affected fixture's existing signed-mirror fallback over both
  apt-get update and prerequisite installation, retaining Incus 6.0.0-1ubuntu0.3,
  IPv4/retry options, TLS CA paths and release/package verification. A temporary
  command harness exercises primary success, update failure, download failure
  and both mirrors failing for each actual shell block: fallback succeeds only
  where its commands succeed, and both-down keeps exit 100. Existing Ubuntu
  source-format tests still pass. Complete workspace check passes again with
  429 tests, one existing skip and 8,372 assertions; both structure graphs,
  actionlint, shell syntax and whitespace pass. Express the concurrency probe's
  unchanged one-millisecond poll through SECOND_MS as required. Hosted mirror
  recovery and a final complete green source remain required before K23 closes.
- Superseded runs 43218393906, 43218403357, 43214488216, 43214477883 and
  43218393796 were inspected and canceled before this head's validation. Their
  only final failed checks are the expected cancellation aggregates; those logs
  were read. Interrupted jobs establish no pass. PR 21's body now distinguishes
  installed proof from unfinished final CI and states the measured PR cohort
  cost. Existing security review notices refer to fixed test nonce vectors;
  the production challenge uses a fresh UUIDv4 in process/exec.rs.
- Source 2db00ba normal push 43228041376 fails one-box job 131216334843
  during its final host capability summary: the probe exits 137 after entering
  wasm-distribution verification. The complete log was read; the caller retains
  its thirty-second bound. Failure-only OOM evidence used absent rg, so no OOM
  conclusion is inferred from that missing diagnostic. Other current-source CI
  remains separate and in progress; no workflow rerun substitutes for a fix.
- The real local debug probe takes 25.329 seconds, with 25.109 seconds spent
  verifying the pinned Wasm distribution. Reuse the existing locked Ring 0.17.14
  SHA-256 backend, already shipped by the agent/TLS stack, for the same streaming
  file verifier. Cargo.lock adds only the existing direct dependency, with no
  version or archive change. Full content hashing, no-follow opens, ancestor/
  ownership/mode guards and the thirty-second caller bound remain unchanged.
  A meaningful cross-implementation/multiple-buffer regression rejects content
  changes, symlinks and writable files/ancestors. All 479 Rust tests pass with
  25 existing ignored cases. Afterward the same complete probe takes 0.874 s,
  including 0.768 s of distribution hashing, and every capability/tier state is
  unchanged. Exact fixed-phase logs, source/policy hashes, bytes and local-only
  qualifications are in wasm-probe-ring-local.json. Replace only the failed
  diagnostic's unavailable rg with installed grep; preserve its kernel patterns.
  Strict all-target Clippy, Rust structure, formatting, actionlint and whitespace
  pass; unchanged TypeScript retains 429 passing tests and its existing skip.
  Hosted one-box proof and complete final CI remain required before K23 closes.

- K23 final proof: implementation 5945724a4ac8fbc28610292d701ffa7e9bffc040
  passes complete normal push 43233699289 and PR 43233712859, all sixteen
  Any-host jobs in 43233699231, both Rootless-engine runs 43233699487 /
  43233712730, and CodeQL 43233712847. Both one-box jobs pass the unchanged
  thirty-second host probe after the complete Wasm verifier correction.
  The earlier 2db00ba PR 43228050073 and full Any-host 43228041477 are also
  green; its push 43228041376 retains only the known one-box timeout and
  consequent aggregate failure, whose complete logs were read. The recovered
  Debian/Fedora/Ubuntu prerequisite jobs use a healthy primary mirror, so those
  passes do not claim a hosted outage or forced fallback; the temporary
  command harness separately proves both fallback paths and failed exits.
- Final-source policy job 131235345512 passes both real kernels, state/PNGs,
  browser screenshot/forwarded CDP, same-UID neighbour refusal and the no-fetch
  namespace refusal in 46.24 seconds. Installed bundle job 131239358446 passes
  the signed fresh UID-1002 user install without engine access, both SDKs and
  Python modes, actual pinned runtime paths/versions, host-glibc comparison,
  MCP/installed CLI, browser guarantee and all ninety process samples; actual
  default-Docker kernels/refusals, purge and both relocation checks pass too.
  Its release-equivalent bundle build takes 1,037 seconds. The process-tools
  and signed bundle costs belong to the existing branch/scheduled workflow;
  the new sixty-kernel container cohort adds 99.9 seconds to normal PR E2E,
  within the recorded one-to-three-minute estimate, without a new gate.
- Retain the final-source complete-policy, signed-user/default-image and
  container reports, raw thirty-sample cohorts and producer archive digests
  under docs/checks/measurements/k23 with suffix 5945724. Every downloaded ZIP
  digest is checked before extraction. Source-matched startup figures replace
  the earlier comparison in docs/speed.md; different runner and resource
  guarantees remain explicit. Normal E2E job 131235487060 passes its first
  suite with 33 tests, 15 existing skips and no failures, then Wasm, all ten
  two-node cases, integrations and store-cut/watchdog recovery. The final
  source keeps 479 local Rust passes / 25 existing ignored and 429 workspace
  passes / one existing skip, with strict checks previously recorded. Main,
  release publication and unchanged credentialed cases remain unrun. K23 is
  done with no parked pieces; both K22 Docker exceptions stay parked until
  their required second try after K27.
- Start K24 immediately after that final green source. Re-read its brief,
  AGENTS.md, the progress record and relevant decisions. First prove the X11
  socket design using unmodified, pinned external programs under real process
  policy; no pathname privacy is inferred from a private directory. Existing
  K18 Xvfb/xdotool notices and archive locks are available for the fixture.
  Read-only upstream review has not adopted or executed the downloaded
  TigerVNC 1.16.2 binary; complete its dependency/source licence audit before
  any use. Native desktop/SSH availability remains off pending implementation
  and hosted proof; no K24 acceptance is claimed at this checkpoint.
- First K24 checkpoint adds an explicit X11 fixture, separate from the runtime
  archive, from eleven already reviewed K18 Debian package digests/notices and
  the K23 shared libraries. dpkg-deb extracts bytes without installing packages
  or running hooks; the fixture rejects private glibc and copies no implementation
  into pryx. Original executable SHA-256 hashes are checked by the real-agent
  design test. Positive -help/--version controls prove the programs load; private
  display refusals are compared with the same real programs outside confinement,
  so no vendor exit or error text is invented. The local explicit test passes,
  but this machine cannot enforce Landlock: x11-design-local.json records only
  unconfined argument behavior and the complete policy's refusal, not hosted
  isolation. No live X11 endpoint or headful Chromium acceptance is claimed.
  The existing Any-host tool job explicitly runs the new artifact-dependent test
  with filesystem/private-IPC enforcement required, and retains its summary.
  Estimated addition: under two minutes to that branch/scheduled job and under
  one minute of ordinary PR test compilation, with no new job or required gate.
  Desktop/SSH availability stays off. Full local Rust passes 479 tests with
  26 ignored cases (the new fixture-dependent case runs explicitly in CI).
  Strict all-target Clippy, all three structure checks, fmt, actionlint, Python
  syntax, docs and whitespace pass. Rebuilt fixture/explicit local acceptance
  passes; a nested private-glibc marker is refused before any acquisition or
  extraction. Source/report hashes and local-only qualifications are retained
  beside the report. Hosted design evidence remains pending.
- Hosted first X11 result: source 3b43da1, run 43245895630, job
  131278307230 is green. Both existing native kernel/browser policy tests pass
  in 44.04 seconds, then the new real X11 argument test passes in 0.08 seconds
  as UID 1001 with filesystem/private-IPC enforcement required. Both original
  programs load and produce their real private-path display refusal under the
  complete policy. Artifact 13956942717 has verified ZIP SHA-256
  f46154bfa36ed4670f5de3f236778016b668174a5fe940f718dc6d8a0b229ec4;
  retain its exact desktop-design.json and provenance under k24. This selects
  the user-namespace/private-temporary route, while a live display, own clients,
  scoped neighbours/proc-root aliases and headful Chromium remain unproved.
  Full source CI is still running; this job alone does not complete K24.
- Documentation-only source e4d7a23 CI runs 43243427316, 43243444866 and
  43243427238 had no failed jobs when inspected and were canceled after the
  new K24 source was pushed. Interrupted jobs are no passes. The two completed
  failed aggregates 131279136338 / 131279157507 were read in full and contain
  only the canceled E2E/one-box dependencies. Their rootless and CodeQL runs
  had already passed. K23 completion continues to cite tested implementation
  5945724, whose complete proof is retained separately.
- Stage the unchanged X11 component separately from the runtime: 26 digest-locked
  Debian archives, reviewed full notices/licences and the K23 own libraries;
  no package hooks, executable rewriting or private glibc. Retain only Xvfb,
  xdotool, xauth and xkbcomp executables from these inputs. A complete ELF audit
  first finds unused XKB GUI utilities need additional host libraries; removing
  those unrelated programs makes every shipped non-glibc dependency resolve
  inside the component. All selected programs pass real controls after moving
  the tree to a longer path containing spaces. The measured desktop floor is
  glibc 2.35; existing browser/kernel 2.34 remains unchanged. Four Python cases
  pass, including actual missing-own-library and fake-private-libc refusal;
  generated-launcher ShellCheck, actionlint and existing old/non-glibc Rust
  prerequisite test pass. Preserve raw local inventory and provenance under k24.
  This adds under two minutes to the existing branch/scheduled tool job and no
  job or required gate; ordinary PR fixture tests remain explicitly conditional.
  Corresponding-source export, live owner/neighbour/proc-root confinement and
  product acceptance remain required before publication or capability enablement.
- Read source-3b43da1 push E2E 131278406825 and aggregate 131305882152 in full:
  the field test sees current metrics but zero historical samples in its last
  minute. Its other suites pass; the normal PR 43245914144 is fully green.
  Preserve the failed push and investigate the metrics ingestion boundary;
  no retry or assertion relaxation substitutes for a cause-specific correction.
- The staged component's Any-host producer 131323890859 is green on 79360fb;
  its full installed matrix is still running. Both normal core jobs fail REUSE
  because a Python string containing a generated SPDX header is read as an
  invalid licence expression. Read both complete logs, correct the literal to
  preserve the same launcher bytes and obtain a clean local REUSE 6.2.0 result.
  A ShellCheck audit also finds the executable missing from local PATH: the
  earlier local ShellCheck pass statement was not execution proof. Acquire the
  official 0.11.0 archive at publisher SHA-256
  8c3be12b05d5c177a04c29e3c78ce89ac86f1595681cab149b65b97c4e227198,
  read its GPL-3.0 licence and run all four generated launchers plus complete
  actionlint with this actual executable; both pass.
- Add explicit live X11 acceptance: unchanged Xvfb starts in a private temporary
  mount through reviewed bubblewrap, keeps its inherited abstract-IPC scope,
  and uses the pinned xkbcomp at its compiled path. Its own confined xdotool
  must work. An independent same-UID, fully confined neighbour must fail ordinary
  file reads, abstract display connection and every real server proc-root alias;
  the unconfined controller requires a reachable alias as its positive control.
  Keep cookie bytes out of reports. Only the new trusted desktop policy grants
  its private temporary mount; ordinary guest filesystem/seccomp is unchanged.
  Local full Rust passes 479 tests, 28 ignored, including the two new explicitly
  hosted fixture entries. Strict Clippy, all three structure checks, fmt,
  actionlint/ShellCheck, REUSE, four assembled-component Python cases and full
  workspace check pass (429 tests, one existing compiled-assets skip, 8,372
  assertions against real PostgreSQL). The explicit live case has not run on
  this machine, whose filesystem policy is unavailable, and hosted success
  remains required. Estimated addition: under one minute to the existing tool
  job plus ordinary test compilation; no new job or required gate.
- Fix the observed field-metrics fixture race: retained history uses minute
  buckets, so the latest observation can exist before a bucket enters the last
  minute. Wait for the actual historical sample within the existing thirty-second
  read-only polling bound and assert the same fixed window afterward. Preserve
  the positive sample assertion, all product behavior and all existing timeouts.
  The full workspace check passes; real E2E proof remains pending.
- Read-only corresponding-source acquisition retains all 74 immutable inputs
  for the staged component's 22 distinct source/version pairs. Each Debian DSC's
  archive size/SHA-256 is verified; no source is executed. This local acquisition
  is not yet the committed source-export implementation or publication proof.
- Complete staged desktop corresponding-source export: pin all 74 DSC/original/
  Debian-change inputs for 22 source/version pairs and 26 binary packages.
  Validate each complete DSC reference set, every input size/digest, binary
  package control source identity and all 26 full copyright hashes. Preserve
  original/source archives, Debian patches, full notices/licences and pryx's
  licence without running hooks or source code. Add this directory to the
  existing separate source asset; runtime availability remains unchanged.
  The actual local export and deterministic packager pass: 37,336,907 archive
  bytes, 184 entries, SHA-256
  122ec130cb2bef6015b70652633706a2c4af42670d5743e7b344712145920f44.
  Raw inventory and source/provenance hashes are under k24. Its example.invalid
  descriptor identifies a local staging artifact, never a published release.
  Five real Python cases pass, including a DSC export missing its actual pinned
  original archive; rejection occurs before output creation. All four source-
  mapped program controls, actual ShellCheck/actionlint, Python syntax, REUSE,
  docs, whitespace and all three structure checks pass. No Rust/TypeScript
  source changes follow their preceding full checks. Combined hosted source
  packaging and the live X11 proof remain pending. Source collection adds an
  estimated under one minute to the branch/scheduled producer and release build;
  the existing source fixture test adds under one second. No job or gate.
- Superseded Any-host runs 43253634559 / 43258722135 and normal source-793
  runs 43258722385 / 43258740558 were inspected and canceled to release queued
  runners for 26c8b10. The only completed failures observed before cancellation
  were the two already-read REUSE core failures. The source-793 staged producer
  remains a passed job; its canceled installed continuation is not a pass.
  Source-26c normal push 43261043234, PR 43261068736 and Any-host 43261043378
  are still running. Preserve any later completed aggregate/failure evidence.
- The two canceled source-793 aggregates 131334515078 / 131334553656
  complete with expected failures. Read both full logs: they contain only the
  already diagnosed REUSE core failures plus canceled E2E/one-box dependencies.
  The canceled Any-host runs have no completed failing jobs. None is labeled
  green or used as final milestone acceptance.
- Read the complete failed live producer 131332095651 on 26c8b10:
  own-library audit, physical relocation and all four loader controls pass as
  UID 1001 on host glibc 2.39, then Xvfb exits before display readiness with
  failed XKB initialization. Preserve its exact diagnostic and pinned-source
  evidence under k24; this is no live desktop success. Inspect the actual
  digest-locked Xorg 21.1.7 source: Popen exits if setgid(getgid()) or
  setuid(getuid()) is denied, as ordinary process seccomp requires.
- Correct only the trusted private-desktop worker: allow setuid/setgid for its
  exact current real IDs, retaining all other credential and namespace denials.
  Prepare its private /tmp/.X11-unix and require the real pinned xkbcomp control
  before Xvfb. The new real subprocess test proves current-ID calls succeed in
  desktop mode, fail in ordinary mode, and foreign IDs/setresuid remain denied
  in both. No existing guest policy, assertion or deadline changes. Complete
  local Rust passes 480 tests, 28 ignored, including real Wasm; strict all-target
  Clippy, all three structure checks, fmt, full actionlint with actual ShellCheck,
  REUSE, docs and whitespace pass. No TypeScript source changes follow the
  preceding full check. Fresh hosted live proof remains required; desktop stays
  unannounced. Added ordinary CI cost is under one second, with no new gate.
- Source 1216943 producer 131336978161 completes the combined source asset,
  unchanged native kernel/browser policy tests, physical X11 relocation and all
  three staged Python cases, then repeats the still-unchanged XKB failure. Read
  its full log; the source-export phase is proved but the failed job is no full
  native acceptance. Source-07b producer 131342060401 gets past XKB: the actual
  Xvfb starts and its own confined xdotool/abstract-connect assertions pass.
  Its parent then fails the leader-only descendant count before neighbour tests.
  Read the full log. A real local worker-thread regression catches the optional
  task/children absence; use procfs parent-PID records and retain all original
  live endpoint/alias/neighbour assertions. The corrected local regression passes.
- Read e69fa1f PR E2E 131306082683 and aggregate 131337562668 in full:
  only the already diagnosed retained-minute metrics race fails; its push
  43253634643 is green. The existing targeted fixture correction is included
  from 26c8b10 onward; do not rerun or relabel that older PR failure.
- HTTP SSH foundation now adds an optional advertised agent lease flag and an
  authenticated binary WebSocket endpoint. It connects through the exact
  session-bound spare forward, preserving existing process ownership checks,
  target/session matching, revocation watch and expiry. Edge authentication and
  live lease lookup precede wake; the stream tracks that SSH route so removing
  it closes the connection even while asleep. Exposed application paths are
  unchanged. Add one migration for live authenticated edge feature metadata:
  HTTP-only edges choose WebSocket, live raw fronts remain default, and legacy
  peers retain raw behavior. All protocol/schema changes are additive.
- Both SDKs retain the key privately and provide the binary connection. The CLI
  keeps ssh-agent/ssh-add stdin and pinned host keys, using OpenSSH ProxyCommand
  only for a selected WebSocket lease. Its private helper passes only sandbox/
  lease identities on argv and uses maintained ws 8.22.1 streams. A real 3-MiB
  binary test first catches an oversized frame; split input at the existing SDK
  chunk bound rather than raising any message limit. The corrected test retains
  first bytes and every payload byte under backpressure. Real Rust TCP/WebSocket
  acceptance proves first bytes, opaque NUL/non-UTF8 payload and expiry; real SQL
  and edge tests prove support negotiation, raw default, authentication before
  wake and exact-session revocation. These are transport tests, not a claim that
  the unimplemented native process SSH server or installed K24 cohort passed.
- Complete local verification for this foundation: 482 Rust tests pass with
  28 explicitly ignored cases, including the corrected thread regression, real
  Wasm and the new stream expiry test. Full workspace check passes 432 tests,
  one existing compiled-assets skip and 8,386 assertions against real PostgreSQL.
  Python generation/check and 23 SDK tests pass, with one existing credential
  skip; initially missing generator is installed from the existing lock. Strict
  all-target Clippy, all three structure checks, fmt, actual ShellCheck/actionlint,
  REUSE, schema generation, docs and whitespace are checked below before push.
  The first workspace run retains its pre-fix large-frame failure; the final
  complete check is green. Estimated new ordinary PR execution cost: under five
  seconds, with no new job/test system/gate. Hosted transport and native product
  acceptance remain required. Native desktop/SSH announcements stay off.
- Next K24 work: complete hosted live X11 privacy; extend the staged component
  with separately executed scrot/x11vnc/OpenSSH, noVNC/websockify and complete
  own libraries/notices/sources; finish actual native desktop and SSH worker
  integration, placement/doctor, HTTP-only installed acceptance, both SDKs and
  thirty samples each. Read-only extraction of the four K18 program packages
  verifies their actual full copyright hashes (OpenSSH server uses its package
  doc symlink to the client notice) and inspects ELF dependencies without running
  them. A new upstream Chromium snapshot candidate is download/review only;
  no headful browser is adopted or executed before its complete licence audit.
- Hosted live X11 privacy is now green: be92e89926c4d31940230c6f79eeb2d263cbbeba,
  Any-host 43268999503, producer 131360151698. The two complete native kernel/
  browser policy tests pass in 59.33 s; the X11 argument test passes; all four
  relocated program controls and all three Python contracts pass on host glibc
  2.39. The live display case passes in 0.11 s as UID 1001: own xdotool returns
  real pointer coordinates and its abstract connection works; the independent
  same-UID neighbour cannot read owner data, connect the abstract display or
  any of five actual proc-root aliases. The unconfined controller also passes
  its positive live alias assertion. Retain exact measurement summaries/provenance under
  k24. Producer artifact 13968396107 ZIP has verified SHA-256
  7c8445aa4ab9360b790319c2e3c0e155ca635ba0ce3995f742adfeceaa05577a.
  This proves the staged Xvfb/xdotool design, not a headful browser or installed
  desktop/SSH. Combined source ZIP 13966708940 is retained by that producer at
  3e2569f97999ab56c7acd663bd21f9c9f64557ddf2d398ee650aa2d462753191;
  its descriptor is in the verified small evidence artifact. The signed bundle
  continuation and full normal CI are still running.
- Complete staged desktop component now builds from 142 digest-pinned Debian
  binary inputs with 159 selected unchanged ELF files, their own non-glibc
  libraries and complete notices. Its 112 source versions have all 370 required
  DSC/original/Debian source archives collected and verified locally. No package
  hooks, binary rewriting or private glibc. The measured full-component minimum
  is host glibc 2.36; K23's core minimum remains 2.34. Actual host glibc is 2.41.
  After a physical move to a longer ordinary path containing spaces, all seven
  loader controls pass: Xvfb, xdotool, scrot, x11vnc, sshd, ssh and Chromium.
  Retain the exact local inventory and provenance under k24. Seven Python cases,
  both structure checks, real generated-launcher ShellCheck, actionlint, docs,
  whitespace and REUSE (1,679 files) pass. These controls do not claim headful
  application interaction, native SSH service or installed-user acceptance.
- Extend the existing Any-host producer with this complete staged/source check;
  expected additional branch/scheduled execution is three to five minutes. No
  new job, required gate or ordinary PR stage. Upload only its JSON evidence;
  the staged runtime and complete source archive remain unpublished until native
  integration includes both assets. K24 remains in progress with no new parked
  piece. Next: real headful application/input, private VNC/noVNC and native SSH
  workers, complete own-source publication, capabilities and installed acceptance.
- Source 78177e5's complete staged producer passes (131382232340), including
  all full-source inputs and seven relocated loader controls on the hosted user.
  Its push/PR core jobs 131382299733 / 131382392789 fail only the attribution
  phrase grep: the unchanged digest-locked GLib copyright contains a literal
  phrase in its Creative Commons legal terms. Read both complete logs. Preserve
  the copyright bytes and exempt precisely that one upstream legal file from
  this prose-only grep; all source checks and licence checks remain intact.
- Add the closed native desktop worker in maintained private user/PID/IPC/UTS,
  temporary and proc mounts. Its existing browser policy permits Chromium's
  own nested sandbox, while ordinary guest filters remain unchanged. The fixed
  adapter starts unchanged Xvfb/headful Chromium, xdotool/scrot, x11vnc inetd
  per private Unix connection, and noVNC/websockify over Unix sockets. No public
  VNC/TCP listener or window manager. Keep capability announcements off pending
  the real hosted acceptance. Reuse the audited K18 web inputs, pin their exact
  wheel and complete source archives, retain full notices, and import only the
  Unix/token-file functionality rather than unrelated optional plugins.
- The explicit hosted case requires an application-observed click and complete
  typed text, a real PNG, noVNC page, actual RFB greeting, active expiry and
  expired reconnect refusal. Require the real renderer's extra PID namespace,
  seccomp filter and no-new-privileges. Probe a live private endpoint as the
  unconfined positive control, then deny the independent same-UID neighbour
  the abstract broker, both private socket proc-root aliases and owner cookie.
  This is a staged worker test, not installed SDK/edge milestone completion.
- Local worker verification: compilation, 482 root/workspace Rust tests with
  30 explicitly ignored cases, another 72 agent tests with eight ignored cases,
  five staged Python cases, all seven relocation controls including the added
  web payload, maintained websockify import, complete web-source acquisition,
  strict Clippy, all three structure checks, actual ShellCheck/actionlint, docs,
  whitespace and REUSE (1,694 files) pass. The initial Rust run omitted this
  machine's existing Wasm sidecar fixture path; the corrected complete run uses
  PRYX_AGENTOS_DISTRIBUTION and passes real Wasm. Local confinement positives
  are not claimed. Added hosted native application case is expected under one
  minute in the existing producer, with no new job or gate.
- Source be92e89's complete Any-host run 43268999503 is now green, including
  all sixteen host jobs and signed ordinary-user/native SDK continuation. A
  later docs-only source 683c4bf installed job 131370282724 passes both SDKs'
  kernel/browser positives but fails with public no_room during its timing
  cohort. Read the full log; no placement-refusal reason is emitted by its
  bounded existing diagnostics. Keep this failure open for diagnosis, do not
  relabel it green or rerun that unchanged source. Normal source-be CI remains
  running at this checkpoint. K24 remains in progress, with no piece parked.
- Source 2b8eb85 producer 131393371543 passes complete sources, both full
  library-boundary cases, all seven relocated controls and live X11 privacy,
  then the new isolated application fixture exits before Chromium observation:
  its reqwest client lacks the Rustls provider. The real agent already installs
  ring at startup. Install that same provider in this isolated fixture entry,
  retain the exact failure under k24, and claim no application pass from it.
- Connect native desktop open/screenshot/input to the closed worker and route
  the existing logical desktop port through its scoped Unix socket. Preserve
  coordinate/text validation, image/capture bounds, caller command deadlines,
  expiring public capabilities and every container path. Worker command paths
  and websockify Unix target markers have one private protocol definition.
  New optional host policy fields are omitted for legacy agent parsers; only
  an executable advertising desktop policy support receives them. Host glibc,
  namespaces, scoped IPC/history and installed descriptor produce named off
  reasons. No new ordinary-command filesystem or syscall grant.
- Assemble the complete desktop and web subtree into the existing lazy runtime
  build, with the 112-version/370-input source closure and complete web sources
  in the separate combined source asset. Do not raise the core host minimum:
  actual local inventory remains core 2.34, desktop 2.36, host observed 2.41.
  A real upstream ELF passes only within its desktop group and is rejected if
  moved into the core group. The complete local source asset builds at
  1,733,694,870 bytes; retain its descriptor and exact pre-final-adapter scope
  under k24. Hosted preview production of the final source remains required;
  nothing is published to a release. Avoid duplicate full-component assembly
  in the producer; test the actual already-relocated combined runtime instead.
- Final local service checks pass 483 workspace tests with 30 ignored, plus
  73 agent tests with eight ignored, eight real Python cases, strict all-target
  Clippy for both crates, all three structure checks, fmt, docs, actionlint with
  actual ShellCheck and REUSE. The first structure run catches a new parent/
  desktop-session cycle; keep orchestration in its owner module and the final
  graph passes. Legacy policy round-trip preserves the old serialized shape.
  No installed desktop/SDK/noVNC-edge or native SSH success is claimed yet.
  Expected added branch/scheduled producer cost is the already disclosed full
  component/source stage plus under a minute of real application acceptance;
  ordinary PR adds no job or required gate. K24 remains in progress.
- Source be92e89 now also has green normal push 43268999621 and PR 43269014308,
  in addition to all sixteen Any-host jobs, rootless regressions and CodeQL.
  The docs-only 683 timing-cohort no_room failure remains explicit and open.
  Next: require the corrected headful worker proof, finish installed desktop
  acceptance and thirty samples, native OpenSSH command/PTY/HTTP-edge wake,
  capability placement and doctor, then continue K25 without parking a main
  deliverable. The two K22 Docker exceptions remain parked until after K27.
- Source f61cc88 producer 131406873836 builds the complete combined runtime
  and corresponding-source asset. All seven relocated controls, three full
  library/floor boundary cases and earlier native/X11 policy checks pass. The
  provider correction gets the isolated client past its prior startup failure,
  then the real desktop reaches its unchanged 30-second readiness deadline;
  only an Xvfb private-directory ownership warning is captured. Retain this
  failure under k24. Add fixed, secret-free startup phases and Chromium startup
  output for the next real observation; do not raise the bound or claim success.
  Separately, actual websockify serves the noVNC page over a local Unix socket
  with HTTP 200; this is explicitly unconfined, not the native worker proof.
- Read all three completed f61 release-image/one-box failures 131406936696,
  131407064340 and 131407064640: the static Docker build omits the new desktop
  protocol JSON from its explicit COPY list. Include that file and the newly
  compiled-in desktop compatibility JSON. No image pass is claimed locally.
- Add a closed native OpenSSH worker using unchanged inetd mode on accepted
  private Unix descriptors, private maintained-helper PID/proc/tmp/account
  views and the ordinary process filter. Preserve the inherited supervisor IPC
  scope without broadening any filesystem or syscall rule; normal process
  entry still creates its original independent domain. Bind ephemeral auth/
  config into the private home view read-only, preserving StrictModes while
  host persistent data remains empty there. A fixed launcher enters the existing
  embedded shell and guest signal restrictions. Product SSH creation remains
  on its existing refusal until the next integration step wires this worker.
- The explicit hosted SSH case requires a real command, real allocated PTY,
  SSH key/host-key verification, denied neighbour data through SSH, and the
  independent same-UID neighbour's denied broker and key/data reads. The client
  key enters maintained ssh-agent only over stdin. Its byte proxy uses bundled
  Node maintained stream pipelines; a real local exchange verifies first bytes
  before stdin EOF, output completion and clean exit. The initial Python fixture
  transport has an EOF/thread shutdown problem and is replaced before adoption.
  This is transport-only local evidence, not native SSH confinement acceptance.
- Keep desktop and SSH assertions as independent steps in the existing producer
  after successful complete-component preparation, so either completed failure
  remains a job failure while the other main proof can run. No new job, required
  gate, skip or test system; expected SSH case is under one minute. Local checks
  pass 484 workspace tests with 32 ignored and 73 agent tests with ten ignored,
  real Wasm, strict Clippy for both crates, all three structure checks, fmt,
  actual ShellCheck/actionlint, docs, whitespace and REUSE. No local fully
  confined positive is claimed. K24 remains active, with no new parked piece.

- Resumed from 86ac7d6 after machine loss. Its complete normal push
  [43286192564](https://github.com/cubepals/pryx/actions/runs/43286192564)
  and PR [43286206558](https://github.com/cubepals/pryx/actions/runs/43286206558)
  are green. Read completed Any-host producer 131420110678 on
  [43286192692](https://github.com/cubepals/pryx/actions/runs/43286192692):
  actual OpenSSH command, allocated PTY, pinned host key, in-memory client key,
  denied neighbour files through SSH, scoped broker denial and empty persistent
  authorization mount all pass as UID 1001. The independent desktop case reaches
  Chromium, then fails Oilpan CagedHeap reservation under the fixture's 64-GiB
  ceiling. Retain both exact observations in worker-86ac7d6.json under k24.
  Use the preset's existing 8-TiB virtual-address default through one imported
  constant; product limits, memory watchdog, startup deadline and every assertion
  remain unchanged. Corrected hosted proof is still required. The lost lease
  integration is being rebuilt; K24 remains in progress with no new parked piece.
- Fixture checkpoint: both changed crates compile; fmt, the mixed structure
  ratchet and whitespace checks pass. The first restored-machine full Rust test
  build is still running, with a second invocation queued using the verified
  agentOS distribution. No local or hosted corrected desktop pass is claimed.
  Push this compiling checkpoint before proceeding with lease integration.
- Rebuild the lost native SSH lease integration: the existing create/view/shorten/
  revoke service now owns either its original container process or one private
  native OpenSSH worker. A reserved logical port resolves only to that worker's
  scoped Unix endpoint; the existing spare validates target/session and watches
  revocation. No native SSH server listens on TCP. Ephemeral host keys, config
  and authorized keys remain in scratch; its temporary data-side mount stays
  empty. Worker lifetime, cancellation cleanup and port reservation have a real
  local Unix-stream regression. Add an explicit hosted real-agent lease test
  for WebSocket SSH greeting, capacity, revoke/expiry stream closure, reconnect
  refusal and no persisted authorization. Actual installed SSH commands/wake,
  desktop/SSH SDK cohorts, announcements and timings remain required.
- The restored fixture check's first run lacks the configured Wasm distribution
  and fails its real sidecar launch. The next run uses the verified distribution
  and passes agent cases but hits the existing three-second guest HTTP readiness
  bound while Clippy compiles concurrently. Retain both failures; serialize the
  following full Rust run after compilation without changing any bound or test.
  Hosted corrected desktop proof on 7664b5b is still pending. Added ordinary PR
  unit execution is under one second; the existing branch/scheduled native
  producer gains an estimated under thirty seconds for its real-agent lease
  case. No new job, required gate, skip or test system.
- Corrected fixture 7664b5b now has green complete-component producer
  [131474013120](https://github.com/cubepals/pryx/actions/runs/43301624317/job/131474013120).
  Read its completed log: real headful application click and complete typed text,
  PNG, noVNC page, actual RFB greeting, active expiry and expired reconnect
  refusal pass in 4.38 seconds as UID 1001. Chromium retains its own nested PID
  namespace, extra seccomp and no-new-privileges. The unconfined controller
  connects thirteen actual private aliases; the independent scoped same-UID
  neighbour cannot connect either private socket through any tested proc-root
  alias, access the abstract broker or read the owner cookie. Retain the exact
  JSON in desktop-application-7664b5b.json under k24. Actual OpenSSH command/PTY
  and all earlier full policy/source/relocation checks also pass. The signed
  installation continuation is running; no installed K24 or full milestone
  completion is claimed from this producer proof.
- Lease checkpoint: current native agent library and test targets type-check;
  fmt, mixed structure and whitespace checks pass. The full Rust structure/
  Clippy process is finishing the node crate; the serialized full Rust suite
  remains queued behind it. The complete Bun workspace check is running against
  verified local PostgreSQL 17.11.0. Push this compiling work in progress now;
  no current-source full-suite or hosted native-lease pass is claimed yet.
- Review finds and reproduces a pre-policy launcher path escape: a real harness
  linked against the retained base agent library puts desktop.stderr behind a
  guest-created symlink. The trusted desktop launch truncates the foreign marker
  before its deliberately absent namespace helper fails. Use the existing
  descriptor-relative no-follow walk for all three launchers' scratch directory
  creation and diagnostic files; add real parent/leaf-symlink and positive own-
  output regressions. This is a concrete base-code failure, not a hypothetical
  relaxation or a new host requirement.
- Remove the native SSH launch's guest-writable source-file and home-mount paths.
  Pass account bytes in anonymous descriptors from the protected downloads root
  using the pinned bubblewrap 0.13.1 ro-bind-data option (checked in its exact
  tagged source), and mount only the stable scratch root read-only onto a fixed
  private authorization path. The closed SSH worker alone reads that namespace-
  private alias; ordinary commands retain all original grants. Keys/configuration
  no longer need even an empty mountpoint in persistent data. Retain all existing
  command, PTY, neighbour, expiry and revocation assertions; fresh hosted proof
  for this follow-up is still required.
- Source-c471 local verification passes its new Unix-stream/lifetime regression
  and all agent tests reached by the complete workspace invocation. Serialized
  node tests still fail the unchanged three-second real Wasm HTTP-readiness case
  (256 node unit tests pass); serialization did not resolve it, so contention is
  not an established diagnosis. The full Bun check passes lint/type/schema/docs/
  release stages and 430 tests, with one existing skip; MCP's thirty-second
  deadline and the fake fleet's stale-node health case fail while compilation
  runs. Retain those failures and require a focused diagnosis/current hosted
  results; do not loosen a timeout, health rule or assertion.
- Source c471eb6 has green complete producer
  [131482068172](https://github.com/cubepals/pryx/actions/runs/43303928155/job/131482068172).
  Its real confined agent creates a native SSH lease, forwards the actual
  OpenSSH greeting over authenticated WebSocket, refuses excess capacity,
  closes existing streams on revocation and shortened expiry, denies reconnects
  and keeps authorization outside data. Retain ssh-lease-c471eb6.json under k24.
  Its actual headful desktop and SSH command/PTY/neighbour cases also pass.
  The signed installed continuation is running; hardened-launcher and installed
  product proof remain required.
- Hardened launcher tests pass locally: 34 agent unit tests (one existing ignored
  subprocess entry) and the new real browser/desktop/SSH foreign-file regression
  all pass. Positive owned-output and parent-symlink tests pass. The old-library
  harness demonstrates the pre-fix truncation; the regression retains its foreign
  marker after every corrected launcher refuses a malicious diagnostic path.
  The source-7664 normal core job 131474102904 is also green, including its complete
  real Wasm/Docker checks; the local Wasm readiness failure remains unresolved,
  not relabeled as a pass. Native capability announcements remain unchanged.
- Final hardened-launcher agent suite passes completely: 76 tests, eleven
  explicitly ignored hosted/subprocess cases. Fmt, mixed structure and whitespace
  checks pass. The first regression compile exposed the browser launcher's private
  Rust visibility; expose its fixed trusted launcher like the desktop/SSH launchers
  so the real external regression exercises it. No HTTP entrypoint is added.
  The idle Bun run passes 431 tests, with one existing skip, but fails the unchanged
  held-upload acknowledgement deadline; the previous MCP and move failures pass.
  This remains an explicit local failure pending focused diagnosis. Fresh hosted
  native SSH/desktop acceptance is required after the safer account mounts.
- Hardened source f3d14ad has green complete producer 131492976250 on
  [43307047875](https://github.com/cubepals/pryx/actions/runs/43307047875).
  Read its completed log: the new fixed private SSH account mounts retain real
  command/PTY and same-UID privacy, and the real-agent WebSocket lease case
  retains capacity, shortened expiry, revocation and reconnect refusal. The
  complete headful application/noVNC/Chromium sandbox/neighbour proof also
  passes. Strict local all-target agent Clippy passes after the follow-up.
- Add installed-user acceptance for application-observed TypeScript and both
  Python-mode desktop input, real noVNC/RFB through the HTTP edge, actual SSH
  commands and wake through HTTP, the installed CLI and both Python transport
  modes. Extend the existing thirty-sample native cohort with desktop images
  and SSH command readiness; retain every existing kernel/browser cohort and
  all container budgets. Default-container namespace refusals now explicitly
  cover desktop and SSH too. This adds an estimated two to four minutes to the
  existing branch/scheduled bundle job, no ordinary PR execution, job or gate.
  The compiled fixture has a narrow ProxyCommand entry which reuses the existing
  CLI/SDK stream and receives only sandbox/lease IDs. A real local compiled-mode
  probe reaches its SDK lookup and emits no stdout. Types, fixture compilation,
  Python syntax, shell syntax, mixed structure and whitespace pass; actual new
  installed cases and timings remain pending fresh hosted execution.
- Current installed-acceptance source passes the full Bun workspace check:
  432 tests, one existing compiled-assets skip, 8,386 assertions against real
  PostgreSQL. All lint/type/generated-schema/docs/release stages pass. Earlier
  local timing failures remain above; the focused temporary RPC trace observed
  its timeout callback and error event promptly, so no speculative node-client
  change or diagnostic logging is retained. Correct the agent suite's ignored
  count to eleven in this checkpoint. New installed K24 proof remains pending.
- Preserve the executable ownership check unchanged in signed source 486ceaf,
  moving it into runtime/agent_metadata for the read-only configured doctor.
  Local cargo check and rustdoc pass. The preceding full Rust invocation passes
  all 487 executable cases (33 explicit ignores), including real Wasm; its final
  rustdoc catches the intermediate move's private-module access, corrected before
  this checkpoint. Do not describe that invocation as an all-stage pass.
- Add optional public requiredTools desktop/SSH intent, generated SDK contracts,
  observed process-tier feature advertisement, placement and unchanged-spec wake
  enforcement. Keep requirements out of the node WorkloadSpec. The configured
  doctor uses existing protected-agent checks and a read-only journal load; its
  missing-history regression must not initialize or repair runtime state. Hosted
  installed capability/doctor proof and the complete current checks are pending.
- Installed source a54b426, bundle job
  [131508549256](https://github.com/cubepals/pryx/actions/runs/43309758259/job/131508549256),
  passes real application-observed TypeScript and both Python-mode desktop input,
  PNG and edge noVNC/RFB. SSH prints the actual marker through the HTTP edge,
  including wake, then the immediate public view assertion sees starting rather
  than running. Inspect route acknowledgement before changing that assertion;
  remaining CLI/Python SSH and thirty-sample cohorts did not run.
- Source 486ceaf producer
  [131514579182](https://github.com/cubepals/pryx/actions/runs/43313268690/job/131514579182)
  fails actual headful navigation: bundled NSS 3.87.1 loads the host's newer
  libsoftokn3, which requires missing NSSUTIL_3.94, and Chromium exits. Its real
  SSH command/PTY/neighbour test still passes. Earlier successful desktop runs
  do not resolve this incomplete dynamic dependency closure; fix and prove it.
- Cancel superseded pending runs 43301624317, 43303928155, 43307047875,
  43303927680, 43307060009 and 43307047872 after inspecting their completed jobs.
  Preserve their recorded successful producer/bundle evidence above; their
  canceled remainder is not aggregate proof. No completed failure was canceled.
  Keep latest source runs and c471eb6 PR CI 43303936669 for complete results.
- PR CI 43303936669 is now completely green for source c471eb6. Capability
  checkpoint's 28 focused CLI/placement/workload tests pass (126 assertions),
  including the no-installation-state doctor regression. Generated Python
  consistency and mixed structure checks pass. The first doctor test observed
  Bun's own transpiler cache in its disposable HOME; disable that child cache
  and retain the exact no-new-state assertion. Full current suites remain running.
- Current cargo test compiles all targets, then passes 34 agent unit and 19 API
  cases before the unchanged bootstrap test fails its positive spare-listener
  assertion. The main TCP socket is bound before the spare sockets, and the
  test treats a successful main connect as their startup completion. Preserve
  the test's interface-denial assertions and bind the spare transport before
  exposing the main listener in a follow-up. This compiling checkpoint is WIP;
  the full node suite and Bun check are still running. The compiled installed
  fixture, typecheck, generated API checks and structure checks pass.
- Capability source c058a1c passes the complete Bun workspace check against real
  PostgreSQL: 436 tests, one existing skip, 8,407 assertions; lint/type/generated
  contract/docs/release stages pass. The bootstrap startup-order correction and
  remaining complete Rust verification are still running.
- The node-only complete invocation passes 258 unit cases, including both new
  actual-agent SSH metadata and read-only-history regressions, then fails the
  unchanged three-second Wasm immediate-HTTP-listener test again. Retain this
  local failure; earlier complete Wasm proof and green hosted core runs do not
  turn it into a local pass. No timeout or assertion changes are made.
- Complete the desktop NSS runtime closure with its pinned dynamic modules and
  pinned SQLite dependency, reviewed notice and all four corresponding-source
  inputs. A real local NSS database/random control loads softoken/freebl/SQLite
  only from the component and passes; the same control against the original
  three-ELF NSS selection fails initialization. Retain this control in the
  existing assembly/relocation acceptance, with no new job. Its first disposable
  fixture copied the SQLite alias as a regular file, causing the exact-map check
  to fail; preserve upstream symlinks as the real assembler already does.
- Keep actual SSH command success and wait for the SDK's existing running-state
  condition before the fixture's final state assertion. This respects the
  existing counted-edge acknowledgement rule; no state policy changes. Updated
  installed fixture compilation, Python syntax, notice/source identity inventory,
  mixed structure, formatting and whitespace checks pass. Fresh hosted producer
  and full installed acceptance remain required after the NSS correction.
- The corrected bootstrap passes both real subprocess tests, including every
  configured-interface positive and foreign-interface negative assertion and
  supervisor-tree forwarding denial. The main listener is exposed only after
  spare binding and native worker preparation; no test assertions are changed.
- Source 6519720 passes the complete local cargo test invocation: 489 tests,
  33 explicit ignores, including the unchanged immediate Wasm HTTP case,
  complete real Wasm integration and rustdoc. An out-of-tree diagnostic using
  the same pinned distribution reaches the Node listener in 1.4 seconds and
  retains its startup output. No Wasm change is justified or retained; keep the
  earlier intermittent local failures above. Updated fixture typecheck passes.
  The original pinned Rust structure checker also passes.
- Inspect superseded 486ceaf/c058a1c normal runs: every completed job is green,
  with only remaining end-to-end/release jobs active. Cancel 43313276118,
  43313268621 and 43316964375 to release capacity for 6519720; cancellation of
  43316955102 returns GitHub HTTP 502, so do not count it as canceled. The
  c058a1c producer 131527347634 passes; cancel its queued bundle continuation
  in 43316955031 in favor of the source with the NSS fix. Latest 6519720 normal
  and Any-host runs remain required; canceled runs are not aggregate proof.
- Strict all-target local Clippy passes for source 6519720. Update native-tools,
  comparison and host-capability documentation with process desktop/SSH host
  prerequisites, scoped transports, optional requiredTools placement/wake intent
  and read-only doctor behavior. Documentation and whitespace checks pass; do
  not add thirty-sample desktop/SSH numbers before fresh installed CI produces
  them. The latest installed proof and aggregate host matrix are still pending.
- Source 6519720 producer 131530530128 in 43317876537 passes the relocated
  componentNssDatabase control, measured glibc 2.36 floor on host 2.39, and real
  private OpenSSH command/PTY/host-key/neighbour checks. The actual-agent lease
  case also retains revocation/expiry closure and denied reconnects. Headful
  input instead fails the immediate click-observation assertion after 3.85s;
  result.json only means xdotool returned, while the application reports input
  asynchronously. Retain desktop-application-6519720.json and wait for both
  actual click and complete typed text within the unchanged startup deadline.
  Keep both final assertions and every privacy check. Installed continuation
  is skipped, not proved.
- Clarify a missing existing process history in doctor with its compatible-node
  startup requirement, keeping the no-write behavior. Add node-side model
  coverage for native feature withdrawal and refusal to advertise another
  backend's tools. All 294 library tests pass (one explicit ignore), including
  both regressions and the unchanged Wasm listener. The updated headful fixture
  compiles; its two hosted cases remain explicitly ignored locally. Fresh
  hosted observation/installed proof and current strict lint remain pending.
- Current strict all-target Clippy passes. The full Rust run during concurrent
  pinned-tool assembly passes its unit/API suites but fails the unchanged real
  Wasm restart command with exit 137 (tests/wasm.rs:93), after runtime worker-stall
  diagnostics and 144.69s. Pause only that optional assembly's process tree for
  a controlled complete invocation: all 490 tests pass, 33 explicit ignores,
  including real Wasm and rustdoc, with every deadline unchanged. Automatically
  resume assembly afterward. Retain both results; no Wasm change is justified
  or retained. Update the NSS runtime-map control to import SQLite's ELF filename
  from its pinned inventory; the real local database/random control passes.
  Python discovery passes its two portable tests, with six explicit asset-based
  skips; do not count those as executed desktop proof.
- After inspecting completed jobs (none failing), cancel superseded normal runs
  43318906923, 43318893241, 43317889920, 43317876556 and 43316955102, and
  superseded producer run 43318893527, to prioritize source 35b8b15. Three much
  older queued 235399d runs report already completed when cancellation is
  attempted; they are not newly canceled or aggregate proof. Source 35b8b15's
  producer/normal CI remain active. Optional local artifact download is denied
  by the environment proxy at Azure storage; assemble the same pinned official
  inputs locally instead, without changing networking policy or verification.
- Source 35b8b15 has green complete producer
  [131537320861](https://github.com/cubepals/pryx/actions/runs/43319847311/job/131537320861).
  Read its completed log: corrected headful input passes in 3.56s, with 27 real
  application observations including the complete typed string and click. Keep
  desktop-application-35b8b15.json: noVNC/RFB, expiry closure/reconnect refusal,
  Chromium's own PID namespace/seccomp/no-new-privileges, independent neighbour
  abstract/cookie/proc-root denial, and thirteen unrestricted positive aliases
  remain proved. The completed producer also passes real SSH command/PTY/host
  key/key-memory/neighbour checks and native-agent lease revocation/expiry.
  Installed bundle job 131543614392 is running; do not infer its outcome from
  this staged-worker proof. Latest source 8d6be2f imports the same pinned SQLite
  filename in the NSS control, with unchanged product behavior; its CI is active.
- Local exact pinned runtime assembly completes successfully. The explicit
  guarded headful invocation then fails immediately at its mandatory filesystem
  confinement probe on this executor; do not bypass that precondition or claim
  local headful/privacy execution. Hosted producer 131537320861 remains the
  positive confined proof. The local staged programs and NSS control provide
  reproducible inputs for further diagnostics, without changing product policy.
- Correct the installed summary's engine-only configured-probe condition:
  process presets have an existing node configuration too, and a bare probe now
  deliberately reports native capabilities off. Pass that existing configuration
  and strengthen the four-process credential-summary fixture to require its real
  probe argv while preserving the one-time credential claim. Both focused
  summary/doctor tests pass; the complete Bun check passes 436 tests, one existing
  skip and 8,407 assertions against real PostgreSQL, with all preceding stages
  green. Fresh installed summary/capability acceptance remains required.
- Installed source 35b8b15, job
  [131543614392](https://github.com/cubepals/pryx/actions/runs/43319847311/job/131543614392),
  passes configured doctor, native requiredTools placement, real TypeScript and
  both Python-mode desktop click/text/PNG/noVNC, and actual TypeScript HTTP SSH
  commands including sleep/wake and the running-state condition. It then fails
  the installed CLI SSH command: Bun reports the explicit glibc loader as
  process.execPath, so ProxyCommand tries to load sandbox as an ELF program.
  Retain installed-35b8b15.json; Python SSH and timing cohorts did not run.
- Reenter the installed wrapper through the existing bundle locator for HTTP
  SSH ProxyCommand. A regression compiles the real CLI and a probe, runs them
  through the glibc --argv0 loader pattern in a path containing spaces, confirms
  the loader-valued execPath, and reaches the real proxy SDK lookup without
  transport stdout. The former executable selection fails that same regression
  with exit 127; the corrected selection and opaque-stream regression both pass.
  Lint/type/schema/docs/release and mixed structure checks pass. The complete Bun
  test stage is still running in this compiling checkpoint; fresh installed CLI,
  Python SSH, timing and full aggregate results remain required.
- The hosted task machine failed a fourth time after d125752 with nothing unpushed. Work continues
  from that commit on a supervised machine that restarts the same task when it stops. No source
  change in this checkpoint.
- On the supervised executor, inspect the completed d125752 normal push
  [43325981612](https://github.com/cubepals/pryx/actions/runs/43325981612), PR
  [43325989142](https://github.com/cubepals/pryx/actions/runs/43325989142), both
  rootless runs and CodeQL: all green. Its Any-host producer
  [131558422100](https://github.com/cubepals/pryx/actions/runs/43325981545/job/131558422100)
  fails before the installed continuation: the page publishes negative target Y
  coordinates while Chromium's outerHeight is zero. Preserve
  desktop-geometry-d125752.json. Earlier 961649e/e52c76a installed failures are
  the already corrected CLI loader problem; they establish no later SSH proof.
- Wait for mapped browser dimensions and visible target centers before publishing
  application geometry. A real local headful Chrome/Xvfb control holds the initial
  outerHeight at zero for 250 ms after load: the original fixture emits negative
  coordinates; the correction waits and emits valid coordinates from the actual
  restored metrics. This is geometry diagnosis only, not confined desktop/input
  or installed acceptance. All original hosted assertions and deadlines remain.
- The supervised machine's Bun 1.4.3 full workspace check passes every stage except
  the SSH stream test: its built-in ws adapter throws Not supported yet in Bun.
  Install pinned Bun 1.4.4 to a disposable task directory from the npm distribution,
  verifying its published SHA-512 integrity; the unchanged stream regression passes.
  Repeat the complete check with the repository's pinned runtime and real local
  PostgreSQL, preserving the initial mismatch failure.
- With pinned Bun 1.4.4, the complete workspace check passes: 437 tests, one
  existing compiled-assets skip, 8,415 assertions against real PostgreSQL; all
  lint/type/generated-schema/docs/release stages pass. Current a421236 producer
  131847490561 is green and its installed continuation is running. The mapped
  geometry correction still requires its own hosted proof.
- Preserve installed timing evidence after each validated sample, before teardown,
  instead of only after all five cohorts finish. A later failure now retains
  completed observations in the existing always-uploaded report. Keep the same
  report shape, timer boundaries, cleanup behavior and thirty-sample assertions;
  partial arrays never count as a complete cohort. No CI job, gate or measurable
  execution cost is added. Current full local checks are running.
- The incremental-retention change passes the complete pinned-Bun workspace
  check: 437 tests, one existing skip, 8,415 assertions with real PostgreSQL,
  including all lint/type/generated-contract/docs/release checks. A separate
  final typecheck and whitespace check pass. Installed cohorts remain pending
  hosted execution; no partial or earlier-source result is promoted to completion.
- Mapped-geometry source b404f56 has green complete producer
  [131852405145](https://github.com/cubepals/pryx/actions/runs/43415551803/job/131852405145).
  Its application reports actual outerHeight 719 and innerHeight 600 before
  accepting coordinates, observes all 27 input events, and passes every noVNC,
  expiry, Chromium sandbox and neighbour assertion, with thirteen real positive
  aliases. Retain desktop-application-b404f56.json. Cancel only its queued bundle
  continuation in favor of latest f07c8c6; this is producer proof, not aggregate.
- After inspecting their completed jobs (all green), cancel superseded normal
  runs 43415564162, 43415551815 and 43414083135 plus intermediate CodeQL
  43415563657 to free runner capacity. Retain a421236 PR run 43414091255 and
  installed run 43414083235 while latest-source workflows start. No canceled
  remainder is treated as passing, and no completed failure was canceled.
- Latest implementation f07c8c6 has green producer 131854811655 in
  [43416258168](https://github.com/cubepals/pryx/actions/runs/43416258168), with
  all 27 mapped-geometry application observations and complete desktop/SSH
  confinement cases. Push core 131855019699 passes the complete Rust/Wasm suite,
  real configured KVM probe and all 21 actual Docker cases; PR core is green too.
  Both current contract/workspace jobs are green. Latest bundle 131859900624
  is building; earlier a421236 bundle 131852252855 has reached signed-user
  native acceptance. Neither installed outcome nor aggregate is inferred.
- Installed a421236 job
  [131852252855](https://github.com/cubepals/pryx/actions/runs/43414083235/job/131852252855)
  proves all functional cases, including the corrected installed CLI SSH command,
  both Python SSH modes, actual HTTP-edge wake, both SDK desktop input and
  noVNC. It later fails a screenshot startup cohort with file_failed/404 and
  ENOENT. Its final-only collector retains no measurement summaries; do not infer completed
  cohorts from elapsed time. Retain installed-a421236.json with the downloaded
  artifact's verified SHA-256. Default-image and host continuation are skipped.
- Independently reproduce a desktop startup race using the actual committed
  adapter publication sequence and a second real Node process: the private
  listener accepts a connection before desktop-ready.json exists. Agent startup
  uses that connection as readiness, then immediately opens the missing metadata.
  Write complete metadata before listen. The retained portable regression fails
  against the old sequence and passes the correction on Node 22.23.4; the existing
  producer suite will run it before real pinned-Node/headful product acceptance.
  Local Python discovery finds nine cases: three pass and six retain their
  existing asset-based skips. Node syntax and whitespace pass. No deadline or assertion is relaxed,
  no job or gate is added, and measured local regression cost is under a second.
- Baseline a421236 PR CI
  [43414091255](https://github.com/cubepals/pryx/actions/runs/43414091255)
  is completely green, including Wasm, two-node, upstream integrations and
  store-loss recovery. This remains baseline proof, not the corrected desktop
  publication sequence's final result. Latest implementation and checkpoint
  installed jobs remain active and must retain their own outcomes.
- Incremental-retention source f07c8c6 installed job
  [131859900624](https://github.com/cubepals/pryx/actions/runs/43416258168/job/131859900624)
  again passes every functional desktop/SSH SDK, CLI and wake case, then fails
  the second desktop timing sample with file_failed/404 and ENOENT. Its verified
  artifact now retains all thirty browser, thirty Python, thirty JavaScript and
  one desktop observation. Preserve partial-process-startup-f07c8c6.json and
  partial-provenance-f07c8c6.json; no SSH timing cohort ran and the failed job is
  not completion. This narrows the missing-file failure to native desktop startup.
- The metadata publication regression also passes on the machine's exact pinned
  Node 24.22.1. Running the original c4afa94 publication sequence through that
  same real listener/second-process control fails at the expected missing metadata
  assertion. Fresh source 53e8350 producer is running, after its portable control.
- Inspect completed jobs (none failing) before canceling superseded c4afa94
  workflows 43419556297, 43419556330 and 43419567404 plus f07c8c6 push CI
  43416258410. Retain f07c8c6 PR CI and both completed installed failures for
  evidence; cancellations are not passing aggregates. Latest 53e8350 workflows
  take the released runner capacity.
- Publication source 53e8350 has green producer
  [131873383538](https://github.com/cubepals/pryx/actions/runs/43421813961/job/131873383538).
  Its portable discovery passes three cases with six explicit asset skips, then
  real confined tools, all 27 headful observations, VNC expiry, Chromium's own
  sandbox, every neighbour denial, and OpenSSH command/PTY/key-memory checks pass.
  The initial nine-case portable discovery takes 5.328 seconds on that runner;
  it adds no ordinary PR execution or new job. Corrected signed bundle
  131880099054 is building; complete timing and installed product proof remain.
- f07c8c6 PR CI
  [43416268136](https://github.com/cubepals/pryx/actions/runs/43416268136)
  finishes completely green, including full deployment/integration/recovery.
  Keep its failed installed-native report separate from that normal-CI success.
- K24 final implementation proof: source 53e83505f823cf4b92aca2b4a952cf084c636f32
  has completely green normal PR CI
  [43421825320](https://github.com/cubepals/pryx/actions/runs/43421825320)
  and all sixteen Any-host jobs
  [43421813961](https://github.com/cubepals/pryx/actions/runs/43421813961).
  Retain host-matrix-53e8350.json with every actual job conclusion. The corrected
  signed bundle 131880099054 passes all functional cases, all 150 startup
  samples, default-Docker kernels and browser/desktop/SSH refusals, purge and
  both image/Alpine relocation. No K24 piece is parked.
- Retain the exact process samples and installed logs, current container
  desktop/browser/kernel samples and unchanged interpreter budget report, with
  separate digest-verified artifact provenance. docs/speed.md records thirty
  samples per tool: process desktop p50/p90 8959.8/9056.8 ms and SSH
  8970.6/9006.4 ms; current container desktop 739.6/846.3 ms. SSH's timer includes
  actual command completion, running acknowledgement and lease revocation.
  Both tiers request four units on separate runners, with their different
  guarantees stated. Existing container trivial/chart p90 budgets pass at
  3.8/111.9 ms, with every original clock point and threshold preserved.
- Actual K24 scheduled/branch-only measurement cost: the new desktop and SSH
  cohorts total 8.956 minutes of timed work, plus cleanup overhead, correcting
  the earlier two-to-four-minute estimate. The bundle build takes 846 seconds
  in this successful run. No ordinary PR execution, required job or gate is
  added by these cohorts or the portable publication regression.
- Latest evidence checkpoint 8e9a066 has green PR CI 43424283667 and producer;
  its installed continuation and normal push CI are still running. Keep each
  result tied to its producing runtime: the two runtime digests differ even
  though the corresponding-source archive matches. Later evidence-only commits
  cite the green implementation above without claiming their future CI outcomes.
  Superseded 53e8350 push CI 43421814064 was canceled after inspecting completed
  jobs (all green); its retained PR CI provides the complete normal proof.
- K24 is complete. Proceed immediately to the K25 brief and its kernel/network
  spikes. Production main/release, version/tag changes and paid provisioning
  remain prohibited. The supervised executor ran no Docker or KVM locally,
  created no signing key, and committed with the configured identity unsigned.
- K25 begins after K24's recorded complete green implementation. Re-read the
  brief, host design, runtime seam/process journal/launch and current decisions.
  Verify Firecracker 1.18.0, gvisor-tap-vsock 0.9.0 and the current official Go SDK
  source through their primary releases/licences. No VM capability is announced.
- Published Firecracker CI build 20261111-778a26c0594c-0 supplies kernel 6.18.62
  and its config. The config enables overlay, virtio block and vsock but explicitly
  disables EROFS. Preserve firecracker-kernel-candidate.json with the configuration
  hash and primary URL; this is not a boot result. Inspect Kata's published
  4.3.1 kernel next. No private kernel build, emulation or weaker filesystem path
  is substituted. Local KVM and Docker execution remain unavailable by design.
- Local filesystem tools are Ubuntu erofs-utils 1.7.1-1build2 and e2fsprogs
  1.47.0-2.4~exp1ubuntu4.1; read their actual complete installed copyright
  records before use. Debian's current pasta source is
  0.0~git20261002.cba3570-1, GPL-2.0-or-later with retained BSD-3-Clause checksum
  code. These are external-program spike candidates, not a shipped VM bundle.
- Add a branch/scheduled-only VM input inventory workflow. Primary expanded-release
  metadata supplies committed SHA-256 pins for Firecracker 1.18.0 x86_64 and
  Kata 4.3.1 amd64. The first stage resolves source commits and streams both
  archives through size/digest checks without extracting or executing them.
  It retains partial candidate evidence if a later candidate fails. API credentials
  stay on their HTTPS origin; public archive redirects carry no token and cannot
  downgrade TLS. Real kernel/network experiments follow verified input identity.
- This initial non-required VM workflow adds zero minutes to ordinary PR execution.
  Its branch/weekly inventory job has a twenty-minute ceiling and an estimated
  one-to-five-minute cost for primary metadata and roughly one GiB of archive
  streaming; record actual hosted duration afterward. No settings, required checks,
  release keys, version or tags change. The one-command installer is untouched.
- Install the existing Go 1.27.3 build-tool pin from its primary archive after
  verifying the published byte length and SHA-256. Build repository-pinned
  actionlint 1.7.12 from its MIT source via the checksum-verified Go module proxy;
  workflow validation passes. Local Python compilation, CLI help, documentation
  checks and whitespace pass. Upstream release inventory is left for CI because
  this executor reaches only the pryx GitHub repository; no inventory success,
  published-kernel boot or network result is claimed locally.
- K25 input source 13d4f5b has green published-archive inventory
  [43437923851](https://github.com/cubepals/pryx/actions/runs/43437923851/job/131927717596).
  The entire hosted job takes eleven seconds, verifying 7,606,252 Firecracker
  bytes and 1,021,129,805 Kata bytes against both committed and actual upstream
  digests. Retain input-inventory-13d4f5b.json and pin the observed source commits;
  a moved tag is now refused. No monitor or kernel was executed in that result.
- Extend that same optional job with a verified private archive cache and streamed
  inspection. Only bounded regular kernel/monitor bytes become content-addressed
  files; archive paths and links never become host paths. Preserve configurations,
  links and exact member hashes for boot selection. The reviewed external zstd
  command decodes the Kata archive. Local real tar/zstd controls pass for path
  escape, untraversed links, configuration retention, repeated staging and corrupt
  or truncated cache rejection; Python compilation, documentation checks,
  actionlint and whitespace pass. Real published-member inspection is left to CI.
- Source 60d5b50's archive inspection
  [43440140299](https://github.com/cubepals/pryx/actions/runs/43440140299/job/131935324964)
  is green in sixteen seconds. Retain archive-members-60d5b50.json. Kata's actual
  vmlinux-6.18.56-208 has SHA-256 f87d8359035cb7557b16a319e6431ea09b183e62268aef436bf8dc6530d786b6;
  its matching published config enables all six required filesystem/virtio options
  built-in. Pin both member identities before execution. This still proves no boot.
- Add the real ordinary-user KVM kernel spike to that same optional workflow:
  thirty cold minimal boots, EROFS lower immutability, ext4-backed overlay writes,
  actual guest-to-host vsock, differing kernel identity and one disk-retaining
  reboot. Retain incremental reports and every serial log. No monitor confinement,
  network comparison, OCI/agent runtime, production latency or VM feature is claimed.
  The existing hosted KVM access setup affects only the disposable runner.
- Build the static guest and SDK-typed configurator locally with pinned Go 1.27.3;
  Go vet passes. Resolve and retain complete primary notices for the exact 23-module
  checksum-pinned closure. Real local EROFS creation/extraction preserves the compiled
  guest byte for byte and immutable marker; the generated configuration validates
  against the upstream SDK and gives the guest no network interface. Python compilation,
  documentation/whitespace and actionlint with actual ShellCheck 0.11.0 pass.
- The kernel experiment still adds zero ordinary PR minutes or required checks.
  Its branch/weekly job retains the twenty-minute ceiling; estimated two-to-five
  minutes now includes private verified build tools and thirty boots. Record the
  actual hosted cost and results, including failures. Local Docker/KVM remain unrun.
- First real kernel spike source 68046d3 fails in
  [43441847267](https://github.com/cubepals/pryx/actions/runs/43441847267/job/131941167952).
  Its digest-verified artifact retains zero completed samples. Preserve the complete
  failed serial log, report and artifact provenance. The published guest kernel
  6.18.56 boots under ordinary host UID 1001, discovers virtio block/vsock, mounts
  EROFS and launches PID 1; that fixture then fails with device-or-resource-busy
  on its duplicate devtmpfs mount. This is partial boot evidence, not acceptance.
- Require CONFIG_DEVTMPFS_MOUNT and verify its actual /dev mount and filesystem
  rather than remounting the kernel's single-instance devtmpfs. Keep every existing
  filesystem, disk-retention, vsock, differing-kernel and sample-count assertion.
  Remove the inspection script's standard-library name collision, which caused
  an unrelated apport exception while reporting the timeout. Local builds, Go vet,
  Python compilation, full actionlint/ShellCheck and whitespace pass; real boot
  success still requires fresh hosted proof.
- The failed job also reveals PATH's zstd is 1.5.7 although the installed Ubuntu
  package is 1.5.5; pin and verify the already reviewed 1.5.5 package privately
  before inspection, retaining its complete notices. Hosted mke2fs's package is
  the reviewed e2fsprogs 1.47.0-2.4~exp1ubuntu4.1. The local machine ran no KVM.
- Inspect completed jobs (none failing) before canceling superseded input-only
  source 13d4f5b runs 43437923673, 43437934155, 43437923840 and source 74d8750
  runs 43435813982, 43435801507, 43435801516. These cancellations free capacity
  for the latest real experiment and count as no passing aggregates. Retain the
  already green K24 implementation and the completed upstream inventory results.
- Kernel spike source 9402ef5 is green in
  [43442622901](https://github.com/cubepals/pryx/actions/runs/43442622901/job/131943829577).
  Verify the actual evidence archive digest before retaining kernel-spike-9402ef5.json,
  kernel-provenance-9402ef5.json and first-boot/reboot serial logs (line endings
  normalized in committed text). All thirty cold samples and the one disk-retaining
  reboot prove EROFS/overlay/ext4/virtio/vsock and unchanged lower bytes; guest
  Linux 6.18.56 differs from host 7.0.0-1017-azure. Host UID is 1001 throughout.
- Minimal kernel launch-to-validated-report p50/p90 is 717.035/725.265 ms;
  disk-retaining reboot is 724.471 ms with boot counter two. Fixture root is
  4,460,544 bytes; initial image preparation is 21.341 ms. The first blocked
  monitor's observed RSS is 56,812 kB with a 256-MiB, one-vCPU guest; this is not
  production idle-memory proof. The entire optional job takes 67 seconds,
  adding zero ordinary PR minutes. Every original thirty-sample assertion passes.
- K25's published-kernel spike is complete and its decision recorded. Continue
  immediately with the pasta/user-namespace TAP versus maintained vsock network
  comparison, including both guest management-path denials, then the production
  lazy VM bundle, OCI conversion, agent/backend, confinement, lifecycle and timings.
  No VM feature is advertised, K25 remains in progress, and no piece is parked.
- Build both network comparison programs plus gvisor-tap-vsock's unchanged guest
  forwarder in a separate Go module, retaining its actual 22-module primary-notice
  closure before execution. The existing kernel fixture keeps its dependency
  selection. Record the Alpine 3.24.4 root archive digest and every actual APK
  licence/source-build commit; compile unchanged pasta from its pinned upstream
  source with the source version supplied through its documented Make variable.
- The guest-facing vsock listener delegates only exported ConnectPath with the
  maintained StdioProtocol. The separate private host management listener remains
  usable; a fresh host-memory token guards the gateway API. The comparison requires
  real guest management denials over both vsock and gateway IP. No token enters an
  argument, image, environment, journal or report. Library API shapes and protocol
  constants are imported; private fixture sizes and paths have one owning module.
- Add the comparison to the same optional VM job: thirty samples each over the
  same published kernel, Alpine EROFS/ext4 overlay, resource size and MTU. Report
  safe HTTP latency, owner-sized 16-MiB throughput, actual outbound DNS, helper
  preparation, validated guest report and RSS separately. Pasta creates its own
  user/network namespace and private TAP bridge; assert it maps no host root and
  does not use the host network namespace. Its monitor is parent-bound. Preserve
  both paths and completed observations if either fails; success requires both.
- Local final-binary checks pass for Go builds/vet, both exact notice closures,
  pinned pasta source preparation and unprivileged Alpine-to-EROFS validation.
  The real maintained proxy serves private management and denies guest-facing
  stats/listener routes; actual loopback HTTP bodies use the owning fixture's
  sizes. Python compilation, CLI help, documentation, complete actionlint with
  ShellCheck 0.11.0 and whitespace pass. No local guest or TAP experiment ran.
- The positive hosted pasta comparison enables user namespaces and the existing
  tun device only on its disposable runner, as other positive capability fixtures
  do. Product setup changes no host policy. The same optional branch/weekly job
  adds zero ordinary PR minutes or required checks; estimated total three-to-six
  minutes now includes ninety cold guests and compilation. Record actual time
  and any real network failures after this push. Production VM support remains off.
- Network source 57c7f93 fails setup in
  [43446165170](https://github.com/cubepals/pryx/actions/runs/43446165170/job/131956052064)
  before either network guest runs. Its digest-verified artifact proves all thirty
  updated kernel boots still pass; preserve network-setup-failure-57c7f93.json
  with zero network observations. Go mod verify tries to verify a nonexistent
  archive for the local replaced fixture module. Reproduce the exact failure
  locally and inspect the installed Go verifier's main-module handling.
- Declare the two local fixture modules as main modules in a workspace within
  the network directory only. The parent kernel build does not discover it;
  its dependency closure stays unchanged. Keep Go's existing verification and
  additionally run it after actual compilation. The real local corrected
  verifier passes, while an isolated downloaded module whose licence bytes are
  deliberately changed fails with Go's own modified-directory error. Remove
  that disposable cache afterward. The exact network notice closure still passes;
  no checksum check, assertion or network cohort is skipped or weakened.
- Corrected-module source 45fa355 reaches both actual guest paths in
  [43447042201](https://github.com/cubepals/pryx/actions/runs/43447042201/job/131959105927).
  All thirty vsock guests pass EROFS/overlay/ext4, real HTTP/bulk/outbound DNS,
  both management denials, guest report and successful monitor exit. The first
  pasta guest boots through the private ordinary-user namespace/bridge, then
  BusyBox reports its guest virtio interface down; it completes zero samples,
  so the entire comparison fails. Retain the digest-verified full partial report,
  pasta serial log, actual namespace mapping and archive provenance separately.
- Bring eth0 up with the already pinned maintained netlink API before requesting
  its DHCP lease. The upstream vsock forwarder already performs that step for
  its own TAP. Preserve both original thirty-sample cohorts, clocks, deadlines
  and every positive and management-denial assertion. No network path is parked.
- The first vsock small-request latency is dominated by the fixture server's
  default Nagle buffering. An independent real local thirty-request persistent
  HTTP control measures p50/p90 43.988/44.108 ms before TCP_NODELAY and
  0.160/0.239 ms afterward. Enable the standard server's TCP_NODELAY setting
  equally for both paths. Keep the earlier 45fa355 samples with that explicit
  caveat, and require fresh real-KVM cohorts; invent no corrected old timings.
- After checking completed jobs (none failed), cancel superseded 9402ef5 normal
  and host runs 43442625755, 43442612553, 43442612871; 8a48e68 runs
  43443131498, 43443118120, 43443118003; and 57c7f93 normal/host runs
  43446176133, 43446164652, 43446164627. These are cancellations, not passes.
  Preserve completed kernel/network failures and green standalone kernel proof,
  while retaining the earlier 60d5b50 full normal and installed continuations.
- Source 4674e2f's real comparison
  [43448545479](https://github.com/cubepals/pryx/actions/runs/43448545479/job/131964295978)
  again passes all thirty vsock guests with both management denials and the
  corrected TCP_NODELAY fixture. Retain its complete partial report, provenance,
  pasta serial and actual namespace mapping. The pasta guest now obtains its
  10.77.25.2 lease and reaches outbound DNS after earlier HTTP/bulk assertions,
  then fails because the guest has no advertised DNS server. No pasta timing is
  retained or invented, and the aggregate comparison is still failed.
- Read the pinned pasta manual and source: pasta deliberately defaults DHCP DNS
  advertisement off. Enable its documented advertisement and gateway-forward
  mapping, leaving discovery of the actual host nameserver to pasta. Preserve
  the exact outbound-DNS assertion and both thirty-sample requirements. Source
  4674e2f's vsock HTTP p50/p90 is 0.526/0.577 ms and helper-plus-guest report is
  964.389/1040.808 ms on its runner; this is a separate host from the earlier
  buffered cohort and is not a production latency or cross-run speedup claim.
- Archive-inspection source 60d5b50 now also has completely green normal push
  [43440140293](https://github.com/cubepals/pryx/actions/runs/43440140293)
  and PR [43440150813](https://github.com/cubepals/pryx/actions/runs/43440150813),
  including every actual aggregate, full Rust/TS/contracts, deployment and end-to-end
  recovery. The installed continuation 131940893233 remains pending at this
  checkpoint. This normal proof does not complete the new network spike or K25.
- Source f5ac55e comparison
  [43449577193](https://github.com/cubepals/pryx/actions/runs/43449577193/job/131967896508)
  retains another thirty successful vsock guests and zero successful pasta guests.
  Preserve the full raw partial report, failed pasta serial and verified artifact
  provenance. Pasta now advertises 10.77.25.1, and the unchanged real-DNS lookup
  times out against that address; the aggregate remains failed.
- Inspect pinned conf.c get_dns and add_dns_resolv4 together with fwd.c: setting
  an explicit guest DNS address marks IPv4 DNS complete and skips the discovery
  that also initializes the host forwarding target. Remove the explicit DNS
  address, keeping documented DHCP DNS advertisement and gateway forwarding so
  the helper's own resolver discovery/remapping runs. No DNS server, reply or
  timing is fabricated, and both thirty-sample cohorts and all denials remain.
  Python compilation, complete actionlint/ShellCheck, docs and whitespace pass;
  another real comparison is required. No new piece is parked.
- Source 187136d's complete kernel/network experiment is green in
  [43450896770](https://github.com/cubepals/pryx/actions/runs/43450896770/job/131972447502).
  Verify artifact a4b67e89bade5121b3d6dd18bc64bdbfb17d68fbebb733efba84aa1cb6de2a1d
  before retaining network-spike-187136d.json, network-provenance-187136d.json
  and representative serial logs. Both candidates complete thirty samples on
  the same host, with all guest filesystem, HTTP/bulk, DNS and namespace checks;
  vsock also passes both management denials in every guest. No path is parked.
- Record the complete input/method/limit account in docs/checks/k25-vm-spikes.md.
  Same-runner vsock/pasta HTTP p50 is 0.648/0.372 ms, 16-MiB throughput p50
  101.167/110.490 MiB/s, and helper-plus-validated-guest-report p50
  840.364/866.682 ms. Each startup report includes filesystem/network checks,
  bulk and DNS, so it is not production create readiness. The entire optional
  job takes 148 seconds; ordinary PR cost remains zero.
- Add a separate single-guest prerequisite case after the unchanged two thirty-
  sample cohorts. On only the disposable runner, make host user namespaces and
  TUN unavailable, assert pasta's real refusal, then require the complete vsock
  guest and both management denials to pass. Restore both host settings through
  a shell trap. The separate report cannot be mistaken for the full comparison;
  its reduced count is enabled only after checking the actual missing prerequisites.
  Also observe pasta helper RSS before the monitor acknowledgement, outside the
  measurement clock. Finalize the production network choice after this proof.
- Repeated source 0b8afec comparison
  [43452903447](https://github.com/cubepals/pryx/actions/runs/43452903447/job/131979462700)
  passes thirty vsock guests and thirteen pasta guests, then the fourteenth
  pasta bulk transfer stalls after 1,890,748 of 16,777,216 bytes and reaches
  the unchanged two-second client deadline. Preserve its full partial report,
  failed serial and verified artifact provenance. The prerequisite case did
  not run because the earlier step failed. The RSS observation works in the
  thirteen successful pasta cases; no cause for the transfer stall is established.
- Keep the exact client deadline, body length, clocks, DNS assertions, both
  management denials and two thirty-sample cohorts. Add pasta's own bounded
  debug log plus a read-only guest TCP_INFO observation before the existing
  deadline, importing the syscall library's layout/constants. A real local HTTP
  server deliberately stalls a partially delivered response: the same two-second
  client fails while the real kernel TCP observation arrives first. Go builds,
  vet and cache verification pass; Python compilation, complete actionlint with
  ShellCheck, docs and whitespace pass. This is diagnosis, not a claimed repair.
- Run the independently useful missing-userns/TUN case even after a comparison
  failure, once its actual positive runner setup succeeded; retain the original
  failure in the aggregate. No assertion is skipped, and its single guest remains
  clearly separate from the two original thirty-sample cohorts. No piece is parked.
- Diagnostic source 194c841
  [43456439308](https://github.com/cubepals/pryx/actions/runs/43456439308/job/131991706889)
  passes all thirty vsock guests and fourteen pasta guests before another pasta
  bulk stall. Preserve its full partial report, failed serial, bounded helper
  log and provenance. The real guest TCP_INFO observation succeeds before the
  unchanged deadline: receive window remains 1,091,200 bytes, with no guest
  retransmits. The helper changes its learned guest MAC away from the VM during
  the bulk transfer; the precise private-interface mapping is not yet retained.
- That same failed aggregate independently passes the missing-prerequisite case.
  Retain network-prerequisites-194c841.json: host user.max_user_namespaces is
  zero, host TUN access is false, actual pasta namespace creation refuses, and
  the full vsock guest passes both management denials, DNS, HTTP and bulk. Its
  single guest is separate from every thirty-sample comparison; it does not
  erase the pasta failure. The kernel spike remains proved, and no piece is parked.
- Finalize the rootless network choice as maintained gvisor-tap-vsock 0.9.0,
  using the same-runner positive numbers in the recorded spike account and the
  real missing-prerequisite proof. Both required input spikes now have decisions;
  the pasta repeat failure remains active work, not a hidden pass or parked piece.
  Production VM bundle/OCI/agent/backend, rootless confinement and rootful
  jailer/TAP/firewall, lifecycle, refusal and full timings remain unimplemented.
- For the suspected bridge-control cause, disable IPv6 only inside the already
  verified private IPv4-only TAP namespace, before making its bridge. Read back
  every actual interface setting and retain the real link addresses. The initial
  namespace/UID-map guards run before mutations. A real local initial-namespace
  invocation refuses with no topology record; this executor exposes no IPv6
  sysctl knobs, so the positive namespace change remains strictly CI-only.
  Python compilation, complete actionlint/ShellCheck, docs and whitespace pass.

- Source 8b2279f passes the complete optional VM experiment in
  [43458372918](https://github.com/cubepals/pryx/actions/runs/43458372918/job/131998442387):
  thirty kernel boots plus disk reboot, thirty guests per network path, and the
  independent complete vsock guest without host user namespaces or TUN. Verify
  artifact 5876e2107835d3b253a40b93ae5cf77797c40cb21dfc6436b9a78378dbc12aee
  before retaining full reports, provenance and representative upstream helper log.
  Every private interface reports IPv6 disabled, but five samples still briefly
  learn a bridge-interface MAC after the VM MAC and before the validated report.
  Suppressing IPv6 alone is not a proved repair of the earlier stalls. Preserve
  that limitation; the recorded production choice remains gvisor-tap-vsock.
- Archive-inspection source 60d5b50's complete Any-host matrix
  [43440140287](https://github.com/cubepals/pryx/actions/runs/43440140287) is green,
  including installed continuation 131940893233 and every one of sixteen actual
  host jobs. Retain baseline-host-matrix-60d5b50.json. This is application baseline
  proof before a production VM exists, not completion of K25.

- Begin the VM bundle with the filesystem component: checksum-pin ten Alpine
  packages, their seven complete upstream source archives, every recipe and
  source patch, and forty-three full original notice files. The original package
  keeper from the already pinned Alpine image verifies signatures; a real empty-
  trust-directory control refuses the same packages. No package script runs.
- Assemble only four unchanged filesystem executables and their actual ELF
  dependency closure, invoked through separate unchanged musl. This independent
  VM component adds no host glibc requirement, private native-tool glibc, package
  installation or ELF rewriting. Keep mke2fs's packaged configuration private.
  The ordinary-user local relocation control checks resolved library paths,
  a real EROFS file round trip and real read-only populated-ext4 integrity.
- Extend the existing optional VM job to assemble and relocate that component,
  then use its actual builders for all thirty published-kernel boots and the
  disk-retaining reboot. Retain its manifest identity in the boot report and
  its signature diagnostics/relocation report with the artifact. Ordinary PR
  cost remains zero; estimated optional-job increment is under one minute.
  Python compilation, complete actionlint/ShellCheck, docs and whitespace pass.
  Real KVM proof of these new builders remains pending. Production acquisition,
  monitor/kernel/network assembly and backend/agent/lifecycle remain incomplete.

- Filesystem source 87a5a0b's VM job
  [43463149416](https://github.com/cubepals/pryx/actions/runs/43463149416/job/132015121945)
  stops during source acquisition with an actual redirected HTTP 403, before any
  filesystem execution or new boot. The former failure diagnostic omitted the
  input URL. Add per-input URL/digest diagnostics and Alpine's official GitHub
  mirror for commit-addressed recipe/patch inputs, always at the same required
  byte count and SHA-256. A real mirror request matches the reviewed patch's
  original bytes. Digest/notice mismatches still fail without fallback. This is
  a source availability repair, not a rerun claimed as successful.

- Source 7efb280's complete VM experiment is green in
  [43464154294](https://github.com/cubepals/pryx/actions/runs/43464154294/job/132018658215).
  Verify artifact 456a962d8038a0600a2864d80f835f06494efff09f28a2173bc003eeceade8c5
  before retaining the real relocation report, fifteen-file component manifest,
  full thirty-boot/disk-reboot report and provenance. The boot report identifies
  filesystem manifest 347d7807cd333961f446951e642872e0e49dfdcd135b542795acd063f5557fa5.
  Both network cohorts and the separate missing-prerequisite guest also pass.
  The retained acquisition log proves actual primary-source 403s followed by
  identical-digest official mirror success; no bytes or assertions change.
- Add optional agent vm-vsock parser metadata and host-CID-only transport to
  guest-loopback main/spare listeners, with existing authentication and port
  ownership intact. Ordinary host processes refuse before binding; incompatible
  process-tier/explicit-host binding combinations refuse. Ingress listener failure
  terminates the supervisor rather than silently leaving an unreachable guest.
  This is preparation for the VM backend, not VM capability advertisement.
- Local full Rust verification runs without fail-fast and with the repository's
  checksum-pinned real agentOS assets. All executables except the existing
  dual-stack peer library test pass, including actual Wasm lifecycle; that test
  fails because this executor's AF_INET6 socket creation returns the real kernel
  EAFNOSUPPORT. Keep the test unchanged for CI. The first unconfigured attempt
  also correctly fails missing Wasm assets; stage them using CONTRIBUTING.md's
  existing fetch script before the complete rerun. Formatting and full Clippy
  with warnings denied pass. Real guest ingress proof remains to be added with
  the production init/backend; no local KVM or engine test is claimed.

- Prepare production OCI inputs with pinned oci-client 0.18.0 and its actual
  owned manifest/configuration/media/digest types. Stream every blob through the
  client's digest verifier to EOF, check declared size and uncompressed rootfs
  diff_id, then convert the tar directly with unchanged mkfs.erofs's aufs-to-
  overlay metadata handling. No image path is extracted onto the host.
- Verify the complete filesystem tool component through the existing descriptor-
  relative no-follow tree walk before execution, refusing links, extra/missing
  members, wrong modes or changed bytes. Publish synced read-only EROFS files and
  atomic cache records under layer digest plus builder identity. Corrupt complete
  cache data yields its own typed runtime refusal and is never silently repaired.
  Bound config/layer expansion, count, diagnostics and acquisition deadlines;
  cancel an active external builder through the process library's kill-on-drop.
- The ECR fixture mirror returns its real registry data-limit error, and the
  Google mirror lacks these fixture tags. Resolve the actual upstream Docker Hub
  manifest digests and retain immutable Alpine/Debian references in image-inputs.json.
  A real local acceptance successfully prepares both, reuses unchanged cached
  outputs and refuses deliberately corrupted cache data. Source images remain
  archives on the host. HTTPS certificate checks remain enabled; external blob
  addresses require credential-free HTTPS, and bounded retries match the OCI
  library's transient error variants/statuses, never Docker/error text.
- Extend only the existing optional VM job to run that real Rust integration
  acceptance, independent of an earlier alternative-network failure once the
  actual filesystem component succeeds. Ordinary PR cost remains zero; estimate
  three additional optional-job minutes on a cold Rust cache. The normal Rust
  test suite retains this explicitly CI-input-backed acceptance as ignored, and
  the optional job explicitly executes it. No existing test or gate is skipped.
- Local full Rust verification with real pinned Wasm assets again runs without
  fail-fast: every executable except the unchanged AF_INET6-dependent library
  test passes. The executor still returns EAFNOSUPPORT for actual IPv6 socket
  creation. No local Docker/KVM proof is claimed. Product VM init, bundle
  acquisition, runtime confinement and lifecycle remain incomplete and unadvertised.
- Source 8b2279f's complete normal push 43458373493 and PR 43458389388 are both
  green; source f2d6b83's optional VM experiment 43465395072 is green. These are
  earlier implementation checkpoints and do not pre-prove the new OCI code.
- Retain the explicit superseded-run cancellation inventories under k25
  measurements: inspect every completed job for failure before cancelling an
  incomplete older run, preserve all failed/green experiment evidence, and keep
  the active implementation and the earlier full application baseline running.
  Cancellation is not counted as a successful check or milestone proof.

- Source 8a9bc80's actual OCI acceptance and complete optional VM experiment
  [43469169462](https://github.com/cubepals/pryx/actions/runs/43469169462) are green.
  Both normal runs retain Structure failures in jobs 132037029037 and
  132038401397: enabling oci-client's required tracing/log feature expands eleven
  unchanged daemon functions beyond the existing cognitive-complexity threshold.
  Preserve the gate and baseline. Move the unchanged OCI implementation and its
  actual acceptance to independent vm-image/; the root workspace excludes that
  helper so its registry dependency cannot change daemon features. Reuse the
  node's verified filesystem component and atomic file publication. The optional
  job builds/tests the helper with its own lock, from the same release source.
- Source 8b2279f's complete Any-host matrix 43458372783 is now green, all sixteen
  actual jobs including signed installed SDK/CLI acceptance and all host cells.
  Its normal push and PR are also green as recorded above. This is the retained
  application baseline, not production VM backend or init proof.

- Source 6cb29c9's independent OCI workspace acceptance and complete optional VM
  experiment [43473962412](https://github.com/cubepals/pryx/actions/runs/43473962412)
  are green. Normal CI retains the doc_lazy_continuation/Parts-list failure in
  jobs 132052890583 and 132052890494: move the VM module's explanatory prose before
  its Parts list. Both rootless-engine runs and CodeQL on that source are green;
  normal CI and the host matrix were still running at this checkpoint.
- Build trusted VM init into the actual agent, guarded by PID 1, root and a
  read-only bootstrap marker before any mount. Receive bounded bootstrap data
  only from the host CID, mount EROFS/overlay/ext4 through checked guest paths,
  run the authenticated agent without root in a root-owned pids-limited cgroup,
  and bind ingress only to the explicit guest CID. Networking remains an explicit
  refusal in this checkpoint; no VM isolation or backend is advertised.
- Init kills and drains its command cgroup, checks syncfs on a disk descriptor
  opened before workload writes, then signs a boot/sandbox/epoch-scoped shutdown
  observation with the existing agent key before powering off. Actual-signature
  tamper/replay checks and the host-side init refusal pass locally. The host VM
  runtime does not yet consume this proof; real guest boot and shutdown still
  require CI acceptance before any production guarantee.
- Locally build the actual static PIE agent with the pinned Rust musl target and
  unchanged host GCC against an originally signature-verified Alpine musl-dev
  1.2.6-r3 private sysroot. The checked build script retains complete musl sources,
  patches and notices first; no host installation or libc rewrite occurs. Build
  the trusted EROFS bootstrap with the unchanged verified filesystem component,
  using the agent's own advertised marker/mode. Both actual build steps pass here.
  Retain a full pinned Linux 6.18.56 source input for the selected published kernel;
  the final bundle also needs Kata's exact recipes/patches, not yet assembled.
- Add a bounded private-stdin HTTP fixture through the pinned official SDK's vsock
  dialer, preserving token privacy and disabling redirects/HTTP replay. Build/vet
  the Go commands and retain the complete 24-module primary-notice closure locally;
  extend only the existing optional job's command build, notice inventory,
  static agent build and trusted bootstrap construction.
  This checkpoint adds no PR job or required gate; ordinary PR cost remains zero,
  with an estimated one additional cold optional-job minute for the static build.
- The local full Rust run with actual agentOS assets again executes every test
  binary without fail-fast. The existing IPv6 socket test still receives real
  EAFNOSUPPORT on this executor; other suites pass. No test is skipped or loosened.
  Docker/KVM/distribution and the new guest-init lifecycle proof did not run here.
  Continue with real-KVM agent acceptance, then the complete VM bundle/backend,
  confinement, lifecycle and required timings before K26. Nothing is parked. Raw output belongs to the cited CI artifacts; the repository keeps only its summary.

- Correct the unadvertised init checkpoint's disk discovery before real boot:
  Firecracker's API drive ID is not its kernel-visible serial. Use the pinned
  monitor's actual backing-file identity and guest-reported truncation, refusing
  missing, duplicate or ambiguous matches. The mutable disk selects the SDK's
  Writeback cache mode so a successful guest flush reaches monitor fsync. Leave
  the original minimal/network fixture configuration and cohorts unchanged.
- Add explicit real-KVM Cargo acceptance to the existing optional VM job: boot
  the actual static agent on both pinned OCI images, exercise authenticated
  process/file APIs, check non-root guest identity and a different kernel, deny
  host-file access and a host-process signal, send one typed monitor shutdown,
  verify the boot-scoped signed clean-disk proof and read-only e2fsck, then boot
  the same ext4 disk again and verify persistent data. Retain failed serial logs,
  non-secret monitor configurations and the actual report as evidence. Secrets
  stay in memory and bounded private stdin; no exec/write/stop HTTP retry occurs.
  This acceptance compiles and passes strict helper Clippy locally; it did not
  execute here because this machine has no KVM. CI proof is pending.
- The optional job gains an estimated additional one minute for four actual
  agent boots and shutdown checks, with zero ordinary PR minutes and no new gate
  or test system. Production bundle acquisition, confinement, network/backend,
  node lifecycle and thirty-sample production measurements remain required. Raw output belongs to the cited CI artifacts; the repository keeps only its summary.

- Source 9496dbf's complete optional VM job
  [43480235917](https://github.com/cubepals/pryx/actions/runs/43480235917)
  is green. Its original artifact 14064419038 is verified against API SHA-256
  787532060b4f951ce295e7f4eb748033930ba5882adfcf7ed943773153c95964;
  retain actual bootstrap/image-preparation reports and provenance under k25
  measurements. This proves static construction plus the earlier input experiments,
  not the new guest init boot. Source 2c25bdc's real-agent acceptance is running.
- Retain checked superseded bootstrap-run cancellations in
  superseded-bootstrap-runs.json. Read both normal runs' failed logs and preserve
  the exact Parts-list/doc-list cause, fixed in 9496dbf; inspect every other
  completed job before cancellation. Preserve the current implementation runs,
  9496dbf's complete optional proof and the earlier full application baseline.
  Interrupted runs are not passes.
- Add a trusted static guest adapter around maintained netlink and DHCP APIs,
  with no image-owned DHCP executable or shell and no packet/framing code.
  Its original gvisor forwarder uses preexisting-TAP mode; the adapter applies
  and renews actual leases and uses private-pipe EOF for readiness. Expand the
  reviewed network closure to the two client-only MIT modules, native 1.1.0 and
  packet 1.1.2; verify all 24 complete primary notices and module archive sums.
  The adapter builds/vets and refuses this ordinary host before any netlink call.
- Include the adapter in the trusted bootstrap under the actual agent's advertised
  filename and use it to bring guest loopback up before the agent, even with no
  network device. The vsock relay needs loopback; Linux does not perform init's
  interface setup. Outbound guest networking remains explicitly refused until
  readiness, resolver mounts, monitoring and policy are wired and proved. This
  adds no PR gate or job and an estimated under ten seconds to the optional build.

- Source 2c25bdc's first actual-agent KVM boot
  [43481338649](https://github.com/cubepals/pryx/actions/runs/43481338649),
  job 132078295325, fails its unchanged readiness deadline: the guest reaches
  trusted PID 1 but correctly refuses the outgoing socket's wildcard local CID.
  Preserve its complete serial log and artifact provenance (14065447643,
  SHA-256 bcf751902ef6bf39065ed5bb6d360a8caf30967231d5a1bf5892fa456ddb442e).
  Import already reviewed vsock 0.5.5 directly and call get_local_cid for actual
  device identity, preserving strict host-peer and bootstrap-CID equality. No
  wildcard acceptance, deadline change or claim that a boot passed is made.
- Source 9496dbf's normal PR contract job 132074668098 retains a 180-second archive
  adoption timeout followed by endpoint-reset failures. The remaining earlier
  lifecycle tests pass; the exact cause is not established by its bounded logs,
  which print only the upgrade fixture log. This is a required failure to
  investigate, not a reason to rerun or relax that test. The paired normal push's
  contract job succeeds on the same implementation source; record both outcomes.
- Extend the existing failure diagnostics with bounded, no-follow retained-fixture
  file metadata to identify its missing main capture; never read configuration,
  credentials or workload data. Existing log reads now open regular files without
  following links and read only their bounded tail. Actual temporary-tree controls
  verify nested metadata, refusal to expose an outside symlink target, and bounded
  output from a large log. Keep the contract deadline and test unchanged; the
  unexplained archive-adoption failure still needs diagnosis if it recurs.

- Source 5d64d60's actual KVM job 132086766078 in run 43483785231 now boots
  the real non-root agent, mounts the actual OCI lowers/overlay/ext4 and passes
  its authenticated file round trip. The fixture then receives the correct
  UNPROCESSABLE_ENTITY refusal for its exec POST without the existing mandatory
  idempotency key. Retain its serial log and artifact 14067315957 (SHA-256
  81d8f3a6625c6574865bf9cdf94031b09742a64134e8bd78e9d1082a2daadddf).
  Export the agent-owned header constant and submit each fixture request with
  a fresh key, once; do not weaken the agent check or any acceptance assertion.
- Wire host-owned guest network setup into init: bind its root-owned tmpfs
  resolver through no-follow handles, run only trusted static bootstrap programs,
  wait for private readiness EOF, monitor helper failure and stop it before disk
  flush. Unexpected helper death withholds the clean-disk proof. Leave absent
  networking with loopback alone, no outbound helper and no VMM network device.
  Keep the earlier private raw-argument checkpoint field explicitly refused.
- Extend the same actual-agent KVM acceptance with a third boot of each retained
  disk using the selected vsock network, the maintained DHCP client and original
  forwarder. A static unprivileged fixture checks actual HTTP, real public DNS,
  gateway management denial and direct management denial without image-owned
  tools. Plain boots assert that the guest has loopback alone. This fixture
  network is not production confinement or hostname-egress-policy proof.
  The existing optional job retains its original network cohorts and all earlier
  assertions; estimated extra optional cost is under one minute, zero PR minutes.
- Current local full Rust verification still completes every binary with only the
  executor's unchanged AF_INET6/EAFNOSUPPORT failure. Strict root Clippy passes;
  current helper Clippy, Go build/vet/notices and static-bootstrap construction
  are checked before this step is pushed. No local KVM/Docker proof is claimed.

- K25 housekeeping: replace committed raw measurements with compact source/run/aggregate
  summaries, retaining the two process-tool build inputs unchanged. Raw output belongs only
  in CI artifacts. Record immediate scanner handling and ordered per-milestone landing as
  the final plan step; no pull request is opened. Local JSON/build-input and reference checks
  pass; this documentation cleanup does not establish new VM acceptance.

- K25 scanner handling: read all seventeen PR 21 findings with unauthenticated curl.
  Replace fixed process/VM observation challenges with OS-backed random values, and derive
  invalid/oversized API cases from a fresh challenge. Keep all replay, forgery, scope and
  authentication assertions, with explicit input-validity checks. Strict all-target Clippy
  and fmt pass. Complete local Rust execution still encounters the existing IPv6 socket
  EAFNOSUPPORT failure; all other test binaries, including actual Wasm, finish successfully.
  Real engine/KVM acceptance remains CI-only; hosted results and scanner reanalysis are pending.

- K25 independent-image helper is executable through a bounded private pipe contract, using
  the OCI-owned image-configuration type. Preserve typed cache-corruption/timeout refusals
  and serialize cache publication with a kernel lock across processes. Actual local Alpine
  and Debian registry acceptance passes, including real subprocess lock contention, timeout,
  cache reuse and corruption refusal (15.45 s). Helper and root strict all-target Clippy and
  fmt pass. Complete local Rust again finishes with only the unchanged IPv6 EAFNOSUPPORT
  failure; no engine/KVM check runs locally. The existing optional registry CI test now
  exercises the real executable too, estimated under three added seconds and no PR minutes.
- Source d41c32d real KVM run 43486064853, job 132094719558, fails its original 15-second
  power-off deadline after reaching the real agent and earlier guest assertions. Download
  artifact 14068097020 and verify SHA-256 ca78db57e0563d78864874f2cdeae1a61c7c5d9cab2c9e1d208191d0c1df4bde.
  Its serial output proves the monitor accepts SendCtrlAltDel; its published kernel config
  disables INPUT_KEYBOARD and SERIO, so no keyboard event reaches init. Summary:
  checks/measurements/k25/agent-shutdown-d41c32d.json; raw output stays in CI artifacts.
  Add authenticated boot-scoped trusted-init shutdown without changing the kernel, deadline,
  existing monitor-action check or clean-disk assertions. Production VM remains incomplete.

- K25 guest shutdown now has a host-CID-only trusted-init listener. Verify a distinct signed
  boot/sandbox/epoch request with the existing agent key, bound input size and read deadline,
  and never resubmit a received shutdown. The maintained SDK owns the host vsock connection;
  no CONNECT framing is copied. Actual KVM acceptance first rejects another boot’s signed
  request, proves the agent remains healthy, retains the original monitor action, then sends
  the valid request once. Its acknowledgement and monitor exit share the original fifteen
  seconds; clean-disk proof, read-only fsck and all reboot/network assertions stay required.
- Local current-source full Rust finishes with 494 passed, one unchanged IPv6 EAFNOSUPPORT
  failure and 33 existing ignored cases. Both strict root/helper all-target Clippy, fmt, Go
  build/vet, complete 24-module notice closure, static-musl agent and actual EROFS bootstrap
  construction pass. No local KVM or engine execution is claimed. Existing optional CI cost
  changes by under one second for rejection/acknowledgement; no new job or gate is added.
  Read the same shutdown failures in fea728a run 43486862080/job 132097499485 and b33d097 run
  43487324558/job 132099099215; both precede this correction and neither is counted as a pass.

- K25 corresponding-source collection now retains the original checksum-pinned Linux
  archive, every primary Linux licence text, and complete commit-verified Kata packaging
  and Firecracker source archives. Git verifies the fixed source trees without checking
  them out or executing their code; no kernel is built. Python compilation, the real
  pinned Linux COPYING extraction and actionlint pass locally. Upstream GitHub source
  repositories are inaccessible from this machine; actual source-tree acquisition is
  assigned to the existing optional VM job and not claimed locally. Estimated additional
  optional cost is one minute including artifact retention, zero ordinary PR minutes.
  Runtime assembly, signed lazy acquisition and production backend remain required.

- K25 bundle checkpoint: assemble static same-source host helpers, production-only trusted
  bootstrap, pinned monitor/jailer/kernel, and verified read-only filesystem tools using
  the existing artifact packager. Separate original Linux/Kata/filesystem/Rust sources;
  retain primary notices and lock-authenticate the actual 452-package musl graph. Actual
  local source/notice collection and static-musl OCI helper build pass. Complete bundle
  assembly still needs upstream monitor/kernel/source inputs available only in CI here.
- Reuse the existing bounded HTTPS artifact cache for lazy VM acquisition, then verify
  every member, byte digest, mode and required program with the descriptor-relative tree
  walker. Source descriptors belong to the signed host installation, never requests. The
  existing optional job now tests the actual assembled archive, wrong-digest refusal,
  offline reuse and byte/mode/extra/missing/link corruption. It has not executed locally.
  Full local Rust again has 494 passes and only the unchanged IPv6 EAFNOSUPPORT failure;
  root/helper strict Clippy, fmt, Python compilation and actionlint pass. Additional optional
  cost is estimated four minutes for static helper build, packaging and archive acceptance,
  with zero ordinary PR minutes and no new job or gate. Release wiring, production VM
  confinement/backend/lifecycle and measurements remain incomplete and unadvertised.
- Inspected all completed jobs before cancelling twenty-two superseded unfinished workflows
  on d41c32d/fea728a/b33d097/ac69b1e. None of those workflows had a completed failed job.
  Keep their already completed failed VM runs, current 48dc958/3cd50fa implementation runs
  and the earlier green application baseline. Interrupted work is never a pass.
- Source 48dc958 real KVM run 43489144084/job 132105365697 rejects the replay and retains
  agent health, then its valid signed shutdown receives EOF without an acknowledgement.
  Preserve artifact 14068977689, verified SHA-256 afc2b68a4bedc791b555545889c31401f8cbb66910b5697a6c9f3777f3d23b67,
  and its serial output in CI: no clean-disk proof is present. Investigate request EOF/
  half-close handling before claiming orderly shutdown; no deadline or assertion is loosened.

- K25 shutdown framing correction: send one bounded JSON line rather than waiting for a
  host half-close before replying. A real Unix-stream pair proves a valid signed request
  receives its acknowledgement while the sender’s write side stays open. Host-CID, size,
  signature, boot/epoch checks and the shared fifteen-second shutdown deadline remain.
  Full local Rust has 495 passes and only the existing IPv6 EAFNOSUPPORT failure; strict
  root/helper Clippy, fmt, Go build/vet and static agent/bootstrap construction pass. Actual
  KVM proof is pending; no shutdown or reboot success is inferred from local framing checks.
- Immediate scanner check finds PR comment 4567580276 on the real-guest fixture: replace
  timestamp/prefix-derived boot and replay nonces with two independently generated UUIDs,
  preserving the explicit inequality and all rejection/assertion paths. The already linked
  UUID version is a direct helper dev dependency; its lock changes only that dependency edge.
  Source 3cd50fa’s run 43489699301/job 132107275312 retains the same pre-correction EOF
  shutdown failure; its new complete-source collection succeeds. Both rootless-engine runs
  and CodeQL are green on 3cd50fa; full normal and host acceptance remain pending.
- Reclaim unused root-package build variants with cargo clean after their checks complete:
  target output shrinks from 11 GiB to 5.9 GiB and free workspace disk rises to 9.1 GiB.
  Keep active inputs and current helper executables; no source or user data is removed.

- K25 monitor launch checkpoint: exact Landlock read/write grants and the existing process
  syscall policy surround the ordinary-user monitor; extra rules deny host signals, limit/
  scheduler changes and non-Unix sockets. Root must invoke the unchanged jailer with fixed
  non-root IDs. Preserve its own seccomp filter. A kernel lease lives on stdin because the
  pinned jailer closes every inherited descriptor above stdio. A second filter prevents
  closing, overwriting, unlocking or setting close-on-exec on that lease; its path is outside
  all writable grants. Parent identity is checked before launch. No VM tier is advertised.
- Actual local host-process acceptance passes file/signal/network/lease-release denials,
  permitted private writes, competing-launch refusal and lease release only after process
  exit (0.04 s). This is not a substitute for Firecracker/jailer/KVM proof. Full local Rust
  has 496 passes, the unchanged IPv6 EAFNOSUPPORT failure and 33 existing ignored cases;
  strict root/helper Clippy, fmt, Python compilation, actionlint and static-musl agent build
  pass. Existing KVM acceptance now requires this launcher and tests its lease around each
  boot/exit. Estimated cost is under one added PR second and under one optional-job second.
  The root jailer launch path is implemented but has not executed here or been proved in CI.
- Read both new VM failures: cabbc97 run 43491406115/job 132113238299 and 6df0993 run
  43492477562/job 132116968546. Their bundle step cannot find the independent workspace’s
  static helper: set the build step’s CARGO_TARGET_DIR explicitly to the shared root target.
  Do not relax static verification. In 6df0993 the valid shutdown acknowledgement succeeds,
  but the guest kernel halts instead of exiting the monitor: its retained serial says power
  off is unavailable (PM is disabled). Artifact 14070876767 has verified SHA-256
  f6fc50da71163a803dd28a90ca15be26d0df338ab7d72da2f26b0a5c63b9eb3c. Its proof line is
  also interrupted by the kernel console. Next drain the signed observation and use the
  same reboot path already proved by the minimal kernel spike, preserving the disk checks
  and fifteen-second deadline. Neither failed run establishes guest lifecycle or bundle proof.

- K25 guest reset correction: retain data/disk descriptors before any workload can rename
  ancestors. After emptying the command cgroup and stopping trusted networking, remount
  the overlay, bound data and ext4 disk read-only, then sign the clean observation. Drain
  the actual console with the existing nix terminal API before reboot=k resets the monitor,
  matching the already-green kernel spike. Do not change the published kernel, original
  fifteen-second deadline, read-only fsck, signature, secrecy, replay or reboot assertions.
  Full local Rust remains 496 passes with the unchanged IPv6 EAFNOSUPPORT failure; strict
  root/helper Clippy, fmt and static-musl agent build pass. Real remount/drain/reset and disk
  integrity are still pending CI proof; no local KVM or engine result is claimed.

- K25 measurement cleanup follow-up: older raw logs and full reports also existed directly
  under docs/checks. Replace them with source/run/aggregate summaries, correct every renamed
  link (including historical K14 relative paths), and describe licence links as review
  summaries rather than full inventories. Primary notices and source inputs remain with
  builds/distributions. The edge collector now writes raw observations to temporary storage
  or its configured artifact path. Preserve the two required process-tool reviews unchanged.
- Read the six retained normal-CI failures found before superseded cancellation: control jobs
  132122048291, 132119391080, 132119587489, 132119095462, 132111003598 and 132110900206 all
  reject generated docs/api.md drift. The initial cleanup appended a raw-output note to
  generated API prose; regenerate with its owner and keep the gate unchanged. JSON, links,
  Python compilation, whitespace and generated-doc checks now pass. The first full local
  Bun check used host Bun 1.4.3 and failed its unsupported ws stream API; switching to the
  repository-pinned 1.4.4 fixes the environment. Complete pinned Bun check passes with real
  local Postgres: 437 pass, one existing compiled-assets skip and 8,415 assertions. No test
  or assertion is disabled. The superseded cancellation audit retains all completed failures.

- Source ccd8221’s complete optional VM run
  [43497519965](https://github.com/cubepals/pryx/actions/runs/43497519965), job 132134408877,
  is green, with every step successful. Verify artifact 14073581063 against SHA-256
  861fbdb1396b0ddcce116ec3008f8b9d0fe3719ca1a5829edb883870f202f6fb.
  All six actual Alpine/Debian boots pass confined monitor/lease checks, authenticated
  process/file APIs, host isolation, replay refusal, signed clean shutdown, read-only
  fsck and persistent disk reboots; both networked boots pass real DHCP/DNS/HTTP and both
  management denials. Separate bundle acquisition/offline/corruption acceptance passes
  in 6.34 s. Runtime archive is 71,233,881 bytes (201,926,123 expanded, 2,391 entries);
  corresponding sources are 291,441,041 bytes (551 entries). Short summaries are
  checks/measurements/k25/agent-ccd8221.json and bundle-ccd8221.json. Raw evidence remains
  in that CI artifact. This proves these inputs/fixtures on ccd8221, not the latest head’s
  normal/host workflows, production VM backend, root jailer/firewall or required timings.
  Continue production port forwarding and daemon lifecycle; K25 remains in progress.


- K25 stream forwarding checkpoint: map application TCP ports above the complete TCP port
  range in vsock, separate from init's private control channels. Preserve older bootstrap
  defaults; the advertised private agent capability gives the recommended service base.
  The maintained SDK dialer owns connection framing, while a separate bounded-concurrency
  helper carries opaque TCP with backpressure, half-close and parent-bound lifetime. It
  accepts only private/loopback host bind addresses and never retries application bytes.
- Extend the existing six real-guest cases with a 16 MiB authenticated binary file round
  trip and a declared-port service returning exact opaque bytes after client write EOF.
  The disposable echo program stays out of production bundles. These new assertions await
  actual KVM CI; existing shutdown, filesystem, isolation and replay assertions stay intact.
  Full local Rust has 496 passes, the unchanged IPv6 socket failure and 33 ignored cases;
  strict root/helper Clippy, fmt, Go build/vet and static-musl agent/bootstrap builds pass.
  No new CI job or gate; estimated optional KVM cost is under ten seconds, no added PR minute.
- Latest committed source 5fde98c has green VM, both rootless engines, CodeQL, core Rust,
  control workspaces and contract checks. Normal end-to-end/one-box and host bundle cases
  are still running or queued. PR 21 still has the same eighteen addressed scanner comments.


- K25 portable-launch checkpoint: include the same static agent as bin/vm-launcher in the
  separate VM distribution. The whole-bundle verifier requires it, ties its digest to the
  trusted guest init, verifies the bootstrap image digest and rejects incompatible init,
  monitor or service-port versions. Production bootstraps still exclude disposable probes.
  The generated monitor model now takes explicit physical-memory and vCPU inputs, keeping
  existing spike defaults. Placement and a production runtime remain unannounced.
- Local bundle acquisition/offline/corruption acceptance passes in 6.44 s using verified CI
  component bytes plus the current rebuilt launcher/bootstrap/transport, repackaged locally;
  this is neither a release artifact nor real-KVM proof. The first extraction changed readonly
  helper modes; restore their recorded modes before rebuilding, without loosening verification.
  Actual launcher capability execution and generated configuration/owner validation pass.
  Full local Rust remains 496 passes, one unchanged IPv6 failure and 33 ignored cases; strict
  root/helper Clippy, fmt, Python compilation, Go build/vet and actionlint pass. Existing optional
  CI assembles and verifies the complete same-source distribution; no added job/gate or PR minute.
- Read source 858e068 run 43496424066/job 132130602591: its retained guest shutdown timeout
  predates the already-proved ccd8221 reset/drain correction. Also read its control failure
  132134747929 and 132134847610: generated API doc drift
  predates 2969a46's regeneration. Do not count either old failure as a current-source pass.


- K25 structure preparation: move the existing reflink/copy implementation from tree.rs
  into tree/copy.rs without changing its behavior. The parent lists the new concern and
  reexports the same functions. This separate move precedes sparse VM-disk work. Full local
  Rust keeps 496 passes, the same single IPv6 failure and 33 ignored cases; fmt passes.
- Read acd52ed run 43506064338/job 132163924616: whole-bundle acceptance and the new 16 MiB
  file round trip pass, but opaque TCP after client EOF receives no bytes. The pinned monitor
  source explicitly turns host EOF into shutdown of both directions. Preserve the assertion
  and fix the stream transport through maintained HTTP/2 CONNECT, not custom framing.


- K25 TCP half-close correction: keep the underlying SDK-owned vsock connection open and
  relay application streams through maintained HTTP/2 CONNECT. Hyper/h2 owns guest framing,
  flow control and stream shutdown; the pinned Go standard library owns host framing on a
  single ClientConn, without application-byte retries. A separate advertised private stream
  port range/version preserves the earlier raw service transport. Host-CID, fixed TCP target
  and private bind restrictions stay. Bundle verification requires the stream capability.
- A real local Unix-stream HTTP/2 round trip passes a 64 KiB opaque payload and the reply
  generated only after write EOF. This proves library stream semantics, not Firecracker or
  the Go-to-Rust KVM path. Preserve the exact real-KVM byte assertions and all prior isolation,
  auth, replay, disk and shutdown checks. Full local Rust: 497 passes, the existing IPv6 failure,
  33 ignored cases; strict root/helper Clippy, fmt, Go build/vet, all 24 primary module notices
  and static agent/bootstrap builds pass. No dependency package or version changes: lockfiles
  add existing HTTP-library edges only. No new workflow/job/gate or added PR minute.
- Also read fcce6bc run 43506802018's completed VM failure: whole same-source launcher bundle
  assembles and verifies, then the same pre-correction opaque-EOF assertion fails. Raw evidence
  remains in CI; forwarding-acd52ed.json records the decisive preceding measurement only.


- K25 sparse-disk checkpoint: after reflink refusal, preserve the kernel's SEEK_DATA/SEEK_HOLE
  extents in the existing descriptor-relative copy path, with dense copying only where extent
  lookup is unsupported. Source-size changes fail instead of completing a truncated copy.
  Keep apparent-byte bounds, the original disk-floor refusal and all existing walk assertions.
  The new direct descriptor test bypasses reflink, checks every byte and real allocation, and
  caught a data/hole-boundary gap in progress checks; cap each extent chunk at the remaining
  progress threshold. No assertion was removed or relaxed.
- Full local Rust now has 498 passes, the same IPv6 failure and 33 ignored cases; strict
  Clippy and fmt pass. Existing CI runs the new real-filesystem test; no new job/gate or PR
  minute. Reclaim obsolete project build variants after this checkpoint before disk work.
- Cancel superseded pending/running workflows only after reading every completed failure.
  Retain raw failed jobs and the cancellation inventory outside the repository; cancellations
  are not passes. Preserve 2969a46's full normal/host baseline and the latest HTTP/2 runs.
  Additional old ccd8221 control job 132137784482 has the known pre-regeneration docs drift.


- Source 28e2ec4's full optional VM run
  [43508458035](https://github.com/cubepals/pryx/actions/runs/43508458035), job 132172138729,
  is green. All six Alpine/Debian KVM boots pass the unchanged exact HTTP/2 forwarding
  assertions: 16 MiB binary file round trips and 64 KiB opaque payloads with replies after
  client write EOF. All previous confinement/lease, authenticated process/file, host isolation,
  signed shutdown/replay, read-only fsck and persistent-reboot assertions pass; both networked
  boots still pass DHCP/DNS/HTTP and management denials. Whole same-source launcher bundle
  acquisition/offline/corruption verification passes too. Summary: forwarding-28e2ec4.json;
  raw evidence stays in that CI artifact. No production backend/edge/root/firewall/timing
  success is inferred. PR 21 still has the same eighteen addressed scanner comments.


- K25 writable-disk checkpoint: a host-owned preparation transaction holds the same kernel
  lease as the monitor before touching data. New ext4 images are made beside current data,
  verified read-only, fsynced and atomically published without replacing an existing name.
  Existing valid disks keep their inode and bytes. Recovery uses the existing no-follow,
  sparse copy and safe automatic preen on the staged image, followed by an independent
  read-only check and atomic replacement. Size changes, unknown members, links, active
  monitors, space-floor failures and unsafe recovery are refused. Cancellation leaves the
  owned transaction running to completion; deadlines remain inside its bounded stages.
- Actual local component/bundle acceptance passes in 6.49 s: 64 MiB ext4 create/reuse,
  unchanged hashes, active lease/size/floor/link refusals, whole original-disk preservation
  after failed recovery, and successful staged repair of e2fsprogs' own damaged-bitmap fixture.
  The pinned source archive is hashed against its owning input before the bounded fixture
  is read. No upstream code/data is copied into the repository or linked into pryxd. The
  test-only tar dependency is an existing pinned package; the lock adds one dependency edge.
  Full local Rust remains 498 passes, the same IPv6 failure and 33 ignored cases; strict
  root/helper Clippy, fmt and actionlint pass. Existing optional bundle CI now runs these
  actual filesystem assertions; no new gate/job, under one optional second and no PR minute.
  This is still preparation only: production backend lifecycle, root jailer/helper confinement,
  edge/data API integration and required production measurements are not yet proved.


- Normal CI caught the tree split's parent-to-child copy callback as a module cycle:
  28e2ec4 jobs 132174819628/132174988142 fail Structure; control jobs 132174819638/
  132174988278 pass their 438 tests but fail the same Structure gate. Read all four logs.
  Move the remaining copy method into tree/copy.rs, leaving its behavior and assertions
  unchanged. The parent now only reexports the copy API; no baseline or check is relaxed.
  Full local Rust and strict root/helper Clippy are rerun before the fix is pushed.
- Preserved source 2969a46 has every normal-CI job green on push run 43499581589 and
  PR run 43499595059, including Rust/structure, control, real end-to-end, contract, one-box
  and the PR release image builds. Host run 43499581572's real booted Fedora/Debian/Ubuntu
  and several Docker/bootstrap cases pass; its remaining matrix is still active. These
  results establish that earlier doc regeneration, not the latest head's full acceptance.


- K25 production monitor component: one owned Machine launches the verified static agent's
  monitor mode with the host confinement plan on private stdin and boot secrets on a bounded,
  one-shot Unix socket. It clears host environment, creates private serial output exclusively,
  owns the child identity, cancels bootstrap listeners and retains kill-on-drop. Readiness
  requires completed bootstrap delivery and a live monitor; timeout cancels and joins the
  sender rather than leaving a detached secret listener. Process drop never certifies a clean
  disk; the actual kernel lease remains authoritative until monitor exit.
- Existing six-boot KVM acceptance now invokes that production component directly, retaining
  its exact health/bootstrap deadlines, lease probes, authenticated shutdown, fsck, stream,
  network and isolation assertions. Full local Rust keeps 498 passes, one unchanged IPv6
  failure and 33 ignored cases; strict root/helper Clippy and fmt pass. Real execution of this
  new host component awaits CI, not this no-KVM host. No new job/gate or measurable PR minute.
  Continue ContainerRuntime integration, helper/launcher journal adoption, node data/agent
  metadata forwarding, root jailer/firewall, release/installer wiring and production timings.


- K25 production transport checkpoint: bounded SDK control requests carry credentials only
  on private stdin, clear host environment, bound encoded/decoded responses and own helper
  cancellation. Each mutation is submitted once. A separate owned forwarding process verifies
  the exact count, private address and requested/ephemeral port of every readiness binding;
  its opaque HTTP/2 streams and parent lifetime remain in the maintained helper.
- The six-boot KVM fixture now uses both production adapters directly, keeping the existing
  35-second control-call, five-second forwarding, exact byte, shutdown and other assertions.
  Actual local Go-helper smoke proves a private listener is held until owned stop and its
  port is released afterward, without simulating a VM or SDK reply. Whole bundle plus real
  ext4/staged upstream-fixture recovery acceptance passes in 6.72 s. Full local Rust remains
  498 passes, one unchanged IPv6 failure and 33 ignored cases; strict root/helper Clippy and
  fmt pass. No new job/gate or added PR minute; real adapter/KVM proof awaits CI.


- K25 cold-boot runtime checkpoint: compose verified bundle acquisition, daemonless OCI,
  spec-sized ext4, generated physical-memory/vCPU configuration, non-root guest bootstrap,
  confined owned monitor and private TCP forwarder behind ContainerRuntime. Keep it OUTSIDE
  configured/automatic tier selection. Carry disk bytes and explicit CPU-weight intent as
  defaulted internal ContainerSpec fields; public protocol shapes and formats do not change.
- One private owner lock serializes runtime instances; stable sandbox leases fence monitor
  generations and startup refuses any predecessor still holding its disk. Cold starts and
  stops remain owned across request cancellation. Only safe health reads retry; each launch,
  bootstrap and signed shutdown is submitted once. A clean exit needs the scoped signed disk
  proof; forced retirement remains distinct. Dead-monitor cleanup and unsupported signal/log
  behavior still need completion before exposure. Root, public networking/hostname policy,
  fractional vCPUs, CPU weights, UDP, registry packages and memory sleep have named refusals.
- Add actual runtime-seam acceptance to the EXISTING optional VM job: both pinned OCI images,
  two-vCPU boot, authenticated exec, clean stop, cold wake, copied-disk fork independence and
  remove. This new test is ignored on hosts without KVM and explicitly run there. Estimated
  optional cost is up to two minutes including initial root test compilation; zero new PR
  jobs/gates and no ordinary PR minute. Real execution is pending, not inferred locally.
  Full local Rust: 498 passes, the unchanged IPv6 failure and 34 ignored cases; strict root/
  helper Clippy, fmt and actionlint pass. No tier availability is advertised by this checkpoint.
- Next finish backend failure/adoption cleanup, legacy logs/stats and SIGINT semantics;
  integrate node agent state and data APIs with stopped-disk access and snapshot/export
  barriers; implement root helper/jailer/TAP/firewall and ordinary-user network policy;
  then wire protected descriptors into configuration/release/install, prove node+edge
  lifecycle and thirty production samples. K25 main deliverable remains in progress.


- Read the retained follow-up Structure failures on 217029b, 9784438 and 5c56818:
  moving the callback alone did not remove the facade/copy import cycle. In its own move,
  put traversal, accounting and source descriptors in tree/walk.rs, let copy.rs import
  that leaf directly, and retain the public facade/reexports. Move the unchanged real-file
  assertions to tree/tests.rs. No cycle suppression, baseline expansion or assertion change.
  Local full Rust/strict root+helper Clippy/fmt pass apart from the known IPv6 test; actual
  Structure verification is still CI's. Keep K25 in progress until that gate passes.
- The latest component-only source 5c56818 VM run 43514863702/job 132194126718 is green,
  covering the production SDK request/forward adapters in all six real guest boots. Scanner
  remains at eighteen addressed comments. Runtime-seam source e701cb9 run 43519382502/job
  132209502842 fails before monitor execution because its tempfile launch directory is 0755;
  preserve the private-directory refusal and fix creation to request 0700 explicitly next.


- K25 launch/exit corrections: tempfile's directory defaults produced 0755 in CI. Request
  0700 in the builder before any bootstrap file/listener exists; a real local permission
  regression test now pins it. Preserve Machine's private-directory guard unchanged.
- Observe spontaneous monitor exits under the runtime transition lock, retire forwarding,
  record the signed guest result and distinguish missing proof with unknown exit status.
  A failed clean-stop proof still retires confirmed-dead compute and returns the error;
  it cannot leave a dead monitor permanently occupying the live map. Already-exited guests
  are not signalled again. Separate one-shot delivery from liveness so short jobs are accepted
  only as running agents or verified signed completions. Existing fixture bootstrapped
  still requires a live monitor; its readiness and shutdown assertions are not loosened.
- Extend real runtime CI with immediate exit-code-7 jobs on both OCI images and require their
  verified completion/retirement. Full local Rust is 499 passes, the existing IPv6 failure
  and 34 ignored cases; strict root/helper Clippy and fmt pass. CI must prove the boot-mode,
  lifecycle, quick-job and one-way Structure fixes before any runtime availability is wired.


- K25 source 71d9767 optional VM run 43522794421/job 132220983145 is green: both
  pinned OCI images complete eight actual runtime boots covering two vCPUs, durable exec,
  clean stop, cold wake, independent copied-disk fork and signed immediate exit code 7.
  Runtime acceptance takes 33.12 seconds. The unchanged six component boots and real
  bundle/ext4 recovery acceptance also pass. Short evidence: measurements/k25/runtime-71d9767.json.
  Normal CI/Structure remains queued; this is not public-node, edge or root-path proof.
- Bound monitor diagnostics by the host's existing log byte budget: continuously drain the
  real monitor socket and retain its tail in a private exclusive file, finishing capture
  before interpreting the signed exit proof. Child retirement stays separate from capture
  I/O, so a diagnostics error cannot prevent recording confirmed-dead compute. The KVM
  fixture rejects truncated diagnostics before its existing secrecy/proof assertions.
  Real-socket bounded-tail regression passes. Full local Rust: 500 passes, one unchanged
  IPv6 failure and 34 ignored; strict root/helper Clippy and fmt pass. Scanner still has
  eighteen addressed comments. Continue K25 backend/node integration while CI checks this head.


- K25 node metadata: agent health adds a typed snapshot of port revision/HTTP assignments
  and activity. Read it over the owned VM forwarder using Routes-scoped tokens, carry it
  only in memory through ContainerInfo, and let existing node views use it instead of host
  file reads. Stopped observations clear active streams. Existing backends retain their
  descriptor-relative checkpoint reads. No fleet/public protocol field is renamed or removed.
- Real local agent tests prove readiness polling adds no activity stream, returns current
  activity, and never includes raw SSH leases in HTTP routing metadata. Extend the existing
  real-KVM runtime test to require metadata through the forwarder and unchanged activity
  on another node poll. Full local Rust: 500 passes, the unchanged IPv6 failure and 34 ignored;
  strict Clippy/fmt pass. Real KVM execution of these additions awaits CI.
- Read the remaining e701cb9 normal-CI failures: both are the already corrected tree facade
  cycle; checksum refusals in their passing tests are expected assertions, not install failures.
  Cancel superseded queued/running sources after preserving completed failure logs outside
  the repository; retain 2969a46 evidence and the leaf-split/current normal runs. Scanner stays
  at eighteen addressed comments. Continue stopped-disk file access and snapshot/export barriers.


- K25 snapshot/export barriers now stop VM compute even when application writes are quiesced,
  stage recovery if required, independently verify ext4 and hold the actual monitor lifetime
  lease throughout the existing no-follow copy/archive walk. A live monitor refuses that
  lease; the real-KVM test now also requires it to fence a new boot until copying completes.
  Disk-image exports explicitly refuse guest-path exclusions rather than silently ignore them.
- Keep snapshot/export work and its workload lock owned across caller cancellation. Reserve
  the running guest's memory through the copy and cold resume; recheck execution lease,
  store lease and queued fences before starting it. On copy errors still attempt the one
  authorized resume; on resume refusal keep compute stopped and expose the issue. Real local
  HTTP-sink tests prove cancellation retains the lock/reservation and a fence prevents resume.
  Full local Rust: 502 passes, the unchanged IPv6 failure and 34 ignored; strict Clippy/fmt
  pass. Actual VM copy-lease assertions await the existing optional CI job.
- Source 3372788 VM run 43525889729/job 132231412140 is green, proving bounded capture in
  all eight runtime and six component boots. Summary: checks/measurements/k25/capture-3372788.json.
  Read source 861dd13 normal-CI job 132226131822: the tree import cycle is gone and Control
  workspaces passes, but Rust Structure flags transaction (118 lines) and launch_entry
  (242), with a 100-line lint limit. Next split those functions in their own Move change;
  retain the gate/baseline. Scanner remains at eighteen addressed findings. K25 is in progress.


- Move only: extract SDK monitor configuration and private guest bootstrap into instance/
  configuration.rs and guest.rs; extract readiness, confinement-plan assembly and atomic disk
  publication into focused functions. Preserve the original order, inputs, checks, deadlines
  and assertions. The daemon function-length probe now has no diagnostics; its agent dependency
  reports three existing length warnings outside that daemon gate. Normal strict workspace
  Clippy/fmt pass; full local Rust remains 502 passes, one known IPv6 failure and 34 ignored.
  CI must confirm the complete Structure gate. Continue stopped-VM node file APIs next.


- K25 trusted file-service preparation: add a parser-advertised guest mode exposing only the
  existing authenticated read/write/list/stat handlers and health. Refuse image layers, network,
  workload commands and spare forwarding. Keep service logs in guest tmpfs; do not start tools,
  entrypoints or the persistent telemetry/ports reaper. Actual local route acceptance streams
  more than the JSON request limit, checks traversal/authentication and command-route refusals,
  and preserves existing port metadata. Host stopped-disk/session integration is next; no public
  VM availability is announced by this checkpoint.
- Full local Rust ran every target: 503 passes, 34 ignored, the recorded IPv6 fixture failure
  and a Wasm immediate-listener readiness failure (running guest process, no observed listener
  within the unchanged deadline). Preserve that result; its isolated recheck is pending. Both
  strict Clippy modes, fmt and whitespace checks pass. Real file-only boot remains CI work.
- Source eabb37a VM run 43528855604/job 132241251645 is green: eight runtime boots in 38.47 s
  prove both images, live-monitor copy refusal, copy-held boot refusal, unchanged metadata poll
  activity, cold wake, independent forks and signed quick completion. Six component boots and
  bundle/actual-filesystem acceptance also pass. Short summary: checks/measurements/k25/
  copy-lease-eabb37a.json. Normal Structure/aggregate remains pending; scanner stays at eighteen
  addressed findings. Keep K25 in progress.
- Extend the continuation instruction through K28, K29 and K30, after K27 and parked-piece
  second attempts, merging origin/main when those briefs are reached. No pull request is opened.


- K25 stopped-disk runtime session: launch the verified layerless file guest under the same
  confinement and disk lease, with a private ephemeral relay and a newly generated signing key.
  Remove workload environment, commands and spare ingress. Hold the transition lock until owned
  shutdown validates its signed disk observation; dropping a caller schedules the same bounded
  cleanup and does not certify disk safety. No normal workload state is changed.
- Extend the existing optional KVM runtime test with a stopped-disk binary round trip larger
  than the JSON limit, command-route refusal, actual disk-copy lease refusal during service,
  clean service shutdown and ordinary cold-wake visibility. This adds two boots to that existing
  job, estimated under fifteen seconds, zero new gates/jobs or ordinary PR minutes. Real boot
  proof is pending. Local full Rust is 505 passes, the unchanged IPv6 fixture failure and
  34 ignored cases. Strict Clippy/fmt pass; the daemon length probe has no new length warnings.
  The earlier isolated Wasm listener recheck passes in 1.89 s; retain its full-suite failure.
- Read normal source eabb37a job 132241586356: Structure now passes. Its agent SSH lease test
  fails the final immediate rebind after dropping its listener; the test currently discards
  the OS error. All other Rust targets pass. Preserve this CI failure and investigate the
  shared ephemeral-port fixture independently before claiming the aggregate green. Scanner
  remains eighteen addressed comments; K25 and public node/edge integration remain in progress.


- K24 CI fixture correction kept in its own commit: the scoped SSH lease fixture now binds a
  fresh loopback address, separating its final immediate-rebind check from other localhost
  ephemeral allocation. Preserve every reservation, byte, stopped-connect and release assertion;
  retain the OS error if rebind fails. Local stress passes 64 invocations in eight concurrent
  processes. Full Rust remains 505 passes, one known IPv6 failure and 34 ignored; strict Clippy
  in both modes, fmt and whitespace checks pass. CI must establish this fix, not a local retry.
  Resume K25 after this independent correction; its stopped-disk KVM proof is pending.


- K25 runtime journals now durably retain specifications and credentials outside data/export
  roots. Launch intent is on disk before compute starts; clean/forced observations and removal
  are persisted, while an interrupted journal restores only an unknown exit status. Startup
  still waits for every actual monitor lease to be released. Private journal reads are bounded,
  no-follow, owner/mode/link checked and never quote rejected values.
- Real local filesystem acceptance preserves subsecond timestamps and credentials, distinguishes
  interrupted launches, and refuses public modes, hardlinks, symlinks, malformed values and
  duplicate names without exposing the generated secret. Existing optional KVM acceptance now
  restarts the actual runtime before first boot and after clean stop, preserves its ID/env/status,
  and refuses a second owner. No new boots/jobs/gates or ordinary PR minutes. Full local Rust:
  506 passes, one unchanged IPv6 failure, 34 ignored; strict Clippy modes/fmt pass, no new daemon
  length warnings. Real journal adoption proof remains pending.
- Read failed VM source 7cc98b6 run 43530436293/job 132246481239: runtime acceptance times out
  waiting for an exec POST response within the original five seconds (16.71 s into the test).
  Bundle, registry and six component boots still pass. This is an unresolved production-path
  failure, not a pass; keep the deadline and add retained runtime-stage/monitor evidence before
  diagnosing further. Runtime boot directories currently disappear when the failing test unwinds.
  Scanner remains eighteen addressed comments. K25 stays in progress.


- K25 failure evidence: test-only Live cleanup retains at most one existing request-size bound
  of complete serial lines in the existing VM artifact, with every launch environment value
  redacted. No private journal, bootstrap input or environment dump is uploaded. Fixture stage
  markers identify the image and exact fixed test command before exec. Production logging,
  all five-second exec deadlines and the one-shot mutation policy are unchanged.
- Actual local file acceptance proves tail bounds, value redaction and symlink/hardlink refusals.
  Full local Rust: 507 passes, the known IPv6 failure, 34 ignored; strict Clippy in both modes,
  fmt, actionlint and whitespace checks pass. Workflow changes only extend existing artifact
  retention; no new gate/job or material CI time. Failure summary: checks/measurements/k25/
  runtime-7cc98b6.json. Stopped-file service and journal adoption still await actual KVM CI.


- K25 node-file transport preparation: a shared authenticated file session offers only four
  fixed operation routes and encodes paths through the URL library. It streams bodies without
  JSON-sized buffering, sends each mutation once, and owns backend cleanup independently of
  caller cancellation. The stopped guest can now become this session without releasing its
  transition/disk lease. Node API handlers are not yet connected and no VM tier is announced.
- A real local HTTP server proves scoped authentication, query-injection-safe path encoding,
  a binary response above the JSON limit, one mutation, and retention of the backend guard
  after cancelling a close wait. Full Rust: 508 passes, one unchanged IPv6 failure, 34 ignored;
  both strict Clippy modes/fmt pass. Existing KVM acceptance now uses this same session for its
  stopped-disk binary transfer; no new boot/job/gate or material CI time. Real proof is pending.
- Source 04ead4a VM run 43531094740/job 132248711101 has failed its runtime step; other steps
  are still active and GitHub's job-log endpoint currently returns 404. Read that completed
  failure as soon as the log is published. Do not infer the cause or a successful file guest.
  Scanner remains eighteen addressed comments. Continue K25 node-session integration.


- Read completed failed file-service runs 43531094740/job 132248711101 at 04ead4a and
  43531398107/job 132249720572 at f967ed6: both reach open_files, then the monitor exits
  during bootstrap (15.40/15.36 s into runtime acceptance). Their component boots, bundle and
  registry checks pass. Summary: checks/measurements/k25/file-bootstrap-04ead4a-f967ed6.json.
- Correct a concrete non-root identity bug: vsock 0.5.5 get_local_cid opens /dev/vsock, whose
  published kernel misc device has no mode override and devtmpfs defaults to root-only 0600.
  Instead explicitly bind the expected guest CID at the library’s any-port value and require
  the socket’s actual CID to match; the published kernel rejects non-local explicit CIDs.
  Keep non-root/PID-1-parent checks, the root init’s device/host-peer CID checks and all device
  permissions. Local host-refusal acceptance passes before any socket/device access. CI must
  still establish the positive guest boot; do not infer that it does from this diagnosis.
- Startup failures now retire the owned monitor and drain its serial capture before the
  temporary directory disappears in test builds; preserve the original startup error/deadline.
  Full local current Rust: 509 passes, the unchanged IPv6 failure and 34 ignored. Strict Clippy
  modes/fmt and daemon length probe pass. An earlier diagnostic-only full run also hit a real
  TCP connect timeout in the existing handshake-cap test after 135.10 s; preserve that failure.
  The current full run passes that test without changing it. Scanner stays eighteen addressed
  comments. Next finish node-session/retained-data access, including stream-duration handling,
  while real KVM CI verifies this correction. K25 is still in progress.


- K25 stream duration: private file sessions now bound TCP/HTTP setup separately from the
  upload body. The existing control/readiness client and its deadlines remain unchanged.
  This prevents a valid slow upload from inheriting the small-control-call wall clock limit;
  request bodies and response bodies still stream through the maintained HTTP library once.
- The actual HTTP regression now withholds its body beyond a one-second connection deadline,
  then verifies the full binary round trip, query encoding, one mutation and cleanup ownership
  after cancelling close. Its current-source isolated run passes in 1.12 s. Full local Rust
  is 509 passes, one unchanged IPv6 failure and 34 ignored; both strict Clippy modes, fmt and
  whitespace checks pass. This adds about one second to an existing unit target, no job/gate.
- Read failed VM source 53b3da6 run 43532126674/job 132252154171: journal restart checks
  reach the same file-service bootstrap exit (12.93 s), not a journal parse failure. Preserve
  it in the same short bootstrap-attempt summary. The later CID correction is still awaiting
  its own actual KVM run. Scanner stays at eighteen addressed findings. Continue the node
  backend file-session seam and retained-disk access before announcing a VM tier.


- K25 backend file-session seam: an explicit remote-data capability defaults off for existing
  backends. VM requests reuse an authenticated running agent under its transition guard, or
  launch a layerless guest for stopped/retained disks using only host-owned data identity and
  resource bounds. Refuse a mismatch with an existing sandbox/epoch/data path. Retained access
  creates no compute journal, inventory entry, workload command or image pull. Node handlers
  and their owned workload/memory guards are the next integration step; the tier stays unwired.
- Local policy acceptance proves that retained-file construction carries no image, workload
  environment, labels, packages or extra ports and keeps admitted user/disk/memory bounds with
  a fresh signing key. Full Rust: 510 passes, one unchanged IPv6 failure, 34 ignored; strict
  Clippy modes/fmt and daemon length probe pass. Existing KVM acceptance now also reads a live
  agent, refuses an epoch mismatch, removes compute, reads retained binary data without an
  inventory entry, recreates compute and cold-boots it. Two additional maintenance boots in
  that existing optional job, estimated under fifteen seconds; zero ordinary PR minutes or
  new gates/jobs. Real execution is pending, as is the earlier CID fix. Scanner remains eighteen
  addressed findings. No public/fleet/edge protocol shape changes in this checkpoint.


- Read source 480ddfc VM run 43534556211/job 132260289915 and its actual CI artifact via
  plain curl. The two bounded serial captures (24,422/24,822 bytes) identify the file guest’s
  failure before agent startup: overlayfs rejects the persistent workload upper’s origin
  against the new empty lower with ESTALE. The non-root CID correction is independent of
  this earlier mount failure; neither prior source established a successful file service.
  Summary: checks/measurements/k25/file-bootstrap-480ddfc.json. The original two-vCPU guest
  produces its signed clean-disk proof, then the monitor reports a vCPU teardown timeout;
  retain that observation separately and do not call it a clean monitor exit.
- Fix maintenance overlay construction: validate the parsed maintenance mode before mounts,
  create its upper/work only in the trusted guest tmpfs, and bind persistent data separately.
  Do not remount the workload upper over a different lower, edit its origin metadata or
  relax the kernel’s origin check. Actual local directory acceptance leaves the persistent
  root bytes/mtime unchanged and proves normal boots retain their original paths.
- Connect the existing node file routes to backend file sessions. Workload guards and admitted
  memory survive stream/caller cancellation through owned backend cleanup. Exact epoch checks
  precede preparation; each upload chunk checks the host floor; file bytes/metadata stream
  without a JSON body cap; agent refusals use existing node error shapes. Existing local file
  backends retain their descriptor walk. No public/fleet/edge protocol field or path changes.
- Real local mTLS node-to-agent HTTP acceptance proves the binary transfer, list/stat shapes,
  epoch-before-preparation, missing-file/traversal errors, no workload start, and preservation
  of both the workload guard and memory after cancelling cleanup. This uses a real agent
  router with an in-memory compute seam; it is not KVM proof. Full local Rust: 512 passes,
  one unchanged IPv6 failure, 34 ignored; strict Clippy modes/fmt and daemon length probe pass.
  Agent package independently has 43 passes, the same IPv6 failure and one ignored case.
- Existing optional VM CI now uploads its bounded runtime evidence separately, avoiding a
  366 MiB bundle download for two tiny serial files. Actionlint passes; one additional
  existing artifact-upload step costs a few optional seconds, no new gate/job or ordinary
  PR minute. The downloaded bundle ZIP was removed; free disk remains about 3.3 GiB.
- Next prove this maintenance fix and actual node file handlers on real KVM, finish VM logs/
  stats/SIGINT/adoption failure paths, root jailer/TAP/firewall and ordinary-user networking,
  protected configuration/release/installer wiring, node+edge lifecycle and thirty production
  samples. Check regenerated OCI lower identity across caches before cross-node disk moves;
  current mkfs arguments specify neither a stable UUID nor build time. K25 remains in progress.


- K25 reproducible OCI lower identity: use the verified diff_id to derive a version-8 UUID,
  fix only EROFS build time, and preserve image file mtimes. A separate private conversion
  cache leaves old random-identity bytes untouched. The actual registry acceptance independently
  rebuilds every Alpine/Debian layer and compares complete hashes; it passes locally in 32.93 s,
  retaining the competing-process lock, cached-byte and corruption assertions. Existing optional
  KVM acceptance now forks through an independent runtime/cache; no new boot/job/gate, estimated
  under ten extra optional seconds for the rebuild. Cross-cache guest proof remains pending.
- Full local Rust: 512 passes, the unchanged unsupported-IPv6 fixture failure and 34 ignored;
  both strict daemon Clippy modes, helper strict all-target Clippy and both fmt checks pass.
  Helper default tests pass with their existing external-input ignores. The unfinished checkpoint
  initially failed compilation after moving diff_id into its worker; computing the UUID before
  that move resolves it. Removed unused local musl build output, recovering about 2 GiB.
- Read completed VM failures feda4d0 (43535042318/job 132261871813, 14.67 s) and 9988a72
  (43535774318/job 132264283949, 14.15 s): both still fail file bootstrap before the later
  maintenance-overlay correction. Retain them in the existing short bootstrap-attempt summary.
  Current 28c34f0 VM run 43537774307 and normal/host runs remain queued; scanner remains eighteen
  addressed comments. K25 stays in progress; finish actual maintenance/node KVM proof, logs/stats/
  SIGINT/adoption failures, both networking/confinement paths, install/config and production samples.


- K25 selected shutdown signals: authenticate INT with the boot-scoped request while preserving
  the original TERM wire/signature. Guest init forwards the selected signal to its supervised
  agent group before the unchanged bounded cgroup drain, disk sync and signed observation.
  Verified bundle metadata must announce signal support before any guest is launched.
- Real Unix-pair acceptance covers TERM and INT acknowledgements without peer EOF; cryptographic
  acceptance rejects signal substitution, an authenticated unsupported signal, another identity
  and another boot, and preserves legacy TERM verification. Existing KVM acceptance now runs
  Debian with an INT handler and verifies its retained-disk marker through the maintenance guest;
  no additional boots/jobs/gates or material CI time. That actual guest proof is pending.
- Current-source full local Rust: 513 passes, the unchanged unsupported-IPv6 failure and 34
  ignored; strict Clippy modes, helper all-target Clippy/default tests, fmt and whitespace checks
  pass. The length probe adds no daemon warnings. An initial compile used a nonexistent rustix
  signal constructor; the library's checked from_named_raw resolves it without unsafe conversion.
  CI 43539225548 at 021d73c remains queued; scanner stays eighteen addressed comments. K25 is
  still in progress; continue VM logs/stats and the other pending production integration.


- K25 move-only preparation: move the unchanged persistent-entrypoint decoder and its existing
  tests from the process backend into runtime/logs.rs; the process reader retains its original
  descriptor, bounds, rotation detection and following. Current local Rust is 513 passes, the
  unchanged IPv6 fixture failure and 34 ignored; both strict Clippy modes/fmt/whitespace pass.
  No workflow, gate or runtime behaviour changes. VM log transport and admitted stopped-disk
  replay are next. Source 73d00ba VM run 43539908502 remains queued; scanner stays eighteen.


- K25 logs: authenticated live agent frames now drive runtime entrypoint output with bounded
  frames, explicit gaps, timestamps/since/tail filtering, setup-only deadlines and idle-reader
  cancellation. A stopped remote backend replays its persistent log via the manager's existing
  admitted file session; exact workload/data guards and memory outlive cancellation through
  owned cleanup. Monitor serial and bootstrap never become application output.
- Actual local agent HTTP acceptance covers replay, follow beyond the setup deadline, epoch
  authentication, oversized/incomplete frames, explicit gaps and early follow disconnection.
  Real mTLS node acceptance verifies stopped log contents and cancelled log-cleanup memory/
  guard retention without starting compute. Existing KVM acceptance verifies live and stopped
  Debian entrypoint output with no added boots/jobs/gates; about two extra seconds in ordinary
  unit CI and under one optional second. Actual guest log proof is still pending.
- Current-source local Rust: 515 passes, the unchanged IPv6 fixture failure and 34 ignored;
  both strict Clippy modes/fmt/whitespace pass, and the length probe has no new daemon warning.
  Earlier Clippy catches a redundant closure and items after a test module; both are corrected.
  The current complete Wasm case passes in 26.77 s while retaining its termination warning.
- Real KVM proof at source 28c34f0: VM run 43537774307/job 132270844126 is green. Both images
  complete active/stopped/retained file sessions, binary transfers, journals, epochs, real copy
  leases, cold wake, fork independence and quick signed completion in 49.43 s. Bundle (6.12 s),
  registry (7.67 s) and six component boots (30.79 s) pass too. Short summary:
  checks/measurements/k25/maintenance-28c34f0.json. Its normal/host runs remain queued; this does
  not prove later cache identity, signals or logs. Scanner stays eighteen addressed comments.
  K25 remains in progress; continue stats/adoption failures, networking/root confinement,
  protected install/config wiring, full node/edge lifecycle and thirty production samples.


- K25 guest stats: use the already pinned procfs library for CPU durations, RAM/cache and
  interface counters, plus trusted init's command-cgroup PID count. Only VM bootstrap enables
  these optional health observations. Routes-scoped sampling executes no workload command or
  activity touch; stopped guests return no stale stats. Limits remain the admitted VM sizes.
- Real kernel/library acceptance checks units, monotonic counters and disabled sampling; the
  actual agent API confirms other tiers omit host counters. Existing KVM runtime acceptance now
  checks positive CPU/RAM/PID/interface observations, admitted bounds, unchanged activity and
  stopped absence. No additional boots/jobs/gates or material CI time. Actual guest proof is
  pending. Full local Rust: 516 passes, the unchanged IPv6 fixture failure and 34 ignored;
  strict Clippy modes/fmt/whitespace and the nineteen-test agent API pass, with no new daemon
  length warning.
- A supplementary fs-only agent build initially exposes server-only bootstrap references and
  then the unused private PNG signature. Gate those executable-only items on server, retaining
  metadata types; strict fs-only Clippy now passes. No CI job/check is added for this local probe.
  Source 021d73c VM run 43539225548 remains active without a failed step; scanner stays eighteen.
- Next finish VM adoption/failure acceptance and audit compute admission across caller
  cancellation: Manager::start currently drops its workload guard when cancelled while the VM
  runtime's owned launch may continue, and admission counts only observed/copying workloads.
  The VM tier stays unregistered until those paths, networking/root policy, protected installer/
  configuration and full node/edge lifecycle plus thirty production samples are proved.


- K25 compute admission: requested starts and replacements acquire their workload lock before
  handing it to an owned task, so waiting requests remain cancellable and accepted transitions
  retain fencing authority after disconnection. All start paths, including automatic and copy
  resumes, atomically reserve memory before compute. Failed/unobserved starts stay charged
  through outages; a fresh observation resolves the same reservation ticket atomically with
  publishing its actual state. Old inventory cannot release a pending transition.
- Three actual async-boundary regressions pass (0.10 s): cancelled start, cancelled replacement
  at its new memory size, competing-start refusal, fencing order, and an unobserved start across
  a runtime outage. The first full run catches a cancelled lock waiter executing later in the
  existing file-cleanup test; acquiring the lock before task handoff fixes it without changing
  that test's assertions. Current complete local Rust: 519 passes, the unchanged IPv6 fixture
  failure and 34 ignored; both strict Clippy modes/fmt/whitespace pass. No new CI job/gate or
  material ordinary CI time. This proves manager ownership/admission, not engine enforcement.
- Source 021d73c VM run 43539225548/job 132275590623 is green: full rebuilt-layer hashes and
  both independently cached KVM forks pass, runtime 68.50 s and registry 13.50 s. The earlier
  runtime was 49.43 s; these are separate hosted runs, not a controlled causal timing comparison.
  Summary: checks/measurements/k25/cross-cache-021d73c.json. Source 73d00ba VM run 43539908502/
  job 132277787678 is also green: actual INT marker and TERM lifecycle, runtime 71.13 s;
  summary: checks/measurements/k25/shutdown-73d00ba.json. Move-only VM run 43540196186 is green.
  These do not prove later logs/stats/admission. Normal source-28c34f0 workflows still have only
  queued initial jobs; scanner remains eighteen addressed comments. K25 stays in progress.
- Next make failed VM launch retirement authoritative before returning a stopped observation,
  prove interrupted-live adoption/failed ingress on real KVM, then finish ordinary-user/root
  networking and confinement, protected installer/configuration, full node+edge lifecycle and
  thirty production samples. Do not register the VM tier before those paths and refusals work.


- K25 unfinished-launch observations now require the actual kernel monitor lease to be free
  before reporting stopped compute. Unsigned startup failures and interrupted journals report
  Dead/unknown, never a clean guest exit; signed and explicit forced completions retain their
  known status. A real local flock regression proves refusal while held and conservative status
  after retirement, including known completion and never-started cases.
- Existing optional KVM acceptance now drops a live runtime owner, adopts its unknown exit,
  occupies the actual ingress before a refused launch, waits for lease retirement, then cold
  boots and verifies both retained data files. Four additional boot attempts in the optional
  job, estimated under twenty seconds; zero ordinary PR minutes or new jobs/gates. Actual
  execution remains pending. Current full local Rust: 520 passes, the unchanged IPv6 failure
  and 34 ignored; both strict Clippy modes/fmt/whitespace pass, no new daemon length warnings.
- Source ed5091d VM run 43541250656/job 132282077573 is green: live/stopped Debian logs plus
  all existing runtime assertions, 71.52 s; bundle/registry/component boots pass too. Summary:
  checks/measurements/k25/logs-ed5091d.json. This does not prove later stats/admission/retirement.
  Scanner remains eighteen addressed comments. K25 stays in progress; next implement both
  network/confinement paths, protected configuration/install/release wiring, actual node/edge
  lifecycle and thirty production samples. The published kernel's retained configuration has
  built-in nftables/NAT/redirect support; use maintained firewall tooling for guest policy.


- K25 firewall component preparation: build unchanged nftables 1.1.6 with its documented
  batch/no-JSON/embedded-mini-GMP profile. Upstream's notice retains GPLv2-only code, while
  Alpine's nft executable links GPLv3 Readline; the separate upstream build selects mini-GMP's
  compatible GPLv2 option and needs only existing musl plus pinned libmnl/libnftnl. It links
  nothing into pryx. Retain complete sources/notices/recipes for runtime and build inputs,
  including Linux UAPI headers; the published guest kernel is neither changed nor built.
- Actual ordinary-user signature-checked component build and relocation pass: nineteen runtime
  files, working nft version output, EROFS round trip (4,096 bytes) and ext4 check (128 MiB).
  Every one of the prior fifteen filesystem members remains byte-identical. Readline, shared
  GMP, JSON and development programs are outside the actual ELF closure. Real OCI/cache/
  corruption acceptance with this expanded verified component passes locally in 22.10 s.
  Python compilation/whitespace checks pass. This is packaging proof, not guest firewall proof.
- The existing optional VM workflow automatically runs this builder/relocation check; estimate
  one to two additional optional minutes for source compilation, the retained 150-MiB UAPI
  source and artifact transport. Zero new jobs/gates or ordinary PR minutes. The prototype
  output is removed after validation; about four GiB remain free. Next include the verified nft
  closure in trusted bootstrap and prove immutable guest policy through real KVM before network
  availability is announced. K25 remains in progress.
- Source a91941b VM run 43542004659/job 132284453447 is green: real guest stats and unchanged
  activity/bounds/stopped absence, runtime 78.29 s. Summary: checks/measurements/k25/
  stats-a91941b.json. Source af20fc0 VM run 43543882525 is green too, but normal CI must still
  prove manager admission's unit/integration assertions. Scanner remains eighteen addressed
  comments; later unfinished-launch and firewall-component sources await their own KVM results.

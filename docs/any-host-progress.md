# Any-host progress

Continue on `any-host`. Follow `PLAN-ANY-HOST.md`, `AGENTS.md`, `PLAN.md`,
`docs/any-host.md`, `docs/decisions.md` and the current milestone brief.
Only recorded CI evidence establishes completion; a local pass alone does not.

| Milestone | Status | Finishing commit | CI proof | Ran | Did not run | Parked and why | Left |
| --- | --- | --- | --- | --- | --- | --- | --- |
| K19 | done | `b535d24b95ab2046aa6178fdc1d2683640ce2544` | [42777018303](https://github.com/cubepals/pryx/actions/runs/42777018303), green including aggregate | Real hosted KVM, full Rust and Docker suites, CLI doctor, default-profile image probes, non-root API refusal, full end-to-end attacks, one-box/Action/Python, Wasm, two-node, upstream integrations and recovery; local checks below | Live paid-provider provisioning and publication; neither is authorized | None | None |
| K20 | done | `54bd364` | Normal CI 42832474233 and Any host 42832474296, both green | All 13 host jobs: 8 user distro contexts, root container/PG UID, fallback, refusal, sudo, relocation; full normal CI including real Docker/KVM, one-box/Action, Wasm, 2-node, Rivet/celld and recovery | Production main/release prohibited; unchanged credentialed cloud/bootstrap cases skipped | None | Complete; proceed to K21 |
| K21 | in progress | — | Routing CI 42849302425 / 42849303232 green; final source pending | Router, saved tiers, shared placement, auto selection, both doctors and local checks | Final mixed-tier image acceptance | None | Prove final source in normal and Any host CI |
| K22 | not started | — | — | — | All | — | Complete brief after K21 |
| K23 | not started | — | — | — | All | — | Complete brief after K22 |
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

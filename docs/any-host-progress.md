# Any-host progress

Continue on `any-host`. Follow `PLAN-ANY-HOST.md`, `AGENTS.md`, `PLAN.md`,
`docs/any-host.md`, `docs/decisions.md` and the current milestone brief.
Only recorded CI evidence establishes completion; a local pass alone does not.

| Milestone | Status | Finishing commit | CI proof | Ran | Did not run | Parked and why | Left |
| --- | --- | --- | --- | --- | --- | --- | --- |
| K19 | done | `b535d24b95ab2046aa6178fdc1d2683640ce2544` | [42777018303](https://github.com/cubepals/pryx/actions/runs/42777018303), green including aggregate | Real hosted KVM, full Rust and Docker suites, CLI doctor, default-profile image probes, non-root API refusal, full end-to-end attacks, one-box/Action/Python, Wasm, two-node, upstream integrations and recovery; local checks below | Live paid-provider provisioning and publication; neither is authorized | None | None |
| K20 | in progress | — | Pending implementation | Read the brief, current installer/supervisor/release paths and versioned dependency licences | User bundle and distribution matrix | None | Presets, signed relocatable bundle, shared supervision, upgrade/removal, docs and real-host CI |
| K21 | not started | — | — | — | All | — | Complete brief after K20 |
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

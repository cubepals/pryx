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

# pryx

The guide for anyone changing this repository, agent or human. [CONTRIBUTING.md](CONTRIBUTING.md)
covers setup and tests; the rules below apply to every change.

pryx runs sandboxes on Linux hosts you own, one or many: it makes them from images and snapshots,
puts them to sleep and wakes them on a connection or an API call with their state kept, snapshots
and forks them, moves and rebuilds them, and keeps them off the host and off each other. Three
parts: `pryxd`, the daemon on each host (`src/`, Rust); the control service (`control/`,
TypeScript on Bun, Postgres); and the edge (`edge/`, TypeScript on Bun). A node never decides what
should run; control does. Control never touches a host but through `pryxd`'s API.

## How pryx decides

pryx adapts to the host. The host never has to adapt to pryx. These rules come before any
feature, and [docs/any-host.md](docs/any-host.md) is the plan that follows from them.

- **One command.** Installing pryx is one command that asks nothing, on every host pryx supports.
  The installer finds out where it is and takes the preset for it. A change that adds a step, a
  question or a required flag to installing is refused unless nothing else can work.
- **Presets, not options.** What differs between hosts is a named preset with one command in the
  documentation: an ordinary user with no root, a container, a server with root, a host with KVM,
  a node joining a fleet. A person names a preset only to overrule what was detected. A new
  setting takes its value from the host and stays off the first screen.
- **Rootless first.** A capability is built first to run as an ordinary user. Root, Docker, KVM
  and other host features turn on more: stronger isolation, images, virtual machines. None of
  them is needed for pryx to start.
- **Everything, by tier.** pryx aims to be the most complete sandbox platform: processes,
  containers, WebAssembly and virtual machines; shells, files, ports, browsers, desktops, SSH,
  mounts and interpreters. What a host can't give is off on that host alone, and pryx says which
  capability, why, and what would turn it on.
- **Never a weaker guarantee in silence.** A request that names an isolation or a capability gets
  it, or a refusal that names what is missing. pryx chooses a backend only when the request left
  the choice open, and says which it chose.
- **Wrap, don't hand-roll.** A browser, a virtual machine monitor, a desktop server, a container
  runtime, a language kernel: pryx wraps a maintained open-source one, at a pinned version, with
  its licence recorded in `docs/decisions.md` before it is used. Copyleft software runs as a
  separate program and is never linked or copied in. Writing our own needs a reason that no
  existing project answers, written in the same file.
- **Decide, record, go on.** Whoever makes a change takes the engineering decisions these rules
  leave open, records each in `docs/decisions.md` (one line: the choice and why) and continues. A
  maintainer is asked only about money, a licence that changes what pryx may ship, and anything
  that can't be undone.

## The protocol only adds

- **The Rust types are the node protocol.** `openapi/` is generated from them
  (`BLOCKLYD_WRITE_SCHEMA=1 cargo test --lib protocol::schema`) and committed with the change that
  moves them.
- **Within v1 a change only adds.** A new field is optional or defaulted, a new request field is
  announced in `features` before a control plane may send it, nothing is renamed or removed. A
  node and a control plane one release apart always understand each other. The `contract` job
  runs Cubepals' main against this build and must pass.
- **Format v1 keeps its names.** The `blocklyd-epoch` and `blocklyd-protocol` headers, the
  `blocklyd.*` container labels, `blocklyd.record`, `workload.json`, the `bk1.` token prefix and
  the fleet endpoint's paths are the wire and on-disk format this daemon was first published with.
  They are a format, not a name, and stay until protocol v2. Everything else says pryx.
- **The public API (`/v1`) and the edge protocol (`/edge/v1`) follow the same rule**: their
  OpenAPI documents are committed, the generated clients are checked against them in CI, and
  within a major version a change only adds.
- **pryx never links Cubepals' code, and Cubepals never links pryx's.** The two talk over pryx's
  APIs only. pryx is FSL-1.1-ALv2; Cubepals is AGPL-3.0.

## Business internals stay private

Costs, margins, prices, other products, strategy and tuning never go in this repository, its
commits, pull requests or issues. A number like that reaches pryx from its environment or its
configuration, and the code's default is a harmless one.

## Values from elsewhere

A value that belongs to another system is taken from that system, never typed out again: a
library's error codes and messages, a protocol's numbers (HTTP statuses, Docker Engine API answers),
an API's shapes, another module's enum or limit.

- **Import the owner's constant or type.** An HTTP status is `StatusCode`, never a number. A signal
  is `libc::SIGKILL`, and the exit it causes `runtime::exit_by(libc::SIGKILL)`, never 137. Another
  module's limit, label or enum is that module's item, imported.
- **When nothing exports it**, only its documentation gives it, it is one named constant in one
  place, with a comment saying where it comes from (the kernel's cpu.shares bounds and R2's PUT
  limit in `protocol/`).
- **Docker's answers are read once, in `runtime/docker.rs`,** and turned into `RuntimeError`'s
  variants. The rest of the crate matches variants, never a status or a message's text.
- **Tests take errors and answers from the real thing:** the real library, a real Docker in
  `tests/docker.rs`, a real Postgres in `control/`'s tests. The fake runtime answers with the same
  variants in its own words, not Docker's.
- `tests/formats.rs` and `tests/fixtures/` pin the wire and on-disk formats byte for byte, so they
  spell them out. CI's `Values from elsewhere` step greps for what a grep can see.
- **Durations** in TypeScript come from `control/src/durations.ts`, never written out.

## Structure

A source file at most about 800 lines, an `impl` block, class or function at most about 600, split
by concern before more is added, in its own `Move: …` change in which code moves and nothing else
does. No file or module called `utils`, `helpers`, `misc`, `common` or `shared`. A file over 50
lines opens with a doc comment (`//!` in Rust, `/** … */` in TypeScript) saying what it is for and
what it is not for. The file named after a split's directory lists every file in it under
``Parts (`dir/`):``. No cycle between modules. No comment saying who decided something or when.

CI's `Structure` step runs Cubepals' `scripts/check-structure.ts` against
`structure-baseline.json` (with `--rust` for the crate). It is a ratchet: the baseline only
shrinks, and nobody adds an entry without saying why in it.

## Build, test, lint

```sh
cargo build --locked                                   # pryxd, debug (only debug takes PRYXD_TEST_VERSION)
cargo test --locked                                    # unit, API, lifecycle, epochs, reconcile, resume, transfer; no root
cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings
cargo deny --locked check                              # deny.toml
sudo cargo test --locked --test docker -- --ignored    # a real Docker daemon, as root: CI runs it
bun install --frozen-lockfile                          # control/, edge/, cli/, sdk/typescript/
bun run check                                          # biome, typecheck, tests of every TypeScript workspace
bun run e2e                                            # control and edge against a real pryxd: Docker, Postgres and an S3 store (CI runs it)
```

Run what you changed before you push: `cargo test --locked` for Rust, `bun run check` for
TypeScript. The Docker-backed and end-to-end suites are CI's; say in the pull request that they
ran there, never that they passed on a machine where they didn't run.

## Releases

- A release is a pull request that raises `version` in `Cargo.toml`, and nothing else does.
  `release.yml` tags and publishes it once CI passes on that commit of `main`. Never push a tag by hand.
- The control service, the edge, the CLI and the SDKs release with the daemon, from the same tag.

## CI

A change isn't done when its PR is green. It is done when the runs it starts on `main` are green
too, the release included.

- **Workflows are code.** A change to `.github/workflows/` passes `actionlint` before it is
  pushed, and CI runs it again.
- **No new gate without asking.** A new required check, CI job or test system is a maintainer's
  decision. A change that adds more than a minute to CI says so in the pull request.
- **A failure outside the code is still a failure.** A registry's rate limit or an outage gets a
  fix (a mirror, a retry), not a rerun until it passes.

## Commits

Signed, as their author. No attribution lines in commits or pull requests.

A commit's subject, and a pull request's title (the squash commit on `main`), says the change in
50 characters or fewer, with no closing period; the detail goes in the body. It says what changed,
plainly, never the motive. `scripts/check-subject.sh` enforces it, in lefthook's `commit-msg` hook
and in CI's `subject` job.

## No accidental mentions

Never put `@` before a name in a commit message or a pull request's title or body, not even inside
backticks: a squash merge copies the body into the commit, where `@s` mentions the GitHub user "s".
Write "the s selector" or "the player" instead. `scripts/check-mentions.sh` holds commits and
pull requests to it.

## Code Review Rules

Only what was checked against the code and is wrong:

### Bugs
- Wrong behaviour, broken edge cases, races, errors swallowed. A verb that stopped being
  idempotent. A retry of a verb that isn't safe to repeat (kill, delete, fence, exec, transfers).

### Security
- Anything a spec can relax of the host's policy (the hardening is the host's, never the
  request's). A walk that follows a link. A secret value in a record, a label, a log or a view.
  A listener bound to a public address. A certificate check loosened.

### Data
- A copy of a sandbox that could run twice (an epoch check missing on a mutating verb). A restore
  that touches the current data before it is whole. A snapshot's deletion that drops the only copy.

### The rules above
- A protocol change that renames or removes within v1, or a new request field sent before it is
  announced in `features`. Cubepals' code linked. A business number in the code.
- A step, a question or a required flag added to installing. Root, Docker or KVM made a condition
  of pryx starting. A requested isolation or capability replaced by a weaker one without a
  refusal. Something hand-written that a maintained open-source project already does, or a
  dependency used before its licence is recorded.

Never: praise, a summary of the diff, style, or anything CI already reports.

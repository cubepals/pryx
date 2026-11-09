# Contributing

pryxd is the node daemon of [Cubepals](https://github.com/cubepals/cubepals), developed under
the codename Blockly. [AGENTS.md](AGENTS.md) holds the rules every change follows, for people and
agents alike; this page is how to work on it.

## What every change keeps

[How pryx decides](AGENTS.md#how-pryx-decides) is short and comes before any feature. In one
breath: installing stays one command that asks nothing; hosts differ by preset, not by options; a
capability works as an ordinary user first, and root, Docker and KVM only add to it; a request
never gets a weaker isolation than it named without being told; and pryx wraps maintained
open-source software, with its licence recorded, before anyone writes a replacement. A pull
request that breaks one of these says so in its description and why nothing else worked.

## Setup and tests

The Rust `rust-toolchain.toml` names, which rustup installs, on Linux (pryxd uses Linux-only
APIs; on macOS, work in a Linux container or VM).

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo clippy --locked -- -D warnings                 # as the release builds, without the fake runtime
# Python 3.11+ stages the real wasm test dependency; use a new destination once per clone.
python3 scripts/fetch-agentos.py /tmp/pryx-agentos-test-assets
PRYX_AGENTOS_DISTRIBUTION=/tmp/pryx-agentos-test-assets cargo test --locked
sudo cargo test --locked --test docker -- --ignored   # against a real Docker daemon
cargo deny --locked check
```

The wasm case runs as your normal user against the checksum-pinned sidecar; missing assets fail
setup. Reuse the staged directory on later test runs. Docker and full end-to-end acceptance run
in CI.

The structure rules are Cubepals' `scripts/check-structure.ts`, run here against
`structure-baseline.json`; CI's `Structure` step shows how to run it. CI's `contract` job runs
Cubepals' control plane against your build. `bun install --frozen-lockfile` and `bun run check` check every TypeScript workspace;
`bun run e2e` uses real Docker, Postgres and S3, as detailed in AGENTS.md.

## How changes land

`main` is the trunk. Branch from it, keep the change small, and open a pull request. CI's `check`
must pass. Pull requests are squash-merged, so the title becomes the commit on `main`: a plain
sentence about what changes, of 50 characters or fewer. `main` keeps a linear history.

Commit subjects follow the same rule, which [lefthook](https://lefthook.dev)'s `commit-msg` hook
checks (`lefthook.yml`, `scripts/check-subject.sh`), with `scripts/check-mentions.sh` for names
GitHub would turn into mentions. Install it once per clone, with Node:

```sh
npx lefthook@2.1.15 install
```

A change to the protocol only adds within v1 (a new field, a new endpoint, announced in
`features`), so a control plane one release behind keeps working. Regenerate the specs with
`PRYXD_WRITE_SCHEMA=1 cargo test --lib protocol::schema` and commit `openapi/`.

A release is a pull request that raises `version` in `Cargo.toml`; [AGENTS.md](AGENTS.md#releases) describes publication after it merges.

## Your contribution's licence

pryxd is published under [FSL-1.1-ALv2](LICENSE.md), and each version becomes Apache-2.0 two
years later. So that a contribution can follow it there, by opening a pull request you license
your change to The Cubepals Authors under the [Apache License, Version
2.0](https://www.apache.org/licenses/LICENSE-2.0), and you confirm that it is yours to license.
You keep the copyright in it.

Each Rust file opens with two SPDX lines, its copyright and its license, above its `//!` comment;
everything else is covered by [REUSE.toml](REUSE.toml). CI runs `reuse lint`, so a new `.rs` file
needs them:

```rust
// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2
```

## Conduct and security

Everyone here follows the
[code of conduct](https://github.com/cubepals/.github/blob/main/CODE_OF_CONDUCT.md), which every
Cubepals repository shares. Security problems are reported privately, as [SECURITY.md](SECURITY.md)
says, never in an issue. Where to get help is in [SUPPORT.md](SUPPORT.md).

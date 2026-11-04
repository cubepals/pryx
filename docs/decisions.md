# Decisions

- K1: Restore the missing PLAN.md from origin/main c253af4 by fast-forwarding the plan-only commit; it is the repository's authoritative plan.
- K1: Install Rust using rustup with the pinned 1.99.1 toolchain and Bun using its installer; neither tool was on PATH.
- K1: Use a formats test harness with an extern-crate alias; tests/formats.rs and fixtures stay byte-for-byte unchanged. A renamed self dev-dependency panicked cargo-deny, so the final self dev-dependency uses pryxd.
- K1: Keep debug aliases for BLOCKLYD_TEST_VERSION and BLOCKLYD_TEST_RELEASE_KEY; Cubepals owns the contract suite's environment names.
- K1: Reject the old release-signature fixture rather than rewrite it; the new release statement is pryxd release.
- K1: Sign commits with an environment-local SSH signing key under the configured Git author; no existing author key was available and GitHub verification is not implied.
- K1: The unchanged Cubepals main upgrade contract signs the old release statement and supervises state/bin/blocklyd; it conflicts with the required new release statement and install paths. Do not weaken release verification or modify Cubepals to hide it.
- K1: Keep PLAN.md and AGENTS.md authoritative text unchanged; their historical names, test-only crate aliases, schema-writer compatibility alias and existing format fixtures remain in the recorded rename audit.
- K1: Keep Docker-backed and full contract suites for CI as the plan specifies; a working local Docker daemon is available for later real-Postgres unit tests.

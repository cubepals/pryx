<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

Add a remote container engine boundary

Separate container effects from the current ownership and inactivity coordinator. Keep the
existing local Docker implementation and add a prepared pryx public-API implementation with
private TCP endpoints, host-enforced isolation and a durable creation journal.

The prepared remote backend supports the single-node development store and refuses distributed
ownership, local OCI runtime or DNS overrides, remote exec and root-process signaling. An uncertain
creation blocks replacement until reconciled. Distributed deployment needs a remote fencing
handoff before this backend can be enabled there.

Prepared against public release 0.6.3 at commit 8702f6d186c77698dbfb30cf29e3b1922a88f008.
Locked cargo check and three actual-filesystem journal tests pass locally. CI 42077009774 builds
the refined executable and passes the unchanged upstream container example through both engines,
30 samples each on the same host/profile. Backend-start p90 is 154 ms locally and 233 ms through
pryx; warm HTTP p90 is 5.72 and 7.24 ms. The Docker-backed example did not run locally. All test
jobs succeed; GitHub blocks the final aggregate job on an account-level runner-allocation restriction, so the workflow is not
green. No upstream pull request, issue or message was sent.

Private journal operations use a default-refusing capability on celld's injected filesystem.
The POSIX host implementation retains private modes, no-follow reads and fsynced atomic replacement;
three real-filesystem regression tests cover symlinks, public modes, bounds and stale temporary state.
Creation keys and timing use the execution-domain RNG and clock.

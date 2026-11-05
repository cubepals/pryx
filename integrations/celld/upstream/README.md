<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

# Prepared native engine patch

Base: denoland/celld `8702f6d186c77698dbfb30cf29e3b1922a88f008`, public release 0.6.3.
Apply `change.patch` using `git apply --check` and `git apply` in a fresh checkout.
`bun integrations/celld/upstream/check.ts OUTPUT` checks out exactly that source, applies it and
builds the executable. It never changes the reference checkout or contacts upstream maintainers.

The container coordinator keeps ownership, inactivity, instance ceilings, monitoring and release.
A `ContainerBackend` seam has the current local Docker implementation and a remote public-API
implementation. The latter selects `CELLD_CONTAINER_BACKEND=pryx`, `PRYX_API_URL`, `PRYX_API_KEY`,
and explicit private TCP ports such as `CELLD_PRYX_PORTS=8080`. HTTPS is required except on loopback;
redirects are refused. `CELLD_PRYX_IMAGES` optionally maps celld content references to published
images allowed by the pryx host. The CI dev example shares a Docker image store with pryx, so its
unchanged Dockerfile's built content reference is available directly. The public API receives
environment, resources, network policy and private ports; host isolation remains authoritative.

This prepared backend is restricted to celld's single-node dev store (or a configuration without
a distributed bucket). Production distributed ownership needs an explicit remote fencing handoff
before enabling it. OCI runtime and DNS overrides, root process signals and remote exec are
refused explicitly. The current local engine retains all of those operations. A private, fsynced
journal tracks remote sandbox identities and pending creation keys. Uncertain creation blocks
replacement and startup until reconciled. Old sandboxes must be deleted and absent before another
is created. Neither credentials nor command environments enter the journal. Parent directory and
journal symlinks and public permissions are refused. These limits are deliberate fail-closed
boundaries, not claims of full Docker compatibility.

Local validation: locked cargo check and all three filesystem journal tests pass against the
public release. The patch applies cleanly to the unmodified reference checkout. CI 42077009774
builds the refined executable and passes the release's unchanged examples/container Worker,
Python server and Dockerfile on both engines, n=30 each on the same host and executable profile.
Backend-start p90 is 154 ms locally and 233 ms through pryx; warm HTTP p90 is 5.72 and 7.24 ms.
No Docker-backed example has run locally. GitHub blocks the final aggregate job on an account-level runner-allocation restriction
after every test job succeeds, so that whole workflow is not green. See
[docker-api.md](../../../docs/docker-api.md) for all samples, costs and the separate unshipped spike.

Private journal operations use a default-refusing capability on celld's injected filesystem.
The POSIX host implementation retains private modes, no-follow reads and fsynced atomic replacement;
three real-filesystem regression tests cover symlinks, public modes, bounds and stale temporary state.
Creation keys and timing use the execution-domain RNG and clock.

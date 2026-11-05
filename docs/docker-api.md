<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

# celld engines and the Docker socket experiment

The prepared native seam is in [integrations/celld/upstream](../integrations/celld/upstream/README.md),
based on public celld 0.6.3 at `8702f6d186c77698dbfb30cf29e3b1922a88f008`. It keeps celld's
coordinator and default Docker implementation and adds a pryx public-API implementation. Its own
container example is assigned to CI from the patched source build. Validation remains pending;
no native example success or cost is claimed before that run completes.

The native backend currently supports celld's single-node development store. Distributed ownership
is refused until a remote fencing protocol is designed and proven. Host policy owns isolation;
local runtime/DNS overrides, root signals and exec are explicit unsupported operations. Creation
is sent once, with its key durably recorded first; an uncertain reply blocks replacement. Existing
remote compute must be deleted and absent before a replacement. The project key stays only in the
service environment and memory. Production images must already be available through a permitted
registry reference; local CI shares the dev image store.

CI measures 30 starts and first HTTP requests for each engine on the same host, from the same
patched executable built with the development profile, debug information and incremental builds
disabled. It retains every sample in `celld-native-measurements.json`, including partial failed
cohorts. The Worker, Python server, Dockerfile and class configuration are the released example.
There are no invented measurements, discarded failures or cross-machine speed claims.

The separate [Unix socket spike](../integrations/celld/src/docker-api-spike.ts) is unshipped and
absent from node features and package exports. It implements `_ping` and `version` discovery using
API version facts read from the baseline engine. Unsupported operations answer 501 explicitly.
It does not import arbitrary image tars, start privileged fence containers or relax host policy.
Celld's current engine builds/loads images and installs a privileged host-network fence; those
operations have no public sandbox API equivalent. A Unix listener alone cannot safely make that
engine compatible. CI attempts the unchanged release and its own example with `DOCKER_HOST`
pointed at the private mode-0600 socket, retaining actual requests, exit status and output in
`docker-api-spike.json`. The attempt and result remain pending. No `docker-api` node feature is
advertised or shipped unless the unchanged example passes.

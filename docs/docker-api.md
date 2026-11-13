<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

# celld engines and the Docker socket experiment

The prepared native seam is in [integrations/celld/upstream](../integrations/celld/upstream/README.md),
based on public celld 0.6.3 at `8702f6d186c77698dbfb30cf29e3b1922a88f008`. It keeps celld's
coordinator and default Docker implementation and adds a pryx public-API implementation. Its own
container example passes in [CI 42077009774](https://github.com/cubepals/pryx/actions/runs/42077009774)
from the patched public source build, through both its local engine and the native pryx backend.
All 30 samples complete for both engines. GitHub blocks that run's final aggregate job on an account-level
runner-allocation restriction after every test job succeeds; the overall workflow is not green.

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
`docker-api-spike.json`. Run [42066544946](https://github.com/cubepals/pryx/actions/runs/42066544946)
completes the experiment: discovery passes, then the unmodified release fails building its
example image. BuildKit POST `/grpc` and its buildkit-container inspection receive explicit 501
replies. The [summary](checks/docker-api-spike-observed.json) retains actual requests and exit 1.
No start or first-request samples exist for this failed route (n=0). The spike remains unshipped
and no `docker-api` node feature is advertised.

The [first native experiment](checks/celld-native-initial.json) in the same run builds the
prepared patch and passes its three filesystem journal tests, then fails during the local
engine baseline before any accepted sample. The harness's log file was inside celld dev's
watched directory, causing hot reload to interrupt its first request. Logs now live outside
the unchanged example. The later complete comparison below includes both cohorts.

## Complete native comparison

The [complete summary](checks/celld-native-complete.json) is from CI 42077009774, source
8128648, on one two-core Intel Xeon Platinum 8370C host running Linux 7.0.0-1017-azure.
Both engines use the same patched celld 0.6.3 executable, development profile, debug information
and incremental compilation disabled. The Worker, Python server, Dockerfile and class configuration
are unchanged from the public release. Each cell's cleanup is outside its timer. Counts are n=30
for every row and backend; p50 is the sorted upper median and p90 the nearest-rank observation.

| operation | local engine p50 / p90 ms | native pryx p50 / p90 ms |
| --- | --- | --- |
| backend start | 142.00 / 154.00 | 215.00 / 233.00 |
| start through first HTTP response | 1881.78 / 1889.72 | 1274.87 / 1284.34 |
| warm HTTP response | 5.50 / 5.72 | 5.74 / 7.24 |

The native backend adds 79 ms to backend-start p90 and 1.52 ms to warm-response p90 on this
host. Its start-through-first-response sample is lower; that includes celld's readiness and
polling sequence, and is not attributed to an isolated transport or kernel optimization.
The seam is prepared and remains limited to the single-node development store as above.

The [repeated unmodified-release socket report](checks/docker-api-spike-complete.json) from
the same run again reaches discovery and explicitly unsupported image-build operations, exiting
with failure. It has no start/request samples. The original failed report remains retained.

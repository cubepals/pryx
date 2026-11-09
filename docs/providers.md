<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

# Machine providers

Control persists an acquisition identity before contacting a vendor. A lost create reply becomes
uncertain: reconciliation finds its deployment/acquisition labels before any further action. If
nothing is visible, it waits rather than risking another paid host. Boat additionally receives its
native Idempotency-Key. Manual SSH uses a root-owned acquisition marker and strict host verification.
Only the enrollment token control minted binds a node to an acquisition; labels confer no trust.
One-time join secrets are derived with HMAC-SHA256 from an environment key and the acquisition UUID; only their hashes reach Postgres.
Provider tokens are never stored in Postgres or exposed by operator responses.

Hetzner attaches an existing private network and firewall and runs cloud-init with the join line.
Its dynamic x86 catalog and gross hourly/monthly quotes come from the provider's server-types and
pricing endpoints. Quotes cover the server, not extra public IP charges, traffic, taxes specific to
an account, or other extras. Prices are preserved with acquisition/release events, never private
business constants. See [Hetzner API](https://docs.hetzner.cloud/reference/cloud).

Boat exposes VM creation, file and command endpoints, but its published API has no geographic region
selector or live price catalog. The operator supplies a logical region and explicit shape/price
catalog. A bootstrap first proves Docker and a working systemd, then installs the configured private
network and verifies a private address before joining. No live Boat VM was available here, so that
runtime support remains unproven. Credentials go through its authenticated file endpoint into a
0600 bootstrap file, never the command-history string; the file is deleted after execution.
See [Boat API](https://docs.boat.dev/openapi/boat-v1.yaml).

Manual hosts require a private IP, an SSH key and a pretrusted known_hosts entry. They are joined
with the same token flow but are never returned by control. Unknown provider hosts are reported as
orphans and retained. Failed, unused acquisitions can be cancelled through the operator API/CLI;
cancellation revokes enrollment under a row lock before deleting a known rental. Joined hosts must
use the ordinary drain/release path.

Fly Machines have an exec API and private networking, but that does not prove that the selected
Machine image supports the Docker daemon, kernel controls, and systemd used by pryxd. This
credential-free environment could not run that probe. No Fly adapter is shipped and no claim is
made that Docker is impossible there. See [Machines API](https://fly.io/docs/machines/api/) and
[remote builders](https://fly.io/docs/reference/remote-builder/).

Releases preserve all live local archive copies in the object store before removing a placement,
not only the latest snapshot. Active sandboxes keep a draining host alive. Lost/quarantined nodes,
manual nodes, unknown observed copies, insufficient healthy regional minimums, and missing stored
copies prevent release. Provider deletion is tracked through uncertainty and retried by the same
host identity; Hetzner waits for its GET to return not-found. No provider timeout declares a node lost.

## Join measurements

Measure from the durable acquire timestamp to the first authenticated, reconciled, runtime-up,
nonduplicate heartbeat accepted by Postgres. The trigger binds through the consumed enrollment
token. CI logs `acquireToHealthySeconds`, with its sample count; aggregate repeated independent
runs before publishing percentiles. No provider credentials were configured in this workspace.

| Shipped provider | n | p50 seconds | p90 seconds | Status |
| --- | ---: | ---: | ---: | --- |
| Hetzner | 0 | unavailable | unavailable | Live CI suite added; not run locally |
| Boat | 0 | unavailable | unavailable | No token/private network; Docker/systemd probe not run |
| Manual | 0 | unavailable | unavailable | No SSH host/key supplied |
| Fake (tests only) | 0 | not applicable | not applicable | Synthetic calls are not provisioning measurements |

CI reads the repository secret PRYX_HETZNER_TOKEN only on pushes to `refs/heads/main`.
Both the credential check and live test independently require that event and ref; branch pushes
and pull requests run the other suites without receiving provider credentials. The live test
creates temporary network, firewall and
SSH keys, restricts public ingress to the runner's SSH address, and uses strict known-host tunnels
for bootstrap, node API and snapshot storage. It invokes the operator CLI for acquire/release and
the public SDK for a real sandbox. Cleanup deletes only test-created resources. The step visibly
skips when the secret is absent. The rented box installs the released CLI and joins only on a
release-signed bootstrap offer, so the step hands control the current release's pryxd, pryx-agent
and bootstrap signature rather than the run's own build; a change to the join script or
daemon.json needs a new release before the live step passes again. The full Docker/e2e suite belongs to CI and was not run locally.

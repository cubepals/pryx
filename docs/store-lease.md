<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

# Experimental store lease

This feature is off on both host and control by default. Feature-off nodes keep the existing
execution lease: a control outage prevents unasked restarts but does not stop sandboxes, and an
operator supplies the fence before declaring a host lost. This object-store lease is separate.

## Host and control setup

A lease host requires Linux cgroup v2 with cgroup.kill, rootful Docker's cgroupfs driver, writable
cgroup controls, and a host/hypervisor that does not suspend leased execution. Drain/rest existing
sandboxes before enabling it. The daemon refuses startup unless it can prepare the cgroups and
start/fence a protected watchdog. Install deploy/pryxd-store-lease.conf as the documented systemd
drop-in, reload systemd, and configure Docker's native.cgroupdriver=cgroupfs deliberately on a
drained host. The ordinary unit retains ProtectControlGroups=yes.

```toml
[store_lease]
enabled = true
non_suspending_host = true
max_seconds = 60
```

On control set PRYX_STORE_LEASE=on. PRYX_STORE_LEASE_SECONDS defaults to 30 and
PRYX_STORE_LEASE_RENEW_SECONDS to 10; the host enforces its maximum and a renewal interval below
half of the usable term. Control uses its configured S3 store/runtime endpoint. At startup it
probes conditional create, rejected duplicate create, conditional update, and rejected stale
update. An incompatible store refuses enabled startup. Nodes must advertise store-lease in their
accepted heartbeat before control sends POST /v1/workloads/{id}/store-lease with that epoch.

The new request/object schemas come from Rust and are generated into the node contract. No spec,
label, original record field, public API field, or format-v1 name is renamed. Grants contain
presigned read/write URLs held only in memory; their Debug output omits credentials. URLs live
for one day and upkeep refreshes them. A longer control outage eventually expires them and stops
leased placements even if the store is reachable. Preserve the lease objects and disable bucket
lifecycle rules for this prefix; no operator or external program may overwrite/delete them.

## Authority and fencing

Each placement has a versioned object under deployment/store-leases/v1/workload.json, containing
its node, fresh daemon session, epoch, generation and store-time expiry. Control conditionally creates/updates it before
granting a node permission to run. A feature-gated GET on the same endpoint supplies the current in-memory daemon session; a restart or disk clone cannot share the prior session's live authority. The node GETs it, checks every identity field, and renews with
If-Match from that GET. Failed or stale writes grant no time. A late reply cannot revive authority.
HTTP Date supplies store time; CLOCK_BOOTTIME supplies local deadlines anchored before the
request. The node also checks the PUT response's Date, refuses cached authority, and fences two
seconds early for whole-second Date rounding and watchdog scheduling. The presigner leaves the
changing If-Match header unsigned; transport and store conditional-write guarantees remain required.

A protected process outside pryxd's service cgroup watches an stdin pipe and the boot clock. EOF,
invalid input or expiry freezes the placement's parent cgroup and writes cgroup.kill, independent
of Docker's API and Tokio. A root-only file lock and a preserved runtime directory serialize cgroup reuse across daemon restarts. The frozen parent prevents a Docker start that completes late from
running a descendant. Starts, resumes, replacement restarts and exec need a live grant and the
expected cgroup parent. Existing unleased execution is killed on enabled startup. A parent monitor
also fences if the watchdog exits. Deleting or fencing a copy ends renewal. A stopped or suspended
placement retains its lease for a quick wake; its request cost continues until rest/delete/fence.
Graceful daemon upgrades/restarts can stop leased processes: drain/rest before maintenance. Enabled hosts latch their policy in the non-secret store-lease-mode-v1 file. Disabling is refused until an operator drains/rests every copy, waits out all published terms, and deliberately removes that marker; restoring retained data then takes a new epoch. This prevents an ordinary configuration toggle from silently starting an old leased epoch without its fence.

Control never equates a network timeout with a fence. Once the store's Date passes the last
published expiry, control must win a conditional overwrite with an exact fence marker. A racing
renewal wins instead or loses to that marker; both cannot win the same ETag. Only then do a
Postgres proof and displacement commit together. A crash between the store write and database
commit is recovered by recognizing that exact generation marker. The whole node is not marked
lost. Recovery uses a ready stored snapshot, excludes the prior host and takes a new epoch;
missing snapshots or capacity leave the old data retained and placement displaced. Planned
handoffs also wait out any remaining published term after the prior copy stops renewing.

## Fault model and limits

This is an opt-in prototype for store/control partitions and daemon stalls/crashes on a trusted,
non-suspending Linux host. It assumes the independent watchdog and kernel run within the reserved
scheduling margin, and that the store supplies current Date plus linearizable conditional writes.
It is not a hard real-time guarantee against an arbitrarily stalled kernel, a paused hypervisor,
a compromised host, a live-memory VM clone, an externally killed watchdog plus daemon, or an administrator disabling
containment. Whole-host suspension must stay disabled; use an external power fence before
recovering such a host. non_suspending_host is an explicit host policy prerequisite, not a runtime
proof of a hypervisor's behavior. Do not enable automatic lease recovery where this fault model
cannot hold. No live fault measurements or production-readiness claim is made by the local units.

## Measurements

No privileged/root systemd test host was available here. The full Docker/e2e suites are CI-only
under AGENTS.md. The table records actual samples, not modeled successes:

| Measurement | n | p50 | p90 | Status |
| --- | ---: | ---: | ---: | --- |
| Store cut to sandbox stopped | 0 | unavailable | unavailable | CI collector added, not run locally |
| Daemon SIGSTOP to sandbox stopped | 0 | unavailable | unavailable | Independent-watchdog CI case, not run locally |
| Store renewal requests per sandbox-hour | 0 | unavailable | unavailable | CI counts actual node GET/PUTs over three windows and normalizes by elapsed time |

The analytical steady-state renewal count is two requests (GET + conditional PUT) per interval:
720 requests per placement-hour at the default ten seconds, excluding startup, failures, signing
and upkeep. Current control refresh adds two GETs per placement per minute (120 per hour), giving
840 total modeled steady-state requests. These are calculations, not measurements or store prices.
A watchdog process/thread and parent monitor also have overhead that has not been profiled here.

The CI lease step runs after the other e2e suites on its disposable Ubuntu runner, switches that
runner's Docker to cgroupfs, restarts its test stores, and runs:

```sh
PRYX_LEASE_E2E=1 PRYXD_SUDO=1 PRYXD_BIN=$PWD/target/debug/pryxd \
  DATABASE_URL=... S3_TEST_ENDPOINT=http://127.0.0.1:9000 \
  bun test control/e2e/store-lease.e2e.test.ts
```

It creates two real local nodes, cuts only root node traffic to a counting S3 proxy, records three
15-second renewal windows and three stop samples, asserts each stop precedes its ten-second term,
conditionally fences the expired generation, and restores snapshot data on the other node under a
higher epoch. A separate case SIGSTOPs pryxd and observes Docker's running state end while the
daemon remains stopped. Raw counts/windows and nearest-rank p50/p90 go to JSON and the job summary.
Replace the zero-sample rows only with completed run artifacts. Added CI runtime can be several
minutes; there is no claimed remote green result yet.

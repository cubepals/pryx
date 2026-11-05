<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

# agentOS on pryx

The `wasm` backend runs one agentOS VM per sandbox. pryxd owns the VMs through the published
Rust client and a separate native sidecar. Control retains placement, projects, units, epochs,
leases and lifecycle; clients use the same authenticated agent API and edge routes.

The dependency is agentOS **0.4.7**, published from
[`a4c049b2e25ceb2098889a307b71853526de2bdf`](https://github.com/rivet-dev/agentos/tree/a4c049b2e25ceb2098889a307b71853526de2bdf).
The current source and self-hosting boundary were also read at
[`4662e362fb93adcfbfc86bfd51b51855d463f371`](https://github.com/rivet-dev/agentos/tree/4662e362fb93adcfbfc86bfd51b51855d463f371).
The client and native protocol have no cross-version compatibility promise: both Rust crates
and the sidecar are pinned together. No agentOS source is vendored into pryx.

## Licence and distribution

agentOS is Apache-2.0 licensed. Sections 2 and 4 permit embedding and redistribution with
its licence and required notices retained; section 4 permits different terms for the aggregate.
That permits this dependency in pryx, without relicensing agentOS as FSL. Its Apache licence
and notice ship under the distribution's `licenses/` and `NOTICE` paths. Bundled software
retains its own terms and sources in the pinned upstream software tree.

`deploy/agentos.json` pins the Linux x86-64 GNU sidecar and Core software archive by npm
SHA-512 integrity, and pins the extracted sidecar and each default software package by SHA-256.
`scripts/fetch-agentos.py` verifies archives before extracting regular files, refuses links
and paths outside its staging root, checks the binary again, and never executes downloads.
The image includes the verified distribution. The release publishes the sidecar binary and
software/licence archive alongside pryxd, under the existing checksum and attestation steps.

The linked dependency is `agentos-client`; its same-version `agentos-sidecar-client` owns the
wire types. The native sidecar speaks its own framed protocol over stdio and an inherited
Unix socket. The static pryx-agent does not link agentOS. The daemon reuses that agent's
router through explicit virtual-process and guest-ingress adapters.

## State and lifecycle

The sandbox data directory contains `root.sqlite` for the VM's chunked SQLite root, plus a
separate `content/` directory mounted at the sandbox's configured data path. That explicit
agentOS host-directory mount is the only guest access to host files. The existing descriptor
walk serves the same content through the agent and stopped-node files API, including large
streams; it does not expose the root database, runtime journal or signing keys.

Snapshot, export, restore and fork copy the parent data directory, retaining both filesystem
parts. VM disposal completes before the filesystem is copied. Sleep disposes compute and wake
recreates it with the same root and content; processes, terminals, language contexts and open
connections do not survive that recreation. Stop/start follow the same filesystem rule.
The measured alternatives and the selected image default are in [tiers.md](tiers.md).

The shared sidecar has a no-network manager VM that keeps its worker alive between sandbox
creations. Its fixed memory cost is distinct from incremental VM memory. A private launcher
holds a kernel lock across sidecar exec and arms Linux's parent-death signal. A new daemon
cannot create VMs while the preceding worker can execute. Pending VM creation/disposal is
durably recorded before effects and remains pending after uncertain outcomes. Environment values,
including signing seeds and project secrets, are excluded from this journal. After daemon restart,
old VMs are gone and control must send a full ensure before a VM is recreated on its retained data.

The VM's initial identity matches its host-directory worker identity, so virtual permission
checks agree with native mount metadata. A root daemon drops its sidecar to
`wasm.worker_uid`/`worker_gid` (both default 60000, outside the default process UID pool),
clears supplementary groups and sets no-new-privileges; an unprivileged daemon keeps its own
UID. Reserve those worker IDs for the service and use consistent worker IDs across a fleet.
The worker has a separate private home/cache. Project and provider environment are removed;
proxy and CA configuration remain.
Host-owned `wasm.max_ingress_requests` (default eight) bounds simultaneous exposed-application
requests across VMs, in addition to each VM's connection and upstream frame bounds. Process
counts and JavaScript, Python and WASM heap settings come from the declared resource envelope.
These virtual runtime bounds are not per-VM Linux cgroups; the sidecar is a shared fault domain.
Requested CPU-millis count toward control admission, but this shared worker does not enforce a
per-VM CPU rate. Host service limits must bound the worker as a whole.

## API mapping

| pryx surface | agentOS operation |
| --- | --- |
| Create/start/wake | Create a VM with the same-version client, root database, explicit mount and host policy |
| Stop/sleep/delete compute | Dispose the VM and await acknowledgement |
| Run and processes | Spawn, retained output pages, exit observation, stdin and awaited signals |
| Terminal | Open shell, ordered output, stdin, resize and close |
| Files | The explicit persistent data mount, shared safe streaming path and agentOS virtual filesystem |
| Declared ports and expose | Fixed and spare-port/revision listeners forwarded to the VM's streaming HTTP API |
| Network | The VM network permission policy; `none` denies traffic outside its virtual network |
| Packages | Pinned-release registry names resolved and digest-checked before installation |

Application ingress supports HTTP over TCP. UDP is rejected at admission; raw TCP, TLS
passthrough and application WebSockets are unavailable through this pinned streaming HTTP API.
The agent API itself retains its authenticated WebSocket terminal and streaming routes.
All listeners use the host-selected audience addresses. Safe GET/HEAD requests may wait up to
three seconds for a newly starting listener; mutations and started responses are never repeated.
The default wasm terminal uses the bundled shell's minimal input backend to preserve pasted
multi-line commands; its rich line editor drops subsequent pasted lines in the pinned release.

Requests authenticate with the same scoped, expiring sandbox capabilities as the other tiers.
Project credentials never enter the guest. Commands execute inside the VM. A virtual PID is
never passed to a host signal call. Output remains bounded and sequenced; sidecar callbacks
only wake readers of retained output. Uncertain process termination fences that VM's work.

`image` is refused on wasm with `422 invalid_request`, field `image`, and problem
`this host runs the wasm tier; images are not available`. `packages` is optional, accepts
registry names rather than URLs or host paths, and is sent only to nodes advertising
`wasm-packages`. The eight bundled default packages need no runtime download. Additional
names use the matching release catalog and fail if it is unavailable; operators may configure
the upstream client's HTTPS registry mirror. No package mutation is retried automatically.

## Running agentOS as a pryx guest

Hosted agentOS has two services: Rivet's control plane and Rivet Services, which owns its
`agentOS` actor and VMs. The embedded Core library needs neither. Its public groups cover
processes, terminals, filesystem/mounts, JavaScript, TypeScript, Python, contexts, networking,
software and cron. Core supports ephemeral roots, chunked SQLite roots and chunked S3 mounts;
durable files survive recreation, while running processes and contexts do not.

The current `AgentOsSandboxProvider.start` boundary still supplies an external process and
filesystem client with disposal. The [prepared provider patch](../integrations/rivet/upstream/agentos/README.md)
places that external guest on a pryx sandbox-agent template with a stable creation key and a
service-held project key. It applies to the current source. This is a useful supported seam,
but it does not replace the hosted actor's VM identity, storage or ownership handoff with a
remote VM factory. A full hosted placement adapter would need those additional contracts.
The patch is prepared for review; it has not been submitted upstream.

## Local real-sidecar checks

Stage the distribution into a new directory, then run the ordinary Rust suite unprivileged:

```sh
python3 scripts/fetch-agentos.py /tmp/pryx-agentos-test
PRYX_AGENTOS_DISTRIBUTION=/tmp/pryx-agentos-test cargo test --locked
```

The wasm test uses the built pryxd as its private launcher. Missing assets are a setup failure;
the test does not skip, substitute a fake backend or require a container engine.

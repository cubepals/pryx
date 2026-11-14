<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->
# Host capabilities and guarantees

`pryxd probe --json` tests the current caller's access without changing host configuration.
It needs no node configuration or root. `pryx doctor` prints the same document as text;
`pryx doctor --json` preserves its JSON form. Evidence names what was tested and an action
names what would enable an unavailable capability. A permission failure is `unknown`, not
evidence that the kernel lacks the capability. Only `on` can become a promised guarantee.

The probe creates a disposable user namespace and uses a capability in its private UTS
namespace; asks the Landlock ABI; reads the daemon's own cgroup controllers; opens devices;
and, on x86_64 with KVM access, measures ten real guest HLT exits. Namespace and guest
children have a two-second deadline. Engine discovery asks for version and info, in order:
Unix `DOCKER_HOST`, system Docker, user Docker, user Podman, system Podman. The first complete
answer selects the engine. An explicit `docker.socket` in the configuration never falls back.
Remote engines are refused because they cannot establish this host's data bind paths. The report also covers the firewall table,
systemd, the user session, linger, architecture and the host C library.

`pryxd doctor --config …` checks the selected runtime. Process nodes check enforceable
Landlock rather than requiring Docker; Wasm nodes check their pinned distribution. State
ownership uses the same runtime-specific policy as `serve`.

`pryx doctor` also discovers the caller's protected installed configuration. With it,
`process-desktop` and `process-ssh` report the verified native bundle, host glibc, agent support,
namespaces and scoped-peer prerequisites. A bare probe keeps them off until a configured
installation can be checked. This inspection reads existing process history without creating
or repairing it, starting compute or downloading tools.

A public sandbox view carries `guarantees`, with a boolean for every name below. A false
value includes unknown or unannounced enforcement; it is never an implicit promise.
The view reads the latest node announcement. A resting sandbox with no node reports false.

| Guarantee | Current evidence |
| --- | --- |
| `separate-users` | A process uid pool, with an actual credential-drop probe; shared-uid process nodes report false |
| `memory-limit` | Container engine info reports kernel memory limits and a cgroup driver |
| `cpu-limit` | Engine info reports CPU quota and weights and a cgroup driver |
| `private-network` | The container backend's separate network namespace |
| `host-firewall` | Rootful engine with a readable installed nftables table; refreshed before heartbeats |
| `own-kernel` | False until the virtual-machine tier exists |
| `memory-sleep` | Process suspension or a rootful container engine with cgroup enforcement; false on Wasm and rootless engines |
| `browser-sandbox` | Observed browser launch policy plus usable namespaces and the approved container or verified native process policy |
| `private-pids` | The container backend's private PID namespace |

Containers do not claim a unique numeric host uid: several containers can use the same
configured uid. Their namespace boundaries remain the container tier's boundary. Use a
requirement only for a guarantee the deployment actually provides.

Creation accepts an optional list, for example:

```json
{"image":"alpine:3.22","require":["memory-limit","host-firewall"]}
```

`PRYX_REQUIRE=memory-limit,host-firewall` adds deployment requirements to every creation.
A request cannot remove a deployment requirement. Placement refuses missing guarantees
with `no_room` and names them. Wake checks them again, including when the spec has not
changed. Omitting requirements keeps the previous placement behavior.

Requested `egress`, `network:none`, `user`, `workdir` and nonempty `packages` require their
node feature on both create and wake. They are refused before a write or start when the
feature is missing. Default Internet networking remains compatible with older v1 nodes,
where the absent field already means Internet. CPU reservation remains admission metadata
and retains its existing feature gate; it never becomes an implied kernel quota.

Optional `requiredTools: ["desktop", "ssh"]` selects a process tier that announces those native
tools and checks them again before wake. It remains control placement intent rather than an
extra node request field. Native tool features are scoped to their tier; container or Wasm
requests cannot inherit the process tier's readiness. Existing container tool calls continue
to use their image and host-policy checks. See [native tools](native-tools.md) for prerequisites.

A container request with a memory limit is refused when the engine lacks its controller;
the refusal names `memory-limit`. An ordinary user can explicitly accept a reservation:

```json
{"image":"alpine:3.22","resources":{"memoryMb":256,"allowUnenforcedMemory":true}}
```

The node must announce `unenforced-memory` before control sends this field. Without a
controller, the sandbox reports `memory-limit: false` and 256 MiB still counts for admission.
Every capable engine applies the hard limit even when acceptance is true. This field cannot
remove deployment requirements, CPU limits or PID limits: a requested CPU quota, weight or
PID limit that the engine cannot enforce is refused. Omitted fields retain their original
spec and record bytes. Doctor explains missing controller delegation and slow VFS storage. Podman is currently
refused by name because its compatible API ignores the required bridge isolation option;
its adapter and hosted acceptance remain unfinished.

The static `suspend` protocol feature remains for compatibility. `memory-sleep` is the
additional statement that sleep preserves processes: Wasm data sleep disposes and recreates
the VM. A browser on a host without usable namespaces has no `browser-sandbox` promise;
the existing native-tool startup refusal remains. Nothing in this milestone adds a backend.

See [any-host progress](any-host-progress.md) for the exact commits, checks and outstanding
CI evidence. The probe reports facts about this caller; a privileged service can have
different access from an interactive user on the same machine.

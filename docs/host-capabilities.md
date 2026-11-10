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
children have a two-second deadline. Engine discovery reads info from system and user
Docker/Podman sockets and a Unix `DOCKER_HOST`. The report also covers the firewall table,
systemd, the user session, linger, architecture and the host C library.

`pryxd doctor --config …` checks the selected runtime. Process nodes check enforceable
Landlock rather than requiring Docker; Wasm nodes check their pinned distribution. State
ownership uses the same runtime-specific policy as `serve`.

A public sandbox view carries `guarantees`, with a boolean for every name below. A false
value includes unknown or unannounced enforcement; it is never an implicit promise.
The view reads the latest node announcement. A resting sandbox with no node reports false.

| Guarantee | Current evidence |
| --- | --- |
| `separate-users` | A process uid pool, with an actual credential-drop probe; shared-uid process nodes report false |
| `memory-limit` | Container engine info reports kernel memory limits and a cgroup driver |
| `cpu-limit` | Engine info reports CPU quota and weights and a cgroup driver |
| `private-network` | The container backend's separate network namespace |
| `host-firewall` | The node can read its installed nftables table; refreshed before heartbeats |
| `own-kernel` | False until the virtual-machine tier exists |
| `memory-sleep` | Process suspension or a rootful container engine with cgroup enforcement; false on Wasm and rootless engines |
| `browser-sandbox` | Usable user namespaces and the host's native-tool seccomp policy on a container node |
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

The static `suspend` protocol feature remains for compatibility. `memory-sleep` is the
additional statement that sleep preserves processes: Wasm data sleep disposes and recreates
the VM. A browser on a host without usable namespaces has no `browser-sandbox` promise;
the existing native-tool startup refusal remains. Nothing in this milestone adds a backend.

See [any-host progress](any-host-progress.md) for the exact commits, checks and outstanding
CI evidence. The probe reports facts about this caller; a privileged service can have
different access from an interactive user on the same machine.

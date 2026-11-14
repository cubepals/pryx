<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

# Virtual machines

The VM backend is under construction in K25 and is not registered for placement.
Requests naming `vm` are refused; other tiers remain available. The
[progress record](any-host-progress.md) identifies the exact commits and CI evidence,
including incomplete acceptance and measurements.

The backend needs Linux on x86_64, access to `/dev/kvm`, and a successful KVM VM
creation probe. A device pathname alone does not establish access. A host without
working KVM keeps the tier off; there is no software-emulation fallback. A virtual
host may need nested virtualization enabled by its operator. The measured host
capability is decisive, rather than its provider's name.

An ordinary-user monitor inherits Landlock filesystem confinement and seccomp
restrictions, including preservation of its kernel lifetime lease. Root launch uses
the unchanged Firecracker jailer with fixed non-root monitor identities. Root disk
staging, jailed boot, and the installed daemon's host authority are separate checks.
The daemon's existing service does not grant the jailer's mount/device authority.
Production root startup remains refused while that integration is unfinished.

Host configuration may name a bounded immutable VM descriptor installed with the
signed host release. It contains the artifact owner's archive and inventory pins.
Workloads cannot supply that descriptor, select host programs or change monitor
policy. Acquisition verifies the whole bundle before executing any component.
The monitor, published kernel, filesystem tools and network helper have pinned
inputs under [templates/vm-tools](../templates/vm-tools/). Their licence choices
are recorded in [decisions](decisions.md); copyleft tools and the kernel remain
separate programs with their corresponding source distribution.

An ordinary OCI image is pulled without an engine. Its layers become immutable
EROFS files; an ext4 file retains the writable overlay and data. Memory uses the
configured guest size, and CPU uses a positive whole vCPU count. Fractional vCPUs,
CPU weights and UDP forwarding currently refuse. The existing agent serves
authenticated exec, files, logs and TCP services through private vsock transport.
Host service listeners remain private.

Cold sleep stops the guest and retains its disk. Wake boots it again. Copying a
disk requires actual monitor retirement and independent filesystem validation;
recovery works on a complete staged copy before replacing current data. A
boot-scoped signed shutdown proves clean guest flushing. Daemon loss or an
uncertain launch never invents that proof. Memory-preserving sleep is outside K25.

Networking is still under acceptance. Disconnected guests receive no network
device or outbound helper. The ordinary-user implementation wraps the maintained
packet helper and host-owned hostname gateways, with trusted guest rules installed
before image code. Guest packet-vsock access is denied to workloads. Production
packet-flow, root TAP/firewall and node/edge acceptance remain unfinished.

The existing optional VM workflow runs the real KVM, registry, bundle, confinement
and disk cases. Root staging and jailed boot use real root authority there.
The full installed lifecycle and thirty-sample latency/memory cohorts must be
recorded in [tiers](tiers.md) before the milestone is complete.

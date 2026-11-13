# VM input experiments

K25 first measures published kernels and two unprivileged network paths with a real KVM in CI.
These files verify candidate pins before a monitor or kernel is executed. They do not build
kernels, publish a runtime bundle, announce VM support or substitute emulation.

Parts (`vm-tools/`):

- `candidates.json`: versioned primary monitor/kernel candidates, upstream archive digests and licences.
- `inventory.py`: bounded primary metadata/archive acquisition, source identity and SHA-256.
- `inspect.py`: bounded streamed member inspection and regular-file staging by content digest.
- `kernel-selection.json`: exact published kernel/monitor members and required built-in configuration.
- `spike-inputs.json`: pinned private Go toolchain and separate EROFS builder inputs.
- `prepare.py`: verify those tool inputs before private extraction.
- `spike-module-notices.json`: exact dependency versions and complete primary-notice digests.
- `notices.py`: verify and retain the actual Go dependency closure's complete notices.
- `go.mod` and `go.sum`: checksum-pinned Go experiment dependencies.
- `experiment/experiment.go`: application-owned report port for both fixture programs.
- `cmd/config/main.go`: configuration through the monitor owner's generated models.
- `cmd/guest/main.go`: static PID 1 filesystem and vsock probe.
- `kernel.py`: thirty real ordinary-user KVM boots and a disk-retaining reboot.

The inventory workflow runs on `any-host` pushes while manual dispatch is unavailable before
main. Its output records upstream asset identities, actual byte hashes and source commits.
Its downloads must match the committed primary-release digests before a later experiment boots a guest. A candidate inventory is not kernel,
network or confinement proof. The Firecracker CI kernel configuration examined in
[the K25 record](../../docs/checks/measurements/k25/firecracker-kernel-candidate.json) lacks EROFS;
the next candidate is Kata's already published kernel, with its exact upstream and configuration
sources retained. No pryx-built kernel is an alternative in this milestone.

The kernel probe measures launch to a validated EROFS/overlay/ext4 report over real vsock.
It keeps every sample and failed serial log. These minimal fixture measurements do not prove
production create, image pulling, agent latency, networking, monitor confinement or VM support.

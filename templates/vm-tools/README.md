# VM input experiments

K25 first measures published kernels and two unprivileged network paths with a real KVM in CI.
These files verify candidate pins before a monitor or kernel is executed. They do not build
kernels, publish a runtime bundle, announce VM support or substitute emulation.

Parts (`vm-tools/`):

- `candidates.json`: versioned primary monitor/kernel candidates, upstream archive digests and licences.
- `inventory.py`: bounded primary metadata/archive acquisition, source identity and SHA-256.
- `inspect.py`: bounded streamed member inspection and regular-file staging by content digest.

The inventory workflow runs on `any-host` pushes while manual dispatch is unavailable before
main. Its output records upstream asset identities, actual byte hashes and source commits.
Its downloads must match the committed primary-release digests before a later experiment boots a guest. A candidate inventory is not kernel,
network or confinement proof. The Firecracker CI kernel configuration examined in
[the K25 record](../../docs/checks/measurements/k25/firecracker-kernel-candidate.json) lacks EROFS;
the next candidate is Kata's already published kernel, with its exact upstream and configuration
sources retained. No pryx-built kernel is an alternative in this milestone.

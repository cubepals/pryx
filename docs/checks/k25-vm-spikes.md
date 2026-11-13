# Published-kernel and network experiments

K25 is still in progress. These experiments select inputs for the production VM implementation;
they do not advertise a VM tier or prove production lifecycle, confinement or agent performance.

The separate monitor is Firecracker 1.18.0 (Apache-2.0). The published kernel is Kata Containers
4.3.1's `vmlinux-6.18.56-208` (Linux GPL-2.0), with its matching published configuration.
[The catalogue](../../templates/vm-tools/candidates.json) pins primary archive hashes and source
commits; [the selected members](../../templates/vm-tools/kernel-selection.json) pin exact extracted
bytes. No kernel was built by pryx. The examined Firecracker CI kernel lacks EROFS and was
rejected before boot. Archive paths and links are never extracted onto host paths.

## Kernel

Source `9402ef5` passes [the real-KVM job](https://github.com/cubepals/pryx/actions/runs/43442622901/job/131943829577)
as ordinary host UID 1001. All thirty cold guests prove EROFS, an unchanged lower layer,
ext4-backed overlay writes, virtio block and guest-to-host vsock. A separate reboot retains the
disk and increments its boot count. Guest Linux 6.18.56 differs from host 7.0.0-1017-azure.

| Minimal fixture operation | Samples | p50 ms | p90 ms |
| --- | ---: | ---: | ---: |
| Launch to validated filesystem/vsock report | 30 | 717.035 | 725.265 |
| Disk-retaining reboot | 1 | 724.471 | 724.471 |

The guest has one vCPU and 256 MiB. The root image is 4,460,544 bytes; initial image preparation
takes 21.341 ms. The first blocked monitor has 56,812 kB resident memory. These are minimal
fixture measurements, not production create, agent, idle-memory or image-pull measurements.
The entire hosted job takes 67 seconds.

[Raw kernel observations](measurements/k25/kernel-spike-9402ef5.json) and
[verified artifact provenance](measurements/k25/kernel-provenance-9402ef5.json) retain the evidence.
The initial duplicate-devtmpfs-mount failure remains in the same measurements directory.

## Network

Source `187136d` passes [both real network cohorts](https://github.com/cubepals/pryx/actions/runs/43450896770/job/131972447502).
The same runner boots thirty guests per path, with the same published kernel, pinned Alpine
3.24.4 image, EROFS/ext4 overlay, one vCPU, 256 MiB and 1500-byte fixture MTU.

The paths are pasta `0.0~git20261002.cba3570` behind a bridge/TAP in its private user and network
namespace, and gvisor-tap-vsock 0.9.0's maintained packet transport over vsock. GPL utilities stay
separate executables. Each input and its actual source/licence identity is retained. Neither
TCP/IP nor the packet framing is implemented by pryx.

| Observation | vsock p50 / p90 | pasta p50 / p90 |
| --- | ---: | ---: |
| Helper preparation plus launch to validated guest report, ms | 840.364 / 928.266 | 866.682 / 922.678 |
| Warm HTTP response, ms | 0.648 / 0.714 | 0.372 / 0.409 |
| 16-MiB transfer, MiB/s | 101.167 / 107.639 | 110.490 / 135.422 |
| Outbound DNS lookup, ms | 2.752 / 4.168 | 2.027 / 2.135 |

Each row has thirty raw observations per path; percentiles use nearest ranks 15 and 27. The
startup row includes guest filesystem checks, DHCP/network readiness, management-denial checks
on vsock, the HTTP requests, bulk transfer and DNS. It excludes image preparation and is not a
production create-to-ready measurement. DNS queries use the host's actual resolver through the
maintained helper. The HTTP server uses TCP_NODELAY equally for both paths. Earlier buffered
cohorts remain separate and are not retroactively corrected. The complete optional job takes
148 seconds, including the thirty minimal kernel guests and a disk reboot.

Every vsock guest fails to reach host management through both the packet transport and gateway
API. The private host API remains usable; its gateway credential stays only in host memory.
The guest-facing listener serves only the maintained packet-connect route. Pasta's observed
namespace maps host UID 1001, excludes host root and differs from the initial host network
namespace. These tests do not prove production monitor confinement or hostname egress policy.

[Raw network observations](measurements/k25/network-spike-187136d.json) and
[verified artifact provenance](measurements/k25/network-provenance-187136d.json) retain all samples.
Earlier setup, interface and DNS failures also remain recorded. Both candidates now work;
the separate vsock test with host user namespaces and TUN unavailable is the remaining
prerequisite experiment before the production network choice is finalized.

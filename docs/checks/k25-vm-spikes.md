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
Earlier setup, interface and DNS failures also remain recorded. Both candidates complete this cohort; later repeats and the missing-prerequisite experiment
are recorded below.

A repeat on source `0b8afec` passes all thirty vsock guests and thirteen pasta guests, then the
fourteenth pasta bulk response stalls after 1,890,748 bytes and hits the unchanged two-second
client deadline. Its complete partial report and serial log remain under `measurements/k25/`.
That stall remains unresolved. The next run adds bounded upstream pasta logs and real guest
TCP_INFO observation before the same deadline; it changes neither the deadline nor the transfer.
The separate missing-prerequisite case runs independently of the comparison's outcome, while
the aggregate still fails if any original comparison assertion fails.

Source `194c841` independently passes the full vsock guest with host user namespaces disabled
and host TUN inaccessible; pasta refuses actual namespace creation. The guest still proves DNS,
HTTP, bulk transfer and both management denials. This case has one guest and its own report;
it does not replace either thirty-sample cohort. The same run retains a later pasta transfer
stall, so the aggregate is failed.

Choose the maintained vsock helper for rootless production networking: its measured HTTP and
throughput cost is acceptable for the broader proved prerequisites. Rootful networking still
needs its separate jailer/TAP/firewall implementation and proof. The repeated pasta diagnostic
shows its learned guest MAC changing to a namespace-interface MAC during a transfer; the guest
TCP receive window remains open. Test suppression of private bridge IPv6 control traffic in
the already IPv4-only comparison, and retain actual link MACs and per-interface IPv6 settings.
The initial host namespace must remain untouched. This suspected cause still needs CI proof.


Source `8b2279f` passes [the complete experiment](https://github.com/cubepals/pryx/actions/runs/43458372918/job/131998442387):
all thirty guests per path and the separate missing-prerequisite guest pass. Retain
[its raw observations](measurements/k25/network-spike-8b2279f.json),
[prerequisite observations](measurements/k25/network-prerequisites-8b2279f.json) and
[verified provenance](measurements/k25/network-provenance-8b2279f.json). Every actual private
interface has IPv6 disabled. The helper nevertheless learns a bridge-interface MAC briefly
in five samples after seeing the VM MAC, before the validated guest report. Thus IPv6
suppression alone is not a proved repair of the earlier stalls; the earlier failures remain
unresolved. These diagnostic cohorts include upstream helper debug logging, so use the
unmodified successful cohort above for the recorded network comparison.


## Filesystem bundle component

Source `7efb280` passes [the component and real-kernel job](https://github.com/cubepals/pryx/actions/runs/43464154294/job/132018658215).
The component contains four unchanged Alpine filesystem programs and their fifteen-file
musl/library/configuration closure. Original apk-tools verifies every package signature;
the same packages fail with an empty trust directory. Complete upstream sources, recipe
inputs and full notices are retained before execution. The official source mirror supplies
identical pinned bytes when the primary source denies the hosted runner.

The [relocation report](measurements/k25/filesystem-relocation-7efb280.json) proves a real EROFS
round trip and populated ext4 integrity without root or a mount. The
[full boot report](measurements/k25/kernel-filesystems-7efb280.json) identifies the exact
[component manifest](measurements/k25/filesystem-manifest-7efb280.json) used for all thirty
boots and the disk-retaining reboot; [artifact provenance](measurements/k25/filesystem-provenance-7efb280.json)
retains its verified identity. This component does not yet provide runtime acquisition,
OCI image preparation or a production VM backend.

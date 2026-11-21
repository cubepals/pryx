# Guest service activation

The native, dispatch-only candidate compares three fresh ordinary-user KVM
guests at the first successful production authenticated health response. They
share the same one-vCPU plan, kernel, OCI base, disks prepared by the same tools,
EROFS bootstrap bytes, and host/guest management protocols. Fifteen complete
triples alternate the whole order between stock–eager–activated and
activated–eager–stock. Every pair changes direction each round. The eager
control occupies the middle position throughout. Raw events include position
and direction. The existing stock-only baseline runs first on a
separate immutable revision. Select only `lab_agent_candidate=true` for the
subsequent admission. The x86 job has 55 minutes including its stock setup.
The candidate remains experimental until both native architectures have evidence.

| Arm | Kernel PID 1 executable | Real serving executable | Service startup |
| --- | --- | --- | --- |
| Stock | `/init` | `/.pryx-bootstrap/init` | Stock |
| Eager split | `/lab-control` | `/.pryx-bootstrap/init` | Stock |
| Activated split | `/lab-init` | `/.pryx-bootstrap/lab-service` | First raw or framed management connection from host CID |

The C entry runs the maintained Rust PID 1 supervisor and its original
CID-bound bootstrap, mount, cgroup, network, signed sleep, resume, fork and
shutdown code. Its small nonroot child waits on both fixed VSOCK management
ports. After accepting a host-CID stream, it transfers both listen queues and
the first unread connection through in-place exec. The receiving build compiles
the actual serving entry with a feature for those three inherited descriptors.
It checks the local guest CID and ports, the socket roles, the remote host CID,
the descriptor identities, its parent, its nonroot IDs and no-new-privileges.
It sets close-on-exec, binds the other configured listeners, and installs the
existing VM policy before any handler serves a request. The original byte relay,
HTTP/2 framing and HTTP authorization handle the first response.

The stock server binds its own VSOCK listeners. A helper forwarding only the
first accepted stream to it would have to close both existing listeners before
the stock bind. Concurrent peers could arrive between the last accept and
close. Keeping the descriptors makes the stock bind fail. The opt-in receiver
preserves those queues. A normal host makes a sandbox live after its framed
health check; a memory fork requires an accepted sleep image. The original
PID 1 owns the signed sleep and shutdown handlers before the child starts.
After activation the child keeps its PID, cgroup, parent and stdin identity
handover channel.

The stock supervisor places the existing signing credential in its child's OS
environment. In the activation arm that environment remains present while the
gate waits. The small C code and its Rust socket gate never access an environment
value or parse, sign or log a credential. The gate verifies that it is nonroot,
non-dumpable and under no-new-privileges. The receiver restores non-dumpability
after exec. The original acceptance checks that neither the credential nor the
bearer token appeared in the complete guest serial output.

Two additional fresh boots use a barrier-equipped test supervisor and stay
outside the timed triples. They wait for the first host connection to carry
bytes, then for a second connection on that same fixed listen queue. The
guest leaves those bytes unread before exec. Raw HTTP/1.1 and framed HTTP/2
run separately. In the framed case the first bytes can be the HTTP/2 preface.
The host verifies the inner authorized HTTP 200 response on the first original
connection with a fresh signed nonce, sandbox, epoch and serving PID, and the
normal 401 on the second. Each connection sends one HTTP request.

Readiness uses the production framed management forwarder and the production
retry. Authenticated guest exec then reads PID 1 and serving-process VmRSS,
verifies both role command lines and checks that PID 1 parents the serving
process. The report shows p50/p95 for each role and the within-boot sum.
That sum can count shared pages twice. The separate activated runtime tests
exercise the normal signed sleep, wake and fork from a privately regenerated
runtime inventory. Its launcher and service policy match the stock bundle.
Only aggregate observations, inventory/configurator hashes, outcomes and host
facts enter CI artifacts.

Parts (`candidate/`):

- `pid1.c` and `supervisor.rs` enter the maintained guest supervisor and child credential drop.
- `activation.rs` binds the two fixed queues, passes descriptors and admits the first host stream.
- `service.rs` builds the optional receiver from the real serving entry in a distinct output path.
- `build.sh` links the isolated role against the Rust target's static musl runtime.
- `config.py` builds two fixed Go configurators and checks their full outputs against stock.
- `trial.rs` runs the two functional boots and 15 paired triples.
- `bundle.py` checks the stock inventory and recomputes a private candidate archive and descriptor.
- `prove.sh` stages the binaries and executes the native acceptance and signed lifecycle proofs.
- `evidence.py` validates observations and computes role-specific comparisons.
- `test_candidate.py` checks partial cohort refusal and the private archive inventory.
- `README.md` explains the variants, trust boundary and measurement endpoint.

The host first-request proof in `request/main.go` and its Go tests in
`request/main_test.go` enter the pinned Go module only in the private build.
The source receipt records the production revision, experiment revision and
fixture patch hashes.

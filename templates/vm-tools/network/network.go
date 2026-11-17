// SPDX-License-Identifier: FSL-1.1-ALv2
// Package networkfixture names the upstream HTTP paths that its API does not export.
// Source: gvisor-tap-vsock 0.9.0 pkg/virtualnetwork/mux.go and pkg/services/forwarder.go.
//
// Parts (`network/`):
// - cmd/proxy/main.go: maintained stack behind a restricted guest transport.
// - cmd/proxy/production.go: inherited-listener runtime profile without management services.
// - cmd/proxy/stack.go: the maintained parts assembled with every forwarder behind admission.
// - cmd/proxy/admission.go: which destinations the host dials for a guest, from its egress.
// - cmd/proxy/route.go: the kernel's route type for a destination, for admission.
// - cmd/proxy/admission_test.go: each refusal, the kernel's answer for the host's own addresses,
//   and real host loopback services through the stack.
// - cmd/proxy/tap.go: attached TAP queue adapter for the maintained non-stream transport.
// - cmd/proxy/tap_test.go: actual packet-socket ARP/cancellation and kernel non-TAP refusal.
// - cmd/guest/main.go, lease.go and static.go: trusted DHCP or host-selected TAP configuration.
// - cmd/guest/lease_test.go: real no-follow resolver link checks.
// - cmd/guest/carrier.go and carrier_test.go: the first DHCP request waits for the forwarder's TAP carrier.
// - cmd/probe/main.go: real guest networking measurements and management denials.
// - cmd/probe/policy.go: inherited packet-transport denials as an unprivileged workload.
// - inputs.json: immutable Alpine and pasta source inputs.
// - module-notices.json: pinned dependency closure and complete primary-notice digests.
// - go.mod, go.sum: the separate comparison module and unchanged upstream forwarder tool.
// - go.work, go.work.sum: local main modules for network-directory cache verification.
// - README.md: experiment scope and listener boundaries.
package networkfixture

const ExposePath = "/services/forwarder/expose"
const StatsPath = "/stats"

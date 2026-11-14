// SPDX-License-Identifier: FSL-1.1-ALv2
// Package networkfixture names the upstream HTTP paths that its API does not export.
// Source: gvisor-tap-vsock 0.9.0 pkg/virtualnetwork/mux.go and pkg/services/forwarder.go.
//
// Parts (`network/`):
// - cmd/proxy/main.go: maintained stack behind a restricted guest transport.
// - cmd/guest/main.go: trusted guest loopback and maintained DHCP configuration.
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

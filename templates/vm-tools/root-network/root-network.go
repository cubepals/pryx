// SPDX-License-Identifier: FSL-1.1-ALv2
// Package rootnetwork wraps maintained CNI programs for root TAP experiments only. It changes
// no original-host route, forwarding setting or firewall and does not enable placement.
//
// Parts (`root-network/`): go.mod and go.sum pin the independent dependency closure;
// namespace.go owns anonymous namespace descriptors; handoff.go and handoff_test.go own
// bounded descriptor delivery, rejection cleanup and real-socket acceptance; module-notices.json pins
// original licence texts; cmd/tap-probe/main.go checks actual
// device creation, ordinary-owner open and identity-bound cleanup in the existing VM CI job.
// protocol.go owns the unchanged private metadata shapes and descriptor roles;
// cmd/scope/packet.go submits one bounded ordinary launch plan and grants its kernel identity;
// cmd/scope/main.go grants actual namespace/listener descriptors and waits on the monitor lease
// before retiring one scope; cmd/tap-probe/scope.go checks that actual process and lease;
// cmd/tap-probe/bridge.go checks actual bridge topology, firewall hooks and retirement.
// It is not installed or registered as a production worker.
// boundary.go reads opaque policy inputs from fixed protected executables; bridge.go owns
// the two-port anonymous L2 bridge and queue; bridge_firewall.go installs its independent
// public IPv4 boundary; cmd/scope/bridge.go selects that private parser path.
// authority.go owns unchanged protected root program/directory checks; packet.go owns
// fixed ordinary packet-program staging, configuration grants and confirmed retirement.
// firewall.go owns hostname-gateway admission and kernel readback.
// scope.go owns one private TAP and local addressing; listeners.go binds actual gateway
// descriptors there; credentials.go reads actual drop-user authority on fixture failure; cmd/tap-probe/gateways.go checks serving them in the parent namespace.
package rootnetwork

import "github.com/containernetworking/cni/pkg/types"

// Tap is the maintained plugin's configuration with fixed host-selected ownership.
// Its embedded API type owns the CNI spelling and versioned protocol fields.
type Tap struct {
	types.NetConf
	Owner uint32 `json:"owner"`
	Group uint32 `json:"group"`
}

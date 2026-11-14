// SPDX-License-Identifier: FSL-1.1-ALv2
// Package rootnetwork wraps maintained CNI programs for root TAP experiments only. It changes
// no original-host route, forwarding setting or firewall and does not enable placement.
//
// Parts (`root-network/`): go.mod and go.sum pin the independent dependency closure;
// namespace.go owns anonymous namespace descriptors; handoff.go and handoff_test.go own
// bounded descriptor delivery and its real-socket acceptance; module-notices.json pins
// original licence texts; cmd/tap-probe/main.go checks actual
// device creation, ordinary-owner open and identity-bound cleanup in the existing VM CI job.
// cmd/scope/main.go grants actual namespace/listener descriptors and waits on the monitor lease
// before retiring one scope; it is not installed or registered as a production worker.
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

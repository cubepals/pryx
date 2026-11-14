// SPDX-License-Identifier: FSL-1.1-ALv2
// Package rootnetwork wraps maintained CNI programs for root TAP experiments only. It does
// not configure forwarding, change the host firewall, allocate VM addresses or enable placement.
//
// Parts (`root-network/`): go.mod and go.sum pin the independent dependency closure;
// module-notices.json pins its original licence texts; cmd/tap-probe/main.go checks actual
// device creation, ordinary-owner open and identity-bound cleanup in the existing VM CI job.
package rootnetwork

import "github.com/containernetworking/cni/pkg/types"

// Tap is the maintained plugin's configuration with fixed host-selected ownership.
// Its embedded API type owns the CNI spelling and versioned protocol fields.
type Tap struct {
	types.NetConf
	Owner uint32 `json:"owner"`
	Group uint32 `json:"group"`
}

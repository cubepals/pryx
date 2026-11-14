// SPDX-License-Identifier: FSL-1.1-ALv2
// Select only fixed host programs for a private scope. Workload policy never supplies
// a replacement IPv4 boundary. The empty arguments retain the existing hostname path.
package main

import (
	"context"
	"fmt"
	"os"

	rootnetwork "cubepals/pryx/vm-root-network-experiment"
)

type hostedScope interface {
	Files(rootnetwork.Ports) ([]*os.File, error)
	Grant() rootnetwork.ScopeHandoff
	Close(context.Context) error
}

func openScope(ctx context.Context, directory, cache string, uid, gid uint32, agent, packet string) (hostedScope, error) {
	if agent == "" && packet == "" {
		return rootnetwork.Open(ctx, directory, cache, "vm0", uid, gid)
	}
	if agent == "" || packet == "" {
		return nil, fmt.Errorf("Root public bridge requires both fixed protected host programs")
	}
	inputs, err := rootnetwork.ReadBridgeInputs(ctx, agent, packet)
	if err != nil {
		return nil, err
	}
	return rootnetwork.OpenBridge(ctx, directory, cache, "vm0", uid, gid, inputs)
}

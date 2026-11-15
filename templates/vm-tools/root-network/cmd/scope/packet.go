// SPDX-License-Identifier: FSL-1.1-ALv2
// Read one bounded private launch plan for the already fixed ordinary packet agent.
// Configuration is written through the granted file, never through a privileged path.
// EOF requests retirement; main retains the actual monitor lease before stopping anything.
package main

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"

	rootnetwork "cubepals/pryx/vm-root-network-experiment"
)

func runPacket(ctx context.Context, packet *rootnetwork.PacketProcess, queue *os.File, grant rootnetwork.ScopeHandoff) error {
	input := bufio.NewReaderSize(os.Stdin, rootnetwork.MaxHandoffBytes+1)
	plan, err := input.ReadSlice('\n')
	if err != nil || len(plan) > rootnetwork.MaxHandoffBytes {
		return fmt.Errorf("Packet launch plan is missing, unterminated or exceeds its bound: %v", err)
	}
	if err := packet.Start(ctx, plan, queue); err != nil {
		return err
	}
	identity, err := packet.Identity()
	if err != nil {
		return err
	}
	defer identity.Close()
	grant.Descriptors = []rootnetwork.ScopeRole{rootnetwork.ScopePacketIdentity}
	metadata, err := json.Marshal(grant)
	if err != nil {
		return err
	}
	if err := rootnetwork.SendFiles(int(os.Stdout.Fd()), metadata, []*os.File{identity}); err != nil {
		return err
	}
	ended := make(chan error, 1)
	go func() {
		var command [1]byte
		count, err := input.Read(command[:])
		if count != 0 || !errors.Is(err, io.EOF) {
			ended <- fmt.Errorf("Packet lifetime requires parent EOF: %v", err)
		} else {
			ended <- nil
		}
	}()
	select {
	case err := <-ended:
		return err
	case <-packet.Exited():
		return fmt.Errorf("Confined packet process exited before its scope retired")
	}
}

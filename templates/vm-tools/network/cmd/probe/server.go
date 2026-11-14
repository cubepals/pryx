// SPDX-License-Identifier: FSL-1.1-ALv2
// A disposable unprivileged TCP service proves raw forwarding and bytes after client EOF.
// This program exists only in the acceptance bootstrap and never in the production bundle.
package main

import (
	"fmt"
	"io"
	"net"
	"os"

	"cubepals/pryx/vm-experiment/experiment"
)

func serve(port uint, ready string) error {
	if os.Geteuid() == 0 || port == 0 || port > uint(^uint16(0)) || ready == "" {
		return fmt.Errorf("The forwarding fixture requires non-root guest IDs and a ready file")
	}
	listener, err := net.Listen("tcp", net.JoinHostPort("127.0.0.1", fmt.Sprint(port)))
	if err != nil {
		return err
	}
	defer listener.Close()
	if err := os.WriteFile(ready, []byte("ready\n"), 0o600); err != nil {
		return err
	}
	for {
		connection, err := listener.Accept()
		if err != nil {
			return err
		}
		go func() {
			defer connection.Close()
			data, err := io.ReadAll(io.LimitReader(connection, experiment.NetworkBulkBytes+1))
			if err != nil || len(data) > experiment.NetworkBulkBytes {
				return
			}
			connection.Write([]byte(experiment.ForwardPrefix))
			connection.Write(data)
			connection.Write([]byte(experiment.ForwardSuffix))
		}()
	}
}

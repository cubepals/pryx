// SPDX-License-Identifier: FSL-1.1-ALv2
// Keep the same connected UDP DNS socket across a sleep and the restores of a fork and its source.
// Fresh sockets miss conntrack mappings that still point at the gateways of the original boot.
package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"strconv"
	"time"
)

// The resolver closes its connection after each lookup; this fixture keeps the owned socket.
type retainedUDP struct{ *net.UDPConn }

func (retainedUDP) Close() error { return nil }

func retainDNS(path, ready string) error {
	data, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	var fixture productionFixture
	if err := json.Unmarshal(data, &fixture); err != nil {
		return err
	}
	if os.Geteuid() == 0 || fixture.Hostname == "" || fixture.DNS == 0 || ready == "" {
		return fmt.Errorf("Retained DNS acceptance requires an ordinary workload and complete fixture")
	}
	remote, err := net.ResolveUDPAddr("udp4", net.JoinHostPort("203.0.113.1", strconv.Itoa(int(fixture.DNS))))
	if err != nil {
		return err
	}
	socket, err := net.DialUDP("udp4", nil, remote)
	if err != nil {
		return err
	}
	defer socket.Close()
	return retainedLookups(socket, fixture.Hostname, ready)
}

func retainedLookups(socket *net.UDPConn, hostname, ready string) error {
	resolver := &net.Resolver{PreferGo: true, Dial: func(context.Context, string, string) (net.Conn, error) {
		return retainedUDP{socket}, nil
	}}
	for phase := 0; phase < 3; phase++ {
		if phase != 0 {
			until := time.Now().Add(time.Minute)
			for {
				if _, err := os.Stat(fmt.Sprintf("%s.continue%d", ready, phase)); err == nil {
					break
				} else if !os.IsNotExist(err) {
					return err
				}
				if time.Now().After(until) {
					return fmt.Errorf("Retained DNS continuation %d did not arrive", phase)
				}
				time.Sleep(10 * time.Millisecond)
			}
		}
		ctx, cancel := context.WithTimeout(context.Background(), requestTimeout)
		addresses, err := resolver.LookupIP(ctx, "ip4", hostname)
		cancel()
		if err != nil || len(addresses) == 0 {
			return fmt.Errorf("Retained UDP DNS after phase %d: %v", phase, err)
		}
		// The actual local endpoint is unchanged in every phase and in every memory copy.
		if err := publishEndpoint(fmt.Sprintf("%s%d", ready, phase), socket.LocalAddr().String()); err != nil {
			return err
		}
	}
	return nil
}

// Make the ready name visible only after the endpoint has been written and closed.
func publishEndpoint(path, endpoint string) error {
	file, err := os.CreateTemp(filepath.Dir(path), ".dns-ready-*")
	if err != nil {
		return err
	}
	defer os.Remove(file.Name())
	_, writeErr := file.WriteString(endpoint)
	if err := errors.Join(writeErr, file.Close()); err != nil {
		return err
	}
	return os.Rename(file.Name(), path)
}

// SPDX-License-Identifier: FSL-1.1-ALv2
// Adopt an already attached private TAP queue without opening a device or changing namespaces.
// The pinned virtual network owns all packet parsing, switching and forwarding through its
// existing non-stream BESS transport, behind the same admission as the vsock profile. This
// adapter supplies only Go's net.Conn interface; no HTTP management, NAT-to-host alias or
// host-facing listener is created.
package main

import (
	"context"
	"encoding/json"
	"fmt"
	"net"
	"os"
	"os/signal"
	"syscall"

	"github.com/containers/gvisor-tap-vsock/pkg/types"
	"golang.org/x/sys/unix"
)

const tapDescriptorFlag = "tap-packet-fd"

type tapAddress string

func (address tapAddress) Network() string { return "tap" }
func (address tapAddress) String() string  { return string(address) }

type tapConnection struct{ *os.File }

func (connection tapConnection) LocalAddr() net.Addr  { return tapAddress(connection.Name()) }
func (connection tapConnection) RemoteAddr() net.Addr { return tapAddress(connection.Name()) }

func attachedTap(descriptor int, name string) (*os.File, error) {
	if descriptor <= int(os.Stderr.Fd()) || name == "" {
		return nil, fmt.Errorf("Packet TAP requires a separately inherited attached queue")
	}
	request, err := unix.NewIfreq("")
	if err == nil {
		err = unix.IoctlIfreq(descriptor, unix.TUNGETIFF, request)
	}
	if err == nil && (request.Name() != name || request.Uint16()&(unix.IFF_TAP|unix.IFF_TUN|unix.IFF_NO_PI|unix.IFF_VNET_HDR) != unix.IFF_TAP|unix.IFF_NO_PI) {
		err = fmt.Errorf("Actual inherited TAP identity or packet format differs from its grant")
	}
	if err == nil {
		// NewFile must observe O_NONBLOCK to use Go's poller; cancellation can then
		// close a blocked TAP read without leaving a native reader behind.
		err = unix.SetNonblock(descriptor, true)
	}
	if err != nil {
		unix.Close(descriptor)
		return nil, err
	}
	file := os.NewFile(uintptr(descriptor), "owned packet TAP")
	return file, nil
}

func serveTap(path string, descriptor int) error {
	if os.Geteuid() == 0 {
		return fmt.Errorf("Packet TAP stack requires an ordinary user")
	}
	configuration, err := readRuntime(path)
	if err != nil {
		return err
	}
	if configuration.Parent != os.Getppid() || configuration.Parent <= 0 || configuration.Concurrency == 0 {
		return fmt.Errorf("Packet TAP requires its real parent and admitted capacity")
	}
	file, err := attachedTap(descriptor, configuration.TapInterface)
	if err != nil {
		return err
	}
	defer file.Close()
	profile, err := productionMetadata()
	if err != nil {
		return err
	}
	policy, err := newAdmission(profile.Host, configuration.Egress)
	if err != nil {
		return err
	}
	network, err := newPacketStack(profile, stackOptions{policy: policy, concurrency: int(configuration.Concurrency)})
	if err != nil {
		return err
	}
	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGTERM, syscall.SIGINT)
	defer stop()
	finished := make(chan error, 1)
	go func() { finished <- network.Accept(ctx, tapConnection{file}, types.BessProtocol) }()
	if err := json.NewEncoder(os.Stdout).Encode(profile); err != nil {
		return err
	}
	if err := os.Stdout.Close(); err != nil {
		return err
	}
	select {
	case <-ctx.Done():
		return nil
	case err := <-finished:
		return err
	}
}

// SPDX-License-Identifier: FSL-1.1-ALv2
// Exercise the actual maintained non-stream network stack over kernel packet sockets.
// This proves the net.Conn adapter and cancellation, never a real privileged TAP or firewall.
package main

import (
	"bytes"
	"context"
	"errors"
	"net"
	"os"
	"testing"
	"time"

	"github.com/containers/gvisor-tap-vsock/pkg/types"
	"github.com/containers/gvisor-tap-vsock/pkg/virtualnetwork"
	"github.com/google/gopacket"
	"github.com/google/gopacket/layers"
	"golang.org/x/sys/unix"
)

func TestPacketAdapterUsesMaintainedARP(t *testing.T) {
	pair, err := unix.Socketpair(unix.AF_UNIX, unix.SOCK_SEQPACKET|unix.SOCK_CLOEXEC|unix.SOCK_NONBLOCK, 0)
	if err != nil {
		t.Fatal(err)
	}
	client, device := os.NewFile(uintptr(pair[0]), "actual packet client"), os.NewFile(uintptr(pair[1]), "actual packet device")
	defer client.Close()
	defer device.Close()
	profile, err := productionMetadata()
	if err != nil {
		t.Fatal(err)
	}
	stack, err := virtualnetwork.New(&types.Configuration{MTU: profile.MTU, Subnet: profile.Subnet,
		GatewayIP: profile.Gateway, DeviceIP: profile.Device, GatewayMacAddress: profile.GatewayMAC,
		Protocol: types.BessProtocol})
	if err != nil {
		t.Fatal(err)
	}
	defer stack.Close()
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	finished := make(chan error, 1)
	go func() { finished <- stack.AcceptBess(ctx, tapConnection{device}) }()
	guestMAC, err := net.ParseMAC(profile.GuestMAC)
	if err != nil {
		t.Fatal(err)
	}
	gatewayMAC, err := net.ParseMAC(profile.GatewayMAC)
	if err != nil {
		t.Fatal(err)
	}
	request := gopacket.NewSerializeBuffer()
	err = gopacket.SerializeLayers(request, gopacket.SerializeOptions{FixLengths: true},
		&layers.Ethernet{SrcMAC: guestMAC, DstMAC: layers.EthernetBroadcast, EthernetType: layers.EthernetTypeARP},
		&layers.ARP{AddrType: layers.LinkTypeEthernet, Protocol: layers.EthernetTypeIPv4, Operation: layers.ARPRequest,
			SourceHwAddress: guestMAC, SourceProtAddress: net.ParseIP(profile.Device).To4(),
			DstHwAddress: make([]byte, len(guestMAC)), DstProtAddress: net.ParseIP(profile.Gateway).To4()})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := client.Write(request.Bytes()); err != nil {
		t.Fatal(err)
	}
	if err := client.SetReadDeadline(time.Now().Add(time.Second)); err != nil {
		t.Fatal(err)
	}
	response := make([]byte, profile.MTU)
	count, err := client.Read(response)
	if err != nil {
		t.Fatal(err)
	}
	packet := gopacket.NewPacket(response[:count], layers.LayerTypeEthernet, gopacket.Default)
	arp, ok := packet.Layer(layers.LayerTypeARP).(*layers.ARP)
	if !ok || arp.Operation != layers.ARPReply || !bytes.Equal(arp.SourceHwAddress, gatewayMAC) || !bytes.Equal(arp.DstHwAddress, guestMAC) {
		t.Fatal("Maintained stack did not return the actual gateway ARP reply")
	}
	cancel()
	device.Close()
	select {
	case <-finished:
	case <-time.After(time.Second):
		t.Fatal("Closing packet transport left its reader blocked")
	}
}

func TestPacketTAPRejectsRealRegularFile(t *testing.T) {
	file, err := os.CreateTemp(t.TempDir(), "not-a-tap")
	if err != nil {
		t.Fatal(err)
	}
	defer file.Close()
	descriptor, err := unix.Dup(int(file.Fd()))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := attachedTap(descriptor, "packet0"); !errors.Is(err, unix.ENOTTY) {
		t.Fatalf("Expected the actual kernel's non-TAP refusal: %v", err)
	}
}

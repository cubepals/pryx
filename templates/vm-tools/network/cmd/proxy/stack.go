// SPDX-License-Identifier: FSL-1.1-ALv2
// Assemble gvisor-tap-vsock's own exported parts around a gvisor stack, as virtualnetwork.New
// does, with each forwarder behind the host's admission. That constructor keeps its stack
// private and offers no admission hook. The library's switch, link endpoint, DHCP server and
// TCP, UDP and ICMP forwarders still own every frame, address, connection and dial. No resolver
// runs here: guest DNS reaches only the host-owned gateway the admission names.
package main

import (
	"errors"
	"log"
	"net"
	"sync"

	"github.com/containers/gvisor-tap-vsock/pkg/services/dhcp"
	"github.com/containers/gvisor-tap-vsock/pkg/services/forwarder"
	"github.com/containers/gvisor-tap-vsock/pkg/tap"
	"github.com/containers/gvisor-tap-vsock/pkg/types"
	"gvisor.dev/gvisor/pkg/tcpip"
	"gvisor.dev/gvisor/pkg/tcpip/network/arp"
	"gvisor.dev/gvisor/pkg/tcpip/network/ipv4"
	"gvisor.dev/gvisor/pkg/tcpip/stack"
	"gvisor.dev/gvisor/pkg/tcpip/transport/icmp"
	"gvisor.dev/gvisor/pkg/tcpip/transport/tcp"
	"gvisor.dev/gvisor/pkg/tcpip/transport/udp"
)

// The single NIC every gvisor-tap-vsock service binds to.
const packetNIC tcpip.NICID = 1

type stackOptions struct {
	// virtualIP, when set, is answered on the link and translated to host for dialing.
	virtualIP, host string
	policy          admission
	concurrency     int
}

func ipv4Address(text string) (tcpip.Address, error) {
	address := net.ParseIP(text).To4()
	if address == nil {
		return tcpip.Address{}, errors.New("Packet stack address must be IPv4")
	}
	return tcpip.AddrFrom4Slice(address), nil
}

// newPacketStack returns the switch a guest transport is accepted on.
func newPacketStack(profile runtimeProfile, options stackOptions) (*tap.Switch, error) {
	configuration := &types.Configuration{MTU: profile.MTU, Subnet: profile.Subnet, GatewayIP: profile.Gateway,
		DeviceIP: profile.Device, GatewayMacAddress: profile.GatewayMAC}
	pool, err := tap.NewIPPool(profile.Subnet)
	if err != nil {
		return nil, err
	}
	if err := pool.Reserve(profile.Gateway, profile.GatewayMAC); err != nil {
		return nil, err
	}
	if err := pool.Reserve(profile.Device, profile.GuestMAC); err != nil {
		return nil, err
	}
	var virtualIPs []string
	translation := map[tcpip.Address]tcpip.Address{}
	if options.virtualIP != "" {
		virtual, err := ipv4Address(options.virtualIP)
		if err != nil {
			return nil, err
		}
		if translation[virtual], err = ipv4Address(options.host); err != nil {
			return nil, err
		}
		virtualIPs = []string{options.virtualIP}
	}
	endpoint, err := tap.NewLinkEndpoint(false, uint32(profile.MTU), profile.GatewayMAC, profile.Gateway, virtualIPs)
	if err != nil {
		return nil, err
	}
	network := tap.NewSwitch(false)
	endpoint.Connect(network)
	network.Connect(endpoint)
	gateway, err := ipv4Address(profile.Gateway)
	if err != nil {
		return nil, err
	}
	_, subnet, err := net.ParseCIDR(profile.Subnet)
	if err != nil {
		return nil, err
	}
	route, err := tcpip.NewSubnet(tcpip.AddrFromSlice(subnet.IP.To4()), tcpip.MaskFromBytes(subnet.Mask))
	if err != nil {
		return nil, err
	}
	s := stack.New(stack.Options{
		NetworkProtocols:   []stack.NetworkProtocolFactory{ipv4.NewProtocol, arp.NewProtocol},
		TransportProtocols: []stack.TransportProtocolFactory{tcp.NewProtocol, udp.NewProtocol, icmp.NewProtocol4},
	})
	if err := s.CreateNIC(packetNIC, endpoint); err != nil {
		return nil, errors.New(err.String())
	}
	if err := s.AddProtocolAddress(packetNIC, tcpip.ProtocolAddress{Protocol: ipv4.ProtocolNumber,
		AddressWithPrefix: gateway.WithPrefix()}, stack.AddressProperties{}); err != nil {
		return nil, errors.New(err.String())
	}
	if err := s.SetSpoofing(packetNIC, true); err != nil {
		return nil, errors.New(err.String())
	}
	if err := s.SetPromiscuousMode(packetNIC, true); err != nil {
		return nil, errors.New(err.String())
	}
	s.SetRouteTable([]tcpip.Route{{Destination: route, NIC: packetNIC}})
	var lock sync.Mutex
	policy := options.policy
	s.SetTransportProtocolHandler(tcp.ProtocolNumber, policy.admitted(tcp.ProtocolNumber,
		forwarder.TCP(s, translation, &lock, false, options.concurrency, 0).HandlePacket))
	s.SetTransportProtocolHandler(udp.ProtocolNumber, policy.admitted(udp.ProtocolNumber,
		forwarder.UDP(s, translation, &lock, false).HandlePacket))
	s.SetTransportProtocolHandler(icmp.ProtocolNumber4, policy.admitted(icmp.ProtocolNumber4,
		forwarder.ICMP(s, translation, &lock).HandlePacket))
	addresses, err := dhcp.New(configuration, s, pool)
	if err != nil {
		return nil, err
	}
	go func() { log.Print(addresses.Serve()) }()
	return network, nil
}

// SPDX-License-Identifier: FSL-1.1-ALv2
// Decide, before the maintained forwarders dial, which guest flows the host carries. The guest's
// own firewall is not trusted here: a guest kernel can drop its rules. The host-owned gateway
// address is reachable only at the ports the configuration names, a public IPv4 destination only
// when the configuration allows public egress and the kernel would route it away from this host,
// and nothing else. The forwarders dial from the host's own network, so an address of the host
// itself (on any interface, public or not) would otherwise reach whatever listens there. This
// file neither dials nor parses packets; the maintained stack hands it each flow's destination.
package main

import (
	"fmt"
	"net/netip"

	"golang.org/x/sys/unix"

	"gvisor.dev/gvisor/pkg/tcpip"
	"gvisor.dev/gvisor/pkg/tcpip/header"
	"gvisor.dev/gvisor/pkg/tcpip/stack"
	"gvisor.dev/gvisor/pkg/tcpip/transport/tcp"
	"gvisor.dev/gvisor/pkg/tcpip/transport/udp"
)

// egressConfiguration is the host's network policy for one guest, written by pryxd.
type egressConfiguration struct {
	// Public allows public IPv4 destinations; without it only the named gateway ports remain.
	Public bool `json:"public"`
	// TCP and UDP are the exact ports the guest may reach at the profile's host address.
	TCP []uint16 `json:"tcp"`
	UDP []uint16 `json:"udp"`
	// NonPublic is pryx-agent's own non-public IPv4 boundary, never dialed.
	NonPublic []string `json:"nonPublic"`
}

type admission struct {
	gateway  netip.Addr
	tcp, udp map[uint16]bool
	public   bool
	blocked  []netip.Prefix
	// route is the kernel's route type for a dial to an address (hostRouteType in route.go).
	route func(netip.Addr) (uint8, error)
}

func ports(values []uint16) (map[uint16]bool, error) {
	set := make(map[uint16]bool, len(values))
	for _, port := range values {
		if port == 0 {
			return nil, fmt.Errorf("Packet proxy gateway ports must be nonzero")
		}
		set[port] = true
	}
	return set, nil
}

func newAdmission(gateway string, egress *egressConfiguration) (admission, error) {
	address, err := netip.ParseAddr(gateway)
	if err != nil || !address.Is4() {
		return admission{}, fmt.Errorf("Packet proxy gateway must be an IPv4 address")
	}
	if egress == nil {
		return admission{}, fmt.Errorf("Packet proxy configuration must name its egress")
	}
	policy := admission{gateway: address, public: egress.Public, route: hostRouteType}
	if policy.tcp, err = ports(egress.TCP); err != nil {
		return admission{}, err
	}
	if policy.udp, err = ports(egress.UDP); err != nil {
		return admission{}, err
	}
	if egress.Public && len(egress.NonPublic) == 0 {
		return admission{}, fmt.Errorf("Public packet egress requires the non-public IPv4 boundary")
	}
	for _, text := range egress.NonPublic {
		prefix, err := netip.ParsePrefix(text)
		if err != nil || !prefix.Addr().Is4() || prefix != prefix.Masked() {
			return admission{}, fmt.Errorf("Packet proxy non-public boundary %q is not an IPv4 network", text)
		}
		policy.blocked = append(policy.blocked, prefix)
	}
	return policy, nil
}

// publicIPv4 reports a destination outside Go's own non-routable classes and the agent's boundary.
func (policy admission) publicIPv4(address netip.Addr) bool {
	if !address.Is4() || address.IsLoopback() || address.IsPrivate() || address.IsLinkLocalUnicast() ||
		address.IsMulticast() || address.IsUnspecified() || address == netip.AddrFrom4(header.IPv4Broadcast.As4()) {
		return false
	}
	for _, prefix := range policy.blocked {
		if prefix.Contains(address) {
			return false
		}
	}
	return true
}

// routedAway reports a destination the kernel would send away from this host, asked as the flow
// is dialled so it holds when the host's addresses change. One the host itself would receive
// (RTN_LOCAL: its own address on any interface, or a local route), a broadcast, and one the
// kernel can't route or doesn't answer for are all refused.
func (policy admission) routedAway(address netip.Addr) bool {
	kind, err := policy.route(address)
	return err == nil && kind == unix.RTN_UNICAST
}

// readsRoutes refuses to start a public profile whose route lookups don't work, since it would
// refuse every public destination in silence: IPv4's loopback address must read as local.
func (policy admission) readsRoutes() error {
	if !policy.public {
		return nil
	}
	loopback := netip.AddrFrom4([4]byte{127, 0, 0, 1})
	if kind, err := policy.route(loopback); err != nil || kind != unix.RTN_LOCAL {
		return fmt.Errorf("Packet proxy cannot read this host's routes (type %d): %v", kind, err)
	}
	return nil
}

func (policy admission) allows(protocol tcpip.TransportProtocolNumber, destination netip.Addr, port uint16) bool {
	if destination == policy.gateway {
		var allowed map[uint16]bool
		switch protocol {
		case tcp.ProtocolNumber:
			allowed = policy.tcp
		case udp.ProtocolNumber:
			allowed = policy.udp
		default:
			return false
		}
		return allowed[port]
	}
	return policy.public && policy.publicIPv4(destination) && policy.routedAway(destination)
}

// admitted hands a flow to the maintained forwarder only when the policy allows it. A refused
// flow is left unhandled, so the stack answers it as a closed port and nothing is dialed.
func (policy admission) admitted(protocol tcpip.TransportProtocolNumber,
	forward func(stack.TransportEndpointID, *stack.PacketBuffer) bool) func(stack.TransportEndpointID, *stack.PacketBuffer) bool {
	return func(id stack.TransportEndpointID, packet *stack.PacketBuffer) bool {
		if id.LocalAddress.Len() != header.IPv4AddressSize || !policy.allows(protocol, netip.AddrFrom4(id.LocalAddress.As4()), id.LocalPort) {
			return false
		}
		return forward(id, packet)
	}
}

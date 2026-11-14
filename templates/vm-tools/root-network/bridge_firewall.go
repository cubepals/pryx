// SPDX-License-Identifier: FSL-1.1-ALv2
// Bound both directions of the private L2 bridge independently of any guest firewall.
// Only the two actual TAP ports, fixed MAC/IP peers, gateway DNS and public IPv4 traffic
// can pass. Bridge input/output, IPv6, other EtherTypes and non-public destinations drop.
// The maintained user-mode stack owns connection state; no global bridge sysctl is changed.
package rootnetwork

import (
	"fmt"
	"net"
	"net/netip"

	"github.com/google/nftables"
	"github.com/google/nftables/binaryutil"
	"github.com/google/nftables/expr"
	"golang.org/x/sys/unix"
)

// RFC 826 places sender hardware after htype/ptype/hlen/plen/op. nftables exports
// address widths, not this header offset; the existing IPv4/port offsets have one owner.
const arpSourceHardwareOffset = 8

const BridgeTable = "pryx_packet"

func bridgePayload(base expr.PayloadBase, offset uint32, value []byte) []expr.Any {
	return []expr.Any{
		&expr.Payload{DestRegister: unix.NFT_REG_1, Base: base, Offset: offset, Len: uint32(len(value))},
		&expr.Cmp{Op: expr.CmpOpEq, Register: unix.NFT_REG_1, Data: value},
	}
}

func bridgeProtocol(protocol uint16) []expr.Any {
	return []expr.Any{
		&expr.Meta{Key: expr.MetaKeyPROTOCOL, Register: unix.NFT_REG_1},
		&expr.Cmp{Op: expr.CmpOpEq, Register: unix.NFT_REG_1, Data: binaryutil.BigEndian.PutUint16(protocol)},
	}
}

func bridgePorts(source, destination int) []expr.Any {
	return []expr.Any{
		&expr.Meta{Key: expr.MetaKeyIIF, Register: unix.NFT_REG_1},
		&expr.Cmp{Op: expr.CmpOpEq, Register: unix.NFT_REG_1, Data: binaryutil.NativeEndian.PutUint32(uint32(source))},
		&expr.Meta{Key: expr.MetaKeyOIF, Register: unix.NFT_REG_1},
		&expr.Cmp{Op: expr.CmpOpEq, Register: unix.NFT_REG_1, Data: binaryutil.NativeEndian.PutUint32(uint32(destination))},
	}
}

func bridgeLayer4(protocol byte) []expr.Any {
	return []expr.Any{
		&expr.Meta{Key: expr.MetaKeyL4PROTO, Register: unix.NFT_REG_1},
		&expr.Cmp{Op: expr.CmpOpEq, Register: unix.NFT_REG_1, Data: []byte{protocol}},
	}
}

func bridgePrefix(offset uint32, prefix netip.Prefix) []expr.Any {
	mask := net.CIDRMask(prefix.Bits(), net.IPv4len*8)
	return []expr.Any{
		&expr.Payload{DestRegister: unix.NFT_REG_1, Base: expr.PayloadBaseNetworkHeader, Offset: offset, Len: net.IPv4len},
		&expr.Bitwise{SourceRegister: unix.NFT_REG_1, DestRegister: unix.NFT_REG_1, Len: net.IPv4len, Mask: mask, Xor: make([]byte, net.IPv4len)},
		&expr.Cmp{Op: expr.CmpOpEq, Register: unix.NFT_REG_1, Data: prefix.Addr().AsSlice()},
	}
}

func (bridge *Bridge) secure() error {
	connection, err := nftables.New(nftables.WithNetNSFd(int(bridge.scope.handle)))
	if err != nil {
		return err
	}
	table := connection.AddTable(&nftables.Table{Name: BridgeTable, Family: nftables.TableFamilyBridge})
	drop := nftables.ChainPolicyDrop
	var chains []*nftables.Chain
	for _, value := range []struct {
		name string
		hook *nftables.ChainHook
	}{{"input", nftables.ChainHookInput}, {"output", nftables.ChainHookOutput}, {"forward", nftables.ChainHookForward}} {
		chains = append(chains, connection.AddChain(&nftables.Chain{Name: value.name, Table: table, Type: nftables.ChainTypeFilter,
			Hooknum: value.hook, Priority: nftables.ChainPriorityFilter, Policy: &drop}))
	}
	forward := chains[len(chains)-1]
	guest, err := netip.ParsePrefix(bridge.inputs.profile.GuestCIDR)
	if err != nil {
		return err
	}
	gateway, err := netip.ParseAddr(bridge.inputs.profile.Host)
	if err != nil {
		return err
	}
	guestMAC, err := net.ParseMAC(bridge.inputs.profile.GuestMAC)
	if err != nil {
		return err
	}
	rules := 0
	add := func(expressions []expr.Any, verdict expr.VerdictKind) {
		connection.AddRule(&nftables.Rule{Table: table, Chain: forward,
			Exprs: append(expressions, &expr.Counter{}, &expr.Verdict{Kind: verdict})})
		rules++
	}
	for _, direction := range []struct {
		from, to           int
		sourceMAC, destMAC net.HardwareAddr
		sourceIP, destIP   netip.Addr
		outgoing           bool
	}{
		{bridge.scope.identity, bridge.packetIdentity, guestMAC, bridge.inputs.gatewayMAC, guest.Addr(), gateway, true},
		{bridge.packetIdentity, bridge.scope.identity, bridge.inputs.gatewayMAC, guestMAC, gateway, guest.Addr(), false},
	} {
		base := bridgePorts(direction.from, direction.to)
		// Ethernet's source immediately follows its destination address.
		base = append(base, bridgePayload(expr.PayloadBaseLLHeader, nftables.TypeEtherAddr.Bytes, direction.sourceMAC)...)
		arp := append(append([]expr.Any{}, base...), bridgeProtocol(unix.ETH_P_ARP)...)
		header := append(binaryutil.BigEndian.PutUint16(unix.ARPHRD_ETHER), binaryutil.BigEndian.PutUint16(unix.ETH_P_IP)...)
		header = append(header, byte(nftables.TypeEtherAddr.Bytes), byte(net.IPv4len))
		arp = append(arp, bridgePayload(expr.PayloadBaseNetworkHeader, 0, header)...)
		arp = append(arp, bridgePayload(expr.PayloadBaseNetworkHeader, arpSourceHardwareOffset, direction.sourceMAC)...)
		sourceOffset := uint32(arpSourceHardwareOffset) + nftables.TypeEtherAddr.Bytes
		targetOffset := sourceOffset + net.IPv4len + nftables.TypeEtherAddr.Bytes
		arp = append(arp, bridgePayload(expr.PayloadBaseNetworkHeader, sourceOffset, direction.sourceIP.AsSlice())...)
		arp = append(arp, bridgePayload(expr.PayloadBaseNetworkHeader, targetOffset, direction.destIP.AsSlice())...)
		add(arp, expr.VerdictAccept)
		base = append(base, bridgeProtocol(unix.ETH_P_IP)...)
		base = append(base, bridgePayload(expr.PayloadBaseLLHeader, 0, direction.destMAC)...)
		fixed, filtered := uint32(ipv4SourceOffset), uint32(ipv4DestinationOffset)
		fixedIP := guest.Addr()
		dnsPortOffset := uint32(transportDestinationOffset)
		if !direction.outgoing {
			fixed, filtered = filtered, fixed
			// Both TCP and UDP source ports begin their transport header.
			dnsPortOffset = 0
		}
		base = append(base, bridgePayload(expr.PayloadBaseNetworkHeader, fixed, fixedIP.AsSlice())...)
		for _, protocol := range []byte{unix.IPPROTO_TCP, unix.IPPROTO_UDP} {
			dns := append(append([]expr.Any{}, base...), bridgeLayer4(protocol)...)
			dns = append(dns, bridgePayload(expr.PayloadBaseNetworkHeader, filtered, gateway.AsSlice())...)
			dns = append(dns, bridgePayload(expr.PayloadBaseTransportHeader, dnsPortOffset, binaryutil.BigEndian.PutUint16(bridge.inputs.dns))...)
			add(dns, expr.VerdictAccept)
		}
		for _, prefix := range bridge.inputs.blocked {
			denied := append(append([]expr.Any{}, base...), bridgePrefix(filtered, prefix)...)
			add(denied, expr.VerdictDrop)
		}
		for _, protocol := range []byte{unix.IPPROTO_TCP, unix.IPPROTO_UDP, unix.IPPROTO_ICMP} {
			allowed := append(append([]expr.Any{}, base...), bridgeLayer4(protocol)...)
			add(allowed, expr.VerdictAccept)
		}
	}
	if err := connection.Flush(); err != nil {
		return err
	}
	if err := confirmChains(connection, table, chains...); err != nil {
		return err
	}
	for _, chain := range chains {
		actual, err := connection.GetRules(table, chain)
		want := 0
		if chain.Name == forward.Name {
			want = rules
		}
		if err != nil || len(actual) != want {
			return fmt.Errorf("Private bridge firewall installation was not confirmed: %v", err)
		}
	}
	bridge.confirmed = true
	return nil
}

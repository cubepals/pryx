// SPDX-License-Identifier: FSL-1.1-ALv2
// Install hostname-gateway admission in this scope's anonymous namespace through the pinned
// nftables library. No original-host rule is changed; forwarding and all other input default
// to drop, independently of the guest's own immutable redirect policy.
package rootnetwork

import (
	"fmt"
	"net/netip"

	"github.com/google/nftables"
	"github.com/google/nftables/binaryutil"
	"github.com/google/nftables/expr"
	"golang.org/x/sys/unix"
)

// RFC 791 section 3.1 gives these IPv4 header offsets; RFC 9293 section 3.1 and
// RFC 768 place the destination port after the two-byte source port. The API
// exports payload bases, but not these wire-field offsets.
const ipv4SourceOffset = 12
const ipv4DestinationOffset = 16
const transportDestinationOffset = 2

func state(mask uint32) []expr.Any {
	bits := binaryutil.NativeEndian.PutUint32(mask)
	zero := make([]byte, len(bits))
	return []expr.Any{
		&expr.Ct{Register: unix.NFT_REG_1, Key: expr.CtKeySTATE},
		&expr.Bitwise{SourceRegister: unix.NFT_REG_1, DestRegister: unix.NFT_REG_1, Len: uint32(len(bits)), Mask: bits, Xor: zero},
		&expr.Cmp{Op: expr.CmpOpNeq, Register: unix.NFT_REG_1, Data: zero},
	}
}

func (scope *Scope) guestInput() ([]expr.Any, error) {
	guest, err := netip.ParsePrefix(scope.Profile.GuestCIDR)
	if err != nil || !guest.Addr().Is4() {
		return nil, fmt.Errorf("Invalid private firewall guest prefix: %v", err)
	}
	host, err := netip.ParseAddr(scope.Profile.Host)
	if err != nil || !host.Is4() || !guest.Contains(host) {
		return nil, fmt.Errorf("Invalid private firewall host address: %v", err)
	}
	source, destination := guest.Addr().AsSlice(), host.AsSlice()
	return []expr.Any{
		&expr.Meta{Key: expr.MetaKeyIIF, Register: unix.NFT_REG_1},
		&expr.Cmp{Op: expr.CmpOpEq, Register: unix.NFT_REG_1, Data: binaryutil.NativeEndian.PutUint32(uint32(scope.identity))},
		&expr.Meta{Key: expr.MetaKeyNFPROTO, Register: unix.NFT_REG_1},
		&expr.Cmp{Op: expr.CmpOpEq, Register: unix.NFT_REG_1, Data: []byte{unix.NFPROTO_IPV4}},
		&expr.Payload{DestRegister: unix.NFT_REG_1, Base: expr.PayloadBaseNetworkHeader, Offset: ipv4SourceOffset, Len: uint32(len(source))},
		&expr.Cmp{Op: expr.CmpOpEq, Register: unix.NFT_REG_1, Data: source},
		&expr.Payload{DestRegister: unix.NFT_REG_1, Base: expr.PayloadBaseNetworkHeader, Offset: ipv4DestinationOffset, Len: uint32(len(destination))},
		&expr.Cmp{Op: expr.CmpOpEq, Register: unix.NFT_REG_1, Data: destination},
	}, nil
}

func (scope *Scope) secure(ports Ports) error {
	connection, err := nftables.New(nftables.WithNetNSFd(int(scope.handle)))
	if err != nil {
		return err
	}
	table := connection.AddTable(&nftables.Table{Name: "pryx_vm", Family: nftables.TableFamilyINet})
	drop := nftables.ChainPolicyDrop
	input := connection.AddChain(&nftables.Chain{Name: "input", Table: table, Type: nftables.ChainTypeFilter,
		Hooknum: nftables.ChainHookInput, Priority: nftables.ChainPriorityFilter, Policy: &drop})
	forward := connection.AddChain(&nftables.Chain{Name: "forward", Table: table, Type: nftables.ChainTypeFilter,
		Hooknum: nftables.ChainHookForward, Priority: nftables.ChainPriorityFilter, Policy: &drop})
	rules := [][]expr.Any{{
		&expr.Meta{Key: expr.MetaKeyIIF, Register: unix.NFT_REG_1},
		&expr.Cmp{Op: expr.CmpOpEq, Register: unix.NFT_REG_1, Data: binaryutil.NativeEndian.PutUint32(uint32(scope.loopback))},
	}}
	base, err := scope.guestInput()
	if err != nil {
		return err
	}
	rules = append(rules, append(append([]expr.Any{}, base...), state(expr.CtStateBitESTABLISHED|expr.CtStateBitRELATED)...))
	for _, service := range []struct {
		protocol byte
		port     uint16
	}{{unix.IPPROTO_TCP, ports.HTTP}, {unix.IPPROTO_TCP, ports.TLS}, {unix.IPPROTO_TCP, ports.DNS}, {unix.IPPROTO_UDP, ports.DNS}} {
		if service.port == 0 {
			return fmt.Errorf("Root firewall requires actual bound service ports")
		}
		port := binaryutil.BigEndian.PutUint16(service.port)
		rule := append([]expr.Any{}, base...)
		rule = append(rule, state(expr.CtStateBitNEW)...)
		rules = append(rules, append(rule,
			&expr.Meta{Key: expr.MetaKeyL4PROTO, Register: unix.NFT_REG_1},
			&expr.Cmp{Op: expr.CmpOpEq, Register: unix.NFT_REG_1, Data: []byte{service.protocol}},
			&expr.Payload{DestRegister: unix.NFT_REG_1, Base: expr.PayloadBaseTransportHeader, Offset: transportDestinationOffset, Len: uint32(len(port))},
			&expr.Cmp{Op: expr.CmpOpEq, Register: unix.NFT_REG_1, Data: port}))
	}
	for _, rule := range rules {
		connection.AddRule(&nftables.Rule{Table: table, Chain: input, Exprs: append(rule, &expr.Verdict{Kind: expr.VerdictAccept})})
	}
	if err := connection.Flush(); err != nil {
		return err
	}
	if err := confirmChains(connection, table, input, forward); err != nil {
		return err
	}
	actual, err := connection.GetRules(table, input)
	if err != nil || len(actual) != len(rules) {
		return fmt.Errorf("Root firewall rule installation was not confirmed: %v", err)
	}
	scope.firewallConfirmed = true
	return nil
}

func confirmChains(connection *nftables.Conn, table *nftables.Table, input, forward *nftables.Chain) error {
	chains, err := connection.ListChainsOfTableFamily(table.Family)
	if err != nil {
		return err
	}
	required := map[string]*nftables.Chain{input.Name: input, forward.Name: forward}
	for _, actual := range chains {
		if actual.Table.Name != table.Name || actual.Table.Family != table.Family {
			continue
		}
		wanted, ok := required[actual.Name]
		if !ok {
			continue
		}
		if actual.Policy == nil || *actual.Policy != nftables.ChainPolicyDrop || actual.Type != wanted.Type ||
			actual.Hooknum == nil || *actual.Hooknum != *wanted.Hooknum || actual.Priority == nil || *actual.Priority != *wanted.Priority {
			return fmt.Errorf("Root firewall chain policy was not confirmed")
		}
		delete(required, actual.Name)
	}
	if len(required) != 0 {
		return fmt.Errorf("Root firewall base chains were not confirmed")
	}
	return nil
}

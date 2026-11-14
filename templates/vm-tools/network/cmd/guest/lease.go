// SPDX-License-Identifier: FSL-1.1-ALv2
// Apply the maintained DHCP client's real lease to the trusted guest interface and resolver.
// This does not request leases, run image tools or own packet transport.
package main

import (
	"fmt"
	"github.com/insomniacslk/dhcp/dhcpv4/nclient4"
	"github.com/vishvananda/netlink"
	"golang.org/x/sys/unix"
	"net"
	"os"
	"strings"
)

func lease(link netlink.Link, value *nclient4.Lease, resolver string) error {
	ack := value.ACK
	mask := ack.SubnetMask()
	if ack.YourIPAddr.To4() == nil || mask == nil || len(ack.Router()) == 0 || len(ack.DNS()) == 0 {
		return fmt.Errorf("Guest DHCP lease lacks an IPv4 address, route or resolver")
	}
	address := &netlink.Addr{IPNet: &net.IPNet{IP: ack.YourIPAddr, Mask: mask}}
	if err := netlink.AddrReplace(link, address); err != nil {
		return err
	}
	route := &netlink.Route{LinkIndex: link.Attrs().Index, Gw: ack.Router()[0]}
	if err := netlink.RouteReplace(route); err != nil {
		return err
	}
	var data strings.Builder
	for _, ip := range ack.DNS() {
		if ip.To4() == nil {
			return fmt.Errorf("Guest DHCP resolver is not IPv4")
		}
		fmt.Fprintf(&data, "nameserver %s\n", ip.String())
	}
	descriptor, err := unix.Open(resolver, unix.O_WRONLY|unix.O_TRUNC|unix.O_NOFOLLOW|unix.O_CLOEXEC, 0)
	if err != nil {
		return err
	}
	file := os.NewFile(uintptr(descriptor), "guest resolver")
	defer file.Close()
	_, err = file.WriteString(data.String())
	return err
}

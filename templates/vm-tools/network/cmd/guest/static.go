// SPDX-License-Identifier: FSL-1.1-ALv2
// Install only host-selected connected private TAP addresses through maintained netlink.
// The existing resolver writer retains its no-follow file open; no image tool is executed.
package main

import (
	"fmt"
	"github.com/vishvananda/netlink"
	"net"
)

func staticNetwork(link netlink.Link, prefix, gateway, resolver string) error {
	address, err := netlink.ParseAddr(prefix)
	if err != nil {
		return err
	}
	route := net.ParseIP(gateway)
	if address.IP.To4() == nil || !address.IP.IsPrivate() || route.To4() == nil || !route.IsPrivate() || !address.Contains(route) || address.IP.Equal(route) {
		return fmt.Errorf("Static TAP requires distinct connected private IPv4 addresses")
	}
	if err := netlink.AddrReplace(link, address); err != nil {
		return err
	}
	if err := netlink.RouteReplace(&netlink.Route{LinkIndex: link.Attrs().Index, Gw: route}); err != nil {
		return err
	}
	return writeResolver(resolver, fmt.Sprintf("nameserver %s\n", route.String()))
}

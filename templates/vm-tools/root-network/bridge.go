// SPDX-License-Identifier: FSL-1.1-ALv2
// Own a private two-port L2 bridge between the CNI monitor TAP and a nonpersistent packet TAP.
// Only fixed root-owned executable metadata supplies addressing and the host deny boundary.
// No original-host interface, address pool, route or forwarding setting changes. Descriptors
// are granted only after the separate namespace firewall has been installed and confirmed.
package rootnetwork

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"net"
	"os"

	"github.com/containernetworking/plugins/pkg/ns"
	"github.com/vishvananda/netlink"
	"golang.org/x/sys/unix"
)

const bridgeName = "packet-br0"
const packetTapName = "packet0"

type Bridge struct {
	scope          *Scope
	queue          *os.File
	inputs         BridgeInputs
	bridgeIdentity int
	packetIdentity int
	confirmed      bool
	grantAttempted bool
	attempted      bool
}

func OpenBridge(ctx context.Context, directory, cache, name string, uid, gid uint32, inputs BridgeInputs) (*Bridge, error) {
	if len(inputs.blocked) == 0 || inputs.profile.Host == "" || name == bridgeName || name == packetTapName {
		return nil, fmt.Errorf("Root bridge requires the actual fixed host-owned packet policy")
	}
	scope, err := Open(ctx, directory, cache, name, uid, gid)
	if err != nil {
		return nil, err
	}
	bridge := &Bridge{scope: scope, inputs: inputs}
	err = scope.Space.Do(func(ns.NetNS) error {
		monitor, err := netlink.LinkByName(scope.Tap)
		if err != nil || monitor.Attrs().Index != scope.identity {
			return fmt.Errorf("Private monitor TAP identity changed: %v", err)
		}
		addresses, err := netlink.AddrList(monitor, netlink.FAMILY_V4)
		if err != nil || len(addresses) != 1 || addresses[0].IP.String() != scope.Profile.Host {
			return fmt.Errorf("Private initial TAP addressing differs from its owner: %v", err)
		}
		if err := netlink.AddrDel(monitor, &addresses[0]); err != nil {
			return err
		}
		link := &netlink.Bridge{LinkAttrs: netlink.LinkAttrs{Name: bridgeName, MTU: inputs.profile.MTU}}
		if err := netlink.LinkAdd(link); err != nil {
			return err
		}
		actual, err := netlink.LinkByName(bridgeName)
		if err != nil {
			return err
		}
		bridge.bridgeIdentity = actual.Attrs().Index
		packet := &netlink.Tuntap{LinkAttrs: netlink.LinkAttrs{Name: packetTapName, MTU: inputs.profile.MTU},
			Mode: netlink.TUNTAP_MODE_TAP, Flags: netlink.TUNTAP_DEFAULTS | netlink.TUNTAP_NO_PI, Queues: 1,
			NonPersist: true, Owner: uid, Group: gid}
		if err := netlink.LinkAdd(packet); err != nil {
			return err
		}
		if len(packet.Fds) != 1 {
			for _, file := range packet.Fds {
				file.Close()
			}
			return fmt.Errorf("Private packet TAP must retain exactly one queue")
		}
		bridge.queue = packet.Fds[0]
		packetLink, err := netlink.LinkByName(packetTapName)
		if err != nil {
			return err
		}
		bridge.packetIdentity = packetLink.Attrs().Index
		for _, port := range []netlink.Link{monitor, packetLink} {
			if err := netlink.LinkSetMTU(port, inputs.profile.MTU); err != nil {
				return err
			}
			if err := netlink.LinkSetMasterByIndex(port, bridge.bridgeIdentity); err != nil {
				return err
			}
			if err := netlink.LinkSetUp(port); err != nil {
				return err
			}
		}
		if err := netlink.LinkSetUp(actual); err != nil {
			return err
		}
		final, err := netlink.LinkByName(bridgeName)
		if err != nil {
			return err
		}
		guestMAC, err := net.ParseMAC(inputs.profile.GuestMAC)
		if err != nil {
			return err
		}
		if bytes.Equal(final.Attrs().HardwareAddr, inputs.gatewayMAC) || bytes.Equal(final.Attrs().HardwareAddr, guestMAC) {
			return fmt.Errorf("Kernel bridge MAC collides with an admitted packet peer")
		}
		return nil
	})
	if err == nil {
		guest := inputs.profile
		// The packet program names its vsock-created TAP. Root guests keep the
		// existing physical virtio interface from their root scope profile.
		guest.Interface = scope.Profile.Interface
		scope.Profile = guest
		err = bridge.secure()
	}
	if err != nil {
		return nil, errors.Join(err, bridge.Close(ctx))
	}
	return bridge, nil
}

// Files grants only the actual namespace and attached queue, once. Its caller keeps the
// independent lifetime channel and waits on the actual monitor lease before Close.
func (bridge *Bridge) Files(ports Ports) ([]*os.File, error) {
	if bridge.grantAttempted || !bridge.confirmed || ports != (Ports{}) {
		return nil, fmt.Errorf("Root bridge grant is repeated, unsecured or names gateway ports")
	}
	bridge.grantAttempted = true
	var files []*os.File
	for _, descriptor := range []uintptr{uintptr(bridge.scope.handle), bridge.queue.Fd()} {
		copy, err := unix.FcntlInt(descriptor, unix.F_DUPFD_CLOEXEC, 0)
		if err != nil {
			for _, file := range files {
				file.Close()
			}
			return nil, err
		}
		files = append(files, os.NewFile(uintptr(copy), "owned root packet authority"))
	}
	return files, nil
}

func (bridge *Bridge) Profile() Profile { return bridge.scope.Profile }

// Close is called only after the monitor's lifetime lease clears; a mutation is never retried.
func (bridge *Bridge) Close(ctx context.Context) error {
	if bridge.attempted {
		return fmt.Errorf("Root bridge cleanup was already submitted")
	}
	bridge.attempted = true
	err := bridge.scope.Space.Do(func(ns.NetNS) error {
		for _, expected := range []struct {
			name     string
			identity int
		}{{packetTapName, bridge.packetIdentity}, {bridgeName, bridge.bridgeIdentity}} {
			if expected.identity == 0 {
				continue
			}
			link, err := netlink.LinkByName(expected.name)
			if err != nil || link.Attrs().Index != expected.identity {
				return fmt.Errorf("Private bridge retirement identity changed: %v", err)
			}
			if err := netlink.LinkDel(link); err != nil {
				return err
			}
		}
		return nil
	})
	if err != nil {
		return err
	}
	if bridge.queue != nil {
		if err := bridge.queue.Close(); err != nil {
			return err
		}
	}
	return bridge.scope.Close(ctx)
}

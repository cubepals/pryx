// SPDX-License-Identifier: FSL-1.1-ALv2
// Inspect real root bridge descriptors, topology, drop-policy readback and explicit retirement.
// These checks run inside the existing bounded authority, and claim no guest IP forwarding.
package main

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"

	rootnetwork "cubepals/pryx/vm-root-network-experiment"
	"github.com/containernetworking/plugins/pkg/ns"
	"github.com/google/nftables"
	"github.com/vishvananda/netlink"
	"golang.org/x/sys/unix"
)

func inspectBridge(space ns.NetNS, grant rootnetwork.ScopeHandoff, roles map[rootnetwork.ScopeRole]*os.File) (int, error) {
	file := roles[rootnetwork.ScopePacketTap]
	if file == nil || grant.PacketTap == "" || grant.Ipv4Exclusions <= 0 {
		return 0, fmt.Errorf("Root bridge omits its actual queue or host address boundary")
	}
	request, err := unix.NewIfreq("")
	if err == nil {
		err = unix.IoctlIfreq(int(file.Fd()), unix.TUNGETIFF, request)
	}
	if err != nil || request.Name() != grant.PacketTap || request.Uint16()&(unix.IFF_TAP|unix.IFF_TUN|unix.IFF_NO_PI|unix.IFF_VNET_HDR) != unix.IFF_TAP|unix.IFF_NO_PI {
		return 0, fmt.Errorf("Granted queue is not the actual packet TAP: %v", err)
	}
	identity := 0
	err = space.Do(func(ns.NetNS) error {
		monitor, err := netlink.LinkByName(grant.Tap)
		if err != nil {
			return err
		}
		packet, err := netlink.LinkByName(grant.PacketTap)
		if err != nil {
			return err
		}
		identity = monitor.Attrs().MasterIndex
		if identity == 0 || identity != packet.Attrs().MasterIndex {
			return fmt.Errorf("Root TAPs are not peers of one private kernel bridge")
		}
		bridge, err := netlink.LinkByIndex(identity)
		if err != nil {
			return err
		}
		if _, ok := bridge.(*netlink.Bridge); !ok {
			return fmt.Errorf("Actual TAP master is not a kernel bridge")
		}
		for _, link := range []netlink.Link{monitor, packet, bridge} {
			addresses, err := netlink.AddrList(link, netlink.FAMILY_V4)
			if err != nil || len(addresses) != 0 || link.Attrs().MTU != grant.Profile.MTU {
				return fmt.Errorf("Private bridge has kernel IPv4 addressing or an inconsistent MTU: %v", err)
			}
		}
		return nil
	})
	if err != nil {
		return 0, err
	}
	connection, err := nftables.New(nftables.WithNetNSFd(int(space.Fd())))
	if err != nil {
		return 0, err
	}
	chains, err := connection.ListChainsOfTableFamily(nftables.TableFamilyBridge)
	if err != nil {
		return 0, err
	}
	wanted := map[nftables.ChainHook]bool{*nftables.ChainHookInput: true, *nftables.ChainHookOutput: true, *nftables.ChainHookForward: true}
	for _, chain := range chains {
		if chain.Table.Name != rootnetwork.BridgeTable {
			continue
		}
		if chain.Hooknum == nil || !wanted[*chain.Hooknum] || chain.Policy == nil || *chain.Policy != nftables.ChainPolicyDrop || chain.Type != nftables.ChainTypeFilter {
			return 0, fmt.Errorf("Actual private bridge default-drop chain differs from its owner")
		}
		rules, err := connection.GetRules(chain.Table, chain)
		if err != nil || ((*chain.Hooknum == *nftables.ChainHookForward) != (len(rules) != 0)) {
			return 0, fmt.Errorf("Actual bridge input/output or forwarding rules differ: %v", err)
		}
		delete(wanted, *chain.Hooknum)
	}
	if len(wanted) != 0 {
		return 0, fmt.Errorf("Actual private bridge lacks its drop-policy hooks")
	}
	return identity, nil
}

func retiredBridge(space ns.NetNS, packet string, identity int) error {
	return space.Do(func(ns.NetNS) error {
		_, packetError := netlink.LinkByName(packet)
		_, bridgeError := netlink.LinkByIndex(identity)
		var packetAbsent, bridgeAbsent netlink.LinkNotFoundError
		if !errors.As(packetError, &packetAbsent) || !errors.As(bridgeError, &bridgeAbsent) {
			return fmt.Errorf("Retired private packet TAP or bridge remains observable")
		}
		return nil
	})
}

func confinedPacketProof(ctx context.Context, program, agent string, queue *os.File, uid, gid uint32) error {
	if err := rootnetwork.Ancestry(filepath.Dir(program)); err != nil {
		return err
	}
	if err := rootnetwork.Executable(program); err != nil {
		return err
	}
	command := exec.CommandContext(ctx, program, "--exact", "attached_queue_inherits_host_confinement_and_lease", "--ignored", "--nocapture")
	command.Env = []string{"PATH=" + os.Getenv("PATH"), "PRYX_VM_PACKET_TEST_AGENT=" + agent}
	command.Stderr = queue
	var output bytes.Buffer
	command.Stdout = io.MultiWriter(os.Stdout, &output)
	command.SysProcAttr = &syscall.SysProcAttr{Credential: &syscall.Credential{Uid: uid, Gid: gid, Groups: []uint32{}}, Pdeathsig: syscall.SIGKILL}
	if err := command.Run(); err != nil {
		return err
	}
	if !strings.Contains(output.String(), "1 passed; 0 failed") {
		return fmt.Errorf("Actual packet confinement proof did not execute its kernel test")
	}
	return nil
}

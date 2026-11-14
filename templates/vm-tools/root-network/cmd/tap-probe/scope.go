// SPDX-License-Identifier: FSL-1.1-ALv2
// Exercise the actual scope subprocess, transferred namespace and kernel lease. Holding the
// test's real flock represents only lease ownership; this does not claim a VM guest boot.
package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"syscall"
	"time"

	rootnetwork "cubepals/pryx/vm-root-network-experiment"
	"github.com/containernetworking/plugins/pkg/ns"
	"github.com/vishvananda/netlink"
	"golang.org/x/sys/unix"
)

func scopeProbe(helper, directory string, uid, gid uint32, agent, packet, packetProof string) error {
	if os.Geteuid() != 0 || uid == 0 || gid == 0 || !filepath.IsAbs(helper) {
		return fmt.Errorf("Scope acceptance requires root and fixed ordinary identities")
	}
	before, err := links()
	if err != nil {
		return err
	}
	work, err := os.MkdirTemp("", "pryx-scope-lifetime-")
	if err != nil {
		return err
	}
	defer os.RemoveAll(work)
	held, err := os.OpenFile(filepath.Join(work, "monitor.lock"), os.O_RDWR|os.O_CREATE|os.O_EXCL, 0600)
	if err != nil {
		return err
	}
	defer held.Close()
	if err := unix.Flock(int(held.Fd()), unix.LOCK_EX|unix.LOCK_NB); err != nil {
		return err
	}
	pair, err := unix.Socketpair(unix.AF_UNIX, unix.SOCK_DGRAM|unix.SOCK_CLOEXEC, 0)
	if err != nil {
		return err
	}
	defer unix.Close(pair[0])
	sender := os.NewFile(uintptr(pair[1]), "scope grant channel")
	defer sender.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	command := exec.CommandContext(ctx, helper, "--plugins", directory, "--state", work,
		"--lease", held.Name(), "--uid", fmt.Sprint(uid), "--gid", fmt.Sprint(gid))
	if agent != "" || packet != "" {
		command.Args = append(command.Args, "--bridge-agent", agent, "--bridge-packet", packet)
	}
	command.Stdout, command.Stderr = sender, os.Stderr
	input, err := command.StdinPipe()
	if err != nil {
		return err
	}
	if err := command.Start(); err != nil {
		return err
	}
	finished := make(chan struct{})
	var result error
	go func() { result = command.Wait(); close(finished) }()
	defer func() {
		input.Close()
		unix.Flock(int(held.Fd()), unix.LOCK_UN)
		cancel()
		<-finished
	}()
	deadline, _ := ctx.Deadline()
	timeout := unix.NsecToTimeval(time.Until(deadline).Nanoseconds())
	if err := unix.SetsockoptTimeval(pair[0], unix.SOL_SOCKET, unix.SO_RCVTIMEO, &timeout); err != nil {
		return err
	}
	metadata, files, err := rootnetwork.ReceiveFiles(pair[0])
	if err != nil {
		return err
	}
	defer func() {
		for _, file := range files {
			file.Close()
		}
	}()
	var grant rootnetwork.ScopeHandoff
	if err := json.Unmarshal(metadata, &grant); err != nil {
		return err
	}
	expected := rootnetwork.ScopeAdvertisement().Descriptors
	if agent != "" || packet != "" {
		expected = rootnetwork.ScopeAdvertisement().BridgeDescriptors
	}
	if !grant.HostFirewall || grant.Version != rootnetwork.ScopeVersion || !slices.Equal(grant.Descriptors, expected) || len(files) != len(grant.Descriptors) {
		return fmt.Errorf("Actual scope metadata differs from its protocol owner")
	}
	roles := map[rootnetwork.ScopeRole]*os.File{}
	for index, role := range grant.Descriptors {
		attributes, err := unix.FcntlInt(files[index].Fd(), unix.F_GETFD, 0)
		if err != nil || attributes&unix.FD_CLOEXEC == 0 {
			return fmt.Errorf("Scope authority must be close-on-exec: %v", err)
		}
		roles[role] = files[index]
	}
	original, err := os.Stat("/proc/self/ns/net")
	if err != nil {
		return err
	}
	received, err := roles[rootnetwork.ScopeNamespace].Stat()
	if err != nil {
		return err
	}
	if os.SameFile(original, received) {
		return fmt.Errorf("Scope did not grant a distinct kernel network namespace")
	}
	space, err := ns.GetNS(fmt.Sprintf("/proc/%d/fd/%d", os.Getpid(), roles[rootnetwork.ScopeNamespace].Fd()))
	if err != nil {
		return err
	}
	defer space.Close()
	bridgeIdentity := 0
	if agent != "" || packet != "" {
		bridgeIdentity, err = inspectBridge(space, grant, roles)
		if err != nil {
			return err
		}
	}
	if packetProof != "" {
		if bridgeIdentity == 0 {
			return fmt.Errorf("Packet confinement requires the actual bridge queue")
		}
		if err := confinedPacketProof(ctx, packetProof, agent, roles[rootnetwork.ScopePacketTap], uid, gid); err != nil {
			return err
		}
	}
	link, err := tapLink(space, grant.Tap)
	if err != nil {
		return err
	}
	identity := link.Attrs().Index
	executable, err := os.Executable()
	if err != nil {
		return err
	}
	if err := space.Do(func(ns.NetNS) error {
		child := exec.CommandContext(ctx, executable, "--open", grant.Tap)
		child.SysProcAttr = &syscall.SysProcAttr{Credential: &syscall.Credential{Uid: uid, Gid: gid, Groups: []uint32{}}}
		return child.Run()
	}); err != nil {
		return err
	}
	if err := input.Close(); err != nil {
		return err
	}
	select {
	case <-finished:
		return fmt.Errorf("Scope retired while the kernel monitor lease was held: %v", result)
	case <-time.After(200 * time.Millisecond):
	}
	link, err = tapLink(space, grant.Tap)
	if err != nil || link.Attrs().Index != identity {
		return fmt.Errorf("Lease did not preserve the actual TAP: %v", err)
	}
	if err := unix.Flock(int(held.Fd()), unix.LOCK_UN); err != nil {
		return err
	}
	select {
	case <-finished:
	case <-ctx.Done():
		return ctx.Err()
	}
	if result != nil {
		return result
	}
	_, err = tapLink(space, grant.Tap)
	var absent netlink.LinkNotFoundError
	if !errors.As(err, &absent) {
		return fmt.Errorf("Retired scope TAP remains observable: %v", err)
	}
	if bridgeIdentity != 0 {
		if err := retiredBridge(space, grant.PacketTap, bridgeIdentity); err != nil {
			return err
		}
	}
	after, err := links()
	if err != nil {
		return err
	}
	if len(after) != len(before) {
		return fmt.Errorf("Scope changed a host interface")
	}
	for name, identity := range before {
		if after[name] != identity {
			return fmt.Errorf("Scope changed a host neighbour")
		}
	}
	return json.NewEncoder(os.Stdout).Encode(map[string]bool{"actualSubprocessGrant": true,
		"ordinaryOwnerOpened": true, "leaseBlockedRetirement": true, "explicitDeleteConfirmed": true, "neighboursUnchanged": true, "bridgeDescriptorsAndFirewall": bridgeIdentity != 0})
}

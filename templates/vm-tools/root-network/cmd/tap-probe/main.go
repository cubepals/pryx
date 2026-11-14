// SPDX-License-Identifier: FSL-1.1-ALv2
// Exercise the actual pinned CNI TAP plugin on the disposable root runner. No VMM is
// attached, so this proves neither production lifetime ownership nor packet/firewall policy.
package main

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"math"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"syscall"
	"time"

	rootnetwork "cubepals/pryx/vm-root-network-experiment"
	"github.com/containernetworking/cni/libcni"
	"github.com/containernetworking/cni/pkg/types"
	current "github.com/containernetworking/cni/pkg/types/100"
	"github.com/containernetworking/cni/pkg/version"
	"github.com/containernetworking/plugins/pkg/ns"
	"github.com/vishvananda/netlink"
	"golang.org/x/sys/unix"
)

func openTap(name string) error {
	if os.Geteuid() == 0 {
		return fmt.Errorf("The monitor TAP open must run as an ordinary user")
	}
	fd, err := unix.Open("/dev/net/tun", unix.O_RDWR|unix.O_CLOEXEC, 0)
	if err != nil {
		return err
	}
	defer unix.Close(fd)
	request, err := unix.NewIfreq(name)
	if err != nil {
		return err
	}
	request.SetUint16(unix.IFF_TAP | unix.IFF_NO_PI | unix.IFF_VNET_HDR)
	return unix.IoctlIfreq(fd, unix.TUNSETIFF, request)
}

func links() (map[string]int, error) {
	all, err := netlink.LinkList()
	if err != nil {
		return nil, err
	}
	names := map[string]int{}
	for _, link := range all {
		names[link.Attrs().Name] = link.Attrs().Index
	}
	return names, nil
}

func tapLink(space ns.NetNS, name string) (netlink.Link, error) {
	var link netlink.Link
	err := space.Do(func(ns.NetNS) error {
		var err error
		link, err = netlink.LinkByName(name)
		return err
	})
	return link, err
}

func probe(directory string, uid, gid uint32) error {
	if os.Geteuid() != 0 || uid == 0 || gid == 0 || uid == ^uint32(0) || gid == ^uint32(0) {
		return fmt.Errorf("Root TAP acceptance requires root and fixed non-root monitor IDs")
	}
	if !filepath.IsAbs(directory) {
		return fmt.Errorf("CNI programs require an absolute fixture directory")
	}
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	before, err := links()
	if err != nil {
		return err
	}
	var nonce [8]byte
	if _, err := rand.Read(nonce[:]); err != nil {
		return err
	}
	name := "pv" + hex.EncodeToString(nonce[:])[:unix.IFNAMSIZ-3]
	if _, exists := before[name]; exists {
		return fmt.Errorf("TAP fixture name is occupied")
	}
	handle, err := rootnetwork.NewNamespace()
	if err != nil {
		return err
	}
	defer handle.Close()
	// The parent retains this namespace descriptor through ADD, monitor open and DEL.
	// CNI requires a target namespace distinct from the plugin's own namespace.
	space, err := ns.GetNS(fmt.Sprintf("/proc/%d/fd/%d", os.Getpid(), int(handle)))
	if err != nil {
		return err
	}
	defer space.Close()
	configuration := rootnetwork.Tap{NetConf: types.NetConf{CNIVersion: version.Current(), Name: "pryx-tap-probe", Type: "tap"}, Owner: uid, Group: gid}
	data, err := json.Marshal(configuration)
	if err != nil {
		return err
	}
	network, err := libcni.ConfFromBytes(data)
	if err != nil {
		return err
	}
	runtime := &libcni.RuntimeConf{ContainerID: hex.EncodeToString(nonce[:]), NetNS: space.Path(), IfName: name}
	cache, err := os.MkdirTemp("", "pryx-cni-")
	if err != nil {
		return err
	}
	defer os.RemoveAll(cache)
	cni := libcni.NewCNIConfigWithCacheDir([]string{directory}, cache, nil)
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	result, err := cni.AddNetwork(ctx, network, runtime)
	if err != nil {
		return err
	}
	link, err := tapLink(space, name)
	if err != nil {
		return err
	}
	identity := link.Attrs().Index
	attempted := false
	cleanup := func() error {
		if attempted {
			return fmt.Errorf("TAP cleanup was already submitted")
		}
		attempted = true
		observed, err := tapLink(space, name)
		if err != nil {
			return err
		}
		if observed.Attrs().Index != identity {
			return fmt.Errorf("TAP identity changed; deletion is refused")
		}
		return cni.DelNetwork(ctx, network, runtime)
	}
	defer func() {
		if !attempted {
			_ = cleanup()
		}
	}()
	typed, err := current.NewResultFromResult(result)
	if err != nil {
		return err
	}
	if len(typed.Interfaces) != 1 || typed.Interfaces[0].Name != name || len(typed.IPs) != 0 {
		return fmt.Errorf("The actual CNI result does not describe the isolated L2 fixture")
	}
	tap, ok := link.(*netlink.Tuntap)
	if !ok || tap.Owner != uid || tap.Group != gid || tap.Mode != netlink.TUNTAP_MODE_TAP {
		return fmt.Errorf("The actual TAP ownership/type differs from the host plan")
	}
	executable, err := os.Executable()
	if err != nil {
		return err
	}
	if err := space.Do(func(ns.NetNS) error {
		child := exec.CommandContext(ctx, executable, "--open", name)
		child.SysProcAttr = &syscall.SysProcAttr{Credential: &syscall.Credential{Uid: uid, Gid: gid, Groups: []uint32{}}}
		if output, err := child.CombinedOutput(); err != nil {
			observed, _ := json.Marshal(rootnetwork.Credentials())
			return fmt.Errorf("Ordinary monitor TAP open: %w: %s; actual parent authority: %s", err, output, observed)
		}
		return nil
	}); err != nil {
		return err
	}
	if err := cleanup(); err != nil {
		return err
	}
	_, err = tapLink(space, name)
	var absent netlink.LinkNotFoundError
	if !errors.As(err, &absent) {
		return fmt.Errorf("Retired TAP is still observable: %v", err)
	}
	after, err := links()
	if err != nil {
		return err
	}
	if len(before) != len(after) {
		return fmt.Errorf("TAP acceptance changed a neighbouring interface")
	}
	for name, index := range before {
		if after[name] != index {
			return fmt.Errorf("A neighbouring interface identity changed")
		}
	}
	return json.NewEncoder(os.Stdout).Encode(map[string]any{"tapCreated": true, "ordinaryOwnerOpened": true, "identityBoundDelete": true, "neighboursUnchanged": true, "cniVersion": version.Current()})
}

func main() {
	scope := flag.String("scope-helper", "", "prove the protected scope subprocess and kernel lease")
	gateways := flag.Bool("gateways", false, "prove namespace-bound gateway descriptor ownership")
	client := flag.String("gateway-client", "", "ordinary namespace gateway client")
	proof := flag.String("proof", "", "live parent proof")
	open := flag.String("open", "", "ordinary monitor open of an existing owned TAP")
	directory := flag.String("plugins", "", "absolute directory of the pinned CNI TAP program")
	uid := flag.Uint64("uid", 0, "fixed host monitor UID")
	gid := flag.Uint64("gid", 0, "fixed host monitor GID")
	flag.Parse()
	var err error
	if *client != "" {
		err = gatewayClient(*client, *proof)
	} else if *open != "" {
		err = openTap(*open)
	} else {
		if *uid > math.MaxUint32 || *gid > math.MaxUint32 {
			err = fmt.Errorf("Monitor IDs exceed their kernel type")
		} else {
			if *scope != "" {
				err = scopeProbe(*scope, *directory, uint32(*uid), uint32(*gid))
			} else if *gateways {
				err = gatewayProbe(*directory, uint32(*uid), uint32(*gid))
			} else {
				err = probe(*directory, uint32(*uid), uint32(*gid))
			}
		}
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

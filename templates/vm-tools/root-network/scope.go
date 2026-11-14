// SPDX-License-Identifier: FSL-1.1-ALv2
// Own one CNI TAP in a private anonymous kernel network namespace. This scope changes no
// original-host interface, route, forwarding setting or firewall and never starts a VM.
package rootnetwork

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"net/netip"
	"os"
	"path/filepath"

	"github.com/containernetworking/cni/libcni"
	"github.com/containernetworking/cni/pkg/types"
	current "github.com/containernetworking/cni/pkg/types/100"
	"github.com/containernetworking/cni/pkg/version"
	"github.com/containernetworking/plugins/pkg/ns"
	"github.com/vishvananda/netlink"
	"github.com/vishvananda/netns"
)

// These addresses exist only inside each independent namespace; no host pool is reserved.
const hostCIDR = "10.73.0.1/30"

// Profile is owned by this root network component, not a workload or a public API.
type Profile struct {
	Host      string `json:"host"`
	GuestCIDR string `json:"guestCidr"`
	GuestMAC  string `json:"guestMac"`
	Interface string `json:"interface"`
	MTU       int    `json:"mtu"`
	Subnet    string `json:"subnet"`
}

type Scope struct {
	Space         ns.NetNS
	Profile       Profile
	Tap           string
	handle        netns.NsHandle
	cni           *libcni.CNIConfig
	configuration *libcni.NetworkConfig
	invocation    *libcni.RuntimeConf
	identity      int
	attempted     bool
}

func (scope *Scope) link() (netlink.Link, error) {
	var link netlink.Link
	err := scope.Space.Do(func(ns.NetNS) error { var err error; link, err = netlink.LinkByName(scope.Tap); return err })
	return link, err
}

func Open(ctx context.Context, directory, cache, name string, uid, gid uint32) (*Scope, error) {
	if os.Geteuid() != 0 || uid == 0 || gid == 0 || uid == ^uint32(0) || gid == ^uint32(0) || !filepath.IsAbs(directory) || !filepath.IsAbs(cache) {
		return nil, fmt.Errorf("Root TAP scope needs fixed ordinary identities and absolute host paths")
	}
	handle, err := NewNamespace()
	if err != nil {
		return nil, err
	}
	scope := &Scope{handle: handle, Tap: name}
	space, err := ns.GetNS(fmt.Sprintf("/proc/%d/fd/%d", os.Getpid(), int(handle)))
	if err != nil {
		handle.Close()
		return nil, err
	}
	scope.Space = space
	successful := false
	defer func() {
		if !successful {
			scope.Space.Close()
			scope.handle.Close()
		}
	}()
	var identity [16]byte
	if _, err := rand.Read(identity[:]); err != nil {
		return nil, err
	}
	configuration := Tap{NetConf: types.NetConf{CNIVersion: version.Current(), Name: "pryx-root-tap", Type: "tap"}, Owner: uid, Group: gid}
	data, err := json.Marshal(configuration)
	if err != nil {
		return nil, err
	}
	scope.configuration, err = libcni.ConfFromBytes(data)
	if err != nil {
		return nil, err
	}
	scope.invocation = &libcni.RuntimeConf{ContainerID: hex.EncodeToString(identity[:]), NetNS: space.Path(), IfName: name}
	scope.cni = libcni.NewCNIConfigWithCacheDir([]string{directory}, cache, nil)
	result, err := scope.cni.AddNetwork(ctx, scope.configuration, scope.invocation)
	if err != nil {
		return nil, err
	}
	typed, err := current.NewResultFromResult(result)
	if err != nil {
		return nil, err
	}
	if len(typed.Interfaces) != 1 || typed.Interfaces[0].Name != name || len(typed.IPs) != 0 {
		return nil, fmt.Errorf("CNI did not return the owned L2 TAP")
	}
	if err := scope.configure(uid, gid); err != nil {
		return nil, err
	}
	successful = true
	return scope, nil
}

func (scope *Scope) configure(uid, gid uint32) error {
	return scope.Space.Do(func(ns.NetNS) error {
		link, err := netlink.LinkByName(scope.Tap)
		if err != nil {
			return err
		}
		tap, ok := link.(*netlink.Tuntap)
		if !ok || tap.Owner != uid || tap.Group != gid || tap.Mode != netlink.TUNTAP_MODE_TAP {
			return fmt.Errorf("Actual TAP ownership or type differs from host policy")
		}
		scope.identity = link.Attrs().Index
		address, err := netlink.ParseAddr(hostCIDR)
		if err != nil {
			return err
		}
		if err := netlink.AddrAdd(link, address); err != nil {
			return err
		}
		if err := netlink.LinkSetUp(link); err != nil {
			return err
		}
		loopback, err := netlink.LinkByName("lo") // Linux's initial namespace loopback.
		if err != nil {
			return err
		}
		if err := netlink.LinkSetUp(loopback); err != nil {
			return err
		}
		prefix, err := netip.ParsePrefix(hostCIDR)
		if err != nil {
			return err
		}
		scope.Profile = Profile{Host: address.IP.String(), GuestCIDR: netip.PrefixFrom(prefix.Addr().Next(), prefix.Bits()).String(),
			GuestMAC: "02:50:59:01:00:02", Interface: "eth0", MTU: link.Attrs().MTU, Subnet: prefix.Masked().String()}
		return nil
	})
}

// Close is submitted only after any attached monitor retires. It never retries DEL.
func (scope *Scope) Close(ctx context.Context) error {
	if scope.attempted {
		return fmt.Errorf("Root TAP cleanup was already submitted")
	}
	scope.attempted = true
	link, err := scope.link()
	if err != nil {
		return err
	}
	if link.Attrs().Index != scope.identity {
		return fmt.Errorf("Root TAP identity changed; deletion refused")
	}
	if err := scope.cni.DelNetwork(ctx, scope.configuration, scope.invocation); err != nil {
		return err
	}
	_, err = scope.link()
	var absent netlink.LinkNotFoundError
	if !errors.As(err, &absent) {
		return fmt.Errorf("Retired root TAP remains observable: %v", err)
	}
	return errors.Join(scope.Space.Close(), scope.handle.Close())
}

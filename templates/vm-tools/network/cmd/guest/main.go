// SPDX-License-Identifier: FSL-1.1-ALv2
// Command guest configures networking inside the trusted VM before any image command runs.
// The maintained netlink and DHCP libraries own device setup and leases. The unchanged
// gvisor forwarder owns all packet framing; this adapter never invokes an image's tools.
package main

import (
	"context"
	"flag"
	"fmt"
	"net"
	"os"
	"os/exec"
	"os/signal"
	"strings"
	"syscall"
	"time"

	"github.com/insomniacslk/dhcp/dhcpv4/nclient4"
	"github.com/vishvananda/netlink"
	"golang.org/x/sys/unix"
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

func run() error {
	loopbackOnly := flag.Bool("loopback-only", false, "Bring up only guest loopback and exit")
	iface := flag.String("iface", "", "Trusted guest interface")
	mac := flag.String("mac", "", "Host-selected guest MAC")
	mtu := flag.Int("mtu", 0, "Host-selected MTU")
	forwarder := flag.String("forwarder", "", "Optional fixed trusted bootstrap forwarder")
	endpoint := flag.String("endpoint", "", "Host-CID packet transport endpoint")
	resolver := flag.String("resolver", "", "Precreated root-owned resolver file")
	flag.Parse()
	if os.Geteuid() != 0 || os.Getppid() != 1 {
		return fmt.Errorf("Guest network setup requires trusted VM init")
	}
	// Linux networking documents lo as the initial namespace loopback interface.
	loopback, err := netlink.LinkByName("lo")
	if err != nil {
		return err
	}
	if err := netlink.LinkSetUp(loopback); err != nil {
		return err
	}
	if *loopbackOnly {
		return nil
	}
	if *iface == "" || *resolver == "" || *mtu <= 0 {
		return fmt.Errorf("Guest network setup requires trusted VM init and complete host configuration")
	}
	hardware, err := net.ParseMAC(*mac)
	if err != nil {
		return err
	}
	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGTERM, syscall.SIGINT)
	defer stop()
	if *forwarder != "" {
		device := &netlink.Tuntap{LinkAttrs: netlink.LinkAttrs{Name: *iface}, Mode: netlink.TUNTAP_MODE_TAP,
			Flags: netlink.TUNTAP_DEFAULTS | unix.IFF_NO_PI}
		if err := netlink.LinkAdd(device); err != nil {
			return err
		}
		defer netlink.LinkDel(device)
	}
	link, err := netlink.LinkByName(*iface)
	if err != nil {
		return err
	}
	if err := netlink.LinkSetHardwareAddr(link, hardware); err != nil {
		return err
	}
	if err := netlink.LinkSetMTU(link, *mtu); err != nil {
		return err
	}
	if err := netlink.LinkSetUp(link); err != nil {
		return err
	}
	errors := make(chan error, 1)
	if *forwarder != "" {
		helper := exec.CommandContext(ctx, *forwarder, "-url", *endpoint, "-iface", *iface,
			"-preexisting", "-stop-if-exist", "", "-mtu", fmt.Sprint(*mtu))
		helper.Env = []string{}
		helper.Stdout, helper.Stderr = os.Stderr, os.Stderr
		if err := helper.Start(); err != nil {
			return err
		}
		done := make(chan struct{})
		defer func() { _ = helper.Process.Signal(syscall.SIGTERM); <-done }()
		go func() { errors <- helper.Wait(); close(done) }()
	}
	client, err := nclient4.New(*iface, nclient4.WithHWAddr(hardware))
	if err != nil {
		return err
	}
	defer client.Close()
	acquire, cancel := context.WithTimeout(ctx, 20*time.Second)
	value, err := client.Request(acquire)
	cancel()
	if err != nil {
		return err
	}
	if err := lease(link, value, *resolver); err != nil {
		return err
	}
	// EOF on this private pipe is readiness, with no repeated wire token or image-controlled file.
	if err := os.Stdout.Close(); err != nil {
		return err
	}
	for {
		// The DHCP owner supplies its absent-option default; never invent a lease duration.
		duration := value.ACK.IPAddressLeaseTime(0)
		if duration <= 0 {
			return fmt.Errorf("Guest DHCP lease has no positive lifetime")
		}
		timer := time.NewTimer(duration / 2)
		select {
		case <-ctx.Done():
			timer.Stop()
			return nil
		case err := <-errors:
			timer.Stop()
			return fmt.Errorf("Guest packet forwarder exited: %v", err)
		case <-timer.C:
			renew, cancel := context.WithTimeout(ctx, 20*time.Second)
			value, err = client.Renew(renew, value)
			cancel()
			if err != nil {
				return err
			}
			if err := lease(link, value, *resolver); err != nil {
				return err
			}
		}
	}
}

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

// SPDX-License-Identifier: FSL-1.1-ALv2
// Command guest configures networking inside the trusted VM before any image command runs.
// Parts (`network/cmd/guest/`): main.go owns process lifetime and trusted device startup;
// lease.go applies maintained DHCP lease values and writes the precreated resolver;
// static.go applies connected host-selected IPv4 TAP addressing without DHCP or a forwarder;
// carrier.go waits for the forwarder to hold the TAP before the first DHCP request.
// The maintained netlink and DHCP libraries own device setup and leases. The unchanged
// gvisor forwarder owns all packet framing; this adapter never invokes an image's tools.
package main

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"net"
	"os"
	"os/exec"
	"os/signal"
	"syscall"
	"time"

	"github.com/containers/gvisor-tap-vsock/pkg/types"
	"github.com/insomniacslk/dhcp/dhcpv4/nclient4"
	"github.com/mdlayher/vsock"
	"github.com/vishvananda/netlink"
	"golang.org/x/sys/unix"
)

func run() error {
	metadata := flag.Bool("metadata", false, "read trusted static network parser support")
	address := flag.String("address", "", "optional host-selected static IPv4 prefix")
	gateway := flag.String("gateway", "", "optional host-selected static gateway and resolver")
	loopbackOnly := flag.Bool("loopback-only", false, "Bring up only guest loopback and exit")
	resetConnections := flag.Bool("reset-connections", false, "Discard the restored guest's old connection tracking")
	iface := flag.String("iface", "", "Trusted guest interface")
	mac := flag.String("mac", "", "Host-selected guest MAC")
	mtu := flag.Int("mtu", 0, "Host-selected MTU")
	forwarder := flag.String("forwarder", "", "Optional fixed trusted bootstrap forwarder")
	port := flag.Uint("vsock-port", 0, "Host-selected packet transport port")
	resolver := flag.String("resolver", "", "Precreated root-owned resolver file")
	flag.Parse()
	if *metadata {
		return json.NewEncoder(os.Stdout).Encode(map[string]bool{"staticConfiguration": true})
	}
	if (*address != "" || *gateway != "") && (*address == "" || *gateway == "" || *forwarder != "") {
		return fmt.Errorf("Static TAP addressing requires both host addresses without a packet forwarder")
	}
	if os.Geteuid() != 0 || os.Getppid() != 1 {
		return fmt.Errorf("Guest network setup requires trusted VM init")
	}
	if *resetConnections {
		return netlink.ConntrackTableFlush(netlink.ConntrackTable)
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
	if *address != "" {
		if err := staticNetwork(link, *address, *gateway, *resolver); err != nil {
			return err
		}
		if err := os.Stdout.Close(); err != nil {
			return err
		}
		<-ctx.Done()
		return nil
	}
	errors := make(chan error, 1)
	if *forwarder != "" {
		if *port == 0 {
			return fmt.Errorf("Guest packet transport port is missing")
		}
		endpoint := fmt.Sprintf("vsock://%d:%d%s", vsock.Host, *port, types.ConnectPath)
		helper := exec.CommandContext(ctx, *forwarder, "-url", endpoint, "-iface", *iface,
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
	if *forwarder != "" {
		if err := awaitForwarder(acquire, link.Attrs().Index, errors); err != nil {
			cancel()
			return err
		}
	}
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

// SPDX-License-Identifier: FSL-1.1-ALv2
// Wait until the packet forwarder holds the guest's TAP queue before the first DHCP DISCOVER.
// The unchanged forwarder dials the host and sends its connect request before it attaches the
// TAP, so the kernel's carrier on that device (IFF_LOWER_UP) means frames now reach the host. A
// DISCOVER sent earlier is lost and the DHCP client waits out its whole retry timeout.
package main

import (
	"context"
	"fmt"

	"github.com/vishvananda/netlink"
	"golang.org/x/sys/unix"
)

func hasCarrier(link netlink.Link) bool { return link.Attrs().RawFlags&unix.IFF_LOWER_UP != 0 }

// carrier returns once the link at index has carrier, or with an error when the forwarder
// exits or ctx ends first. current is read once, after updates are already being received.
func carrier(ctx context.Context, index int, current func() (netlink.Link, error),
	updates <-chan netlink.LinkUpdate, exited <-chan error) error {
	link, err := current()
	if err != nil {
		return err
	}
	if hasCarrier(link) {
		return nil
	}
	for {
		select {
		case <-ctx.Done():
			return fmt.Errorf("Guest packet forwarder did not attach its device: %w", ctx.Err())
		case err := <-exited:
			return fmt.Errorf("Guest packet forwarder exited: %v", err)
		case update, ok := <-updates:
			if !ok {
				return fmt.Errorf("Guest link updates ended before the forwarder attached")
			}
			if update.Link != nil && update.Attrs().Index == index && hasCarrier(update.Link) {
				return nil
			}
		}
	}
}

// awaitForwarder subscribes to the kernel's link updates, then waits for carrier on index.
func awaitForwarder(ctx context.Context, index int, exited <-chan error) error {
	updates := make(chan netlink.LinkUpdate, 16)
	done := make(chan struct{})
	defer close(done)
	if err := netlink.LinkSubscribe(updates, done); err != nil {
		return err
	}
	return carrier(ctx, index, func() (netlink.Link, error) { return netlink.LinkByIndex(index) }, updates, exited)
}

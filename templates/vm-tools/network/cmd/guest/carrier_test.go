// SPDX-License-Identifier: FSL-1.1-ALv2
// Prove the first DHCP request waits for the forwarder's carrier, and no longer than that.
package main

import (
	"context"
	"errors"
	"testing"
	"time"

	"github.com/vishvananda/netlink"
	"golang.org/x/sys/unix"
)

func tap(index int, flags uint32) netlink.Link {
	return &netlink.Tuntap{LinkAttrs: netlink.LinkAttrs{Index: index, RawFlags: flags}}
}

func TestRequestWaitsForTheForwardersCarrier(t *testing.T) {
	updates := make(chan netlink.LinkUpdate, 4)
	returned := make(chan error, 1)
	go func() {
		returned <- carrier(context.Background(), 7, func() (netlink.Link, error) { return tap(7, unix.IFF_UP), nil },
			updates, make(chan error))
	}()
	// Another link's carrier, and this link without one, must not release the request.
	updates <- netlink.LinkUpdate{Link: tap(8, unix.IFF_UP|unix.IFF_LOWER_UP)}
	updates <- netlink.LinkUpdate{Link: tap(7, unix.IFF_UP)}
	select {
	case err := <-returned:
		t.Fatalf("The request was released before the forwarder attached: %v", err)
	case <-time.After(100 * time.Millisecond):
	}
	updates <- netlink.LinkUpdate{Link: tap(7, unix.IFF_UP|unix.IFF_LOWER_UP)}
	select {
	case err := <-returned:
		if err != nil {
			t.Fatal(err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("The forwarder's carrier did not release the request")
	}
}

func TestCarrierAlreadyUpReleasesAtOnce(t *testing.T) {
	err := carrier(context.Background(), 7, func() (netlink.Link, error) { return tap(7, unix.IFF_UP|unix.IFF_LOWER_UP), nil },
		make(chan netlink.LinkUpdate), make(chan error))
	if err != nil {
		t.Fatal(err)
	}
}

func TestForwarderExitOrDeadlineEndsTheWait(t *testing.T) {
	down := func() (netlink.Link, error) { return tap(7, unix.IFF_UP), nil }
	exited := make(chan error, 1)
	exited <- errors.New("exit status 1")
	if err := carrier(context.Background(), 7, down, make(chan netlink.LinkUpdate), exited); err == nil {
		t.Fatal("A forwarder that exited released the request")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 50*time.Millisecond)
	defer cancel()
	if err := carrier(ctx, 7, down, make(chan netlink.LinkUpdate), make(chan error)); !errors.Is(err, context.DeadlineExceeded) {
		t.Fatalf("The wait outlived its deadline: %v", err)
	}
	closed := make(chan netlink.LinkUpdate)
	close(closed)
	if err := carrier(context.Background(), 7, down, closed, make(chan error)); err == nil {
		t.Fatal("Ended link updates released the request")
	}
}

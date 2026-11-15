// SPDX-License-Identifier: FSL-1.1-ALv2
// Prove over real loopback listeners that a full public pool closes the next public connection
// at once and still leaves the reserved binding its own capacity.
package main

import (
	"context"
	"errors"
	"io"
	"net"
	"os"
	"strings"
	"testing"
	"time"
)

func TestPublicConnectionsCannotConsumeReservedCapacity(t *testing.T) {
	const concurrency = 3
	bindings := []binding{{Host: "127.0.0.1", GuestPort: 1}, {Host: "127.0.0.1", GuestPort: 2},
		{Host: "127.0.0.1", GuestPort: 3, Reserved: true}}
	listeners := make([]net.Listener, len(bindings))
	for i := range bindings {
		listener, err := net.Listen("tcp", "127.0.0.1:0")
		if err != nil {
			t.Fatal(err)
		}
		listeners[i] = listener
	}
	ctx, cancel := context.WithCancel(context.Background())
	served := make(chan uint32, 4*concurrency)
	release := make(chan struct{})
	finished := make(chan struct{})
	go func() {
		defer close(finished)
		serve(ctx, cancel, listeners, bindings, capacities(bindings, concurrency), func(client net.Conn, port uint32, _ func(string)) {
			served <- port
			select {
			case <-release:
			case <-ctx.Done():
			}
		}, false)
	}()
	defer func() {
		cancel()
		close(release)
		<-finished
	}()
	dial := func(index int) net.Conn {
		t.Helper()
		connection, err := net.Dial("tcp", listeners[index].Addr().String())
		if err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() { connection.Close() })
		return connection
	}
	expectServed := func(port uint32) {
		t.Helper()
		select {
		case got := <-served:
			if got != port {
				t.Fatalf("Served guest port %d, want %d", got, port)
			}
		case <-time.After(5 * time.Second):
			t.Fatalf("A connection to guest port %d was not served", port)
		}
	}
	// Public connections across two ports fill the one public pool.
	for i := range concurrency {
		dial(i % 2)
		expectServed(bindings[i%2].GuestPort)
	}
	for _, index := range []int{0, 1} {
		refused := dial(index)
		refused.SetReadDeadline(time.Now().Add(5 * time.Second))
		if _, err := refused.Read(make([]byte, 1)); !errors.Is(err, io.EOF) {
			t.Fatalf("A public connection past the shared capacity was not closed at once: %v", err)
		}
	}
	// The agent's reserved binding still has all of its own.
	for range concurrency {
		dial(2)
		expectServed(3)
	}
	refused := dial(2)
	refused.SetReadDeadline(time.Now().Add(5 * time.Second))
	if _, err := refused.Read(make([]byte, 1)); !errors.Is(err, io.EOF) {
		t.Fatalf("A reserved connection past its capacity was not closed at once: %v", err)
	}
	select {
	case port := <-served:
		t.Fatalf("A connection past capacity reached guest port %d", port)
	default:
	}
}

func TestBindingsWithoutReservationShareOnePool(t *testing.T) {
	pools := capacities([]binding{{GuestPort: 1}, {GuestPort: 2}}, 1)
	if !pools[0].take() || pools[1].take() {
		t.Fatal("Unreserved bindings must share one pool")
	}
	reserved := capacities([]binding{{GuestPort: 1}, {GuestPort: 2, Reserved: true}}, 1)
	if !reserved[0].take() || !reserved[1].take() {
		t.Fatal("A reserved binding must have its own pool")
	}
}

func TestForwardingWithoutAReservedBindingIsRefused(t *testing.T) {
	err := forward(request{Socket: "unused", Forward: &forwarding{Bindings: []binding{{Host: "127.0.0.1", GuestPort: 1 << 17}},
		Concurrency: 1, Parent: uint32(os.Getppid())}})
	if err == nil || !strings.Contains(err.Error(), "reserved") {
		t.Fatalf("Forwarding without the agent's reserved binding was not refused: %v", err)
	}
}

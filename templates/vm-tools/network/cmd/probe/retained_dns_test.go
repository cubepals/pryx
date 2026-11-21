// SPDX-License-Identifier: FSL-1.1-ALv2
// A real DNS server observes one client endpoint, and each lookup publishes that endpoint atomically.
package main

import (
	"bytes"
	"encoding/binary"
	"net"
	"os"
	"path/filepath"
	"strconv"
	"testing"
	"time"

	"github.com/miekg/dns"
	"golang.org/x/sys/unix"
)

func TestRetainedDNSUsesOneSocketAcrossLookups(t *testing.T) {
	listener, err := net.ListenPacket("udp4", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	peers := make(chan string, 3)
	server := &dns.Server{PacketConn: listener, Handler: dns.HandlerFunc(func(w dns.ResponseWriter, request *dns.Msg) {
		peers <- w.RemoteAddr().String()
		answer := new(dns.Msg)
		answer.SetReply(request)
		answer.Answer = []dns.RR{&dns.A{Hdr: dns.RR_Header{Name: request.Question[0].Name,
			Rrtype: dns.TypeA, Class: dns.ClassINET}, A: net.IPv4(127, 0, 0, 1)}}
		if err := w.WriteMsg(answer); err != nil {
			t.Error(err)
		}
	})}
	go func() { _ = server.ActivateAndServe() }()
	defer server.Shutdown()
	remote := listener.LocalAddr().(*net.UDPAddr)
	socket, err := net.DialUDP("udp4", nil, remote)
	if err != nil {
		t.Fatal(err)
	}
	defer socket.Close()
	dir := t.TempDir()
	fd, err := unix.InotifyInit1(unix.IN_CLOEXEC | unix.IN_NONBLOCK)
	if err != nil {
		t.Fatal(err)
	}
	watch := os.NewFile(uintptr(fd), dir)
	defer watch.Close()
	if _, err := unix.InotifyAddWatch(fd, dir, unix.IN_CREATE|unix.IN_CLOSE_WRITE|unix.IN_MOVED_TO); err != nil {
		t.Fatal(err)
	}
	ready := filepath.Join(dir, "dns-ready")
	finished := make(chan error, 1)
	go func() { finished <- retainedLookups(socket, "retained.pryx.invalid", ready) }()
	for phase := 0; phase < 3; phase++ {
		if phase != 0 {
			if err := os.WriteFile(ready+".continue"+strconv.Itoa(phase), nil, 0o600); err != nil {
				t.Fatal(err)
			}
		}
		marker := ready + strconv.Itoa(phase)
		awaitRetainedDNSMarker(t, watch, filepath.Base(marker))
		endpoint, err := os.ReadFile(marker)
		if err != nil {
			t.Fatalf("Phase %d did not publish its successful lookup: %v", phase, err)
		}
		if string(endpoint) != socket.LocalAddr().String() {
			t.Fatalf("Phase %d changed the socket's local endpoint: %s", phase, endpoint)
		}
		select {
		case endpoint := <-peers:
			if endpoint != socket.LocalAddr().String() {
				t.Fatalf("DNS server observed a different socket: %s", endpoint)
			}
		case <-time.After(requestTimeout):
			t.Fatalf("Phase %d made no DNS query", phase)
		}
	}
	if err := <-finished; err != nil {
		t.Fatal(err)
	}
}

func awaitRetainedDNSMarker(t *testing.T, watch *os.File, marker string) {
	t.Helper()
	if err := watch.SetReadDeadline(time.Now().Add(requestTimeout)); err != nil {
		t.Fatal(err)
	}
	var buffer [4096]byte
	for {
		n, err := watch.Read(buffer[:])
		if err != nil {
			t.Fatalf("Waiting for %s: %v", marker, err)
		}
		for events := buffer[:n]; len(events) > 0; {
			var event unix.InotifyEvent
			header, err := binary.Decode(events, binary.NativeEndian, &event)
			if err != nil || int(event.Len) > len(events)-header {
				t.Fatalf("Malformed inotify event: %v", err)
			}
			end := header + int(event.Len)
			name := bytes.TrimRight(events[header:end], "\x00")
			events = events[end:]
			if string(name) != marker {
				continue
			}
			if event.Mask&unix.IN_CREATE != 0 {
				t.Errorf("%s became visible as a new file before publication finished", marker)
			}
			// Wait for the direct writer to close so the DNS assertions still see its endpoint.
			if event.Mask&(unix.IN_MOVED_TO|unix.IN_CLOSE_WRITE) != 0 {
				return
			}
		}
	}
}

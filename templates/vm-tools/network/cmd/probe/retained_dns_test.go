// SPDX-License-Identifier: FSL-1.1-ALv2
// A real DNS server observes the same client endpoint for all three continued lookups.
package main

import (
	"net"
	"os"
	"path/filepath"
	"strconv"
	"testing"
	"time"

	"github.com/miekg/dns"
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
	ready := filepath.Join(t.TempDir(), "dns-ready")
	finished := make(chan error, 1)
	go func() { finished <- retainedLookups(socket, "retained.pryx.invalid", ready) }()
	for phase := 0; phase < 3; phase++ {
		if phase != 0 {
			if err := os.WriteFile(ready+".continue"+strconv.Itoa(phase), nil, 0o600); err != nil {
				t.Fatal(err)
			}
		}
		until := time.Now().Add(requestTimeout)
		for {
			endpoint, err := os.ReadFile(ready + strconv.Itoa(phase))
			if err == nil {
				if string(endpoint) != socket.LocalAddr().String() {
					t.Fatalf("Phase %d changed the socket's local endpoint: %s", phase, endpoint)
				}
				break
			}
			if !os.IsNotExist(err) || time.Now().After(until) {
				t.Fatalf("Phase %d did not publish its successful lookup: %v", phase, err)
			}
			time.Sleep(time.Millisecond)
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

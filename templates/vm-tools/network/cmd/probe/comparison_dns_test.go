// SPDX-License-Identifier: FSL-1.1-ALv2
// Exercise positive DNS reads using loopback and the comparison probe's deadline.
package main

import (
	"context"
	"errors"
	"net"
	"sync/atomic"
	"testing"
	"time"

	"github.com/miekg/dns"
)

func TestComparisonDNSReadsAgainAfterTimeout(t *testing.T) {
	listener, err := net.ListenPacket("udp4", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	started := make(chan struct{})
	var calls, withheld atomic.Int32
	const answer = "192.0.2.73"
	server := &dns.Server{
		PacketConn:        listener,
		NotifyStartedFunc: func() { close(started) },
		Handler: dns.HandlerFunc(func(w dns.ResponseWriter, request *dns.Msg) {
			if calls.Load() == 1 {
				withheld.Add(1)
				return
			}
			reply := new(dns.Msg)
			reply.SetReply(request)
			question := request.Question[0]
			if question.Qtype == dns.TypeA {
				reply.Answer = []dns.RR{&dns.A{Hdr: dns.RR_Header{Name: question.Name,
					Rrtype: dns.TypeA, Class: dns.ClassINET}, A: net.ParseIP(answer)}}
			}
			if err := w.WriteMsg(reply); err != nil {
				t.Error(err)
			}
		}),
	}
	go func() { _ = server.ActivateAndServe() }()
	<-started
	t.Cleanup(func() { _ = server.Shutdown() })

	dialer := new(net.Dialer)
	resolver := &net.Resolver{PreferGo: true, Dial: func(ctx context.Context, network, _ string) (net.Conn, error) {
		return dialer.DialContext(ctx, network, listener.LocalAddr().String())
	}}
	lookup := func(ctx context.Context, name string) ([]string, error) {
		calls.Add(1)
		if name != "example.com" {
			t.Fatalf("unexpected comparison hostname %q", name)
		}
		// Query only this name, regardless of the host's DNS search domains.
		return resolver.LookupHost(ctx, name+".")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 4*time.Second)
	defer cancel()
	addresses, attempts, err := lookupComparisonHost(ctx, 400*time.Millisecond, lookup)
	if err != nil || attempts != 2 || calls.Load() != 2 || withheld.Load() == 0 {
		t.Fatalf("lookup attempts=%d calls=%d withheld=%d error=%v", attempts, calls.Load(), withheld.Load(), err)
	}
	if len(addresses) != 1 || addresses[0] != answer {
		t.Fatalf("unexpected addresses %v", addresses)
	}
}

func TestComparisonDNSStopsAfterFailure(t *testing.T) {
	timeout := &net.DNSError{Err: "lookup timed out", IsTimeout: true}
	missing := &net.DNSError{Err: "no such host", IsNotFound: true}
	for _, tc := range []struct {
		name         string
		err          error
		cancelParent bool
		wantCalls    int
	}{
		{name: "empty answer", wantCalls: 1},
		{name: "name not found", err: missing, wantCalls: 1},
		{name: "parent canceled during lookup", err: timeout, cancelParent: true, wantCalls: 1},
		{name: "second timeout", err: timeout, wantCalls: 2},
	} {
		t.Run(tc.name, func(t *testing.T) {
			ctx, cancel := context.WithCancel(context.Background())
			defer cancel()
			calls := 0
			lookup := func(context.Context, string) ([]string, error) {
				calls++
				if tc.cancelParent {
					cancel()
				}
				return nil, tc.err
			}
			addresses, attempts, err := lookupComparisonHost(ctx, time.Second, lookup)
			if len(addresses) != 0 || err == nil || attempts != tc.wantCalls || calls != attempts {
				t.Fatalf("unexpected result: %v, attempts=%d calls=%d error=%v", addresses, attempts, calls, err)
			}
			if tc.err == missing && !errors.Is(err, missing) {
				t.Fatalf("lost the permanent lookup failure: %v", err)
			}
		})
	}
}

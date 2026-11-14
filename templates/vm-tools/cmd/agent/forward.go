// SPDX-License-Identifier: FSL-1.1-ALv2
// Private TCP listeners relay opaque streams through the maintained SDK's vsock dialer.
// Guest services retain their own authentication; this helper never parses or retries data.
package main

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"os/signal"
	"sync"
	"sync/atomic"
	"syscall"
	"time"

	"github.com/firecracker-microvm/firecracker-go-sdk/vsock"
)

type binding struct {
	Host      string `json:"host"`
	Port      uint16 `json:"port"`
	GuestPort uint32 `json:"guestPort"`
}
type forwarding struct {
	Bindings    []binding `json:"bindings"`
	Concurrency uint16    `json:"concurrency"`
	Parent      uint32    `json:"parent"`
	Diagnostics bool      `json:"diagnostics,omitempty"`
}

func forward(input request) error {
	plan := input.Forward
	if input.Socket == "" || plan == nil || len(plan.Bindings) == 0 || plan.Concurrency == 0 || int(plan.Parent) != os.Getppid() {
		return fmt.Errorf("Incomplete private forwarding configuration")
	}
	ctx, cancel := signal.NotifyContext(context.Background(), syscall.SIGTERM, syscall.SIGINT)
	defer cancel()
	listeners := make([]net.Listener, 0, len(plan.Bindings))
	defer func() {
		for _, listener := range listeners {
			listener.Close()
		}
	}()
	addresses := make([]string, 0, len(plan.Bindings))
	for _, bind := range plan.Bindings {
		ip := net.ParseIP(bind.Host)
		// Application service vsock ports occupy a distinct namespace above TCP's
		// u16 port range; this path can never expose init's private control channel.
		if ip == nil || (!ip.IsLoopback() && !ip.IsPrivate()) || bind.GuestPort <= uint32(^uint16(0)) {
			return fmt.Errorf("Forwarding requires a private host address and a service port")
		}
		listener, err := net.Listen("tcp", net.JoinHostPort(bind.Host, fmt.Sprint(bind.Port)))
		if err != nil {
			return err
		}
		listeners = append(listeners, listener)
		addresses = append(addresses, listener.Addr().String())
	}
	if err := json.NewEncoder(os.Stdout).Encode(addresses); err != nil {
		return err
	}
	if err := os.Stdout.Close(); err != nil {
		return err
	}
	go func() {
		ticker := time.NewTicker(250 * time.Millisecond)
		defer ticker.Stop()
		for {
			select {
			case <-ctx.Done():
				return
			case <-ticker.C:
				if os.Getppid() != int(plan.Parent) {
					cancel()
					return
				}
			}
		}
	}()
	capacity := make(chan struct{}, plan.Concurrency)
	var sequence atomic.Uint64
	var active sync.WaitGroup
	var accepting sync.WaitGroup
	accepting.Add(len(listeners))
	for i, listener := range listeners {
		port := plan.Bindings[i].GuestPort
		go func() {
			defer accepting.Done()
			for {
				client, err := listener.Accept()
				if err != nil {
					cancel()
					return
				}
				select {
				case capacity <- struct{}{}:
				default:
					client.Close()
					continue
				}
				id := sequence.Add(1)
				observe := func(stage string) {
					if plan.Diagnostics {
						fmt.Fprintf(os.Stderr, "VM ingress process=%d connection=%d service=%d stage=%s active=%d\n", os.Getpid(), id, port, stage, len(capacity))
					}
				}
				observe("accepted")
				active.Add(1)
				go func() {
					defer active.Done()
					defer func() { <-capacity }()
					defer client.Close()
					if err := relay(ctx, client, input.Socket, port, plan.Diagnostics, observe); err != nil {
						return
					}
				}()
			}
		}()
	}
	<-ctx.Done()
	for _, listener := range listeners {
		listener.Close()
	}
	accepting.Wait()
	active.Wait()
	return nil
}

type observedReader struct {
	io.Reader
	first func()
}

func (reader *observedReader) Read(data []byte) (int, error) {
	n, err := reader.Reader.Read(data)
	if n > 0 && reader.first != nil {
		reader.first()
		reader.first = nil
	}
	return n, err
}

func relay(ctx context.Context, client net.Conn, path string, port uint32, diagnostics bool, observe func(string)) error {
	defer observe("closed")
	observe("dial")
	connecting, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	guest, err := vsock.DialContext(connecting, path, port)
	if err != nil {
		return err
	}
	defer guest.Close()
	observe("connected")
	closeConnection := context.AfterFunc(ctx, func() { client.Close(); guest.Close() })
	defer closeConnection()
	// The SDK owns connection establishment. HTTP/2 CONNECT owns half-close and
	// flow control: Unix EOF would make the monitor close both vsock directions.
	var protocols http.Protocols
	protocols.SetUnencryptedHTTP2(true)
	transport := &http.Transport{Protocols: &protocols, DisableCompression: true,
		DialContext: func(context.Context, string, string) (net.Conn, error) { return guest, nil }}
	connection, err := transport.NewClientConn(connecting, "http", "guest:")
	if err != nil {
		return err
	}
	defer connection.Close()
	observe("h2-ready")
	reader, writer := io.Pipe()
	defer reader.Close()
	defer writer.Close()
	request, err := http.NewRequestWithContext(ctx, http.MethodConnect, "http://guest/", reader)
	if err != nil {
		return err
	}
	done := make(chan struct{})
	go func() {
		var source io.Reader = client
		if diagnostics {
			source = &observedReader{Reader: source, first: func() { observe("input-first-byte") }}
		}
		_, err := io.Copy(writer, source)
		writer.CloseWithError(err)
		observe("input-eof")
		close(done)
	}()
	// A ClientConn has exactly this connection and never retries an application stream.
	response, err := connection.RoundTrip(request)
	if err == nil {
		observe("response")
		if response.StatusCode != http.StatusOK {
			err = fmt.Errorf("Guest refused TCP stream")
		} else {
			var source io.Reader = response.Body
			if diagnostics {
				source = &observedReader{Reader: source, first: func() { observe("output-first-byte") }}
			}
			_, err = io.Copy(client, source)
			observe("output-eof")
		}
		response.Body.Close()
	}
	if half, ok := client.(interface{ CloseWrite() error }); ok {
		half.CloseWrite()
	}
	client.Close()
	reader.Close()
	guest.Close()
	<-done
	return err
}

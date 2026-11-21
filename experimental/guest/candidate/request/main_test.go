// SPDX-License-Identifier: FSL-1.1-ALv2
// Check the management probe against raw TCP and the standard H2 CONNECT server.
package main

import (
	"bufio"
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"strings"
	"testing"
	"time"
)

func TestOriginalConnectionsCarryBothProtocols(t *testing.T) {
	for _, mode := range []string{"raw", "framed"} {
		t.Run(mode, func(t *testing.T) {
			listener, err := net.Listen("tcp", "127.0.0.1:0")
			if err != nil {
				t.Fatal(err)
			}
			var protocols http.Protocols
			protocols.SetHTTP1(true)
			protocols.SetUnencryptedHTTP2(true)
			agent := func(req *http.Request) error {
				if req.Method != "GET" || req.URL.Path != healthPath || req.Header.Get("Authorization") != "Bearer fixture" ||
					req.Header.Get("x-pryx-process-nonce") != "challenge" {
					return fmt.Errorf("wrong inner health request")
				}
				return nil
			}
			served := make(chan error, 1)
			server := &http.Server{Protocols: &protocols, Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if mode == "raw" {
					served <- agent(r)
					io.WriteString(w, "raw reply")
					return
				}
				if r.Method != "CONNECT" || r.ProtoMajor != 2 {
					served <- fmt.Errorf("expected H2 tunnel")
					return
				}
				req, err := http.ReadRequest(bufio.NewReader(r.Body))
				if err != nil {
					served <- err
					return
				}
				served <- agent(req)
				defer req.Body.Close()
				reply := &http.Response{StatusCode: 401, Status: "401 Unauthorized", Proto: "HTTP/1.1", ProtoMajor: 1, ProtoMinor: 1,
					Header: make(http.Header), Body: io.NopCloser(strings.NewReader("inner reply")), ContentLength: 11}
				reply.Write(w)
			})}
			defer listener.Close()
			defer server.Close()
			ctx, cancel := context.WithTimeout(context.Background(), 8*time.Second)
			defer cancel()
			conn, err := (&net.Dialer{}).DialContext(ctx, "tcp", listener.Addr().String())
			if err != nil {
				t.Fatal(err)
			}
			defer conn.Close()
			deadline, _ := ctx.Deadline()
			conn.SetDeadline(deadline)
			marked := &firstWrite{Conn: conn, written: make(chan struct{})}
			// A real guest cannot send HTTP/2 settings until after its exec.
			// Hold the server back until the first actual request bytes leave the host.
			go func() {
				select {
				case <-marked.written:
					_ = server.Serve(listener)
				case <-ctx.Done():
				}
			}()
			status, body, err := exchange(ctx, marked, mode, "fixture", "challenge")
			if err != nil {
				t.Fatal(err)
			}
			if err := <-served; err != nil {
				t.Fatal(err)
			}
			if (mode == "framed" && (status != 401 || string(body) != "inner reply")) ||
				(mode == "raw" && (status != 200 || string(body) != "raw reply")) {
				t.Fatalf("wrong status or response in %s", mode)
			}
		})
	}
}

func TestSignedIdentityAndStockRefusal(t *testing.T) {
	pub, key, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	f := fixture{PublicKey: base64.RawURLEncoding.EncodeToString(pub), Sandbox: "vm-fixture", Epoch: 7}
	const nonce = "probe"
	o := observation{nonce, f.Sandbox, f.Epoch, 29, false}
	wire, err := json.Marshal([]any{"pryx process observation v1", o})
	if err != nil {
		t.Fatal(err)
	}
	signature := base64.RawURLEncoding.EncodeToString(ed25519.Sign(key, wire))
	health, err := json.Marshal(map[string]any{"status": "ok", "apiVersion": 2, "processObservation": map[string]any{
		"observation": o, "signature": signature}})
	if err != nil {
		t.Fatal(err)
	}
	denied := []byte(`{"error":{"code":"unauthorized","message":"Sandbox token refused"}}`)
	if pid, err := verify(f, nonce, 200, health, 401, denied); err != nil || pid != 29 {
		t.Fatalf("stock identity: %v", err)
	}
	if _, err := verify(f, "replayed", 200, health, 401, denied); err == nil {
		t.Fatal("accepted stale challenge")
	}
	if _, err := verify(f, nonce, 200, health, 403, denied); err == nil {
		t.Fatal("accepted a different refusal")
	}
	f.PublicKey = base64.RawURLEncoding.EncodeToString(make([]byte, 32))
	if _, err := verify(f, nonce, 200, health, 401, denied); err == nil {
		t.Fatal("accepted forged signer")
	}
}

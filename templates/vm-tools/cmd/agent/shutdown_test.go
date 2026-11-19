// SPDX-License-Identifier: FSL-1.1-ALv2
package main

import (
	"encoding/json"
	"errors"
	"io"
	"net"
	"strings"
	"testing"
	"time"
)

type shutdownResult struct {
	body []byte
	err  error
}

func openShutdownPeer(t *testing.T, request string, wantProof bool) (net.Conn, <-chan shutdownResult) {
	t.Helper()
	host, guest := net.Pipe()
	return guest, runShutdownPeer(t, host, guest, request, wantProof)
}

func runShutdownPeer(t *testing.T, host, guest net.Conn, request string, wantProof bool) <-chan shutdownResult {
	t.Helper()
	t.Cleanup(func() { guest.Close() })
	deadline := time.Now().Add(shutdownAdmissionTimeout)
	if err := guest.SetDeadline(deadline); err != nil {
		t.Fatal(err)
	}
	var proofDeadline time.Time
	if wantProof {
		proofDeadline = deadline
	}
	done := make(chan shutdownResult, 1)
	go func() {
		defer host.Close()
		body, err := exchangeShutdown(host, json.RawMessage(request), proofDeadline)
		done <- shutdownResult{body: body, err: err}
	}()
	var received json.RawMessage
	if err := json.NewDecoder(guest).Decode(&received); err != nil || string(received) != request {
		t.Fatalf("Guest received %s, want %s: %v", received, request, err)
	}
	return done
}

func writeShutdownFrame(t *testing.T, guest net.Conn, frame string) {
	t.Helper()
	if _, err := io.WriteString(guest, frame); err != nil {
		t.Fatal(err)
	}
}

func expectShutdownPeerClosed(t *testing.T, guest net.Conn) {
	t.Helper()
	var extra [1]byte
	if _, err := guest.Read(extra[:]); !errors.Is(err, io.EOF) {
		t.Fatalf("Expected closure with no extra receipt bytes, got %v", err)
	}
}

func TestShutdownProofWaitsForCompleteFrame(t *testing.T) {
	guest, done := openShutdownPeer(t, `{"shutdown":true}`, true)
	const proof = `{ "observation": {"diskSynced":true}, "signature": "fixture" }`
	// Coalesce the admission and start of the proof. The two later writes can finish only if
	// the host is still reading, so even a complete JSON object without LF cannot earn a receipt.
	writeShutdownFrame(t, guest, "true\n"+proof[:16])
	writeShutdownFrame(t, guest, proof[16:])
	writeShutdownFrame(t, guest, "\n")
	var receipt [1]byte
	if _, err := io.ReadFull(guest, receipt[:]); err != nil || receipt[0] != 1 {
		t.Fatalf("Expected one binary receipt, got %v: %v", receipt, err)
	}
	expectShutdownPeerClosed(t, guest)
	result := <-done
	if result.err != nil || string(result.body) != proof {
		t.Fatalf("Proof changed in transport, got %q: %v", result.body, result.err)
	}
}

type receiptWriteFailure struct {
	net.Conn
	attempted bool
}

func (connection *receiptWriteFailure) Write(data []byte) (int, error) {
	if len(data) == 1 && data[0] == 1 {
		connection.attempted = true
		return 0, io.ErrClosedPipe
	}
	return connection.Conn.Write(data)
}

func TestShutdownProofSurvivesLostReceipt(t *testing.T) {
	host, guest := net.Pipe()
	connection := &receiptWriteFailure{Conn: host}
	done := runShutdownPeer(t, connection, guest, `{"shutdown":true}`, true)
	const proof = `{"observation":{"diskSynced":true},"signature":"fixture"}`
	writeShutdownFrame(t, guest, "true\n"+proof+"\n")
	expectShutdownPeerClosed(t, guest)
	result := <-done
	if !connection.attempted || result.err != nil || string(result.body) != proof {
		t.Fatalf("Lost receipt changed the response, got %q: %v", result.body, result.err)
	}
}

func TestShutdownLegacyAndSleepReturnAfterAcceptance(t *testing.T) {
	for _, name := range []string{"shutdown", "sleep"} {
		t.Run(name, func(t *testing.T) {
			guest, done := openShutdownPeer(t, `{"`+name+`":true}`, false)
			writeShutdownFrame(t, guest, "true\n")
			expectShutdownPeerClosed(t, guest)
			result := <-done
			if result.err != nil || result.body == nil || len(result.body) != 0 {
				t.Fatalf("Expected an empty accepted body, got %v: %v", result.body, result.err)
			}
		})
	}
}

func TestShutdownProofRejectsInvalidOrOversizedFrame(t *testing.T) {
	for name, frame := range map[string]string{
		"invalid":   "{\n",
		"oversized": strings.Repeat("{", maxShutdownProofBytes),
	} {
		t.Run(name, func(t *testing.T) {
			guest, done := openShutdownPeer(t, `{"shutdown":true}`, true)
			writeShutdownFrame(t, guest, "true\n"+frame)
			expectShutdownPeerClosed(t, guest)
			result := <-done
			if result.err == nil || result.body != nil {
				t.Fatalf("Invalid proof received a response: %v: %v", result.body, result.err)
			}
		})
	}
}

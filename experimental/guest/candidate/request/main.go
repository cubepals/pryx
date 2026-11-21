// SPDX-License-Identifier: FSL-1.1-ALv2
// Prove two original management connections survive first-request activation in a lab VM.
// The first connection carries valid authorization; the second must receive the stock refusal.
// Parts (cmd/lab-gate-proof/): main.go carries and verifies the two bounded requests;
// main_test.go checks both management protocols and the signed observation.
package main

import (
	"bufio"
	"bytes"
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"sync"
	"time"

	"github.com/firecracker-microvm/firecracker-go-sdk/vsock"
)

const (
	agentPort   = 5280    // agent/src/contract.rs
	serviceBase = 1 << 16 // agent/src/vm_boot.rs
	healthPath  = "/_pryx/health"
	maxResult   = 1 << 20
)

type fixture struct {
	Socket    string `json:"socket"`
	Mode      string `json:"mode"`
	Token     string `json:"token"`
	Sandbox   string `json:"sandbox"`
	Epoch     uint64 `json:"epoch"`
	PublicKey string `json:"publicKey"`
}

type observation struct {
	Nonce     string `json:"nonce"`
	Sandbox   string `json:"sandbox"`
	Epoch     uint64 `json:"epoch"`
	PID       uint32 `json:"pid"`
	IPCScoped bool   `json:"ipcScoped"`
}

type outcome struct {
	Mode             string `json:"mode"`
	FirstStatus      int    `json:"firstStatus"`
	SecondStatus     int    `json:"secondStatus"`
	PID              uint32 `json:"servingPid"`
	IdentityVerified bool   `json:"identityVerified"`
}

func decodeFixture(input io.Reader) (fixture, error) {
	var f fixture
	data, err := io.ReadAll(io.LimitReader(input, 16*1024+1))
	if err != nil || len(data) > 16*1024 {
		return f, errors.New("invalid private fixture size")
	}
	d := json.NewDecoder(bytes.NewReader(data))
	d.DisallowUnknownFields()
	if d.Decode(&f) != nil || d.Decode(&struct{}{}) != io.EOF {
		return f, errors.New("expected one fixture record")
	}
	key, err := base64.RawURLEncoding.DecodeString(f.PublicKey)
	if err != nil || len(key) != ed25519.PublicKeySize || !filepath.IsAbs(f.Socket) ||
		(f.Mode != "raw" && f.Mode != "framed") || f.Token == "" || len(f.Token) > 4096 || f.Sandbox == "" {
		return f, errors.New("invalid management fixture")
	}
	return f, nil
}

type firstWrite struct {
	net.Conn
	once    sync.Once
	written chan struct{}
}

func (c *firstWrite) Write(data []byte) (int, error) {
	n, err := c.Conn.Write(data)
	if n > 0 {
		c.once.Do(func() { close(c.written) })
	}
	return n, err
}

// Use one fixed dial target. The SDK retries establishment within the input deadline.
// Open the second connection once the first has sent real application bytes.
func run(input io.Reader, output io.Writer) error {
	f, err := decodeFixture(input)
	if err != nil {
		return err
	}
	port := uint32(serviceBase + agentPort)
	if f.Mode == "framed" {
		port = 2*serviceBase + agentPort
	}
	ctx, cancel := context.WithTimeout(context.Background(), 45*time.Second)
	defer cancel()
	var challenge [24]byte
	if _, err := rand.Read(challenge[:]); err != nil {
		return errors.New("generating fixture nonce failed")
	}
	nonce := base64.RawURLEncoding.EncodeToString(challenge[:])
	type result struct {
		status int
		body   []byte
		err    error
	}
	var replies [2]chan result
	for i := range replies {
		conn, err := vsock.DialContext(ctx, f.Socket, port)
		if err != nil {
			return fmt.Errorf("establishing management connection %d failed", i+1)
		}
		defer conn.Close()
		deadline, _ := ctx.Deadline()
		if err := conn.SetDeadline(deadline); err != nil {
			return errors.New("setting management deadline failed")
		}
		var written chan struct{}
		if i == 0 {
			written = make(chan struct{})
			conn = &firstWrite{Conn: conn, written: written}
		}
		reply := make(chan result, 1)
		replies[i] = reply
		token := f.Token
		if i == 1 {
			token = "invalid-lab-gate-proof"
		}
		go func() {
			status, body, err := exchange(ctx, conn, f.Mode, token, nonce)
			reply <- result{status, body, err}
		}()
		if i == 0 {
			select {
			case <-written:
			case <-reply:
				return errors.New("first fixture request ended before the paired connection")
			case <-ctx.Done():
				return errors.New("first fixture write did not complete")
			}
		}
	}
	first, second := <-replies[0], <-replies[1]
	if first.err != nil || second.err != nil {
		return errors.New("management exchange on an original connection failed")
	}
	pid, err := verify(f, nonce, first.status, first.body, second.status, second.body)
	if err != nil {
		return err
	}
	return json.NewEncoder(output).Encode(outcome{f.Mode, first.status, second.status, pid, true})
}

func exchange(ctx context.Context, conn net.Conn, mode, token, nonce string) (int, []byte, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, "http://agent.pryx.invalid"+healthPath, nil)
	if err != nil {
		return 0, nil, err
	}
	req.Header.Set("Authorization", "Bearer "+token)
	req.Header.Set("x-pryx-process-nonce", nonce)
	req.Close = true
	var stream io.Reader = conn
	if mode == "framed" {
		var encoded bytes.Buffer
		if err := req.Write(&encoded); err != nil {
			return 0, nil, err
		}
		var protocols http.Protocols
		protocols.SetUnencryptedHTTP2(true)
		// The SDK already returned an established guest stream. Permit the HTTP/2
		// transport to claim it once, so it cannot reconnect or replay the request.
		used := false
		tr := &http.Transport{Protocols: &protocols, DisableCompression: true, DialContext: func(context.Context, string, string) (net.Conn, error) {
			if used {
				return nil, errors.New("unexpected second dial")
			}
			used = true
			return conn, nil
		}}
		control, err := tr.NewClientConn(ctx, "http", "guest:")
		if err != nil {
			return 0, nil, err
		}
		defer control.Close()
		tunnel, err := http.NewRequestWithContext(ctx, http.MethodConnect, "http://guest/", io.NopCloser(&encoded))
		if err != nil {
			return 0, nil, err
		}
		response, err := control.RoundTrip(tunnel)
		if err != nil {
			return 0, nil, err
		}
		defer response.Body.Close()
		if response.StatusCode != http.StatusOK {
			return 0, nil, errors.New("management tunnel refused")
		}
		stream = response.Body
	} else if err := req.Write(conn); err != nil {
		return 0, nil, err
	}
	resp, err := http.ReadResponse(bufio.NewReader(io.LimitReader(stream, maxResult+1)), req)
	if err != nil {
		return 0, nil, err
	}
	defer resp.Body.Close()
	body, err := io.ReadAll(io.LimitReader(resp.Body, maxResult+1))
	if len(body) > maxResult {
		return 0, nil, errors.New("management response exceeds bound")
	}
	return resp.StatusCode, body, err
}

func verify(f fixture, nonce string, first int, body []byte, second int, denied []byte) (uint32, error) {
	var health struct {
		Status     string `json:"status"`
		APIVersion uint16 `json:"apiVersion"`
		Proof      struct {
			Observation observation `json:"observation"`
			Signature   string      `json:"signature"`
		} `json:"processObservation"`
	}
	var refusal struct {
		Error struct{ Code, Message string } `json:"error"`
	}
	if first != http.StatusOK || json.Unmarshal(body, &health) != nil || health.Status != "ok" || health.APIVersion != 2 {
		return 0, errors.New("expected stock authenticated health")
	}
	o := health.Proof.Observation
	if o.Nonce != nonce || o.Sandbox != f.Sandbox || o.Epoch != f.Epoch || o.PID <= 1 {
		return 0, errors.New("wrong guest observation identity")
	}
	// agent/src/process_observation.rs signs this exact tuple in field declaration order.
	key, _ := base64.RawURLEncoding.DecodeString(f.PublicKey)
	sig, e := base64.RawURLEncoding.DecodeString(health.Proof.Signature)
	var encoded bytes.Buffer
	enc := json.NewEncoder(&encoded)
	enc.SetEscapeHTML(false)
	if e != nil || len(key) != ed25519.PublicKeySize || enc.Encode([]any{"pryx process observation v1", o}) != nil ||
		!ed25519.Verify(ed25519.PublicKey(key), bytes.TrimSuffix(encoded.Bytes(), []byte{'\n'}), sig) {
		return 0, errors.New("invalid signed health identity")
	}
	if second != http.StatusUnauthorized || json.Unmarshal(denied, &refusal) != nil ||
		refusal.Error.Code != "unauthorized" || refusal.Error.Message != "Sandbox token refused" {
		return 0, errors.New("expected stock token refusal")
	}
	return o.PID, nil
}

func main() {
	if err := run(os.Stdin, os.Stdout); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

// SPDX-License-Identifier: FSL-1.1-ALv2
// Command agent carries one authenticated HTTP request through the maintained monitor
// owner's vsock dialer. Credentials stay on bounded private stdin, never argv or logs.
// It neither implements CONNECT framing nor retries an HTTP operation or follows redirects.
package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"time"

	"github.com/firecracker-microvm/firecracker-go-sdk/vsock"
)

const maxRequestBytes = 512 * 1024
const maxResponseBytes = 4 * 1024 * 1024

type request struct {
	Socket     string          `json:"socket"`
	Port       uint32          `json:"port"`
	Method     string          `json:"method"`
	Path       string          `json:"path"`
	Token      string          `json:"token"`
	Body       json.RawMessage `json:"body"`
	BinaryBody []byte          `json:"binaryBody"`
}

func run() error {
	data, err := io.ReadAll(io.LimitReader(os.Stdin, maxRequestBytes+1))
	if err != nil {
		return err
	}
	if len(data) > maxRequestBytes {
		return fmt.Errorf("Private agent request exceeds its bound")
	}
	var input request
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&input); err != nil {
		return fmt.Errorf("Invalid private agent request")
	}
	if input.Socket == "" || input.Port == 0 || input.Token == "" || input.Path == "" || input.Path[0] != '/' {
		return fmt.Errorf("Incomplete private agent request")
	}
	transport := &http.Transport{DisableKeepAlives: true, DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
		return vsock.DialContext(ctx, input.Socket, input.Port)
	}}
	defer transport.CloseIdleConnections()
	client := &http.Client{Transport: transport, Timeout: 30 * time.Second,
		CheckRedirect: func(_ *http.Request, _ []*http.Request) error { return http.ErrUseLastResponse }}
	data = input.Body
	if input.BinaryBody != nil {
		data = input.BinaryBody
	}
	message, err := http.NewRequest(input.Method, "http://agent.pryx.invalid"+input.Path, bytes.NewReader(data))
	if err != nil {
		return fmt.Errorf("Invalid agent HTTP request")
	}
	message.Header.Set("Authorization", "Bearer "+input.Token)
	message.Header.Set("Content-Type", "application/json")
	response, err := client.Do(message)
	if err != nil {
		return fmt.Errorf("Agent request failed: %w", err)
	}
	defer response.Body.Close()
	body, err := io.ReadAll(io.LimitReader(response.Body, maxResponseBytes+1))
	if err != nil {
		return err
	}
	if len(body) > maxResponseBytes {
		return fmt.Errorf("Agent response exceeds its bound")
	}
	return json.NewEncoder(os.Stdout).Encode(map[string]any{"status": response.StatusCode, "body": body})
}

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

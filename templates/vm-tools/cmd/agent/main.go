// SPDX-License-Identifier: FSL-1.1-ALv2
// Command agent carries one authenticated HTTP request through the maintained monitor
// owner's vsock dialer. Credentials stay on bounded private stdin, never argv or logs.
// It neither implements CONNECT framing nor retries an HTTP operation or follows redirects.
//
// Parts (`cmd/agent/`): main.go carries bounded requests; forward.go relays opaque TCP streams
// with reserved capacity for the agent's own binding, proved by forward_test.go; monitor.go
// sends memory sleep's pause, snapshot and restore to the monitor's own API socket.
package main

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"time"

	"github.com/firecracker-microvm/firecracker-go-sdk/client/models"
	"github.com/firecracker-microvm/firecracker-go-sdk/vsock"
)

const maxRequestBytes = 512 * 1024
const maxResponseBytes = 4 * 1024 * 1024
const maxShutdownProofBytes = 4 * 1024
const shutdownAdmissionTimeout = 5 * time.Second

type request struct {
	Socket     string          `json:"socket"`
	Port       uint32          `json:"port"`
	Method     string          `json:"method"`
	Path       string          `json:"path"`
	Token      string          `json:"token"`
	Body       json.RawMessage `json:"body"`
	BinaryBody []byte          `json:"binaryBody"`
	Stop       bool            `json:"stop"`
	Headers    http.Header     `json:"headers"`
	Shutdown   json.RawMessage `json:"shutdown"`
	Sleep      json.RawMessage `json:"sleep"`
	Monitor    *monitorAction  `json:"monitor"`
	Forward    *forwarding     `json:"forward"`

	ShutdownProofTimeoutMs uint32 `json:"shutdownProofTimeoutMs"`
}

func run(source io.Reader) error {
	data, err := io.ReadAll(io.LimitReader(source, maxRequestBytes+1))
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
	if input.ShutdownProofTimeoutMs != 0 && (input.Shutdown == nil || input.Forward != nil || input.Monitor != nil) {
		return fmt.Errorf("A proof timeout requires a private shutdown request")
	}
	if input.Forward != nil {
		return forward(input)
	}
	if input.Monitor != nil {
		return monitor(input)
	}
	if input.Shutdown != nil || input.Sleep != nil {
		return shutdown(input)
	}
	if input.Socket == "" || (!input.Stop && (input.Port == 0 || input.Token == "" || input.Path == "" || input.Path[0] != '/')) {
		return fmt.Errorf("Incomplete private agent request")
	}
	transport := &http.Transport{DisableKeepAlives: true, DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
		if input.Stop {
			return (&net.Dialer{}).DialContext(ctx, "unix", input.Socket)
		}
		return vsock.DialContext(ctx, input.Socket, input.Port)
	}}
	defer transport.CloseIdleConnections()
	client := &http.Client{Transport: transport, Timeout: 30 * time.Second,
		CheckRedirect: func(_ *http.Request, _ []*http.Request) error { return http.ErrUseLastResponse }}
	data = input.Body
	if input.BinaryBody != nil {
		data = input.BinaryBody
	}
	if input.Stop {
		// The generated SDK operations/create_sync_action.go owns this private API path.
		const monitorActionsPath = "/actions"
		input.Method, input.Path = http.MethodPut, monitorActionsPath
		data, err = json.Marshal(&models.InstanceActionInfo{ActionType: new(models.InstanceActionInfoActionTypeSendCtrlAltDel)})
		if err != nil {
			return err
		}
	}
	message, err := http.NewRequest(input.Method, "http://agent.pryx.invalid"+input.Path, bytes.NewReader(data))
	if err != nil {
		return fmt.Errorf("Invalid agent HTTP request")
	}
	message.Header = input.Headers.Clone()
	if message.Header == nil {
		message.Header = make(http.Header)
	}
	if !input.Stop {
		message.Header.Set("Authorization", "Bearer "+input.Token)
	}
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

func shutdown(input request) error {
	if input.Socket == "" || input.Port == 0 || input.Stop || (input.Shutdown != nil) == (input.Sleep != nil) {
		return fmt.Errorf("Incomplete private shutdown request")
	}
	line := input.Shutdown
	if input.Sleep != nil {
		line = input.Sleep
	}
	var proofDeadline time.Time
	if input.ShutdownProofTimeoutMs != 0 {
		proofDeadline = time.Now().Add(time.Duration(input.ShutdownProofTimeoutMs) * time.Millisecond)
	}
	ctx, cancel := context.WithDeadline(context.Background(), shutdownAdmissionDeadline(proofDeadline))
	defer cancel()
	connection, err := vsock.DialContext(ctx, input.Socket, input.Port)
	if err != nil {
		return err
	}
	defer connection.Close()
	body, err := exchangeShutdown(connection, line, proofDeadline)
	if err != nil {
		return err
	}
	return json.NewEncoder(os.Stdout).Encode(map[string]any{"status": http.StatusAccepted, "body": body})
}

func shutdownAdmissionDeadline(proofDeadline time.Time) time.Time {
	deadline := time.Now().Add(shutdownAdmissionTimeout)
	if !proofDeadline.IsZero() && proofDeadline.Before(deadline) {
		return proofDeadline
	}
	return deadline
}

func exchangeShutdown(connection net.Conn, line json.RawMessage, proofDeadline time.Time) ([]byte, error) {
	if err := connection.SetDeadline(shutdownAdmissionDeadline(proofDeadline)); err != nil {
		return nil, err
	}
	if err := json.NewEncoder(connection).Encode(line); err != nil {
		return nil, err
	}
	decoder := json.NewDecoder(io.LimitReader(connection, maxResponseBytes))
	var accepted bool
	if err := decoder.Decode(&accepted); err != nil {
		return nil, err
	}
	if !accepted {
		return nil, fmt.Errorf("Guest init refused the request")
	}
	if proofDeadline.IsZero() {
		return []byte{}, nil
	}
	if err := connection.SetDeadline(proofDeadline); err != nil {
		return nil, err
	}
	// The proof can start in the same read as the boolean acknowledgement. Keep those bytes
	// and consume the two line boundaries before acknowledging delivery to guest init.
	stream := bufio.NewReaderSize(io.MultiReader(decoder.Buffered(), connection), maxShutdownProofBytes)
	boundary, err := stream.ReadByte()
	if err != nil || boundary != '\n' {
		return nil, fmt.Errorf("Incomplete VM shutdown acknowledgement")
	}
	frame, err := stream.ReadSlice('\n')
	if err != nil {
		return nil, fmt.Errorf("Reading VM shutdown proof: %w", err)
	}
	proof := frame[:len(frame)-1]
	if len(proof) == 0 || proof[0] != '{' || !json.Valid(proof) {
		return nil, fmt.Errorf("Invalid VM shutdown proof response")
	}
	// The receipt lets init reset early. Losing it cannot revoke the fully received proof.
	_, _ = connection.Write([]byte{1})
	return proof, nil
}

func main() {
	configuration := flag.String("forward-config", "", "Private non-secret forwarding configuration")
	flag.Parse()
	var source io.Reader = os.Stdin
	if *configuration != "" {
		file, err := os.Open(*configuration)
		if err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
		defer file.Close()
		source = file
	}
	if err := run(source); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

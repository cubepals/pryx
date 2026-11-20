// SPDX-License-Identifier: FSL-1.1-ALv2
// Monitor actions for memory sleep and measurement, with request bodies from the
// monitor owner's generated models. Snapshot paths come from the caller and
// each action is attempted once.
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

	"github.com/firecracker-microvm/firecracker-go-sdk/client/models"
	"github.com/go-openapi/strfmt"
)

type monitorAction struct {
	Action    string `json:"action"`
	Snapshot  string `json:"snapshot"`
	Memory    string `json:"memory"`
	TimeoutMs int64  `json:"timeoutMs"`
}

// Firecracker 1.18.0's src/firecracker/swagger/firecracker.yaml names these paths; the SDK's
// generated operations keep them inside their unexported request builders.
const (
	monitorActionsPath = "/actions"
	vmPath             = "/vm"
	snapshotCreatePath = "/snapshot/create"
	snapshotLoadPath   = "/snapshot/load"
)

type validated interface {
	Validate(strfmt.Registry) error
}

func monitorRequest(action *monitorAction) (string, string, validated, error) {
	switch action.Action {
	case "flushMetrics":
		return http.MethodPut, monitorActionsPath, &models.InstanceActionInfo{
			ActionType: new(models.InstanceActionInfoActionTypeFlushMetrics),
		}, nil
	case "pause", "resume":
		state := models.VMStatePaused
		if action.Action == "resume" {
			state = models.VMStateResumed
		}
		return http.MethodPatch, vmPath, &models.VM{State: &state}, nil
	case "create":
		if action.Snapshot == "" || action.Memory == "" {
			return "", "", nil, fmt.Errorf("A snapshot needs its state and memory paths")
		}
		return http.MethodPut, snapshotCreatePath, &models.SnapshotCreateParams{
			SnapshotType: models.SnapshotCreateParamsSnapshotTypeFull,
			SnapshotPath: &action.Snapshot,
			MemFilePath:  &action.Memory,
		}, nil
	case "load":
		if action.Snapshot == "" || action.Memory == "" {
			return "", "", nil, fmt.Errorf("A restore needs its state and memory paths")
		}
		backend := models.MemoryBackendBackendTypeFile
		return http.MethodPut, snapshotLoadPath, &models.SnapshotLoadParams{
			SnapshotPath: &action.Snapshot,
			MemBackend:   &models.MemoryBackend{BackendType: &backend, BackendPath: &action.Memory},
			ResumeVM:     true,
		}, nil
	}
	return "", "", nil, fmt.Errorf("Unknown monitor action")
}

func monitor(input request) error {
	action := input.Monitor
	if input.Socket == "" || input.Stop || input.Port != 0 || action.TimeoutMs <= 0 {
		return fmt.Errorf("Incomplete private monitor request")
	}
	method, path, body, err := monitorRequest(action)
	if err != nil {
		return err
	}
	if err := body.Validate(strfmt.Default); err != nil {
		return err
	}
	data, err := json.Marshal(body)
	if err != nil {
		return err
	}
	transport := &http.Transport{DisableKeepAlives: true, DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
		return (&net.Dialer{}).DialContext(ctx, "unix", input.Socket)
	}}
	defer transport.CloseIdleConnections()
	client := &http.Client{Transport: transport, Timeout: time.Duration(action.TimeoutMs) * time.Millisecond,
		CheckRedirect: func(_ *http.Request, _ []*http.Request) error { return http.ErrUseLastResponse }}
	message, err := http.NewRequest(method, "http://monitor.pryx.invalid"+path, bytes.NewReader(data))
	if err != nil {
		return err
	}
	message.Header.Set("Content-Type", "application/json")
	response, err := client.Do(message)
	if err != nil {
		return fmt.Errorf("Monitor request failed: %w", err)
	}
	defer response.Body.Close()
	answer, err := io.ReadAll(io.LimitReader(response.Body, maxResponseBytes+1))
	if err != nil {
		return err
	}
	if len(answer) > maxResponseBytes {
		return fmt.Errorf("Monitor response exceeds its bound")
	}
	return json.NewEncoder(os.Stdout).Encode(map[string]any{"status": response.StatusCode, "body": answer})
}

// SPDX-License-Identifier: FSL-1.1-ALv2
// Command probe measures the two network paths inside the real guest. The maintained
// forwarder owns the TAP and packet framing; BusyBox owns DHCP on the virtio path.
// Requests are safe GETs except deliberate, required-to-be-denied proxy management probes.
package main

import (
	"bytes"
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"log"
	"net"
	"net/http"
	"os"
	"os/exec"
	"strconv"
	"syscall"
	"time"

	"cubepals/pryx/vm-experiment/experiment"
	networkfixture "cubepals/pryx/vm-network-experiment"
	"github.com/containers/gvisor-tap-vsock/pkg/types"
	"github.com/mdlayher/vsock"
)

func checkedGet(client *http.Client, url string, expected int64) (float64, error) {
	started := time.Now()
	response, err := client.Get(url)
	if err != nil {
		return 0, err
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusOK {
		return 0, fmt.Errorf("Network fixture returned %s", response.Status)
	}
	n, err := io.Copy(io.Discard, io.LimitReader(response.Body, expected+1))
	if err != nil || n != expected {
		return 0, fmt.Errorf("Network fixture body length %d, expected %d: %v", n, expected, err)
	}
	return float64(time.Since(started).Nanoseconds()) / 1e6, nil
}

func denied(client *http.Client, url string, expected int) error {
	// Use the owner's request shape; an unexpectedly successful attack binds loopback only.
	body, err := json.Marshal(types.ExposeRequest{Local: "127.0.0.1:0", Remote: "127.0.0.1:1", Protocol: types.TCP})
	if err != nil {
		return err
	}
	response, err := client.Post(url, "application/json", bytes.NewReader(body))
	if err != nil {
		return err
	}
	defer response.Body.Close()
	if response.StatusCode != expected {
		return fmt.Errorf("Management access returned %s, expected %s", response.Status, http.StatusText(expected))
	}
	return nil
}

func measure(fixture experiment.Network) (map[string]any, error) {
	started := time.Now()
	var helper *exec.Cmd
	if fixture.Mode == "vsock" {
		endpoint := fmt.Sprintf("vsock://%d:%d%s", vsock.Host, experiment.NetworkPort, types.ConnectPath)
		helper = exec.Command("/gvforwarder", "-url", endpoint, "-mac", fixture.GuestMAC,
			"-mtu", strconv.Itoa(fixture.MTU))
		helper.Stdout, helper.Stderr = os.Stderr, os.Stderr
		if err := helper.Start(); err != nil {
			return nil, err
		}
		defer func() {
			_ = helper.Process.Signal(syscall.SIGTERM)
			_ = helper.Wait()
		}()
	} else if fixture.Mode == "pasta" {
		ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer cancel()
		helper = exec.CommandContext(ctx, "/sbin/udhcpc", "-f", "-q", "-i", "eth0", "-t", "3", "-T", "1")
		helper.Stdout, helper.Stderr = os.Stderr, os.Stderr
		if err := helper.Run(); err != nil {
			return nil, err
		}
	} else {
		return nil, fmt.Errorf("Unknown network fixture path %q", fixture.Mode)
	}
	client := &http.Client{Timeout: 2 * time.Second, Transport: &http.Transport{Proxy: nil}}
	deadline := time.Now().Add(20 * time.Second)
	for {
		if _, err := checkedGet(client, fixture.URL+experiment.NetworkHelloPath, int64(len(experiment.NetworkHello))); err == nil {
			break
		} else if time.Now().After(deadline) {
			return nil, fmt.Errorf("Guest network did not become ready: %w", err)
		}
		time.Sleep(100 * time.Millisecond)
	}
	report := map[string]any{"path": fixture.Mode, "readyMs": float64(time.Since(started).Nanoseconds()) / 1e6}
	if fixture.Mode == "vsock" {
		port, err := net.LookupPort("tcp", "http")
		if err != nil {
			return nil, err
		}
		if err := denied(client, "http://"+net.JoinHostPort(fixture.Gateway, strconv.Itoa(port))+networkfixture.ExposePath, http.StatusUnauthorized); err != nil {
			return nil, err
		}
		direct := &http.Client{Timeout: 2 * time.Second, Transport: &http.Transport{
			DialContext: func(context.Context, string, string) (net.Conn, error) {
				return vsock.Dial(vsock.Host, experiment.NetworkPort, nil)
			},
		}}
		if err := denied(direct, "http://proxy"+networkfixture.ExposePath, http.StatusNotFound); err != nil {
			return nil, err
		}
		report["gatewayManagementDenied"], report["directManagementDenied"] = true, true
	}
	latency, err := checkedGet(client, fixture.URL+experiment.NetworkHelloPath, int64(len(experiment.NetworkHello)))
	if err != nil {
		return nil, err
	}
	bulk, err := checkedGet(client, fixture.URL+experiment.NetworkBulkPath, experiment.NetworkBulkBytes)
	if err != nil {
		return nil, err
	}
	report["httpMs"], report["bulkMs"], report["bulkBytes"] = latency, bulk, experiment.NetworkBulkBytes
	report["bulkMiBPerSecond"] = float64(experiment.NetworkBulkBytes) / (1024 * 1024) / (bulk / 1000)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	dnsStarted := time.Now()
	addresses, err := net.DefaultResolver.LookupHost(ctx, "example.com")
	if err != nil || len(addresses) == 0 {
		return nil, fmt.Errorf("Real outbound DNS failed: %v", err)
	}
	report["dnsMs"], report["dnsAddresses"] = float64(time.Since(dnsStarted).Nanoseconds())/1e6, addresses
	return report, nil
}

func main() {
	configuration := flag.String("config", "", "Private fixture configuration")
	flag.Parse()
	if os.Geteuid() != 0 || os.Getppid() != 1 || os.Getenv("PRYX_VM_NETWORK_SPIKE") != "1" {
		log.Fatal("The network probe runs only under the trusted VM fixture's PID 1")
	}
	data, err := os.ReadFile(*configuration)
	if err != nil {
		log.Fatal(err)
	}
	var fixture experiment.Network
	if err := json.Unmarshal(data, &fixture); err != nil {
		log.Fatal(err)
	}
	report, err := measure(fixture)
	if err != nil {
		log.Fatal(err)
	}
	if err := json.NewEncoder(os.Stdout).Encode(report); err != nil {
		log.Fatal(err)
	}
}

// SPDX-License-Identifier: FSL-1.1-ALv2
// Command probe measures the two network paths inside the real guest. The maintained
// forwarder owns the TAP and packet framing; BusyBox owns DHCP on the virtio path.
// Requests are safe GETs except deliberate, required-to-be-denied proxy management probes.
//
// Parts (`network/cmd/probe/`): main.go measures network paths; server.go serves opaque
// acceptance bytes; policy.go checks inherited packet-transport denials as a workload;
// production.go attacks the immutable production guest policy.
package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"log"
	"net"
	"net/http"
	"net/http/httptrace"
	"os"
	"os/exec"
	"strconv"
	"syscall"
	"time"

	"cubepals/pryx/vm-experiment/experiment"
	networkfixture "cubepals/pryx/vm-network-experiment"
	"github.com/containers/gvisor-tap-vsock/pkg/types"
	"github.com/mdlayher/vsock"
	"github.com/vishvananda/netlink"
	"golang.org/x/sys/unix"
)

const requestTimeout = 2 * time.Second

func stalledConnection(connection net.Conn) {
	// Observe the real guest TCP socket before the existing deadline closes it. No packet,
	// timeout or retry policy changes here; the syscall package owns the TCP_INFO layout.
	owner, ok := connection.(syscall.Conn)
	if !ok {
		return
	}
	raw, err := owner.SyscallConn()
	if err != nil {
		log.Printf("Slow fixture connection observation: %v", err)
		return
	}
	var info *unix.TCPInfo
	var observationError error
	err = raw.Control(func(fd uintptr) {
		info, observationError = unix.GetsockoptTCPInfo(int(fd), unix.IPPROTO_TCP, unix.TCP_INFO)
	})
	if err == nil {
		err = observationError
	}
	if err != nil {
		log.Printf("Slow fixture connection observation: %v", err)
		return
	}
	encoded, err := json.Marshal(map[string]any{"local": connection.LocalAddr().String(),
		"remote": connection.RemoteAddr().String(), "tcpInfo": info})
	if err == nil {
		log.Printf("Slow fixture connection: %s", encoded)
	}
}

func checkedGet(client *http.Client, url string, expected int64) (float64, error) {
	started := time.Now()
	var connection net.Conn
	request, err := http.NewRequest(http.MethodGet, url, nil)
	if err != nil {
		return 0, err
	}
	trace := &httptrace.ClientTrace{GotConn: func(info httptrace.GotConnInfo) { connection = info.Conn }}
	response, err := client.Do(request.WithContext(httptrace.WithClientTrace(request.Context(), trace)))
	if err != nil {
		return 0, err
	}
	defer response.Body.Close()
	timer := time.AfterFunc(requestTimeout*3/4, func() { stalledConnection(connection) })
	defer timer.Stop()
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
		link, err := netlink.LinkByName("eth0")
		if err != nil {
			return nil, err
		}
		if err := netlink.LinkSetUp(link); err != nil {
			return nil, err
		}
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
	client := &http.Client{Timeout: requestTimeout, Transport: &http.Transport{Proxy: nil}}
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
		direct := &http.Client{Timeout: requestTimeout, Transport: &http.Transport{
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

func verify(fixture experiment.Network, restricted bool) error {
	client := &http.Client{Timeout: requestTimeout, Transport: &http.Transport{Proxy: nil}}
	if _, err := checkedGet(client, fixture.URL+experiment.NetworkHelloPath, int64(len(experiment.NetworkHello))); err != nil {
		return err
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	addresses, err := net.DefaultResolver.LookupHost(ctx, "example.com")
	if err != nil || len(addresses) == 0 {
		return fmt.Errorf("Actual guest DNS failed: %v", err)
	}
	port, err := net.LookupPort("tcp", "http")
	if err != nil {
		return err
	}
	if err := denied(client, "http://"+net.JoinHostPort(fixture.Gateway, strconv.Itoa(port))+networkfixture.ExposePath, http.StatusUnauthorized); err != nil {
		return err
	}
	if restricted {
		connection, err := vsock.Dial(vsock.Host, experiment.NetworkPort, nil)
		if connection != nil {
			connection.Close()
		}
		if !errors.Is(err, unix.EPERM) {
			return fmt.Errorf("Workload packet transport was not refused by the kernel: %v", err)
		}
		return nil
	}
	direct := &http.Client{Timeout: requestTimeout, Transport: &http.Transport{
		DialContext: func(context.Context, string, string) (net.Conn, error) {
			return vsock.Dial(vsock.Host, experiment.NetworkPort, nil)
		},
	}}
	return denied(direct, "http://proxy"+networkfixture.ExposePath, http.StatusNotFound)
}

func main() {
	echo := flag.Uint("echo", 0, "Serve the unprivileged forwarding fixture")
	ready := flag.String("ready-file", "", "Private guest readiness file")
	configuration := flag.String("config", "", "Private fixture configuration")
	verifyOnly := flag.Bool("verify", false, "Verify a prepared network without privileged setup")
	verifyRestricted := flag.Bool("verify-restricted", false, "Verify prepared networking and kernel refusal of direct packet transport")
	policyOnly := flag.Bool("policy-only", false, "Check inherited unprivileged guest socket policy")
	production := flag.String("production-policy", "", "Private production policy acceptance configuration")
	flag.Parse()
	if *production != "" {
		if err := productionPolicy(*production); err != nil {
			log.Fatal(err)
		}
		return
	}
	if *policyOnly {
		if err := guestPolicy(); err != nil {
			log.Fatal(err)
		}
		return
	}
	if *echo != 0 {
		if err := serve(*echo, *ready); err != nil {
			log.Fatal(err)
		}
		return
	}
	if !*verifyOnly && !*verifyRestricted && (os.Geteuid() != 0 || os.Getppid() != 1 || os.Getenv("PRYX_VM_NETWORK_SPIKE") != "1") {
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
	if *verifyOnly || *verifyRestricted {
		if err := verify(fixture, *verifyRestricted); err != nil {
			log.Fatal(err)
		}
		return
	}
	report, err := measure(fixture)
	if err != nil {
		log.Fatal(err)
	}
	if err := json.NewEncoder(os.Stdout).Encode(report); err != nil {
		log.Fatal(err)
	}
}

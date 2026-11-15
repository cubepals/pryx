// SPDX-License-Identifier: FSL-1.1-ALv2
// The closed runtime profile accepts one inherited private packet listener. The maintained
// virtual network owns DHCP, framing and forwarding. Management listeners, forwarding
// configuration and gateway HTTP services are absent. The host's admission (admission.go), not
// the guest's firewall, decides which destinations a flow may reach.
package main

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/containers/gvisor-tap-vsock/pkg/tap"
	"github.com/containers/gvisor-tap-vsock/pkg/types"
	"github.com/insomniacslk/dhcp/dhcpv4"
	"golang.org/x/sys/unix"
)

const descriptorFlag = "packet-fd"
const productionFlag = "production"
const maxConfiguration = 64 * 1024

type runtimeProfile struct {
	TapDescriptorArgument string `json:"tapDescriptorArgument,omitempty"`
	Version               uint16 `json:"version"`
	DescriptorArgument    string `json:"descriptorArgument"`
	ConfigurationArgument string `json:"configurationArgument"`
	MaxConfigurationBytes int    `json:"maxConfigurationBytes"`
	Subnet                string `json:"subnet"`
	Gateway               string `json:"gateway"`
	Device                string `json:"device"`
	Host                  string `json:"host"`
	GatewayMAC            string `json:"gatewayMac"`
	GuestMAC              string `json:"guestMac"`
	Interface             string `json:"interface"`
	MTU                   int    `json:"mtu"`
	HTTP                  int    `json:"http"`
	TLS                   int    `json:"tls"`
	DNS                   int    `json:"dns"`
	DHCPClient            int    `json:"dhcpClient"`
	DHCPServer            int    `json:"dhcpServer"`
}

func productionMetadata() (runtimeProfile, error) {
	profile := runtimeProfile{
		TapDescriptorArgument: "--" + tapDescriptorFlag,
		Version:               1, DescriptorArgument: "--" + descriptorFlag,
		ConfigurationArgument: "--" + productionFlag, MaxConfigurationBytes: maxConfiguration,
		// This subnet exists only within each independent maintained user-mode stack.
		Subnet: "192.168.127.0/24", Gateway: "192.168.127.1", Device: "192.168.127.2", Host: "192.168.127.254",
		GatewayMAC: "02:50:59:00:00:01", GuestMAC: "02:50:59:00:00:02", Interface: "pryx0", MTU: types.NewGvproxyCommand().MTU,
		DHCPClient: dhcpv4.ClientPort, DHCPServer: dhcpv4.ServerPort,
	}
	for _, item := range []struct {
		network, service string
		target           *int
	}{
		{"tcp", "http", &profile.HTTP}, {"tcp", "https", &profile.TLS}, {"udp", "domain", &profile.DNS},
	} {
		port, err := net.LookupPort(item.network, item.service)
		if err != nil {
			return runtimeProfile{}, err
		}
		*item.target = port
	}
	return profile, nil
}

type runtimeConfiguration struct {
	TapInterface string `json:"tapInterface,omitempty"`
	Parent       int    `json:"parent"`
	Host         string `json:"host"`
	Concurrency  uint16 `json:"concurrency"`
	// Egress is absent only in a configuration from before it existed (admission.go).
	Egress *egressConfiguration `json:"egress,omitempty"`
}

func readRuntime(path string) (runtimeConfiguration, error) {
	fd, err := unix.Open(path, unix.O_RDONLY|unix.O_CLOEXEC|unix.O_NOFOLLOW|unix.O_NONBLOCK, 0)
	if err != nil {
		return runtimeConfiguration{}, err
	}
	file := os.NewFile(uintptr(fd), "private network configuration")
	defer file.Close()
	info, err := file.Stat()
	if err != nil {
		return runtimeConfiguration{}, err
	}
	if !info.Mode().IsRegular() || info.Mode().Perm()&0o077 != 0 || info.Size() > maxConfiguration {
		return runtimeConfiguration{}, fmt.Errorf("Packet proxy configuration must be a bounded private file")
	}
	data, err := io.ReadAll(io.LimitReader(file, maxConfiguration+1))
	if err != nil {
		return runtimeConfiguration{}, err
	}
	if len(data) > maxConfiguration {
		return runtimeConfiguration{}, fmt.Errorf("Packet proxy configuration exceeds its bound")
	}
	var configuration runtimeConfiguration
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&configuration); err != nil {
		return runtimeConfiguration{}, err
	}
	if err := decoder.Decode(&struct{}{}); err != io.EOF {
		return runtimeConfiguration{}, fmt.Errorf("Packet proxy configuration has trailing input")
	}
	return configuration, nil
}

func serveProduction(path string, descriptor int) error {
	if os.Geteuid() == 0 || descriptor <= int(os.Stderr.Fd()) {
		return fmt.Errorf("Packet proxy requires an ordinary user and inherited private listener")
	}
	configuration, err := readRuntime(path)
	if err != nil {
		return err
	}
	ip := net.ParseIP(configuration.Host)
	if configuration.Parent != os.Getppid() || configuration.Parent <= 0 || configuration.Concurrency == 0 || ip.To4() == nil || !ip.IsLoopback() {
		return fmt.Errorf("Packet proxy requires its real parent, admitted capacity and IPv4 loopback gateways")
	}
	profile, err := productionMetadata()
	if err != nil {
		return err
	}
	file := os.NewFile(uintptr(descriptor), "private packet listener")
	if file == nil {
		return fmt.Errorf("Packet proxy listener descriptor is invalid")
	}
	listener, err := net.FileListener(file)
	file.Close()
	if err != nil {
		return err
	}
	defer listener.Close()
	if _, ok := listener.(*net.UnixListener); !ok {
		return fmt.Errorf("Packet transport requires a private Unix listener")
	}
	policy, err := newAdmission(profile.Host, configuration.Egress)
	if err != nil {
		return err
	}
	network, err := newPacketStack(profile, stackOptions{virtualIP: profile.Host, host: ip.String(),
		policy: policy, concurrency: int(configuration.Concurrency)})
	if err != nil {
		return err
	}
	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGTERM, syscall.SIGINT)
	defer stop()
	server := &http.Server{ReadHeaderTimeout: 10 * time.Second, Handler: http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		if request.URL.EscapedPath() != types.ConnectPath || request.Method != http.MethodPost {
			http.NotFound(writer, request)
			return
		}
		connectGuest(ctx, writer, network)
	})}
	defer server.Close()
	exited := make(chan error, 1)
	go func() { exited <- server.Serve(listener) }()
	if err := json.NewEncoder(os.Stdout).Encode(profile); err != nil {
		return err
	}
	if err := os.Stdout.Close(); err != nil {
		return err
	}
	select {
	case <-ctx.Done():
		return nil
	case err := <-exited:
		return err
	}
}

// bufferedConnection reads first what the HTTP server buffered past the connect request.
type bufferedConnection struct {
	net.Conn
	reader *bufio.Reader
}

func (connection bufferedConnection) Read(data []byte) (int, error) {
	return connection.reader.Read(data)
}

// connectGuest hands one guest packet stream to the switch, as the library's own connect route
// does, so frames and their stream framing stay the library's.
func connectGuest(ctx context.Context, writer http.ResponseWriter, network *tap.Switch) {
	hijacker, ok := writer.(http.Hijacker)
	if !ok {
		http.Error(writer, "Packet transport cannot take over its connection", http.StatusInternalServerError)
		return
	}
	connection, buffered, err := hijacker.Hijack()
	if err != nil {
		http.Error(writer, err.Error(), http.StatusInternalServerError)
		return
	}
	defer connection.Close()
	if err := buffered.Flush(); err != nil {
		return
	}
	network.Accept(ctx, bufferedConnection{Conn: connection, reader: buffered.Reader}, types.StdioProtocol)
}

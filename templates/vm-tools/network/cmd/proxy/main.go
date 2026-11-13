// SPDX-License-Identifier: FSL-1.1-ALv2
// Command proxy wraps the maintained virtual network for the vsock experiment. The guest
// reaches only its packet-connect route; management stays on a separate private host socket.
// It implements neither packet framing nor a network stack and enables no product tier.
package main

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"log"
	"net"
	"net/http"
	"os"
	"os/signal"
	"strconv"
	"syscall"
	"time"

	"cubepals/pryx/vm-experiment/experiment"
	networkfixture "cubepals/pryx/vm-network-experiment"
	"github.com/containers/gvisor-tap-vsock/pkg/tokenauth"
	"github.com/containers/gvisor-tap-vsock/pkg/types"
	"github.com/containers/gvisor-tap-vsock/pkg/virtualnetwork"
)

func main() {
	configuration := flag.String("config", "", "Private network fixture JSON")
	guestSocket := flag.String("guest-socket", "", "Guest-facing packet transport Unix listener")
	controlSocket := flag.String("control-socket", "", "Separate private host-only management listener")
	metadata := flag.Bool("metadata", false, "Export the upstream API's unexported route names")
	flag.Parse()
	if *metadata {
		if err := json.NewEncoder(os.Stdout).Encode(map[string]string{
			"exposePath": networkfixture.ExposePath, "statsPath": networkfixture.StatsPath,
		}); err != nil {
			log.Fatal(err)
		}
		return
	}
	data, err := os.ReadFile(*configuration)
	if err != nil {
		log.Fatal(err)
	}
	var fixture experiment.Network
	if err := json.Unmarshal(data, &fixture); err != nil {
		log.Fatal(err)
	}
	stack, err := virtualnetwork.New(&types.Configuration{
		MTU: fixture.MTU, Subnet: fixture.Subnet, GatewayIP: fixture.Gateway, DeviceIP: fixture.Device,
		GatewayMacAddress: fixture.GatewayMAC, GatewayVirtualIPs: []string{fixture.Host},
		NAT: map[string]string{fixture.Host: "127.0.0.1"}, Protocol: types.StdioProtocol,
		DHCPStaticLeases: map[string]string{fixture.Device: fixture.GuestMAC},
		DNS: []types.Zone{{Name: "spike.pryx.invalid.", Protected: true,
			Records: []types.Record{{Name: "host", IP: net.ParseIP(fixture.Host)}}}},
	})
	if err != nil {
		log.Fatal(err)
	}
	defer stack.Close()
	secret, err := tokenauth.GenerateToken()
	if err != nil {
		log.Fatal(err)
	}
	stack.SetAPIToken(secret)
	packetMux := stack.Mux()
	guestHandler := http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		if request.URL.EscapedPath() != types.ConnectPath || request.Method != http.MethodPost {
			http.NotFound(writer, request)
			return
		}
		packetMux.ServeHTTP(writer, request)
	})
	guest, err := net.Listen("unix", *guestSocket)
	if err != nil {
		log.Fatal(err)
	}
	defer guest.Close()
	control, err := net.Listen("unix", *controlSocket)
	if err != nil {
		log.Fatal(err)
	}
	defer control.Close()
	for _, path := range []string{*guestSocket, *controlSocket} {
		if err := os.Chmod(path, 0o600); err != nil {
			log.Fatal(err)
		}
	}
	httpPort, err := net.LookupPort("tcp", "http")
	if err != nil {
		log.Fatal(err)
	}
	gateway, err := stack.Listen("tcp", net.JoinHostPort(fixture.Gateway, strconv.Itoa(httpPort)))
	if err != nil {
		log.Fatal(err)
	}
	defer gateway.Close()
	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGTERM, syscall.SIGINT)
	defer stop()
	errors := make(chan error, 3)
	for _, endpoint := range []struct {
		listener net.Listener
		handler  http.Handler
	}{{guest, guestHandler}, {control, stack.ServicesMux()}, {gateway, stack.GatewayMux()}} {
		server := &http.Server{Handler: endpoint.handler, ReadHeaderTimeout: 10 * time.Second}
		go func() { errors <- server.Serve(endpoint.listener) }()
		defer server.Close()
	}
	fmt.Println("Private packet transport and host-only management ready")
	select {
	case <-ctx.Done():
	case err := <-errors:
		log.Fatal(err)
	}
}

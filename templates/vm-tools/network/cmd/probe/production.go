// SPDX-License-Identifier: FSL-1.1-ALv2
// Run ordinary-workload requests through the production DHCP, immutable guest firewall and
// actual hostname gateways. The fixture supplies only public names and the host's normal CA
// bundle; TLS verification, request deadlines and the existing comparison probes are retained.
package main

import (
	"context"
	"crypto/tls"
	"crypto/x509"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"strconv"
	"time"
)

type productionFixture struct {
	Unrestricted bool   `json:"unrestricted"`
	Hostname     string `json:"hostname"`
	Host         string `json:"host"`
	PrivatePort  uint16 `json:"privatePort"`
	HTTP         uint16 `json:"http"`
	DNS          uint16 `json:"dns"`
	CAFile       string `json:"caFile"`
}

func productionPolicy(path string) error {
	data, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	var fixture productionFixture
	if err := json.Unmarshal(data, &fixture); err != nil {
		return err
	}
	if os.Geteuid() == 0 || fixture.Hostname == "" || fixture.Host == "" || fixture.PrivatePort == 0 || fixture.HTTP == 0 || fixture.DNS == 0 {
		return fmt.Errorf("Production policy acceptance requires an ordinary workload and complete fixture")
	}
	if err := checkGuestPolicy(); err != nil {
		return err
	}
	certificate, err := os.ReadFile(fixture.CAFile)
	if err != nil {
		return err
	}
	roots := x509.NewCertPool()
	if !roots.AppendCertsFromPEM(certificate) {
		return fmt.Errorf("Host CA bundle contains no certificates")
	}
	transport := &http.Transport{Proxy: nil, DisableKeepAlives: true, TLSClientConfig: &tls.Config{RootCAs: roots}}
	defer transport.CloseIdleConnections()
	client := &http.Client{Transport: transport, Timeout: requestTimeout,
		CheckRedirect: func(_ *http.Request, _ []*http.Request) error { return http.ErrUseLastResponse }}
	for _, scheme := range []string{"http", "https"} {
		response, err := client.Get(scheme + "://" + fixture.Hostname + "/")
		if err != nil {
			return fmt.Errorf("Allowed %s request: %w", scheme, err)
		}
		body, readErr := io.ReadAll(io.LimitReader(response.Body, 64*1024))
		response.Body.Close()
		if readErr != nil {
			return readErr
		}
		if response.StatusCode != http.StatusOK || len(body) == 0 {
			return fmt.Errorf("Allowed %s request returned %s and %d bytes", scheme, response.Status, len(body))
		}
	}
	// RFC 5737's documentation-only address is deliberately not a working resolver.
	// Success proves that arbitrary DNS destinations are redirected into the policy resolver.
	for _, protocol := range []string{"udp", "tcp"} {
		resolver := &net.Resolver{PreferGo: true, Dial: func(ctx context.Context, _, _ string) (net.Conn, error) {
			return (&net.Dialer{}).DialContext(ctx, protocol, net.JoinHostPort("203.0.113.1", strconv.Itoa(int(fixture.DNS))))
		}}
		ctx, cancel := context.WithTimeout(context.Background(), requestTimeout)
		addresses, err := resolver.LookupIP(ctx, "ip4", fixture.Hostname)
		cancel()
		if err != nil || len(addresses) == 0 {
			return fmt.Errorf("Redirected allowed %s DNS: %v", protocol, err)
		}
		ctx, cancel = context.WithTimeout(context.Background(), requestTimeout)
		_, err = resolver.LookupIP(ctx, "ip4", "denied.pryx.invalid")
		cancel()
		if err == nil {
			return fmt.Errorf("Disallowed %s DNS name resolved", protocol)
		}
	}
	if !fixture.Unrestricted {
		for _, authority := range []string{"denied.pryx.invalid", fixture.Host} {
			request, err := http.NewRequest(http.MethodGet, "http://"+net.JoinHostPort(fixture.Host, strconv.Itoa(int(fixture.HTTP)))+"/", nil)
			if err != nil {
				return err
			}
			request.Host = authority
			response, err := client.Do(request)
			if err != nil {
				return err
			}
			response.Body.Close()
			if response.StatusCode != http.StatusForbidden {
				return fmt.Errorf("Hostname/IP bypass returned %s", response.Status)
			}
		}
	}
	for _, address := range []string{fixture.Host, "169.254.169.254"} {
		connection, err := net.DialTimeout("tcp", net.JoinHostPort(address, strconv.Itoa(int(fixture.PrivatePort))), 300*time.Millisecond)
		if connection != nil {
			connection.Close()
		}
		if err == nil {
			return fmt.Errorf("Guest reached a private or metadata endpoint outside its gateways")
		}
	}
	if fixture.Unrestricted {
		return json.NewEncoder(os.Stdout).Encode(map[string]bool{
			"http": true, "tlsVerified": true, "udpDnsRedirected": true, "tcpDnsRedirected": true,
			"privateHostDenied": true, "metadataDenied": true, "packetSyscallsDenied": true,
		})
	}
	return json.NewEncoder(os.Stdout).Encode(map[string]bool{
		"http": true, "tlsVerified": true, "udpDnsRedirected": true, "tcpDnsRedirected": true, "dnsDenied": true,
		"hostnameDenied": true, "rawIPDenied": true, "hostListenerDenied": true, "packetSyscallsDenied": true,
	})
}

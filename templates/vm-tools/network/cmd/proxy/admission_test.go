// SPDX-License-Identifier: FSL-1.1-ALv2
// Prove the host-side admission refuses what a guest whose firewall is gone could ask for, both
// as a decision and through the real assembled stack against real host loopback services. The
// non-public boundary is read from pryx-agent's own source, the list pryxd passes.
package main

import (
	"errors"
	"net"
	"net/netip"
	"os"
	"regexp"
	"strings"
	"testing"
	"time"

	"github.com/containers/gvisor-tap-vsock/pkg/types"
	"github.com/google/gopacket"
	"github.com/google/gopacket/layers"
	"golang.org/x/sys/unix"
	"gvisor.dev/gvisor/pkg/tcpip"
	"gvisor.dev/gvisor/pkg/tcpip/transport/icmp"
	"gvisor.dev/gvisor/pkg/tcpip/transport/tcp"
	"gvisor.dev/gvisor/pkg/tcpip/transport/udp"
)

// agentBoundary reads SPECIAL_V4 from the module that owns it.
func agentBoundary(t *testing.T) []string {
	t.Helper()
	source, err := os.ReadFile("../../../../../agent/src/transfer_target.rs")
	if err != nil {
		t.Fatal(err)
	}
	text := string(source)
	start := strings.Index(text, "const SPECIAL_V4")
	end := strings.Index(text[start:], "];")
	if start < 0 || end < 0 {
		t.Fatal("pryx-agent no longer declares SPECIAL_V4")
	}
	var prefixes []string
	entry := regexp.MustCompile(`Ipv4Addr::new\((\d+), (\d+), (\d+), (\d+)\), (\d+)\)`)
	for _, match := range entry.FindAllStringSubmatch(text[start:start+end], -1) {
		prefixes = append(prefixes, strings.Join(match[1:5], ".")+"/"+match[5])
	}
	if len(prefixes) == 0 {
		t.Fatal("pryx-agent's SPECIAL_V4 holds no networks")
	}
	return prefixes
}

const httpGateway, tlsGateway, dnsGateway = 41001, 41002, 41003

func hostnamePolicy(t *testing.T) admission {
	t.Helper()
	profile, err := productionMetadata()
	if err != nil {
		t.Fatal(err)
	}
	policy, err := newAdmission(profile.Host, &egressConfiguration{TCP: []uint16{httpGateway, tlsGateway, dnsGateway},
		UDP: []uint16{dnsGateway}, NonPublic: agentBoundary(t)})
	if err != nil {
		t.Fatal(err)
	}
	return policy
}

type flow struct {
	protocol tcpip.TransportProtocolNumber
	address  string
	port     uint16
}

func decide(t *testing.T, policy admission, cases []flow, want bool) {
	t.Helper()
	for _, item := range cases {
		if got := policy.allows(item.protocol, netip.MustParseAddr(item.address), item.port); got != want {
			t.Errorf("%v %s:%d admitted=%v, want %v", item.protocol, item.address, item.port, got, want)
		}
	}
}

func TestHostnamePolicyAdmitsOnlyItsGatewayPorts(t *testing.T) {
	policy := hostnamePolicy(t)
	gateway := policy.gateway.String()
	decide(t, policy, []flow{{tcp.ProtocolNumber, gateway, httpGateway}, {tcp.ProtocolNumber, gateway, tlsGateway},
		{tcp.ProtocolNumber, gateway, dnsGateway}, {udp.ProtocolNumber, gateway, dnsGateway}}, true)
	decide(t, policy, []flow{
		{tcp.ProtocolNumber, gateway, 22},          // another host loopback service
		{udp.ProtocolNumber, gateway, httpGateway}, // a TCP gateway's number over UDP
		{icmp.ProtocolNumber4, gateway, 0},
		{tcp.ProtocolNumber, "8.8.8.8", 443}, // public, outside the allowlist's gateways
		{udp.ProtocolNumber, "1.1.1.1", 53},
		{icmp.ProtocolNumber4, "8.8.8.8", 0},
	}, false)
}

func TestNonPublicDestinationsAreRefusedWithPublicEgress(t *testing.T) {
	profile, err := productionMetadata()
	if err != nil {
		t.Fatal(err)
	}
	policy, err := newAdmission(profile.Host, &egressConfiguration{Public: true, TCP: []uint16{dnsGateway},
		UDP: []uint16{dnsGateway}, NonPublic: agentBoundary(t)})
	if err != nil {
		t.Fatal(err)
	}
	decide(t, policy, []flow{{tcp.ProtocolNumber, "8.8.8.8", 443}, {udp.ProtocolNumber, "1.1.1.1", 443},
		{icmp.ProtocolNumber4, "8.8.8.8", 0}, {tcp.ProtocolNumber, profile.Host, dnsGateway}}, true)
	var refused []flow
	for _, address := range []string{
		"127.0.0.1", "127.255.0.9", // loopback
		"169.254.169.254", "169.254.0.1", // link-local and metadata
		"10.0.0.1", "172.16.0.1", "192.168.1.1", profile.Gateway, profile.Device, // private, the stack's own
		"224.0.0.251", "239.255.255.250", // multicast
		"0.0.0.0", "255.255.255.255", // unspecified, broadcast
		"100.64.0.1", "192.0.0.8", "192.0.2.1", "198.18.0.1", "198.51.100.1", "203.0.113.1", "240.0.0.1", // the agent's boundary
	} {
		for _, protocol := range []tcpip.TransportProtocolNumber{tcp.ProtocolNumber, udp.ProtocolNumber, icmp.ProtocolNumber4} {
			refused = append(refused, flow{protocol, address, 80})
		}
	}
	refused = append(refused, flow{tcp.ProtocolNumber, profile.Host, httpGateway}, flow{tcp.ProtocolNumber, profile.Host, 22})
	decide(t, policy, refused, false)
}

func TestConfigurationWithoutEgressIsRefused(t *testing.T) {
	if _, err := newAdmission("192.168.127.254", nil); err == nil {
		t.Fatal("A configuration without egress was accepted")
	}
}

func TestEgressConfigurationIsValidated(t *testing.T) {
	boundary := []string{"10.0.0.0/8"}
	for name, egress := range map[string]*egressConfiguration{
		"zero port":           {TCP: []uint16{0}},
		"zero UDP port":       {UDP: []uint16{0}},
		"public, no boundary": {Public: true},
		"unmasked boundary":   {Public: true, NonPublic: []string{"10.0.0.1/8"}},
		"IPv6 boundary":       {Public: true, NonPublic: []string{"fc00::/7"}},
		"not a network":       {Public: true, NonPublic: []string{"private"}},
	} {
		if _, err := newAdmission("192.168.127.254", egress); err == nil {
			t.Errorf("%s: accepted", name)
		}
	}
	if _, err := newAdmission("::1", &egressConfiguration{Public: true, NonPublic: boundary}); err == nil {
		t.Error("IPv6 gateway accepted")
	}
	if _, err := newAdmission("192.168.127.254", &egressConfiguration{Public: true, NonPublic: boundary}); err != nil {
		t.Fatal(err)
	}
}

func TestRuntimeConfigurationReadsEgress(t *testing.T) {
	path := t.TempDir() + "/network.json"
	write := func(text string) {
		if err := os.WriteFile(path, []byte(text), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	write(`{"parent":1,"host":"127.0.0.1","concurrency":8,"egress":{"public":false,"tcp":[1,2,3],"udp":[3],"nonPublic":["10.0.0.0/8"]}}`)
	configuration, err := readRuntime(path)
	if err != nil {
		t.Fatal(err)
	}
	if configuration.Egress == nil || configuration.Egress.Public || len(configuration.Egress.TCP) != 3 || configuration.Egress.UDP[0] != 3 {
		t.Fatalf("Egress was not read: %+v", configuration.Egress)
	}
	write(`{"parent":1,"host":"127.0.0.1","concurrency":8,"egress":{"public":true,"allow":["10.0.0.0/8"]}}`)
	if _, err := readRuntime(path); err == nil {
		t.Fatal("An unknown egress field was accepted")
	}
}

// guestStack runs the assembled stack over a real packet socket pair, as the TAP transport does.
type guestStack struct {
	client  *os.File
	profile runtimeProfile
	port    uint16
}

func startGuestStack(t *testing.T, policy admission, resolver bool) *guestStack {
	t.Helper()
	pair, err := unix.Socketpair(unix.AF_UNIX, unix.SOCK_SEQPACKET|unix.SOCK_CLOEXEC|unix.SOCK_NONBLOCK, 0)
	if err != nil {
		t.Fatal(err)
	}
	client, device := os.NewFile(uintptr(pair[0]), "guest packets"), os.NewFile(uintptr(pair[1]), "stack packets")
	t.Cleanup(func() { client.Close(); device.Close() })
	profile, err := productionMetadata()
	if err != nil {
		t.Fatal(err)
	}
	network, err := newPacketStack(profile, stackOptions{virtualIP: profile.Host, host: "127.0.0.1", policy: policy,
		concurrency: 8, resolver: resolver})
	if err != nil {
		t.Fatal(err)
	}
	go network.Accept(t.Context(), tapConnection{device}, types.BessProtocol)
	return &guestStack{client: client, profile: profile, port: 40000}
}

// send writes one guest TCP SYN or UDP datagram to destination.
func (guest *guestStack) send(t *testing.T, protocol tcpip.TransportProtocolNumber, destination string, port uint16) {
	t.Helper()
	guestMAC, _ := net.ParseMAC(guest.profile.GuestMAC)
	gatewayMAC, _ := net.ParseMAC(guest.profile.GatewayMAC)
	ip := &layers.IPv4{Version: 4, TTL: 64, SrcIP: net.ParseIP(guest.profile.Device).To4(), DstIP: net.ParseIP(destination).To4()}
	guest.port++
	var transport gopacket.SerializableLayer
	switch protocol {
	case tcp.ProtocolNumber:
		ip.Protocol = layers.IPProtocolTCP
		segment := &layers.TCP{SrcPort: layers.TCPPort(guest.port), DstPort: layers.TCPPort(port), Seq: 1, SYN: true, Window: 65535}
		segment.SetNetworkLayerForChecksum(ip)
		transport = segment
	case udp.ProtocolNumber:
		ip.Protocol = layers.IPProtocolUDP
		datagram := &layers.UDP{SrcPort: layers.UDPPort(guest.port), DstPort: layers.UDPPort(port)}
		datagram.SetNetworkLayerForChecksum(ip)
		transport = datagram
	}
	frame := gopacket.NewSerializeBuffer()
	layersToSend := []gopacket.SerializableLayer{&layers.Ethernet{SrcMAC: guestMAC, DstMAC: gatewayMAC, EthernetType: layers.EthernetTypeIPv4}, ip, transport}
	if protocol == udp.ProtocolNumber {
		layersToSend = append(layersToSend, gopacket.Payload("pryx"))
	}
	if err := gopacket.SerializeLayers(frame, gopacket.SerializeOptions{FixLengths: true, ComputeChecksums: true}, layersToSend...); err != nil {
		t.Fatal(err)
	}
	if _, err := guest.client.Write(frame.Bytes()); err != nil {
		t.Fatal(err)
	}
}

func loopbackTCP(t *testing.T) (*net.TCPListener, uint16) {
	t.Helper()
	listener, err := net.ListenTCP("tcp4", &net.TCPAddr{IP: net.IPv4(127, 0, 0, 1)})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { listener.Close() })
	return listener, uint16(listener.Addr().(*net.TCPAddr).Port)
}

func loopbackUDP(t *testing.T) (*net.UDPConn, uint16) {
	t.Helper()
	socket, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1)})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { socket.Close() })
	return socket, uint16(socket.LocalAddr().(*net.UDPAddr).Port)
}

func reached(t *testing.T, accept func() error) bool {
	t.Helper()
	err := accept()
	var timeout net.Error
	if errors.As(err, &timeout) && timeout.Timeout() {
		return false
	}
	if err != nil {
		t.Fatal(err)
	}
	return true
}

func TestStackReachesOnlyNamedHostLoopbackPorts(t *testing.T) {
	gatewayTCP, gatewayTCPPort := loopbackTCP(t)
	otherTCP, otherTCPPort := loopbackTCP(t)
	gatewayUDP, gatewayUDPPort := loopbackUDP(t)
	otherUDP, otherUDPPort := loopbackUDP(t)
	profile, err := productionMetadata()
	if err != nil {
		t.Fatal(err)
	}
	policy, err := newAdmission(profile.Host, &egressConfiguration{TCP: []uint16{gatewayTCPPort}, UDP: []uint16{gatewayUDPPort},
		NonPublic: agentBoundary(t)})
	if err != nil {
		t.Fatal(err)
	}
	guest := startGuestStack(t, policy, false)
	// Refused flows go first; the admitted ones after them prove the stack was serving.
	guest.send(t, tcp.ProtocolNumber, profile.Host, otherTCPPort)
	guest.send(t, udp.ProtocolNumber, profile.Host, otherUDPPort)
	guest.send(t, tcp.ProtocolNumber, profile.Host, gatewayTCPPort)
	guest.send(t, udp.ProtocolNumber, profile.Host, gatewayUDPPort)
	buffer := make([]byte, 64)
	accept := func(listener *net.TCPListener, wait time.Duration) func() error {
		return func() error {
			listener.SetDeadline(time.Now().Add(wait))
			connection, err := listener.Accept()
			if err == nil {
				connection.Close()
			}
			return err
		}
	}
	receive := func(socket *net.UDPConn, wait time.Duration) func() error {
		return func() error {
			socket.SetReadDeadline(time.Now().Add(wait))
			_, _, err := socket.ReadFrom(buffer)
			return err
		}
	}
	if !reached(t, accept(gatewayTCP, 5*time.Second)) {
		t.Fatal("The named TCP gateway port was not reached")
	}
	if !reached(t, receive(gatewayUDP, 5*time.Second)) {
		t.Fatal("The named UDP gateway port was not reached")
	}
	if reached(t, accept(otherTCP, time.Second)) {
		t.Fatal("A host loopback TCP service outside the gateways was reached")
	}
	if reached(t, receive(otherUDP, time.Second)) {
		t.Fatal("A host loopback UDP service outside the gateways was reached")
	}
}

// answer replies to the stack's ARP requests for the guest and returns the first TCP segment
// from source, or nil when none arrives in time.
func (guest *guestStack) answer(t *testing.T, source string) *layers.TCP {
	t.Helper()
	guestMAC, _ := net.ParseMAC(guest.profile.GuestMAC)
	device := net.ParseIP(guest.profile.Device).To4()
	frame := make([]byte, guest.profile.MTU+64)
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		guest.client.SetReadDeadline(deadline)
		count, err := guest.client.Read(frame)
		if err != nil {
			return nil
		}
		packet := gopacket.NewPacket(frame[:count], layers.LayerTypeEthernet, gopacket.Default)
		if arp, ok := packet.Layer(layers.LayerTypeARP).(*layers.ARP); ok && arp.Operation == layers.ARPRequest &&
			net.IP(arp.DstProtAddress).Equal(device) {
			reply := gopacket.NewSerializeBuffer()
			if err := gopacket.SerializeLayers(reply, gopacket.SerializeOptions{FixLengths: true},
				&layers.Ethernet{SrcMAC: guestMAC, DstMAC: arp.SourceHwAddress, EthernetType: layers.EthernetTypeARP},
				&layers.ARP{AddrType: layers.LinkTypeEthernet, Protocol: layers.EthernetTypeIPv4, Operation: layers.ARPReply,
					SourceHwAddress: guestMAC, SourceProtAddress: device, DstHwAddress: arp.SourceHwAddress,
					DstProtAddress: arp.SourceProtAddress}); err != nil {
				t.Fatal(err)
			}
			if _, err := guest.client.Write(reply.Bytes()); err != nil {
				t.Fatal(err)
			}
			continue
		}
		ip, isIP := packet.Layer(layers.LayerTypeIPv4).(*layers.IPv4)
		segment, isTCP := packet.Layer(layers.LayerTypeTCP).(*layers.TCP)
		if isIP && isTCP && ip.SrcIP.Equal(net.ParseIP(source)) {
			return segment
		}
	}
	return nil
}

func TestOnlyTheBridgedStackServesItsOwnResolver(t *testing.T) {
	profile, err := productionMetadata()
	if err != nil {
		t.Fatal(err)
	}
	policy, err := newAdmission(profile.Host, &egressConfiguration{Public: true, NonPublic: agentBoundary(t)})
	if err != nil {
		t.Fatal(err)
	}
	for _, resolver := range []bool{true, false} {
		guest := startGuestStack(t, policy, resolver)
		guest.send(t, tcp.ProtocolNumber, profile.Gateway, uint16(profile.DNS))
		segment := guest.answer(t, profile.Gateway)
		if segment == nil {
			t.Fatalf("resolver=%v: the stack's gateway did not answer a DNS connection", resolver)
		}
		// With its resolver the gateway accepts; without it the private address is refused.
		if accepted := segment.SYN && segment.ACK; accepted != resolver || (!resolver && !segment.RST) {
			t.Fatalf("resolver=%v: the gateway answered SYN=%v ACK=%v RST=%v", resolver, segment.SYN, segment.ACK, segment.RST)
		}
	}
}

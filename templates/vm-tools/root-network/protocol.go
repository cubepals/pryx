// SPDX-License-Identifier: FSL-1.1-ALv2
// Private root namespace descriptor metadata. These describe parser support and owned
// kernel handles, never host capability readiness or the public node protocol.
package rootnetwork

const ScopeVersion = 1
const BridgeAgentFlag = "bridge-agent"
const BridgePacketFlag = "bridge-packet"
const PacketRuntimeFlag = "packet-runtime"

type ScopeRole string

const (
	ScopeNamespace           ScopeRole = "namespace"
	ScopePacketTap           ScopeRole = "packetTap"
	ScopePacketConfiguration ScopeRole = "packetConfiguration"
	ScopePacketIdentity      ScopeRole = "packetIdentity"
	ScopeHTTP                ScopeRole = "http"
	ScopeTLS                 ScopeRole = "tls"
	ScopeDNSTCP              ScopeRole = "dnsTcp"
	ScopeDNSUDP              ScopeRole = "dnsUdp"
)

type ScopeMetadata struct {
	PacketRuntimeArgument    string      `json:"packetRuntimeArgument,omitempty"`
	PacketRuntimeDescriptors []ScopeRole `json:"packetRuntimeDescriptors,omitempty"`
	HostFirewall             bool        `json:"hostFirewall"`
	Descriptors              []ScopeRole `json:"descriptors"`
	MaxDescriptors           int         `json:"maxDescriptors"`
	MaxMetadataBytes         int         `json:"maxMetadataBytes"`
	Version                  int         `json:"version"`
	BridgeHostFirewall       bool        `json:"bridgeHostFirewall,omitempty"`
	BridgeDescriptors        []ScopeRole `json:"bridgeDescriptors,omitempty"`
	BridgeAgentArgument      string      `json:"bridgeAgentArgument,omitempty"`
	BridgePacketArgument     string      `json:"bridgePacketArgument,omitempty"`
}

type ScopeHandoff struct {
	PacketPaths    *PacketPaths   `json:"packetPaths,omitempty"`
	PacketProfile  map[string]any `json:"packetProfile,omitempty"`
	HostFirewall   bool           `json:"hostFirewall"`
	Descriptors    []ScopeRole    `json:"descriptors"`
	Profile        Profile        `json:"profile"`
	Tap            string         `json:"tap"`
	Version        int            `json:"version"`
	PacketTap      string         `json:"packetTap,omitempty"`
	Ipv4Exclusions int            `json:"ipv4Exclusions,omitempty"`
}

func scopeDescriptors() []ScopeRole {
	return []ScopeRole{ScopeNamespace, ScopeHTTP, ScopeTLS, ScopeDNSTCP, ScopeDNSUDP}
}

func ScopeAdvertisement() ScopeMetadata {
	return ScopeMetadata{PacketRuntimeArgument: "--" + PacketRuntimeFlag, PacketRuntimeDescriptors: []ScopeRole{ScopeNamespace, ScopePacketTap, ScopePacketConfiguration}, HostFirewall: true, Descriptors: scopeDescriptors(), MaxDescriptors: MaxHandoffFiles, MaxMetadataBytes: MaxHandoffBytes, Version: ScopeVersion, BridgeHostFirewall: true, BridgeDescriptors: []ScopeRole{ScopeNamespace, ScopePacketTap}, BridgeAgentArgument: "--" + BridgeAgentFlag, BridgePacketArgument: "--" + BridgePacketFlag}
}

func (scope *Scope) Grant() ScopeHandoff {
	return ScopeHandoff{HostFirewall: scope.firewallConfirmed, Descriptors: scopeDescriptors(), Profile: scope.Profile, Tap: scope.Tap, Version: ScopeVersion}
}

func (bridge *Bridge) Grant() ScopeHandoff {
	return ScopeHandoff{HostFirewall: bridge.confirmed, Descriptors: []ScopeRole{ScopeNamespace, ScopePacketTap},
		Profile: bridge.scope.Profile, Tap: bridge.scope.Tap, Version: ScopeVersion, PacketTap: packetTapName,
		Ipv4Exclusions: len(bridge.inputs.blocked)}
}

// SPDX-License-Identifier: FSL-1.1-ALv2
// Private root namespace descriptor metadata. These describe parser support and owned
// kernel handles, never host capability readiness or the public node protocol.
package rootnetwork

const ScopeVersion = 1

type ScopeRole string

const (
	ScopeNamespace ScopeRole = "namespace"
	ScopeHTTP      ScopeRole = "http"
	ScopeTLS       ScopeRole = "tls"
	ScopeDNSTCP    ScopeRole = "dnsTcp"
	ScopeDNSUDP    ScopeRole = "dnsUdp"
)

type ScopeMetadata struct {
	HostFirewall     bool        `json:"hostFirewall"`
	Descriptors      []ScopeRole `json:"descriptors"`
	MaxDescriptors   int         `json:"maxDescriptors"`
	MaxMetadataBytes int         `json:"maxMetadataBytes"`
	Version          int         `json:"version"`
}

type ScopeHandoff struct {
	HostFirewall bool        `json:"hostFirewall"`
	Descriptors  []ScopeRole `json:"descriptors"`
	Profile      Profile     `json:"profile"`
	Tap          string      `json:"tap"`
	Version      int         `json:"version"`
}

func scopeDescriptors() []ScopeRole {
	return []ScopeRole{ScopeNamespace, ScopeHTTP, ScopeTLS, ScopeDNSTCP, ScopeDNSUDP}
}

func ScopeAdvertisement() ScopeMetadata {
	return ScopeMetadata{HostFirewall: true, Descriptors: scopeDescriptors(), MaxDescriptors: MaxHandoffFiles, MaxMetadataBytes: MaxHandoffBytes, Version: ScopeVersion}
}

func (scope *Scope) Grant() ScopeHandoff {
	return ScopeHandoff{HostFirewall: scope.firewallConfirmed, Descriptors: scopeDescriptors(), Profile: scope.Profile, Tap: scope.Tap, Version: ScopeVersion}
}

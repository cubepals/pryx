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
	Descriptors      []ScopeRole `json:"descriptors"`
	MaxMetadataBytes int         `json:"maxMetadataBytes"`
	Version          int         `json:"version"`
}

type ScopeHandoff struct {
	Descriptors []ScopeRole `json:"descriptors"`
	Profile     Profile     `json:"profile"`
	Tap         string      `json:"tap"`
	Version     int         `json:"version"`
}

func scopeDescriptors() []ScopeRole {
	return []ScopeRole{ScopeNamespace, ScopeHTTP, ScopeTLS, ScopeDNSTCP, ScopeDNSUDP}
}

func ScopeAdvertisement() ScopeMetadata {
	return ScopeMetadata{Descriptors: scopeDescriptors(), MaxMetadataBytes: MaxHandoffBytes, Version: ScopeVersion}
}

func ScopeGrant(profile Profile, tap string) ScopeHandoff {
	return ScopeHandoff{Descriptors: scopeDescriptors(), Profile: profile, Tap: tap, Version: ScopeVersion}
}

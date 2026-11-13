// SPDX-License-Identifier: FSL-1.1-ALv2
// Package experiment owns the spike's application port, not any vsock protocol number.
package experiment

const ReportPort uint32 = 11825
const ForwardPort uint16 = 11829
const ForwardPrefix = "opaque-prefix:"
const ForwardSuffix = ":opaque-suffix"
const RootDrive = "root"
const WriteDrive = "writes"

const NetworkPort uint32 = 11826

const NetworkBulkBytes = 16 * 1024 * 1024
const NetworkHello = "pryx-network-spike\n"
const NetworkHelloPath = "/hello"
const NetworkBulkPath = "/bulk"

// Network is private fixture configuration, never the node or public protocol.
type Network struct {
	Mode       string `json:"mode"`
	URL        string `json:"url"`
	Subnet     string `json:"subnet"`
	Gateway    string `json:"gateway"`
	Device     string `json:"device"`
	Host       string `json:"host"`
	GatewayMAC string `json:"gatewayMac"`
	GuestMAC   string `json:"guestMac"`
	MTU        int    `json:"mtu"`
}

func NetworkFixture() Network {
	return Network{
		Subnet: "10.77.25.0/24", Gateway: "10.77.25.1", Device: "10.77.25.2", Host: "10.77.25.254",
		GatewayMAC: "02:fc:25:00:00:01", GuestMAC: "02:fc:25:00:00:02", MTU: 1500,
	}
}

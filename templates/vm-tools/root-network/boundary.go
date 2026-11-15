// SPDX-License-Identifier: FSL-1.1-ALv2
// Read the existing agent boundary and packet profile from fixed root-owned build programs.
// Opaque bridge inputs prevent an ordinary request from supplying another address policy.
// This reads parser metadata only; it grants no namespace, TAP or packet authority.
package rootnetwork

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/netip"
	"os"
	"os/exec"
	"path/filepath"

	"github.com/google/nftables"
)

type BridgeInputs struct {
	agent          string
	packet         string
	monitorMode    string
	packetTap      bool
	packetMetadata map[string]any
	profile        Profile
	gatewayMAC     net.HardwareAddr
	dns            uint16
	blocked        []netip.Prefix
}

func programMetadata(ctx context.Context, program, argument string) (map[string]json.RawMessage, error) {
	if os.Geteuid() != 0 {
		return nil, fmt.Errorf("Bridge policy metadata requires its root host authority")
	}
	if err := Ancestry(filepath.Dir(program)); err != nil {
		return nil, err
	}
	if err := Executable(program); err != nil {
		return nil, err
	}
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	command := exec.CommandContext(ctx, program, argument)
	command.Env = []string{}
	command.Stderr = io.Discard
	output, err := command.StdoutPipe()
	if err != nil {
		return nil, err
	}
	if err := command.Start(); err != nil {
		return nil, err
	}
	// This is this reader's allocation ceiling, not an imported producer limit.
	data, readError := io.ReadAll(io.LimitReader(output, MaxHandoffBytes+1))
	if readError != nil || len(data) > MaxHandoffBytes {
		cancel()
		command.Wait()
		return nil, fmt.Errorf("Root bridge metadata is unreadable or exceeds its bound: %v", readError)
	}
	if err := command.Wait(); err != nil {
		return nil, err
	}
	var metadata map[string]json.RawMessage
	if err := json.Unmarshal(data, &metadata); err != nil {
		return nil, err
	}
	return metadata, nil
}

func ReadBridgeInputs(ctx context.Context, agent, packet string) (BridgeInputs, error) {
	var inputs BridgeInputs
	inputs.agent, inputs.packet = agent, packet
	metadata, err := programMetadata(ctx, agent, "bootstrap-probe")
	if err != nil {
		return inputs, err
	}
	json.Unmarshal(metadata["vmPacketTap"], &inputs.packetTap)
	json.Unmarshal(metadata["vmMonitorMode"], &inputs.monitorMode)
	var prefixes []string
	if err := json.Unmarshal(metadata["vmNonPublicIpv4"], &prefixes); err != nil || len(prefixes) == 0 {
		return inputs, fmt.Errorf("Actual agent does not export its non-public IPv4 boundary: %v", err)
	}
	for _, text := range prefixes {
		prefix, err := netip.ParsePrefix(text)
		if err != nil || !prefix.Addr().Is4() || prefix != prefix.Masked() {
			return inputs, fmt.Errorf("Actual agent exports an invalid IPv4 boundary: %v", err)
		}
		inputs.blocked = append(inputs.blocked, prefix)
	}
	metadata, err = programMetadata(ctx, packet, "--metadata")
	if err != nil {
		return inputs, err
	}
	if err := json.Unmarshal(metadata["production"], &inputs.packetMetadata); err != nil {
		return inputs, err
	}
	var profile map[string]json.RawMessage
	if err := json.Unmarshal(metadata["production"], &profile); err != nil {
		return inputs, err
	}
	values := map[string]string{}
	for _, key := range []string{"gateway", "device", "guestMac", "gatewayMac", "interface", "subnet", "tapDescriptorArgument"} {
		var value string
		if err := json.Unmarshal(profile[key], &value); err != nil || value == "" {
			return inputs, fmt.Errorf("Actual packet program omits its bridge profile: %v", err)
		}
		values[key] = value
	}
	prefix, err := netip.ParsePrefix(values["subnet"])
	if err != nil || !prefix.Addr().Is4() || prefix != prefix.Masked() {
		return inputs, fmt.Errorf("Packet program exports an invalid IPv4 subnet: %v", err)
	}
	device, deviceError := netip.ParseAddr(values["device"])
	gateway, gatewayError := netip.ParseAddr(values["gateway"])
	if deviceError != nil || gatewayError != nil || device == gateway || !prefix.Contains(device) || !prefix.Contains(gateway) {
		return inputs, fmt.Errorf("Packet program exports inconsistent private addresses")
	}
	inputs.gatewayMAC, err = net.ParseMAC(values["gatewayMac"])
	guestMAC, guestError := net.ParseMAC(values["guestMac"])
	if err != nil || guestError != nil || len(inputs.gatewayMAC) != len(guestMAC) || len(guestMAC) != int(nftables.TypeEtherAddr.Bytes) {
		return inputs, fmt.Errorf("Packet program exports an invalid Ethernet MAC")
	}
	if err := json.Unmarshal(profile["dns"], &inputs.dns); err != nil || inputs.dns == 0 {
		return inputs, fmt.Errorf("Packet program omits its DNS service port: %v", err)
	}
	var mtu int
	if err := json.Unmarshal(profile["mtu"], &mtu); err != nil || mtu <= 0 {
		return inputs, fmt.Errorf("Packet program omits its actual MTU: %v", err)
	}
	inputs.profile = Profile{Host: gateway.String(), GuestCIDR: netip.PrefixFrom(device, prefix.Bits()).String(),
		GuestMAC: values["guestMac"], Interface: values["interface"], MTU: mtu, Subnet: prefix.String()}
	return inputs, nil
}

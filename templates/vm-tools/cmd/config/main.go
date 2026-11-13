// SPDX-License-Identifier: FSL-1.1-ALv2
// Command config emits spike configuration using the monitor owner's generated API models.
// It does not implement a monitor client or enable production VM placement.
package main

import (
	"encoding/json"
	"flag"
	"log"
	"os"

	"cubepals/pryx/vm-experiment/experiment"
	"github.com/firecracker-microvm/firecracker-go-sdk/client/models"
	"github.com/go-openapi/strfmt"
	"github.com/mdlayher/vsock"
)

func main() {
	kernel := flag.String("kernel", "", "Verified published guest kernel")
	root := flag.String("root", "", "Read-only EROFS root")
	writes := flag.String("writes", "", "Writable ext4 disk")
	socket := flag.String("socket", "", "Private monitor vsock path")
	tap := flag.String("tap", "", "Optional private network namespace TAP for the comparison")
	metadata := flag.Bool("metadata", false, "Print experiment-owned transport metadata")
	flag.Parse()
	if *metadata {
		if err := json.NewEncoder(os.Stdout).Encode(map[string]any{
			"reportPort": experiment.ReportPort, "networkPort": experiment.NetworkPort,
			"network":          experiment.NetworkFixture(),
			"networkBulkBytes": experiment.NetworkBulkBytes, "networkHello": experiment.NetworkHello,
			"networkHelloPath": experiment.NetworkHelloPath, "networkBulkPath": experiment.NetworkBulkPath,
		}); err != nil {
			log.Fatal(err)
		}
		return
	}
	for _, value := range []string{*kernel, *root, *writes, *socket} {
		if value == "" {
			log.Fatal("All experiment paths are required")
		}
	}
	config := models.FullVMConfiguration{
		BootSource: &models.BootSource{
			KernelImagePath: kernel,
			BootArgs:        "console=ttyS0 reboot=k panic=1 pci=off rootfstype=erofs init=/init ro",
		},
		MachineConfig: &models.MachineConfiguration{VcpuCount: new(int64(1)), MemSizeMib: new(int64(256))},
		Drives: []*models.Drive{
			{DriveID: new("root"), PathOnHost: root, IsReadOnly: new(true), IsRootDevice: new(true)},
			{DriveID: new("writes"), PathOnHost: writes, IsReadOnly: new(false), IsRootDevice: new(false)},
		},
		NetworkInterfaces: []*models.NetworkInterface{},
		Vsock:             &models.Vsock{GuestCid: new(int64(vsock.Host + 1)), UdsPath: socket},
	}
	if *tap != "" {
		config.NetworkInterfaces = []*models.NetworkInterface{{
			IfaceID: new("network-spike"), HostDevName: tap, GuestMac: experiment.NetworkFixture().GuestMAC,
		}}
	}
	if err := config.Validate(strfmt.Default); err != nil {
		log.Fatal(err)
	}
	if err := json.NewEncoder(os.Stdout).Encode(config); err != nil {
		log.Fatal(err)
	}
}

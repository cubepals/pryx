// SPDX-License-Identifier: FSL-1.1-ALv2
// Command config emits launch configuration using the monitor owner's generated API models.
// It neither implements monitor API shapes nor enables production VM placement.
//
// Parts (`cmd/config/`): main.go assembles owner models; output.go reports metadata and
// identities; jail.go maps staged host inputs into the monitor mount namespace.
package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"log"
	"os"
	"strings"

	"cubepals/pryx/vm-experiment/experiment"
	"github.com/firecracker-microvm/firecracker-go-sdk/client/models"
	"github.com/go-openapi/strfmt"
	"github.com/mdlayher/vsock"
)

func main() {
	memory := flag.Int64("memory-mib", 256, "Guest physical memory in MiB")
	vcpus := flag.Int64("vcpus", 1, "Guest virtual CPU count")
	kernel := flag.String("kernel", "", "Verified published guest kernel")
	root := flag.String("root", "", "Read-only EROFS root")
	writes := flag.String("writes", "", "Writable ext4 disk")
	socket := flag.String("socket", "", "Private monitor vsock path")
	tap := flag.String("tap", "", "Optional private network namespace TAP for the comparison")
	metadata := flag.Bool("metadata", false, "Print experiment-owned transport metadata")
	jailRoot := flag.String("jail-root", "", "Optional host jail root for monitor paths")
	identities := flag.Bool("identities", false, "Print backing-file identities for the configured guest disks")
	initMode := flag.String("init-mode", "", "Optional mode supported by the actual trusted guest executable")
	var layers []*models.Drive
	flag.Func("layer", "Additional read-only OCI drive as ID=PATH", func(value string) error {
		parts := strings.SplitN(value, "=", 2)
		if len(parts) != 2 || parts[0] == "" || parts[1] == "" {
			return fmt.Errorf("Invalid read-only drive argument")
		}
		layers = append(layers, &models.Drive{DriveID: new(parts[0]), PathOnHost: new(parts[1]), IsReadOnly: new(true), IsRootDevice: new(false)})
		return nil
	})
	flag.Parse()
	if *metadata {
		printMetadata()
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
		MachineConfig: &models.MachineConfiguration{VcpuCount: vcpus, MemSizeMib: memory},
		Drives: []*models.Drive{
			{DriveID: new(experiment.RootDrive), PathOnHost: root, IsReadOnly: new(true), IsRootDevice: new(true)},
			{DriveID: new(experiment.WriteDrive), PathOnHost: writes, IsReadOnly: new(false), IsRootDevice: new(false)},
		},
		NetworkInterfaces: []*models.NetworkInterface{},
		Vsock:             &models.Vsock{GuestCid: new(int64(vsock.Host + 1)), UdsPath: socket},
	}
	config.Drives = append(config.Drives, layers...)
	if *initMode != "" {
		config.BootSource.BootArgs += " -- " + *initMode
		config.Drives[1].CacheType = new(models.DriveCacheTypeWriteback)
	}
	if *tap != "" {
		config.NetworkInterfaces = []*models.NetworkInterface{{
			IfaceID: new("network-spike"), HostDevName: tap, GuestMac: experiment.NetworkFixture().GuestMAC,
		}}
	}
	if err := config.Validate(strfmt.Default); err != nil {
		log.Fatal(err)
	}
	if *identities {
		printIdentities(config)
		return
	}
	if *jailRoot != "" {
		if err := jailPaths(&config, *jailRoot); err != nil {
			log.Fatal(err)
		}
	}
	if err := json.NewEncoder(os.Stdout).Encode(config); err != nil {
		log.Fatal(err)
	}
}

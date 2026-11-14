// SPDX-License-Identifier: FSL-1.1-ALv2
// Configurator output reports owner metadata and real backing-file identities.
// It does not rewrite monitor paths or assemble the machine configuration.
package main

import (
	"encoding/json"
	"fmt"
	"log"
	"os"
	"syscall"

	"cubepals/pryx/vm-experiment/experiment"
	"github.com/firecracker-microvm/firecracker-go-sdk/client/models"
	"github.com/mdlayher/vsock"
)

func printMetadata() {
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{
		"jailPaths":  true,
		"reportPort": experiment.ReportPort, "networkPort": experiment.NetworkPort,
		"rootDrive": experiment.RootDrive, "writeDrive": experiment.WriteDrive,
		"guestCid":    vsock.Host + 1,
		"network":     experiment.NetworkFixture(),
		"forwardPort": experiment.ForwardPort, "forwardPrefix": experiment.ForwardPrefix, "forwardSuffix": experiment.ForwardSuffix,
		"networkBulkBytes": experiment.NetworkBulkBytes, "networkHello": experiment.NetworkHello,
		"networkHelloPath": experiment.NetworkHelloPath, "networkBulkPath": experiment.NetworkBulkPath,
	}); err != nil {
		log.Fatal(err)
	}
}

func printIdentities(config models.FullVMConfiguration) {
	result := map[string]string{}
	for _, drive := range config.Drives {
		info, err := os.Stat(*drive.PathOnHost)
		if err != nil {
			log.Fatal(err)
		}
		stat, ok := info.Sys().(*syscall.Stat_t)
		if !ok {
			log.Fatal("Disk identity requires Linux file metadata")
		}
		// Firecracker 1.18.0 DiskProperties::build_device_id, in
		// src/vmm/src/devices/virtio/block/virtio/device.rs, has no exported API.
		// Send the full identity; guest sysfs supplies the actual virtio truncation.
		result[*drive.DriveID] = fmt.Sprintf("%d%d%d", stat.Dev, stat.Rdev, stat.Ino)
	}
	if err := json.NewEncoder(os.Stdout).Encode(result); err != nil {
		log.Fatal(err)
	}
}

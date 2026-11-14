// SPDX-License-Identifier: FSL-1.1-ALv2
// Map verified files staged below the jail into the monitor's mount namespace.
// The owner SDK retains every API shape; host identities are read before rewriting.
// This adapter does not create a jail, bind files, or admit an image-controlled path.
package main

import (
	"fmt"
	"path/filepath"
	"strings"

	"github.com/firecracker-microvm/firecracker-go-sdk/client/models"
)

func jailPaths(config *models.FullVMConfiguration, root string) error {
	if !filepath.IsAbs(root) || filepath.Clean(root) != root || root == string(filepath.Separator) {
		return fmt.Errorf("Jail root must be an absolute clean private directory")
	}
	inside := func(path string) (string, error) {
		if !filepath.IsAbs(path) || filepath.Clean(path) != path {
			return "", fmt.Errorf("Jail inputs require absolute clean host paths")
		}
		relative, err := filepath.Rel(root, path)
		if err != nil || relative == "." || relative == ".." || strings.HasPrefix(relative, ".."+string(filepath.Separator)) {
			return "", fmt.Errorf("Monitor input is outside its host jail")
		}
		return string(filepath.Separator) + relative, nil
	}
	kernel, err := inside(*config.BootSource.KernelImagePath)
	if err != nil {
		return err
	}
	socket, err := inside(*config.Vsock.UdsPath)
	if err != nil {
		return err
	}
	drives := make([]string, len(config.Drives))
	for i, drive := range config.Drives {
		drives[i], err = inside(*drive.PathOnHost)
		if err != nil {
			return err
		}
	}
	// Complete validation before changing even an in-memory owner model.
	config.BootSource.KernelImagePath = &kernel
	config.Vsock.UdsPath = &socket
	for i, drive := range config.Drives {
		drive.PathOnHost = &drives[i]
	}
	return nil
}

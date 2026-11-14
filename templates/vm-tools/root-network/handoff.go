// SPDX-License-Identifier: FSL-1.1-ALv2
// Hand socket and namespace ownership to the parent through Linux's maintained SCM_RIGHTS
// encoder. This grants existing descriptors, never paths, packet framing or arbitrary programs.
package rootnetwork

import (
	"fmt"
	"os"

	"golang.org/x/sys/unix"
)

// MaxHandoffBytes bounds this private application metadata, independently of file streams.
const MaxHandoffBytes = 64 * 1024
const MaxHandoffFiles = 8

func SendFiles(channel int, metadata []byte, files []*os.File) error {
	if len(metadata) == 0 || len(metadata) > MaxHandoffBytes || len(files) == 0 || len(files) > MaxHandoffFiles {
		return fmt.Errorf("Root listener handoff exceeds its private bounds")
	}
	domain, err := unix.GetsockoptInt(channel, unix.SOL_SOCKET, unix.SO_DOMAIN)
	if err != nil {
		return err
	}
	kind, err := unix.GetsockoptInt(channel, unix.SOL_SOCKET, unix.SO_TYPE)
	if err != nil {
		return err
	}
	if domain != unix.AF_UNIX || kind != unix.SOCK_DGRAM {
		return fmt.Errorf("Root listener handoff requires a private Unix datagram pair")
	}
	descriptors := make([]int, len(files))
	for index, file := range files {
		if file == nil {
			return fmt.Errorf("Root listener descriptor is missing")
		}
		descriptors[index] = int(file.Fd())
	}
	count, err := unix.SendmsgN(channel, metadata, unix.UnixRights(descriptors...), nil, 0)
	if err != nil {
		return err
	}
	if count != len(metadata) {
		return fmt.Errorf("Root listener metadata was not sent whole")
	}
	return nil
}

// SPDX-License-Identifier: FSL-1.1-ALv2
// An unprivileged fixture checks the real inherited guest syscall filter. It is uploaded
// through the ordinary file service, never privileged or included in production bootstrap.
package main

import (
	"encoding/json"
	"fmt"
	"os"

	"golang.org/x/sys/unix"
)

func guestPolicy() error {
	if os.Geteuid() == 0 {
		return fmt.Errorf("Guest policy proof requires an unprivileged workload")
	}
	fd, err := unix.Socket(unix.AF_VSOCK, unix.SOCK_STREAM|unix.SOCK_CLOEXEC, 0)
	if err == nil {
		unix.Close(fd)
	}
	if err != unix.EPERM {
		return fmt.Errorf("Guest vsock socket denial: %v", err)
	}
	pair, err := unix.Socketpair(unix.AF_VSOCK, unix.SOCK_STREAM|unix.SOCK_CLOEXEC, 0)
	if err == nil {
		unix.Close(pair[0])
		unix.Close(pair[1])
	}
	if err != unix.EPERM {
		return fmt.Errorf("Guest vsock socketpair denial: %v", err)
	}
	_, _, result := unix.Syscall(unix.SYS_IO_URING_SETUP, 1, 0, 0)
	if result != unix.EPERM {
		return fmt.Errorf("Guest io_uring setup denial: %v", result)
	}
	return json.NewEncoder(os.Stdout).Encode(map[string]bool{
		"vsockSocketDenied": true, "vsockSocketpairDenied": true, "ioUringSetupDenied": true,
	})
}

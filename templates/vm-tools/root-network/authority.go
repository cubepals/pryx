// SPDX-License-Identifier: FSL-1.1-ALv2
// Validate protected root-owned programs and directories before a root scope uses them.
// These checks confer no network authority and do not start, enter or mutate a namespace.
package rootnetwork

import (
	"fmt"
	"os"
	"path/filepath"
	"syscall"

	"golang.org/x/sys/unix"
)

func Ancestry(path string) error {
	if !filepath.IsAbs(path) || filepath.Clean(path) != path {
		return fmt.Errorf("Scope paths require canonical absolute host directories")
	}
	for {
		info, err := os.Lstat(path)
		if err != nil {
			return err
		}
		owner, ok := info.Sys().(*syscall.Stat_t)
		if !ok || !info.IsDir() || owner.Uid != 0 || (info.Mode().Perm()&0022 != 0 && info.Mode()&os.ModeSticky == 0) {
			return fmt.Errorf("Scope ancestry requires protected root-owned directories")
		}
		parent := filepath.Dir(path)
		if parent == path {
			return nil
		}
		path = parent
	}
}

func Executable(path string) error {
	descriptor, err := unix.Open(path, unix.O_RDONLY|unix.O_CLOEXEC|unix.O_NOFOLLOW|unix.O_NONBLOCK, 0)
	if err != nil {
		return err
	}
	defer unix.Close(descriptor)
	var info unix.Stat_t
	if err := unix.Fstat(descriptor, &info); err != nil {
		return err
	}
	if info.Mode&unix.S_IFMT != unix.S_IFREG || info.Uid != 0 || info.Mode&0022 != 0 || info.Mode&0111 == 0 || info.Nlink != 1 {
		return fmt.Errorf("Root CNI executable must be one protected root-owned regular program")
	}
	return nil
}

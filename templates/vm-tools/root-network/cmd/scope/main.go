// SPDX-License-Identifier: FSL-1.1-ALv2
// Grant one owned TAP namespace and its kernel gateway sockets to the actual root parent.
// No VM is launched here. EOF retires this scope only after the monitor's kernel lease clears;
// no host route, global forwarding state, external address allocator or firewall is changed.
package main

import (
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"time"

	rootnetwork "cubepals/pryx/vm-root-network-experiment"
	"golang.org/x/sys/unix"
)

func lease(path string) (*os.File, error) {
	if err := rootnetwork.Ancestry(filepath.Dir(path)); err != nil {
		return nil, err
	}
	descriptor, err := unix.Open(path, unix.O_RDONLY|unix.O_CREAT|unix.O_CLOEXEC|unix.O_NOFOLLOW|unix.O_NONBLOCK, 0600)
	if err != nil {
		return nil, err
	}
	file := os.NewFile(uintptr(descriptor), "owned monitor lease")
	var info unix.Stat_t
	if err := unix.Fstat(descriptor, &info); err != nil {
		file.Close()
		return nil, err
	}
	if info.Mode&unix.S_IFMT != unix.S_IFREG || info.Uid != 0 || info.Nlink != 1 || info.Mode&0077 != 0 {
		file.Close()
		return nil, fmt.Errorf("Scope lease must be one private root-owned regular file")
	}
	return file, nil
}

func grant(directory, state, lock string, uid, gid uint) (problem error) {
	if os.Geteuid() != 0 || uid == 0 || gid == 0 || uint64(uid) >= uint64(^uint32(0)) || uint64(gid) >= uint64(^uint32(0)) {
		return fmt.Errorf("Root scope requires root and fixed ordinary monitor identities")
	}
	if err := rootnetwork.Ancestry(directory); err != nil {
		return err
	}
	if err := rootnetwork.Executable(filepath.Join(directory, "tap")); err != nil {
		return err
	}
	if err := rootnetwork.Ancestry(state); err != nil {
		return err
	}
	held, err := lease(lock)
	if err != nil {
		return err
	}
	defer held.Close()
	cache, err := os.MkdirTemp(state, "root-network-")
	if err != nil {
		return err
	}
	retired := false
	defer func() {
		if retired {
			problem = errors.Join(problem, os.RemoveAll(cache))
		}
	}()
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	scope, err := rootnetwork.Open(ctx, directory, cache, "vm0", uint32(uid), uint32(gid))
	if err != nil {
		return err
	}
	defer func() {
		// A parent disappearing is not authority to detach a live monitor's interface.
		// Linux flock accepts an exclusive lifetime lock on this readonly descriptor.
		if err := unix.Flock(int(held.Fd()), unix.LOCK_EX); err != nil {
			problem = errors.Join(problem, err)
			return
		}
		retirement, stop := context.WithTimeout(context.Background(), 30*time.Second)
		defer stop()
		cleanup := scope.Close(retirement)
		retired = cleanup == nil
		problem = errors.Join(problem, cleanup)
	}()
	files, err := scope.Files(rootnetwork.Ports{})
	if err != nil {
		return err
	}
	defer func() {
		for _, file := range files {
			file.Close()
		}
	}()
	metadata, err := json.Marshal(scope.Grant())
	if err != nil {
		return err
	}
	if err := rootnetwork.SendFiles(int(os.Stdout.Fd()), metadata, files); err != nil {
		return err
	}
	var command [1]byte
	count, err := os.Stdin.Read(command[:])
	if count != 0 || !errors.Is(err, io.EOF) {
		return fmt.Errorf("Scope lifetime channel requires parent EOF: %v", err)
	}
	return nil
}

func main() {
	metadata := flag.Bool("metadata", false, "Read parser support without touching host state")
	directory := flag.String("plugins", "", "Protected pinned CNI executable directory")
	state := flag.String("state", "", "Private host-owned boot directory")
	lock := flag.String("lease", "", "Private actual monitor lifetime lease")
	uid := flag.Uint("uid", 0, "Host-selected ordinary monitor UID")
	gid := flag.Uint("gid", 0, "Host-selected ordinary monitor GID")
	flag.Parse()
	var err error
	if *metadata {
		err = json.NewEncoder(os.Stdout).Encode(rootnetwork.ScopeAdvertisement())
	} else {
		err = grant(*directory, *state, *lock, *uid, *gid)
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

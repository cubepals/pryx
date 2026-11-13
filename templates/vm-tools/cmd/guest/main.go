// SPDX-License-Identifier: FSL-1.1-ALv2
// Command guest is the kernel spike's static PID 1. It proves the published kernel's
// filesystems, virtio devices and real guest-to-host vsock, then reboots on acknowledgement.
// It is not the production agent, an image runtime, a network stack or a confinement claim.
package main

import (
	"bufio"
	"encoding/json"
	"fmt"
	"os"
	"strconv"
	"strings"
	"time"

	"cubepals/pryx/vm-experiment/experiment"
	"github.com/mdlayher/vsock"
	"golang.org/x/sys/unix"
)

func must(err error) {
	if err != nil {
		panic(err)
	}
}

func filesystem(path string, expected int64) {
	var stat unix.Statfs_t
	must(unix.Statfs(path, &stat))
	if stat.Type != expected {
		panic(fmt.Sprintf("Unexpected filesystem at %s: %x", path, stat.Type))
	}
}

func probe() map[string]any {
	if os.Getpid() != 1 {
		panic("The guest experiment may run only as PID 1 inside the VM")
	}
	must(unix.Mount("devtmpfs", "/dev", "devtmpfs", 0, ""))
	must(unix.Mount("proc", "/proc", "proc", 0, ""))
	must(unix.Mount("sysfs", "/sys", "sysfs", 0, ""))
	filesystem("/", unix.EROFS_SUPER_MAGIC_V1)
	if err := os.WriteFile("/baseline", []byte("changed"), 0o600); err == nil {
		panic("EROFS accepted a write")
	}
	must(unix.Mount("/dev/vdb", "/write", "ext4", 0, ""))
	filesystem("/write", unix.EXT4_SUPER_MAGIC)
	for _, directory := range []string{"/write/upper", "/write/work"} {
		must(os.MkdirAll(directory, 0o700))
	}
	must(unix.Mount("/", "/lower", "", unix.MS_BIND, ""))
	must(unix.Mount("overlay", "/merged", "overlay", 0, "lowerdir=/lower,upperdir=/write/upper,workdir=/write/work"))
	filesystem("/merged", unix.OVERLAYFS_SUPER_MAGIC)
	must(os.WriteFile("/merged/baseline", []byte("overlay-write"), 0o600))
	original, err := os.ReadFile("/baseline")
	must(err)
	if string(original) != "immutable-base\n" {
		panic("Overlay changed its EROFS lower")
	}
	upper, err := os.ReadFile("/write/upper/baseline")
	must(err)
	if string(upper) != "overlay-write" {
		panic("Overlay write did not reach the ext4 upper")
	}
	count := 0
	if data, err := os.ReadFile("/write/boots"); err == nil {
		count, err = strconv.Atoi(string(data))
		must(err)
	} else if !os.IsNotExist(err) {
		panic(err)
	}
	count++
	must(os.WriteFile("/write/boots", []byte(strconv.Itoa(count)), 0o600))
	unix.Sync()
	release, err := os.ReadFile("/proc/sys/kernel/osrelease")
	must(err)
	filesystems, err := os.ReadFile("/proc/filesystems")
	must(err)
	return map[string]any{"kernel": strings.TrimSpace(string(release)), "boots": count,
		"erofs": true, "overlay": true, "ext4": true, "virtioBlock": true,
		"rootUnchanged": true, "filesystems": string(filesystems), "pid": os.Getpid()}
}

func main() {
	defer func() {
		if failure := recover(); failure != nil {
			fmt.Printf("PRYX_KERNEL_SPIKE_FAILURE: %v\n", failure)
			time.Sleep(time.Second)
		}
		unix.Sync()
		_ = unix.Reboot(unix.LINUX_REBOOT_CMD_RESTART)
	}()
	report := probe()
	connection, err := vsock.Dial(vsock.Host, experiment.ReportPort, nil)
	must(err)
	defer connection.Close()
	must(connection.SetDeadline(time.Now().Add(30 * time.Second)))
	report["vsock"] = true
	must(json.NewEncoder(connection).Encode(report))
	ack, err := bufio.NewReader(connection).ReadString('\n')
	must(err)
	if ack != "stop\n" {
		panic("Unexpected host acknowledgement")
	}
}

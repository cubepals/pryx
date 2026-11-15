// SPDX-License-Identifier: FSL-1.1-ALv2
// Own fixed ordinary packet programs and their lifetime beneath the root bridge authority.
// Protected root-owned copies stay outside the child's writable work directory. The caller
// writes configuration through a granted descriptor; no privileged path write follows it.
// This starts only the fixed agent after clearing groups/UID/GID, never a requested program.
package rootnetwork

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"syscall"

	"golang.org/x/sys/unix"
)

type PacketPaths struct {
	Launcher      string `json:"launcher"`
	Program       string `json:"program"`
	Directory     string `json:"directory"`
	Configuration string `json:"configuration"`
	Lease         string `json:"lease"`
	Parent        int    `json:"parent"`
}

type PacketProcess struct {
	Paths         PacketPaths
	root          string
	configuration *os.File
	pidfd         *os.File
	command       *exec.Cmd
	done          chan struct{}
	result        error
	mode          string
	uid, gid      uint32
	expected      map[string]any
	attempted     bool
	retired       bool
}

func copyPacketProgram(source, destination string) error {
	if err := Ancestry(filepath.Dir(source)); err != nil {
		return err
	}
	if err := Executable(source); err != nil {
		return err
	}
	descriptor, err := unix.Open(source, unix.O_RDONLY|unix.O_CLOEXEC|unix.O_NOFOLLOW, 0)
	if err != nil {
		return err
	}
	input := os.NewFile(uintptr(descriptor), "protected packet program")
	defer input.Close()
	output, err := os.OpenFile(destination, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0555)
	if err != nil {
		return err
	}
	_, copied := io.Copy(output, input)
	return errors.Join(copied, output.Close())
}

func (bridge *Bridge) PreparePacket(uid, gid uint32) (*PacketProcess, error) {
	if os.Geteuid() != 0 || uid == 0 || gid == 0 || uid == ^uint32(0) || gid == ^uint32(0) || !bridge.confirmed || !bridge.inputs.packetTap || bridge.inputs.monitorMode == "" || bridge.inputs.agent == "" || bridge.inputs.packet == "" {
		return nil, fmt.Errorf("Packet process requires the fixed secured bridge inputs")
	}
	root, err := os.MkdirTemp("", "pryx-root-packet-")
	if err != nil {
		return nil, err
	}
	packet := &PacketProcess{root: root, expected: bridge.inputs.packetMetadata, mode: bridge.inputs.monitorMode, uid: uid, gid: gid}
	success := false
	defer func() {
		if !success {
			os.RemoveAll(root)
		}
	}()
	packet.Paths = PacketPaths{Launcher: filepath.Join(root, "agent"), Program: filepath.Join(root, "proxy"),
		Directory: filepath.Join(root, "work"), Configuration: filepath.Join(root, "work", "network.json"),
		Lease: filepath.Join(root, "locks", "packet.lock"), Parent: os.Getpid()}
	for _, program := range []struct{ source, target string }{{bridge.inputs.agent, packet.Paths.Launcher}, {bridge.inputs.packet, packet.Paths.Program}} {
		if err := copyPacketProgram(program.source, program.target); err != nil {
			return nil, err
		}
	}
	for _, directory := range []string{packet.Paths.Directory, filepath.Dir(packet.Paths.Lease)} {
		if err := os.Mkdir(directory, 0700); err != nil {
			return nil, err
		}
		if err := os.Chown(directory, int(uid), int(gid)); err != nil {
			return nil, err
		}
	}
	packet.configuration, err = os.OpenFile(packet.Paths.Configuration, os.O_RDWR|os.O_CREATE|os.O_EXCL, 0600)
	if err != nil {
		return nil, err
	}
	if err := packet.configuration.Chown(int(uid), int(gid)); err != nil {
		packet.configuration.Close()
		return nil, err
	}
	if err := os.Chmod(root, 0755); err != nil {
		packet.configuration.Close()
		return nil, err
	}
	success = true
	return packet, nil
}

func (packet *PacketProcess) Profile() map[string]any { return packet.expected }

func (packet *PacketProcess) Configuration() (*os.File, error) {
	descriptor, err := unix.FcntlInt(packet.configuration.Fd(), unix.F_DUPFD_CLOEXEC, 0)
	if err != nil {
		return nil, err
	}
	return os.NewFile(uintptr(descriptor), "owned packet configuration"), nil
}

func (packet *PacketProcess) Start(ctx context.Context, plan []byte, queue *os.File) error {
	if packet.attempted || len(plan) == 0 || len(plan) > MaxHandoffBytes {
		return fmt.Errorf("Packet launch is repeated or exceeds its bound")
	}
	packet.attempted = true
	command := exec.Command(packet.Paths.Launcher, packet.mode)
	command.Env = []string{}
	command.Stdin = bytes.NewReader(plan)
	command.Stderr = queue
	pidfd := -1
	command.SysProcAttr = &syscall.SysProcAttr{Credential: &syscall.Credential{Uid: packet.uid, Gid: packet.gid, Groups: []uint32{}}, Pdeathsig: syscall.SIGKILL, PidFD: &pidfd}
	output, err := command.StdoutPipe()
	if err != nil {
		return err
	}
	if err := command.Start(); err != nil {
		return err
	}
	packet.command = command
	packet.done = make(chan struct{})
	go func() { packet.result = command.Wait(); close(packet.done) }()
	if pidfd < 0 {
		return fmt.Errorf("Host did not grant a kernel packet-process identity handle")
	}
	packet.pidfd = os.NewFile(uintptr(pidfd), "owned packet process identity")
	ready := make(chan error, 1)
	go func() {
		data, err := io.ReadAll(io.LimitReader(output, MaxHandoffBytes+1))
		if err != nil {
			ready <- err
			return
		}
		var actual map[string]any
		if len(data) > MaxHandoffBytes {
			ready <- fmt.Errorf("Packet readiness exceeds its private bound")
			return
		}
		if err := json.Unmarshal(data, &actual); err != nil {
			ready <- err
			return
		}
		if !reflect.DeepEqual(actual, packet.expected) {
			ready <- fmt.Errorf("Packet readiness differs from its actual executable")
			return
		}
		ready <- nil
	}()
	select {
	case err := <-ready:
		return err
	case <-ctx.Done():
		return ctx.Err()
	}
}

func (packet *PacketProcess) Exited() <-chan struct{} { return packet.done }

func (packet *PacketProcess) Identity() (*os.File, error) {
	if packet.pidfd == nil {
		return nil, fmt.Errorf("Packet process has no admitted kernel identity")
	}
	descriptor, err := unix.FcntlInt(packet.pidfd.Fd(), unix.F_DUPFD_CLOEXEC, 0)
	if err != nil {
		return nil, err
	}
	return os.NewFile(uintptr(descriptor), "owned packet process identity"), nil
}

// Stop is called after the actual monitor lease clears. Files survive unconfirmed exit.
func (packet *PacketProcess) Stop() error {
	if packet.retired {
		return fmt.Errorf("Packet retirement was already submitted")
	}
	packet.retired = true
	if packet.command != nil {
		select {
		case <-packet.done:
		default:
			if err := packet.command.Process.Kill(); err != nil && !errors.Is(err, os.ErrProcessDone) {
				return err
			}
			<-packet.done
		}
	}
	if packet.command != nil && packet.command.ProcessState == nil {
		return fmt.Errorf("Packet exit remains unconfirmed: %v", packet.result)
	}
	if packet.pidfd != nil {
		packet.pidfd.Close()
	}
	return errors.Join(packet.configuration.Close(), os.RemoveAll(packet.root))
}

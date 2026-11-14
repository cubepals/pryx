// SPDX-License-Identifier: FSL-1.1-ALv2
// Actual SCM_RIGHTS retains a listener after its producer closes every original descriptor.
// This ordinary-user case proves ownership transfer, not namespace or root packet policy.
package rootnetwork

import (
	"bytes"
	"crypto/rand"
	"io"
	"net"
	"os"
	"testing"
	"time"

	"golang.org/x/sys/unix"
)

func TestActualListenerSurvivesProducerClose(t *testing.T) {
	channel, err := unix.Socketpair(unix.AF_UNIX, unix.SOCK_DGRAM|unix.SOCK_CLOEXEC, 0)
	if err != nil {
		t.Fatal(err)
	}
	defer unix.Close(channel[0])
	defer unix.Close(channel[1])
	listener, err := net.ListenTCP("tcp4", &net.TCPAddr{IP: net.IPv4(127, 0, 0, 1)})
	if err != nil {
		t.Fatal(err)
	}
	address := listener.Addr().String()
	file, err := listener.File()
	if err != nil {
		t.Fatal(err)
	}
	listener.Close()
	challenge := make([]byte, 32)
	if _, err := rand.Read(challenge); err != nil {
		t.Fatal(err)
	}
	if err := SendFiles(channel[0], challenge, []*os.File{file}); err != nil {
		t.Fatal(err)
	}
	file.Close()
	metadata := make([]byte, MaxHandoffBytes)
	ancillary := make([]byte, len(unix.UnixRights(0)))
	count, controls, flags, _, err := unix.Recvmsg(channel[1], metadata, ancillary, unix.MSG_CMSG_CLOEXEC)
	if err != nil {
		t.Fatal(err)
	}
	if flags&(unix.MSG_TRUNC|unix.MSG_CTRUNC) != 0 || !bytes.Equal(metadata[:count], challenge) {
		t.Fatal("Actual handoff was truncated or changed")
	}
	messages, err := unix.ParseSocketControlMessage(ancillary[:controls])
	if err != nil || len(messages) != 1 {
		t.Fatalf("Actual control messages: %v %v", messages, err)
	}
	descriptors, err := unix.ParseUnixRights(&messages[0])
	if err != nil || len(descriptors) != 1 {
		t.Fatalf("Actual descriptor grant: %v %v", descriptors, err)
	}
	granted := os.NewFile(uintptr(descriptors[0]), "received listener")
	defer granted.Close()
	attributes, err := unix.FcntlInt(granted.Fd(), unix.F_GETFD, 0)
	if err != nil || attributes&unix.FD_CLOEXEC == 0 {
		t.Fatal("Received authority must not leak through exec")
	}
	received, err := net.FileListener(granted)
	if err != nil {
		t.Fatal(err)
	}
	defer received.Close()
	deadline := time.Now().Add(2 * time.Second)
	if err := received.(*net.TCPListener).SetDeadline(deadline); err != nil {
		t.Fatal(err)
	}
	done := make(chan error, 1)
	go func() {
		connection, err := received.Accept()
		if err != nil {
			done <- err
			return
		}
		defer connection.Close()
		connection.SetDeadline(deadline)
		_, err = connection.Write(challenge)
		done <- err
	}()
	client, err := net.DialTimeout("tcp4", address, time.Until(deadline))
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()
	client.SetDeadline(deadline)
	reply, err := io.ReadAll(client)
	if err != nil || !bytes.Equal(reply, challenge) {
		t.Fatalf("Transferred listener bytes: %v", err)
	}
	if err := <-done; err != nil {
		t.Fatal(err)
	}
}

// SPDX-License-Identifier: FSL-1.1-ALv2
// Actual namespace-bound listeners are served in the original parent namespace. This
// proves the kernel ownership seam, not a guest boot or production hostname/firewall policy.
package main

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"os/exec"
	"syscall"
	"time"

	rootnetwork "cubepals/pryx/vm-root-network-experiment"
	"github.com/containernetworking/plugins/pkg/ns"
	"github.com/vishvananda/netns"
	"golang.org/x/sys/unix"
)

func gatewayClient(address, proof string) error {
	if os.Geteuid() == 0 {
		return fmt.Errorf("Gateway client must use the ordinary monitor identity")
	}
	client := &http.Client{Timeout: 2 * time.Second, Transport: &http.Transport{Proxy: nil}}
	response, err := client.Get("http://" + address + "/")
	if err != nil {
		return err
	}
	defer response.Body.Close()
	data, err := io.ReadAll(io.LimitReader(response.Body, int64(len(proof)+1)))
	if err != nil || response.StatusCode != http.StatusOK || string(data) != proof {
		return fmt.Errorf("Namespace listener did not return its live parent proof: %v", err)
	}
	return nil
}

func received(channel int) ([]*os.File, error) {
	payload := make([]byte, rootnetwork.MaxHandoffBytes)
	ancillary := make([]byte, len(unix.UnixRights(0, 0, 0, 0, 0)))
	count, controls, flags, _, err := unix.Recvmsg(channel, payload, ancillary, unix.MSG_CMSG_CLOEXEC)
	if err != nil {
		return nil, err
	}
	if flags&(unix.MSG_TRUNC|unix.MSG_CTRUNC) != 0 || string(payload[:count]) != "root-gateway-descriptors" {
		return nil, fmt.Errorf("Root gateway handoff changed or truncated")
	}
	messages, err := unix.ParseSocketControlMessage(ancillary[:controls])
	if err != nil || len(messages) != 1 {
		return nil, fmt.Errorf("Root gateway control message is invalid: %v", err)
	}
	descriptors, err := unix.ParseUnixRights(&messages[0])
	if err != nil {
		return nil, err
	}
	files := make([]*os.File, len(descriptors))
	for index, descriptor := range descriptors {
		files[index] = os.NewFile(uintptr(descriptor), "actual root gateway")
	}
	if len(files) != 5 {
		for _, file := range files {
			file.Close()
		}
		return nil, fmt.Errorf("Root gateway descriptor count differs")
	}
	return files, nil
}

func gatewayProbe(directory string, uid, gid uint32) error {
	before, err := links()
	if err != nil {
		return err
	}
	original, err := netns.Get()
	if err != nil {
		return err
	}
	defer original.Close()
	work, err := os.MkdirTemp("", "pryx-root-gateways-")
	if err != nil {
		return err
	}
	defer os.RemoveAll(work)
	var nonce [16]byte
	if _, err := rand.Read(nonce[:]); err != nil {
		return err
	}
	name := "pye" + hex.EncodeToString(nonce[:])[:unix.IFNAMSIZ-4]
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	scope, err := rootnetwork.Open(ctx, directory, work, name, uid, gid)
	if err != nil {
		return err
	}
	attempted := false
	defer func() {
		if !attempted {
			_ = scope.Close(ctx)
		}
	}()
	var ports rootnetwork.Ports
	for _, entry := range []struct {
		service string
		target  *uint16
	}{{"http", &ports.HTTP}, {"https", &ports.TLS}, {"domain", &ports.DNS}} {
		port, err := net.LookupPort("tcp", entry.service)
		if err != nil {
			return err
		}
		*entry.target = uint16(port)
	}
	files, err := scope.Files(ports)
	if err != nil {
		return err
	}
	defer func() {
		for _, file := range files {
			file.Close()
		}
	}()
	pair, err := unix.Socketpair(unix.AF_UNIX, unix.SOCK_DGRAM|unix.SOCK_CLOEXEC, 0)
	if err != nil {
		return err
	}
	defer unix.Close(pair[0])
	defer unix.Close(pair[1])
	if err := rootnetwork.SendFiles(pair[0], []byte("root-gateway-descriptors"), files); err != nil {
		return err
	}
	granted, err := received(pair[1])
	if err != nil {
		return err
	}
	defer func() {
		for _, file := range granted {
			file.Close()
		}
	}()
	listener, err := net.FileListener(granted[1])
	if err != nil {
		return err
	}
	proof := hex.EncodeToString(nonce[:])
	server := &http.Server{ReadHeaderTimeout: 2 * time.Second, Handler: http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		current, err := netns.Get()
		if err != nil {
			http.Error(w, "Namespace observation failed", http.StatusInternalServerError)
			return
		}
		defer current.Close()
		if current.UniqueId() != original.UniqueId() {
			http.Error(w, "Gateway left the parent namespace", http.StatusInternalServerError)
			return
		}
		fmt.Fprint(w, proof)
	})}
	defer server.Close()
	go func() { _ = server.Serve(listener) }()
	executable, err := os.Executable()
	if err != nil {
		return err
	}
	err = scope.Space.Do(func(ns.NetNS) error {
		child := exec.CommandContext(ctx, executable, "--gateway-client", net.JoinHostPort(scope.Profile.Host, fmt.Sprint(ports.HTTP)), "--proof", proof)
		child.SysProcAttr = &syscall.SysProcAttr{Credential: &syscall.Credential{Uid: uid, Gid: gid, Groups: []uint32{}}}
		if output, err := child.CombinedOutput(); err != nil {
			observed, _ := json.Marshal(rootnetwork.Credentials())
			return fmt.Errorf("Ordinary namespace client: %w: %s; actual parent authority: %s", err, output, observed)
		}
		return nil
	})
	if err != nil {
		return err
	}
	if err := server.Close(); err != nil {
		return err
	}
	for _, file := range granted {
		file.Close()
	}
	for _, file := range files {
		file.Close()
	}
	attempted = true
	if err := scope.Close(ctx); err != nil {
		return err
	}
	after, err := links()
	if err != nil {
		return err
	}
	if len(before) != len(after) {
		return fmt.Errorf("Root gateway scope changed a host interface")
	}
	for name, index := range before {
		if after[name] != index {
			return fmt.Errorf("Host neighbour identity changed")
		}
	}
	return json.NewEncoder(os.Stdout).Encode(map[string]bool{"namespaceListenerReached": true, "handlerInOriginalNamespace": true, "tapIdentityDeleted": true, "neighboursUnchanged": true})
}

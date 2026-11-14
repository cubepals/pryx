// SPDX-License-Identifier: FSL-1.1-ALv2
// Bind hostname-gateway sockets inside the owned TAP namespace. The caller receives actual
// descriptors and serves them in its original namespace; this implements no forwarding protocol.
package rootnetwork

import (
	"fmt"
	"net"
	"os"

	"github.com/containernetworking/plugins/pkg/ns"
	"golang.org/x/sys/unix"
)

type Ports struct{ HTTP, TLS, DNS uint16 }

// Files returns owned namespace, HTTP, TLS, DNS-TCP and DNS-UDP descriptors, in that order.
func (scope *Scope) Files(ports Ports) ([]*os.File, error) {
	if ports.HTTP == 0 || ports.TLS == 0 || ports.DNS == 0 || ports.HTTP == ports.TLS || ports.HTTP == ports.DNS || ports.TLS == ports.DNS {
		return nil, fmt.Errorf("Root gateways require three distinct admitted nonzero ports")
	}
	descriptor, err := unix.FcntlInt(uintptr(scope.handle), unix.F_DUPFD_CLOEXEC, 0)
	if err != nil {
		return nil, err
	}
	files := []*os.File{os.NewFile(uintptr(descriptor), "owned root namespace")}
	err = scope.Space.Do(func(ns.NetNS) error {
		address := net.ParseIP(scope.Profile.Host)
		for _, port := range []uint16{ports.HTTP, ports.TLS, ports.DNS} {
			listener, err := net.ListenTCP("tcp4", &net.TCPAddr{IP: address, Port: int(port)})
			if err != nil {
				return err
			}
			file, err := listener.File()
			listener.Close()
			if err != nil {
				return err
			}
			files = append(files, file)
		}
		socket, err := net.ListenUDP("udp4", &net.UDPAddr{IP: address, Port: int(ports.DNS)})
		if err != nil {
			return err
		}
		file, err := socket.File()
		socket.Close()
		if err != nil {
			return err
		}
		files = append(files, file)
		return nil
	})
	if err != nil {
		for _, file := range files {
			file.Close()
		}
		return nil, err
	}
	return files, nil
}

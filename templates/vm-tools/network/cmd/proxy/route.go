// SPDX-License-Identifier: FSL-1.1-ALv2
// Ask the kernel how this host would route a packet to a destination (rtnetlink's RTM_GETROUTE,
// the lookup `ip route get` makes), so admission can refuse an address the host itself would
// receive, however it came to be local, at the moment a flow asks. The request and its answer
// are vishvananda/netlink's own message types. That library binds each socket before it sends,
// and the launcher forbids this helper bind (it has no listener of any kind), so the request goes
// out on an unbound route socket, which the kernel binds itself on its first send (netlink(7)).
// This file neither decides admission (admission.go) nor dials.
package main

import (
	"errors"
	"net/netip"
	"syscall"
	"time"

	"github.com/vishvananda/netlink/nl"
	"golang.org/x/sys/unix"
)

// routeAnswerWait bounds one kernel answer; a lookup without one refuses its flow.
const routeAnswerWait = 2 * time.Second

// hostRouteType is the kernel's route type for a packet this process would send to address, as
// its dial would be routed: unix.RTN_LOCAL when this host would receive it itself.
func hostRouteType(address netip.Addr) (uint8, error) {
	if !address.Is4() {
		return 0, errors.New("Packet route lookups are IPv4 only")
	}
	fd, err := unix.Socket(unix.AF_NETLINK, unix.SOCK_RAW|unix.SOCK_CLOEXEC, unix.NETLINK_ROUTE)
	if err != nil {
		return 0, err
	}
	defer unix.Close(fd)
	wait := unix.NsecToTimeval(routeAnswerWait.Nanoseconds())
	if err := unix.SetsockoptTimeval(fd, unix.SOL_SOCKET, unix.SO_RCVTIMEO, &wait); err != nil {
		return 0, err
	}
	destination := address.As4()
	request := nl.NewNetlinkRequest(unix.RTM_GETROUTE, 0)
	request.AddData(&nl.RtMsg{RtMsg: unix.RtMsg{Family: unix.AF_INET, Dst_len: uint8(address.BitLen()),
		Flags: unix.RTM_F_LOOKUP_TABLE}})
	request.AddData(nl.NewRtAttr(unix.RTA_DST, destination[:]))
	if err := unix.Sendto(fd, request.Serialize(), 0, &unix.SockaddrNetlink{Family: unix.AF_NETLINK}); err != nil {
		return 0, err
	}
	bound, err := unix.Getsockname(fd)
	if err != nil {
		return 0, err
	}
	own, ok := bound.(*unix.SockaddrNetlink)
	if !ok {
		return 0, errors.New("Packet route socket has no netlink address")
	}
	buffer := make([]byte, nl.RECEIVE_BUFFER_SIZE)
	for {
		count, from, err := unix.Recvfrom(fd, buffer, 0)
		if err != nil {
			return 0, err
		}
		if sender, ok := from.(*unix.SockaddrNetlink); !ok || sender.Pid != nl.PidKernel {
			continue
		}
		messages, err := syscall.ParseNetlinkMessage(buffer[:count])
		if err != nil {
			return 0, err
		}
		for _, message := range messages {
			if message.Header.Seq != request.Seq || message.Header.Pid != own.Pid {
				continue
			}
			switch message.Header.Type {
			case unix.NLMSG_ERROR:
				if len(message.Data) < 4 {
					return 0, errors.New("Packet route answer is truncated")
				}
				// netlink(7): struct nlmsgerr begins with the negative errno, 0 being an acknowledgement.
				if errno := int32(nl.NativeEndian().Uint32(message.Data[:4])); errno != 0 {
					return 0, syscall.Errno(-errno)
				}
				return 0, errors.New("Packet route lookup was acknowledged without a route")
			case unix.RTM_NEWROUTE:
				if len(message.Data) < unix.SizeofRtMsg {
					return 0, errors.New("Packet route answer is truncated")
				}
				return nl.DeserializeRtMsg(message.Data).Type, nil
			}
		}
	}
}

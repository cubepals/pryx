// SPDX-License-Identifier: FSL-1.1-ALv2
// Observe actual host credential authority when the root fixture cannot spawn its ordinary
// monitor. This reads no environment, process arguments, private records or guest credentials.
package rootnetwork

import (
	"golang.org/x/sys/unix"
	"os"
)

// Linux UAPI linux/capability.h defines two words for version 3; x/sys exports no word-count constant.
const capabilityWords = 2

func Credentials() map[string]any {
	header := unix.CapUserHeader{Version: unix.LINUX_CAPABILITY_VERSION_3}
	data := [capabilityWords]unix.CapUserData{}
	problem := unix.Capget(&header, &data[0])
	nonew, error := unix.PrctlRetInt(unix.PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0)
	userns, nsError := os.Readlink("/proc/thread-self/ns/user")
	return map[string]any{"uid": os.Getuid(), "euid": os.Geteuid(), "gid": os.Getgid(), "egid": os.Getegid(),
		"setuid":       data[0].Effective&(uint32(1)<<unix.CAP_SETUID) != 0,
		"setgid":       data[0].Effective&(uint32(1)<<unix.CAP_SETGID) != 0,
		"capgetFailed": problem != nil, "noNewPrivileges": nonew, "privilegeObservationFailed": error != nil,
		"userNamespace": userns, "namespaceObservationFailed": nsError != nil}
}

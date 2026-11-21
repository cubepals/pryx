// SPDX-License-Identifier: FSL-1.1-ALv2
/* Read both stock storage values and the kernel cookie for the fixed fixture's four live UDP sockets. */
#define _GNU_SOURCE
#include <errno.h>
#include <inttypes.h>
#include <linux/bpf.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/socket.h>
#include <sys/syscall.h>
#include <unistd.h>
#include "network.h"

static int lookup(int map, int fd, void *out)
{
	union bpf_attr attr = {};
	attr.map_fd = map;
	attr.key = (uintptr_t)&fd;
	attr.value = (uintptr_t)out;
	return syscall(SYS_bpf, BPF_MAP_LOOKUP_ELEM, &attr, sizeof(attr));
}

int main(int argc, char **argv)
{
	if (argc != 7)
		return 2;
	for (int i = 3; i < argc; i++) {
		int fd = atoi(argv[i]);
		int type = 0;
		socklen_t type_len = sizeof(type);
		uint64_t cookie = 0;
		socklen_t cookie_len = sizeof(cookie);
		uint8_t role = 0;
		struct pryx_socket state = {};
		if (getsockopt(fd, SOL_SOCKET, SO_TYPE, &type, &type_len) || type != SOCK_DGRAM ||
		    getsockopt(fd, SOL_SOCKET, SO_COOKIE, &cookie, &cookie_len) || !cookie ||
		    cookie_len != sizeof(cookie) || lookup(atoi(argv[1]), fd, &role) ||
		    lookup(atoi(argv[2]), fd, &state)) {
			perror("read private UDP socket state");
			return 1;
		}
		printf("%" PRIu64 " %u ", cookie, role);
		for (size_t n = 0; n < sizeof(state); n++)
			printf("%02x", ((uint8_t *)&state)[n]);
		putchar('\n');
	}
	return 0;
}

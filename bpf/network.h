// SPDX-License-Identifier: GPL-2.0-only OR BSD-2-Clause
/*
 * The maps a process sandbox's cgroup programs share with pryxd (src/bpf/object.rs mirrors each
 * struct). pryxd decides everything a rule says; the programs only look it up and apply it.
 */
#ifndef PRYX_NETWORK_H
#define PRYX_NETWORK_H

#include <linux/types.h>

/* Port ranges one prefix carries, its own and every wider prefix's, first match wins. */
#define PRYX_PORT_ENTRIES 24
#define PRYX_RULES 1024

/* What a destination gets. */
enum pryx_verdict {
	PRYX_DENY = 0,
	PRYX_ALLOW = 1,
	/* A host address: allowed only to a socket this sandbox's map knows. */
	PRYX_OWNED = 2,
	/* Rewritten to this sandbox's gateway of that kind, allowed only if pryxd's listener is there. */
	PRYX_GATEWAY_HTTP = 3,
	PRYX_GATEWAY_TLS = 4,
	PRYX_GATEWAY_DNS = 5,
	/* An explicitly configured resolver, on its DNS port and protocol only. */
	PRYX_RESOLVER = 6,
};

#define PRYX_TCP 1
#define PRYX_UDP 2

/* The loopback device's index in every network namespace: LOOPBACK_IFINDEX, include/net/flow.h. */
#define PRYX_LOOPBACK 1

/* Our bounded walk covers ordinary Hop-by-Hop, Routing and Destination option chains. */
#define PRYX_IPV6_OPTIONS 8
/* RFC 8200 section 4: these extension headers count 8-octet units after their first unit. */
#define PRYX_IPV6_OPTION_BYTES 8

struct pryx_ports {
	__u16 low;
	__u16 high;
	__u8 protocols;
	__u8 verdict;
	__u16 reserved;
};

struct pryx_rule {
	__u8 fallback;
	__u8 count;
	__u8 reserved[6];
	struct pryx_ports ports[PRYX_PORT_ENTRIES];
};

/* An IPv6 prefix; IPv4 is written as ::ffff:a.b.c.d. */
struct pryx_key {
	__u32 prefixlen;
	__u8 address[16];
};

/*
 * The ports pryxd published for a sandbox, the only ones reached from off this host: a request's
 * own (MAX_PORTS, 8), its agent's and its spares (pryx_agent's MAX_SPARE_PORTS, 32);
 * src/bpf/object.rs holds this to both.
 */
#define PRYX_PUBLISHED_PORTS 41

/*
 * The gateway's IPv4 address and ports, in network order, and the published ports, in host
 * order.
 */
struct pryx_config {
	__u32 gateway;
	__u16 http;
	__u16 tls;
	__u16 dns;
	__u16 published_count;
	__u16 published[PRYX_PUBLISHED_PORTS];
	__u16 reserved;
};

enum pryx_role {
	PRYX_ROLE_NONE = 0,
	/* Created inside the sandbox's cgroup. */
	PRYX_ROLE_SANDBOX = 1,
	/* A listener pryxd serves for this sandbox. */
	PRYX_ROLE_GATEWAY = 2,
};

struct pryx_socket {
	/* Ownership lives separately in the cloneable owners map. */
	__u8 reserved_role;
	__u8 redirected;
	/* The destination a DNS datagram was redirected from, in network order. */
	__u16 port;
	__u32 address[4];
	/* A TCP socket that dialled a destination off this host: what arrives on it is its answer. */
	__u8 dialled;
	__u8 reserved[3];
};

/* The datagram peers a sandbox's UDP sockets were allowed to send to, the most recent kept. */
#define PRYX_PEERS 4096

/*
 * One UDP socket's peer: the socket's cookie, and the peer's address (IPv4 as ::ffff:a.b.c.d)
 * and port, both in network order. What arrives from off this host for a UDP socket must come
 * from one of its own.
 */
struct pryx_peer {
	__u64 cookie;
	__u32 address[4];
	__u16 port;
	__u8 reserved[6];
};

enum pryx_counter_index {
	PRYX_INGRESS = 0,
	PRYX_EGRESS = 1,
	PRYX_REFUSED_ADDRESSES = 2,
	PRYX_REFUSED_SOCKETS = 3,
	PRYX_REFUSED_INGRESS = 4,
	PRYX_COUNTERS = 5,
};

struct pryx_counter {
	__u64 packets;
	__u64 bytes;
};

#endif

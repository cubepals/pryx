// SPDX-License-Identifier: GPL-2.0-only OR BSD-2-Clause
/*
 * A process sandbox's network policy, attached to its own cgroup. Each program has one job:
 * the address hooks allow, refuse or redirect one destination by the rule pryxd wrote for it;
 * the receive hooks put back the address a redirected DNS answer came from; the socket hook
 * refuses what the address hooks don't police and marks the sockets the sandbox made; the
 * ingress hook takes from off this host only the answers to what the sandbox dialled and the
 * ports pryxd published for it; the packet hooks count and check current host-local routes.
 * Only IP and transport headers are read; pryxd decides every policy rule.
 */
#include <linux/bpf.h>
#include <linux/if_ether.h>
#include <linux/in.h>
#include <linux/ip.h>
#include <linux/ipv6.h>
#include <linux/udp.h>
#include <linux/tcp.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_endian.h>

#include "network.h"

char LICENSE[] SEC("license") = "Dual BSD/GPL";

struct {
	__uint(type, BPF_MAP_TYPE_LPM_TRIE);
	__uint(max_entries, PRYX_RULES);
	__uint(map_flags, BPF_F_NO_PREALLOC);
	__type(key, struct pryx_key);
	__type(value, struct pryx_rule);
} rules SEC(".maps");

struct {
	__uint(type, BPF_MAP_TYPE_ARRAY);
	__uint(max_entries, 1);
	__type(key, __u32);
	__type(value, struct pryx_config);
} config SEC(".maps");

struct {
	__uint(type, BPF_MAP_TYPE_SK_STORAGE);
	/* Linux BPF_F_CLONE copies listener ownership to each accepted destination socket. */
	__uint(map_flags, BPF_F_NO_PREALLOC | BPF_F_CLONE);
	__type(key, int);
	__type(value, __u8);
} owners SEC(".maps");

struct {
	__uint(type, BPF_MAP_TYPE_SK_STORAGE);
	/* Direction and redirected DNS state must never be inherited by an accepted child. */
	__uint(map_flags, BPF_F_NO_PREALLOC);
	__type(key, int);
	__type(value, struct pryx_socket);
} sockets SEC(".maps");

struct {
	__uint(type, BPF_MAP_TYPE_LRU_HASH);
	__uint(max_entries, PRYX_PEERS);
	__type(key, struct pryx_peer);
	__type(value, __u8);
} peers SEC(".maps");

struct {
	__uint(type, BPF_MAP_TYPE_PERCPU_ARRAY);
	__uint(max_entries, PRYX_COUNTERS);
	__type(key, __u32);
	__type(value, struct pryx_counter);
} counters SEC(".maps");

static __always_inline void count(__u32 index, __u64 bytes)
{
	struct pryx_counter *counter = bpf_map_lookup_elem(&counters, &index);

	if (counter) {
		counter->packets += 1;
		counter->bytes += bytes;
	}
}

static __always_inline int refuse(void)
{
	count(PRYX_REFUSED_ADDRESSES, 0);
	return 0;
}

static __always_inline __u8 classify(struct pryx_key *key, __u16 port, __u32 protocol)
{
	struct pryx_rule *rule = bpf_map_lookup_elem(&rules, key);
	__u8 bit = protocol == IPPROTO_TCP ? PRYX_TCP : PRYX_UDP;

	/* pryxd always writes ::/0, so a miss means the policy isn't there yet. */
	if (!rule)
		return PRYX_DENY;
	/*
	 * Unrolled, and each range read whole before it is tested, so every read is at a constant
	 * offset and only scalars meet where the passes end: without CAP_PERFMON the verifier refuses
	 * a map pointer moved by a different offset on each path (Spectre v1 hardening).
	 */
#pragma clang loop unroll(full)
	for (int i = 0; i < PRYX_PORT_ENTRIES; i++) {
		const volatile struct pryx_ports *ports = &rule->ports[i];
		__u16 low = ports->low, high = ports->high;
		__u8 protocols = ports->protocols, verdict = ports->verdict;

		if (i >= rule->count)
			break;
		if ((protocols & bit) && port >= low && port <= high)
			return verdict;
	}
	return rule->fallback;
}

/* The destination socket's role in this sandbox's map, or none for another owner. */
static __always_inline __u8 socket_role(void *ctx, struct bpf_sock_tuple *tuple, __u32 size,
                                      __u32 protocol)
{
	struct bpf_sock *sk;
	__u8 *found;
	__u8 role;

	if (protocol == IPPROTO_TCP)
		sk = bpf_sk_lookup_tcp(ctx, tuple, size, BPF_F_CURRENT_NETNS, 0);
	else
		sk = bpf_sk_lookup_udp(ctx, tuple, size, BPF_F_CURRENT_NETNS, 0);
	if (!sk)
		return PRYX_ROLE_NONE;
	found = bpf_sk_storage_get(&owners, sk, 0, 0);
	role = found ? *found : PRYX_ROLE_NONE;
	bpf_sk_release(sk);
	return role;
}

/* Whether the socket listening at an address is one this sandbox's map holds in `role`. */
static __always_inline int held(void *ctx, struct bpf_sock_tuple *tuple, __u32 size,
                               __u32 protocol, __u8 role)
{
	__u8 found = socket_role(ctx, tuple, size, protocol);

	return role == PRYX_ROLE_NONE ? found != PRYX_ROLE_NONE : found == role;
}

static __always_inline int held4(struct bpf_sock_addr *ctx, __u32 address, __u16 port, __u32 protocol,
				 __u8 role)
{
	struct bpf_sock_tuple tuple = {};

	tuple.ipv4.daddr = address;
	tuple.ipv4.dport = port;
	return held(ctx, &tuple, sizeof(tuple.ipv4), protocol, role);
}

static __always_inline __u16 gateway_port(struct pryx_config *gateway, __u8 verdict)
{
	if (verdict == PRYX_GATEWAY_HTTP)
		return gateway->http;
	if (verdict == PRYX_GATEWAY_TLS)
		return gateway->tls;
	return gateway->dns;
}

/*
 * A destination the sandbox may reach takes what comes back from it. A TCP socket hears only its
 * connection's peer, so the socket is marked; a UDP socket hears anyone who sends to its port,
 * so the peer is remembered for that socket alone.
 */
static __always_inline int dialled(struct bpf_sock_addr *ctx, __u32 address[4], __u16 port)
{
	struct pryx_peer peer = {};
	struct pryx_socket *own;
	__u8 seen = 1;

	if (ctx->protocol == IPPROTO_UDP) {
		peer.cookie = bpf_get_socket_cookie(ctx);
		__builtin_memcpy(peer.address, address, sizeof(peer.address));
		peer.port = port;
		return bpf_map_update_elem(&peers, &peer, &seen, BPF_ANY) == 0;
	}
	/* Accepted sockets do not clone storage. A later disconnect/reconnect must become an
	 * initiated connection too, rather than retaining the accepted-response exemption. */
	own = bpf_sk_storage_get(&sockets, ctx->sk, 0, BPF_SK_STORAGE_GET_F_CREATE);
	if (!own)
		return 0;
	own->dialled = 1;
	return 1;
}

/* The DNS server a redirected datagram was meant for, so its answer can say it came from there. */
static __always_inline void remember(struct bpf_sock_addr *ctx, __u32 address[4], __u16 port)
{
	struct pryx_socket *own = bpf_sk_storage_get(&sockets, ctx->sk, 0, 0);

	if (!own)
		return;
	own->redirected = 1;
	own->port = port;
	own->address[0] = address[0];
	own->address[1] = address[1];
	own->address[2] = address[2];
	own->address[3] = address[3];
}

static __always_inline int mapped(__u32 address[4])
{
	return address[0] == 0 && address[1] == 0 && address[2] == bpf_htonl(0xffff);
}

static __always_inline void key_of(struct pryx_key *key, __u32 address[4])
{
	key->prefixlen = 128;
	__builtin_memcpy(key->address, address, sizeof(key->address));
}

/* One destination of an IPv4 socket: connect, or a datagram's address. */
static __always_inline int decide4(struct bpf_sock_addr *ctx)
{
	__u32 address[4] = { 0, 0, bpf_htonl(0xffff), ctx->user_ip4 };
	__u16 port = (__u16)ctx->user_port;
	__u32 protocol = ctx->protocol;
	struct pryx_config *gateway;
	struct pryx_key key;
	__u32 zero = 0;
	__u16 redirect;
	__u8 verdict;

	if (protocol != IPPROTO_TCP && protocol != IPPROTO_UDP)
		return refuse();
	key_of(&key, address);
	verdict = classify(&key, bpf_ntohs(port), protocol);
	if (verdict == PRYX_ALLOW || verdict == PRYX_RESOLVER)
		return dialled(ctx, address, port) ? 1 : refuse();
	if (verdict == PRYX_OWNED)
		return held4(ctx, ctx->user_ip4, port, protocol, PRYX_ROLE_NONE) ? 1 : refuse();
	if (verdict < PRYX_GATEWAY_HTTP || verdict > PRYX_GATEWAY_DNS)
		return refuse();
	gateway = bpf_map_lookup_elem(&config, &zero);
	if (!gateway)
		return refuse();
	redirect = gateway_port(gateway, verdict);
	if (!redirect || !held4(ctx, gateway->gateway, redirect, protocol, PRYX_ROLE_GATEWAY))
		return refuse();
	if (verdict == PRYX_GATEWAY_DNS)
		remember(ctx, address, port);
	ctx->user_ip4 = gateway->gateway;
	ctx->user_port = redirect;
	return 1;
}

/* One destination of an IPv6 socket; an IPv4-mapped address takes IPv4's rules and lookups. */
static __always_inline int decide6(struct bpf_sock_addr *ctx)
{
	__u32 address[4] = { ctx->user_ip6[0], ctx->user_ip6[1], ctx->user_ip6[2], ctx->user_ip6[3] };
	__u16 port = (__u16)ctx->user_port;
	__u32 protocol = ctx->protocol;
	struct bpf_sock_tuple tuple = {};
	struct pryx_config *gateway;
	struct pryx_key key;
	__u32 zero = 0;
	__u16 redirect;
	__u8 verdict;

	if (protocol != IPPROTO_TCP && protocol != IPPROTO_UDP)
		return refuse();
	key_of(&key, address);
	verdict = classify(&key, bpf_ntohs(port), protocol);
	if (verdict == PRYX_ALLOW || verdict == PRYX_RESOLVER)
		return dialled(ctx, address, port) ? 1 : refuse();
	if (verdict == PRYX_OWNED) {
		if (mapped(address))
			return held4(ctx, address[3], port, protocol, PRYX_ROLE_NONE) ? 1 : refuse();
		__builtin_memcpy(tuple.ipv6.daddr, address, sizeof(tuple.ipv6.daddr));
		tuple.ipv6.dport = port;
		return held(ctx, &tuple, sizeof(tuple.ipv6), protocol, PRYX_ROLE_NONE) ? 1 : refuse();
	}
	if (verdict < PRYX_GATEWAY_HTTP || verdict > PRYX_GATEWAY_DNS)
		return refuse();
	gateway = bpf_map_lookup_elem(&config, &zero);
	if (!gateway)
		return refuse();
	redirect = gateway_port(gateway, verdict);
	if (!redirect || !held4(ctx, gateway->gateway, redirect, protocol, PRYX_ROLE_GATEWAY))
		return refuse();
	if (verdict == PRYX_GATEWAY_DNS)
		remember(ctx, address, port);
	/* The gateway listens on IPv4; a dual-stack socket reaches it at its mapped address. */
	ctx->user_ip6[0] = 0;
	ctx->user_ip6[1] = 0;
	ctx->user_ip6[2] = bpf_htonl(0xffff);
	ctx->user_ip6[3] = gateway->gateway;
	ctx->user_port = redirect;
	return 1;
}

SEC("cgroup/connect4")
int connect4(struct bpf_sock_addr *ctx)
{
	return decide4(ctx);
}

SEC("cgroup/connect6")
int connect6(struct bpf_sock_addr *ctx)
{
	return decide6(ctx);
}

SEC("cgroup/sendmsg4")
int sendmsg4(struct bpf_sock_addr *ctx)
{
	return decide4(ctx);
}

SEC("cgroup/sendmsg6")
int sendmsg6(struct bpf_sock_addr *ctx)
{
	return decide6(ctx);
}

/* An answer from the DNS gateway names the server the query was sent to. */
static __always_inline struct pryx_socket *answered(struct bpf_sock_addr *ctx, __u32 address, __u16 port)
{
	struct pryx_config *gateway;
	struct pryx_socket *own;
	__u32 zero = 0;

	gateway = bpf_map_lookup_elem(&config, &zero);
	if (!gateway || !gateway->dns || address != gateway->gateway || port != gateway->dns)
		return 0;
	own = bpf_sk_storage_get(&sockets, ctx->sk, 0, 0);
	if (!own || !own->redirected)
		return 0;
	return own;
}

SEC("cgroup/recvmsg4")
int recvmsg4(struct bpf_sock_addr *ctx)
{
	struct pryx_socket *own = answered(ctx, ctx->user_ip4, (__u16)ctx->user_port);

	if (own && mapped(own->address)) {
		ctx->user_ip4 = own->address[3];
		ctx->user_port = own->port;
	}
	return 1;
}

SEC("cgroup/recvmsg6")
int recvmsg6(struct bpf_sock_addr *ctx)
{
	struct pryx_socket *own;

	if (ctx->user_ip6[0] != 0 || ctx->user_ip6[1] != 0 || ctx->user_ip6[2] != bpf_htonl(0xffff))
		return 1;
	own = answered(ctx, ctx->user_ip6[3], (__u16)ctx->user_port);
	if (own) {
		ctx->user_ip6[0] = own->address[0];
		ctx->user_ip6[1] = own->address[1];
		ctx->user_ip6[2] = own->address[2];
		ctx->user_ip6[3] = own->address[3];
		ctx->user_port = own->port;
	}
	return 1;
}

/* Only TCP and UDP pass, the protocols the address hooks police; each is marked as the sandbox's. */
SEC("cgroup/sock_create")
int sock_create(struct bpf_sock *sk)
{
	struct pryx_socket *own;
	__u8 *owner;

	if (sk->protocol != IPPROTO_TCP && sk->protocol != IPPROTO_UDP) {
		count(PRYX_REFUSED_SOCKETS, 0);
		return 0;
	}
	owner = bpf_sk_storage_get(&owners, sk, 0, BPF_SK_STORAGE_GET_F_CREATE);
	own = bpf_sk_storage_get(&sockets, sk, 0, BPF_SK_STORAGE_GET_F_CREATE);
	if (!owner || !own) {
		count(PRYX_REFUSED_SOCKETS, 0);
		return 0;
	}
	*owner = PRYX_ROLE_SANDBOX;
	return 1;
}

/* Whether pryxd published this port for the sandbox: its agent's, which the edge dials. */
static __always_inline int published(__u32 port)
{
	struct pryx_config *settings;
	__u32 zero = 0;

	settings = bpf_map_lookup_elem(&config, &zero);
	if (!settings)
		return 0;
	/* Unrolled, each read at a constant offset, as classify's. */
#pragma clang loop unroll(full)
	for (int i = 0; i < PRYX_PUBLISHED_PORTS; i++) {
		const volatile __u16 *slot = &settings->published[i];
		__u16 value = *slot;

		if (i >= settings->published_count)
			break;
		if (value == port)
			return 1;
	}
	return 0;
}

/*
 * Whether a datagram comes from a peer its UDP socket was allowed to send to, read from the
 * packet's own IP and UDP headers. An IPv6 datagram behind extension headers isn't read further,
 * so it is no answer.
 */
static __always_inline int answers(struct __sk_buff *skb)
{
	struct pryx_peer peer = {};
	__u32 header;

	peer.cookie = bpf_get_socket_cookie(skb);
	if (skb->protocol == bpf_htons(ETH_P_IP)) {
		struct iphdr ip;

		if (bpf_skb_load_bytes(skb, 0, &ip, sizeof(ip)))
			return 0;
		peer.address[2] = bpf_htonl(0xffff);
		peer.address[3] = ip.saddr;
		header = ip.ihl * 4;
	} else if (skb->protocol == bpf_htons(ETH_P_IPV6)) {
		struct ipv6hdr ip;

		if (bpf_skb_load_bytes(skb, 0, &ip, sizeof(ip)) || ip.nexthdr != IPPROTO_UDP)
			return 0;
		__builtin_memcpy(peer.address, &ip.saddr, sizeof(peer.address));
		header = sizeof(ip);
	} else {
		return 0;
	}
	if (bpf_skb_load_bytes(skb, header + __builtin_offsetof(struct udphdr, source), &peer.port, sizeof(peer.port)))
		return 0;
	return bpf_map_lookup_elem(&peers, &peer) != 0;
}

/*
 * From this host (loopback carries everything local) anything reaches the sandbox's sockets, as
 * the address hooks already decided; from off it, only what answers a destination the sandbox
 * dialled (on TCP its connection, on UDP a peer that socket sent to), and TCP to a port pryxd
 * published. Everything else is dropped, as a container's undeclared port is out of reach.
 */
SEC("cgroup_skb/ingress")
int ingress(struct __sk_buff *skb)
{
	struct bpf_sock *sk = skb->sk;
	struct pryx_socket *own;
	struct bpf_sock *full;

	count(PRYX_INGRESS, skb->len);
	if (skb->ingress_ifindex == PRYX_LOOPBACK)
		return 1;
	if (!sk)
		goto drop;
	full = bpf_sk_fullsock(sk);
	/* A socket that isn't full (a request or time-wait one) is TCP's, on its listener's port. */
	if ((!full || full->protocol == IPPROTO_TCP) && published(sk->src_port))
		return 1;
	if (!full)
		goto drop;
	if (full->protocol == IPPROTO_UDP) {
		if (answers(skb))
			return 1;
		goto drop;
	}
	own = bpf_sk_storage_get(&sockets, full, 0, 0);
	/* A socket that dialled and then listened (after a failed connect) still has no peer. */
	if (own && own->dialled && full->state != BPF_TCP_LISTEN)
		return 1;
drop:
	count(PRYX_REFUSED_INGRESS, skb->len);
	return 0;
}

/* A packet's actual four-tuple, after the kernel has selected its current route. */
struct packet {
	struct bpf_sock_tuple tuple;
	struct pryx_key key;
	__u32 protocol;
	__u16 port;
};

static __always_inline int packet_ports(struct __sk_buff *skb, struct packet *packet,
                                      __u32 offset)
{
	__u16 source;

	if (packet->protocol == IPPROTO_TCP) {
		struct tcphdr tcp;

		if (bpf_skb_load_bytes(skb, offset, &tcp, sizeof(tcp)))
			return 0;
		source = tcp.source;
		packet->port = tcp.dest;
	} else if (packet->protocol == IPPROTO_UDP) {
		struct udphdr udp;

		if (bpf_skb_load_bytes(skb, offset, &udp, sizeof(udp)))
			return 0;
		source = udp.source;
		packet->port = udp.dest;
	} else {
		return 0;
	}
	if (skb->protocol == bpf_htons(ETH_P_IP)) {
		packet->tuple.ipv4.sport = source;
		packet->tuple.ipv4.dport = packet->port;
	} else {
		packet->tuple.ipv6.sport = source;
		packet->tuple.ipv6.dport = packet->port;
	}
	return 1;
}

static __always_inline int packet_tuple(struct __sk_buff *skb, struct packet *packet)
{
	__u32 offset;

	packet->key.prefixlen = 128;
	if (skb->protocol == bpf_htons(ETH_P_IP)) {
		struct iphdr ip;
		__u32 address[4];

		if (bpf_skb_load_bytes(skb, 0, &ip, sizeof(ip)) || ip.ihl * sizeof(__u32) < sizeof(ip))
			return 0;
		packet->tuple.ipv4.saddr = ip.saddr;
		packet->tuple.ipv4.daddr = ip.daddr;
		address[0] = 0;
		address[1] = 0;
		address[2] = bpf_htonl(0xffff);
		address[3] = ip.daddr;
		__builtin_memcpy(packet->key.address, address, sizeof(address));
		packet->protocol = ip.protocol;
		offset = ip.ihl * sizeof(__u32);
	} else if (skb->protocol == bpf_htons(ETH_P_IPV6)) {
		struct ipv6hdr ip;

		if (bpf_skb_load_bytes(skb, 0, &ip, sizeof(ip)))
			return 0;
		__builtin_memcpy(packet->tuple.ipv6.saddr, &ip.saddr, sizeof(ip.saddr));
		__builtin_memcpy(packet->tuple.ipv6.daddr, &ip.daddr, sizeof(ip.daddr));
		__builtin_memcpy(packet->key.address, &ip.daddr, sizeof(ip.daddr));
		packet->protocol = ip.nexthdr;
		offset = sizeof(ip);
		/* Egress runs before normal IPv6 fragmentation. Ordinary extension headers are
		 * bounded here; malformed, fragment, encrypted and excess chains fail closed. */
#pragma clang loop unroll(full)
		for (int i = 0; i < PRYX_IPV6_OPTIONS; i++) {
			struct ipv6_opt_hdr option;

			if (packet->protocol == IPPROTO_TCP || packet->protocol == IPPROTO_UDP)
				break;
			if (packet->protocol != IPPROTO_HOPOPTS && packet->protocol != IPPROTO_ROUTING &&
			    packet->protocol != IPPROTO_DSTOPTS)
				return 0;
			if (bpf_skb_load_bytes(skb, offset, &option, sizeof(option)))
				return 0;
			offset += ((__u32)option.hdrlen + 1) * PRYX_IPV6_OPTION_BYTES;
			if (offset > skb->len)
				return 0;
			packet->protocol = option.nexthdr;
		}
	} else {
		return 0;
	}
	return packet_ports(skb, packet, offset);
}

/* Existing accepted connections and listeners may answer their peer. Every initiated TCP
 * connect creates and marks storage, even when reusing a previously accepted descriptor. */
static __always_inline int tcp_response(struct __sk_buff *skb)
{
	struct bpf_sock *full;
	struct pryx_socket *own;

	if (!skb->sk)
		return 0;
	full = bpf_sk_fullsock(skb->sk);
	/* Only TCP has request/time-wait sockets; they carry the kernel's replies, not a new dial. */
	if (!full)
		return 1;
	if (full->protocol != IPPROTO_TCP)
		return 0;
	if (full->state == BPF_TCP_LISTEN)
		return 1;
	own = bpf_sk_storage_get(&sockets, full, 0, 0);
	return !own || !own->dialled;
}

static __always_inline __u8 packet_role(struct __sk_buff *skb, struct packet *packet)
{
	/* Each helper receives a constant tuple size, as the verifier requires. The complete
	 * source tuple makes wildcard/reuse-port selection the packet's actual destination. */
	if (skb->protocol == bpf_htons(ETH_P_IP))
		return socket_role(skb, &packet->tuple, sizeof(packet->tuple.ipv4), packet->protocol);
	return socket_role(skb, &packet->tuple, sizeof(packet->tuple.ipv6), packet->protocol);
}

static __always_inline int local_egress(struct __sk_buff *skb)
{
	struct packet packet = {};
	__u8 role, verdict;

	if (!packet_tuple(skb, &packet))
		return 0;
	if (packet.protocol == IPPROTO_TCP && tcp_response(skb))
		return 1;
	role = packet_role(skb, &packet);
	/* A rewritten gateway is an internal transport, not the requested upstream address. */
	if (role == PRYX_ROLE_GATEWAY)
		return 1;
	verdict = classify(&packet.key, bpf_ntohs(packet.port), packet.protocol);
	if (verdict == PRYX_RESOLVER)
		return 1;
	if (verdict == PRYX_DENY)
		return 0;
	return role == PRYX_ROLE_SANDBOX;
}

SEC("cgroup_skb/egress")
int egress(struct __sk_buff *skb)
{
	count(PRYX_EGRESS, skb->len);
	/* ip_finish_output/ip6_finish_output set skb->dev from the current kernel route before
	 * this hook. Every host-local destination uses LOOPBACK_IFINDEX, including a newly
	 * assigned address the userspace address watcher has not processed yet. */
	if (skb->ifindex == PRYX_LOOPBACK && !local_egress(skb))
		return refuse();
#ifdef PRYX_REJECTED
	/*
	 * bpf/rejected.bpf.o only: a verdict no cgroup_skb program may return, so the verifier refuses
	 * the last program of the set and a test sees the set refused whole.
	 */
	return 4;
#else
	return 1;
#endif
}

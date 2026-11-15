// SPDX-License-Identifier: GPL-2.0-only OR BSD-2-Clause
/*
 * A process sandbox's network policy, attached to its own cgroup. Each program has one job:
 * the address hooks allow, refuse or redirect one destination by the rule pryxd wrote for it;
 * the receive hooks put back the address a redirected DNS answer came from; the socket hook
 * refuses what the address hooks don't police and marks the sockets the sandbox made; the
 * packet hooks count. Nothing here parses data the sandbox sent; pryxd decides every rule.
 */
#include <linux/bpf.h>
#include <linux/in.h>
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
	__uint(map_flags, BPF_F_NO_PREALLOC);
	__type(key, int);
	__type(value, struct pryx_socket);
} sockets SEC(".maps");

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

/* Whether the socket listening at an address is one this sandbox's map holds in `role`. */
static __always_inline int held(struct bpf_sock_addr *ctx, struct bpf_sock_tuple *tuple, __u32 size,
				__u32 protocol, __u8 role)
{
	struct bpf_sock *sk;
	struct pryx_socket *found;
	int held;

	if (protocol == IPPROTO_TCP)
		sk = bpf_sk_lookup_tcp(ctx, tuple, size, BPF_F_CURRENT_NETNS, 0);
	else
		sk = bpf_sk_lookup_udp(ctx, tuple, size, BPF_F_CURRENT_NETNS, 0);
	if (!sk)
		return 0;
	found = bpf_sk_storage_get(&sockets, sk, 0, 0);
	held = found && (role == PRYX_ROLE_NONE ? found->role != PRYX_ROLE_NONE : found->role == role);
	bpf_sk_release(sk);
	return held;
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
	if (verdict == PRYX_ALLOW)
		return 1;
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
	if (verdict == PRYX_ALLOW)
		return 1;
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

	if (sk->protocol != IPPROTO_TCP && sk->protocol != IPPROTO_UDP) {
		count(PRYX_REFUSED_SOCKETS, 0);
		return 0;
	}
	own = bpf_sk_storage_get(&sockets, sk, 0, BPF_SK_STORAGE_GET_F_CREATE);
	if (!own) {
		count(PRYX_REFUSED_SOCKETS, 0);
		return 0;
	}
	own->role = PRYX_ROLE_SANDBOX;
	return 1;
}

SEC("cgroup_skb/ingress")
int ingress(struct __sk_buff *skb)
{
	count(PRYX_INGRESS, skb->len);
	return 1;
}

SEC("cgroup_skb/egress")
int egress(struct __sk_buff *skb)
{
	count(PRYX_EGRESS, skb->len);
	return 1;
}

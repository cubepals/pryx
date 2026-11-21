// SPDX-License-Identifier: FSL-1.1-ALv2
/* Print the native syscall number, public struct offsets and the nine stock attach types. */
#include <linux/bpf.h>
#include <stddef.h>
#include <stdio.h>
#include <sys/syscall.h>

#define FIELD(name, member) printf(name " %zu\n", offsetof(union bpf_attr, member))
#define VALUE(name, value) printf(name " %ld\n", (long)(value))

int main(void)
{
    VALUE("syscall", SYS_bpf);
    VALUE("attr_size", sizeof(union bpf_attr));
    VALUE("object_get", BPF_OBJ_GET);
    VALUE("test_run", BPF_PROG_TEST_RUN);
    VALUE("link_create", BPF_LINK_CREATE);
    VALUE("enable_stats", BPF_ENABLE_STATS);
    VALUE("runtime_stats", BPF_STATS_RUN_TIME);
    FIELD("pathname", pathname);
    FIELD("test_program", test.prog_fd);
    FIELD("link_program", link_create.prog_fd);
    FIELD("link_target", link_create.target_fd);
    FIELD("link_type", link_create.attach_type);
    FIELD("stats_type", enable_stats.type);
    VALUE("sock_create", BPF_CGROUP_INET_SOCK_CREATE);
    VALUE("connect4", BPF_CGROUP_INET4_CONNECT);
    VALUE("connect6", BPF_CGROUP_INET6_CONNECT);
    VALUE("sendmsg4", BPF_CGROUP_UDP4_SENDMSG);
    VALUE("sendmsg6", BPF_CGROUP_UDP6_SENDMSG);
    VALUE("recvmsg4", BPF_CGROUP_UDP4_RECVMSG);
    VALUE("recvmsg6", BPF_CGROUP_UDP6_RECVMSG);
    VALUE("ingress", BPF_CGROUP_INET_INGRESS);
    VALUE("egress", BPF_CGROUP_INET_EGRESS);
    return 0;
}

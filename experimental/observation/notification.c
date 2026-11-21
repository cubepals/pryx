/* SPDX-License-Identifier: FSL-1.1-ALv2 */
/* One disposable child's marked getpid events. The listener can only acknowledge the
 * call after verifying its identity. Kernel execution supplies the actual PID. No target
 * PID, policy expression, process-memory operation or return value is accepted as input. */
#define _GNU_SOURCE
#include "observation.h"
#include <errno.h>
#include <linux/filter.h>
#include <linux/seccomp.h>
#include <poll.h>
#include <stddef.h>
#include <stdio.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/prctl.h>
#include <sys/socket.h>
#include <sys/syscall.h>
#include <unistd.h>

static int transfer(int channel, int fd) {
    union { struct cmsghdr align; char bytes[CMSG_SPACE(sizeof(int))]; } ancillary = {0};
    char marker = 'L';
    struct iovec data = { .iov_base = &marker, .iov_len = 1 };
    struct msghdr msg = {.msg_iov = &data, .msg_iovlen = 1,
                         .msg_control = ancillary.bytes, .msg_controllen = sizeof(ancillary.bytes)};
    if (fd >= 0) {
        struct cmsghdr *header = CMSG_FIRSTHDR(&msg);
        header->cmsg_level = SOL_SOCKET;
        header->cmsg_type = SCM_RIGHTS;
        header->cmsg_len = CMSG_LEN(sizeof(fd));
        memcpy(CMSG_DATA(header), &fd, sizeof(fd));
        return sendmsg(channel, &msg, MSG_NOSIGNAL) == 1 ? 0 : -1;
    }
    if (recvmsg(channel, &msg, MSG_CMSG_CLOEXEC) != 1 || marker != 'L' || msg.msg_flags & MSG_CTRUNC) return -1;
    struct cmsghdr *header = CMSG_FIRSTHDR(&msg);
    if (!header || header->cmsg_level != SOL_SOCKET || header->cmsg_type != SCM_RIGHTS ||
        header->cmsg_len != CMSG_LEN(sizeof(fd))) return -1;
    memcpy(&fd, CMSG_DATA(header), sizeof(fd));
    return fd;
}

int notify_install(int channel) {
    /* The runner architectures are little endian. Leave all other calls to the existing floor. */
    struct sock_filter code[] = {
        BPF_STMT(BPF_LD | BPF_W | BPF_ABS, offsetof(struct seccomp_data, nr)),
        BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, SYS_getpid, 0, 3),
        BPF_STMT(BPF_LD | BPF_W | BPF_ABS, offsetof(struct seccomp_data, args[0])),
        BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, FIXTURE_MARK, 0, 1),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_USER_NOTIF),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW)
    };
    struct sock_fprog prog = {.len = sizeof(code)/sizeof(code[0]), .filter = code};
    if (prctl(PR_SET_NO_NEW_PRIVS, 1L, 0L, 0L, 0L) < 0) return -1;
    int listener = syscall(SYS_seccomp, SECCOMP_SET_MODE_FILTER, SECCOMP_FILTER_FLAG_NEW_LISTENER, &prog);
    if (listener < 0) return -1;
    int result = transfer(channel, listener);
    close(listener);
    return result;
}

int notify_observe(pid_t child, int channel) {
    int listener = transfer(channel, -1);
    if (listener < 0) return -1;
    int observed = 0;
    while (observed < WARM_CALLS + TIMED_CALLS) {
        struct pollfd events = {.fd = listener, .events = POLLIN};
        if (poll(&events, 1, 30000) <= 0 || !(events.revents & POLLIN)) break;
        struct seccomp_notif event = {0};
        if (ioctl(listener, SECCOMP_IOCTL_NOTIF_RECV, &event)) break;
        if (event.pid != (unsigned int)child || event.data.nr != SYS_getpid ||
            event.data.args[0] != FIXTURE_MARK) break;
        struct seccomp_notif_resp reply = {.id = event.id, .flags = SECCOMP_USER_NOTIF_FLAG_CONTINUE};
        if (ioctl(listener, SECCOMP_IOCTL_NOTIF_SEND, &reply)) break;
        ++observed;
    }
    close(listener);
    return observed;
}

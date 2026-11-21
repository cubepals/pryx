/* SPDX-License-Identifier: FSL-1.1-ALv2 */
/* Run one fixed child and count its returns. The parent never accepts a target, command or
 * address from the caller. The same Rust policy protects each cohort's workload. */
#define _GNU_SOURCE
#include "observation.h"
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/prctl.h>
#include <sys/socket.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

enum mode { RAW, TRACE, NOTIFY, DISPATCH };

static uint64_t now(void) {
    struct timespec stamp;
    if (clock_gettime(CLOCK_MONOTONIC_RAW, &stamp)) _exit(1);
    return (uint64_t)stamp.tv_sec * UINT64_C(1000000000) + (uint64_t)stamp.tv_nsec;
}

static int workload(enum mode method, int channel, pid_t parent) {
    pid_t self = getpid();
    if (prctl(PR_SET_PDEATHSIG, SIGKILL) || getppid() != parent || setpgid(0, 0) < 0) return 1;
    if ((method == TRACE && trace_request() < 0) ||
        (method == NOTIFY && notify_install(channel) < 0)) {
        perror("observer setup unavailable");
        return UNAVAILABLE;
    }
    close(channel);
    if (pryx_observe_floor((unsigned int)parent)) return UNAVAILABLE;
    if (open("read-control", O_RDONLY) >= 0 || errno != EACCES) return 1;
    if (bind(-1, NULL, 0) >= 0 || errno != EPERM) return 1;
    /* musl raise uses tkill, denied by the real guest policy. Self-directed kill is allowed. */
    if (method == TRACE && kill(self, SIGSTOP)) return 1;
    if (method == DISPATCH && dispatch_install() < 0) {
        perror("fixed-site user dispatch unavailable under production policy");
        return UNAVAILABLE;
    }
    long (*call)(void) = method == DISPATCH ? dispatch_getpid : fixed_getpid;
    for (int i = 0; i < WARM_CALLS; ++i)
        if (call() != self) return 1;
    uint64_t begin = now();
    for (int i = 0; i < TIMED_CALLS; ++i)
        if (call() != self) return 1;
    uint64_t elapsed = now() - begin;
    if (dprintf(STDOUT_FILENO, "%" PRIu64 " %d\n", elapsed, TIMED_CALLS) < 0) return 1;
    if (method == DISPATCH) {
        if (dispatch_count() != WARM_CALLS + TIMED_CALLS) return 1;
        if (dprintf(STDOUT_FILENO, "%d %d\n", dispatch_count(), WARM_CALLS) < 0) return 1;
    }
    return 0;
}

int main(int argc, char **argv) {
    enum mode method;
    if (argc != 2) return 2;
    if (!strcmp(argv[1], "raw")) method = RAW;
    else if (!strcmp(argv[1], "trace")) method = TRACE;
    else if (!strcmp(argv[1], "notify")) method = NOTIFY;
    else if (!strcmp(argv[1], "dispatch")) method = DISPATCH;
    else return 2;
    if (getuid() == 0) return 2;
    /* Positive control: a nonexistent sentinel or outer denial cannot count as Landlock proof. */
    int sentinel = open("read-control", O_RDONLY);
    if (sentinel < 0 || close(sentinel)) return 1;
    if (bind(-1, NULL, 0) >= 0 || errno != EBADF) {
        fprintf(stderr, "host policy prevents differentiating the network denial\n");
        return UNAVAILABLE;
    }
    int sockets[2], status;
    if (socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0, sockets)) return 1;
    alarm(45);
    pid_t parent = getpid();
    pid_t child = fork();
    if (child < 0) return 1;
    if (!child) {
        close(sockets[0]);
        _exit(workload(method, sockets[1], parent));
    }
    close(sockets[1]);
    int observed = 0;
    if (method == TRACE) {
        observed = trace_observe(child);
        if (observed == -UNAVAILABLE) return UNAVAILABLE;
        if (observed < 0) return 1;
    } else {
        if (method == NOTIFY) observed = notify_observe(child, sockets[0]);
        if (waitpid(child, &status, 0) != child || !WIFEXITED(status)) return 1;
        if (WEXITSTATUS(status)) return WEXITSTATUS(status);
    }
    close(sockets[0]);
    if (method == DISPATCH) return 0;
    if (method != RAW && observed != WARM_CALLS + TIMED_CALLS) return 1;
    printf("%d %d\n", observed, method == RAW ? 0 : WARM_CALLS);
    return 0;
}

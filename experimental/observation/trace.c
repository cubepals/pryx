/* SPDX-License-Identifier: FSL-1.1-ALv2 */
/* Read syscall metadata for this process's own consenting benchmark child. */
#define _GNU_SOURCE
#include "observation.h"
#include <errno.h>
#include <linux/ptrace.h>
#include <signal.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

int trace_request(void) {
    return syscall(SYS_ptrace, PTRACE_TRACEME, 0, 0, 0);
}

int trace_observe(pid_t child) {
    int status, entries = 0, exits = 0, marked = 0;
    if (waitpid(child, &status, 0) != child) return -1;
    if (WIFEXITED(status)) return WEXITSTATUS(status) == UNAVAILABLE ? -UNAVAILABLE : -1;
    if (!WIFSTOPPED(status) || WSTOPSIG(status) != SIGSTOP) return -1;
    if (syscall(SYS_ptrace, PTRACE_SETOPTIONS, child, 0,
                PTRACE_O_TRACESYSGOOD | PTRACE_O_EXITKILL) < 0) return -1;
    for (;;) {
        if (syscall(SYS_ptrace, PTRACE_SYSCALL, child, 0, 0) < 0) return -1;
        if (waitpid(child, &status, 0) != child) return -1;
        if (WIFEXITED(status))
            return WEXITSTATUS(status) == 0 && !marked && entries == exits ? exits : -1;
        if (!WIFSTOPPED(status) || WSTOPSIG(status) != (SIGTRAP | 0x80)) return -1;
        struct ptrace_syscall_info info = {0};
        if (syscall(SYS_ptrace, PTRACE_GET_SYSCALL_INFO, child, sizeof(info), &info) < 0) return -1;
        if (info.op == PTRACE_SYSCALL_INFO_ENTRY) {
            marked = info.entry.nr == SYS_getpid && info.entry.args[0] == FIXTURE_MARK;
            entries += marked;
        } else if (info.op == PTRACE_SYSCALL_INFO_EXIT && marked) {
            if (info.exit.is_error || info.exit.rval != child) return -1;
            ++exits;
            marked = 0;
        }
    }
}

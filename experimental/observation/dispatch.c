/* SPDX-License-Identifier: FSL-1.1-ALv2 */
/* Count only the fixed getpid site's signals. The child cannot supply code, syscall
 * numbers, arguments, targets or a different dispatch region. */
#define _GNU_SOURCE
#include "observation.h"
#include <asm/unistd.h>
#include <linux/prctl.h>
#include <signal.h>
#include <stdint.h>
#include <unistd.h>

/* Older distribution headers predate Linux's inclusive-only mode. Value from the
 * v6.18 include/uapi/linux/prctl.h used by this experiment. */
#ifndef PR_SYS_DISPATCH_INCLUSIVE_ON
#define PR_SYS_DISPATCH_INCLUSIVE_ON 2
#endif
extern const unsigned char fixture_dispatch_start[], fixture_dispatch_end[];
static volatile sig_atomic_t deliveries;
static volatile sig_atomic_t delivered_pid;

static void observe_getpid(int signal, siginfo_t *info, void *context) {
    (void)context;
    uintptr_t site = (uintptr_t)info->si_call_addr;
    if (signal != SIGSYS || info->si_code != SYS_USER_DISPATCH || info->si_syscall != __NR_getpid ||
        site < (uintptr_t)fixture_dispatch_start || site >= (uintptr_t)fixture_dispatch_end ||
        deliveries >= WARM_CALLS + TIMED_CALLS)
        _exit(1);
    delivered_pid = getpid();
    ++deliveries;
}

int dispatch_install(void) {
    struct sigaction action = {0};
    action.sa_sigaction = observe_getpid;
    action.sa_flags = SA_SIGINFO;
    if (sigemptyset(&action.sa_mask) || sigaction(SIGSYS, &action, NULL)) return -1;
    unsigned long size = (uintptr_t)fixture_dispatch_end - (uintptr_t)fixture_dispatch_start;
    return (int)syscall(__NR_prctl, PR_SET_SYSCALL_USER_DISPATCH, (unsigned long)PR_SYS_DISPATCH_INCLUSIVE_ON,
                 (unsigned long)(uintptr_t)fixture_dispatch_start, size, 0UL);
}

long dispatch_getpid(void) {
    sig_atomic_t before = deliveries;
    (void)fixed_getpid();
    return deliveries == before + 1 ? delivered_pid : -1;
}

int dispatch_count(void) {
    return deliveries;
}

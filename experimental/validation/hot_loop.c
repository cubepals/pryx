// SPDX-License-Identifier: FSL-1.1-ALv2
/** Sample the installed E0 loop on L1-resident zero bytes. The empty noinline
 * function supplies matched call overhead. PMU cycles are reported only when
 * the kernel exposes an uninterrupted hardware counter to this one thread. */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <linux/perf_event.h>
#include <sched.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/syscall.h>
#include <time.h>
#include <unistd.h>

enum { BYTES = 4096, WIDTH = 16, REPEATS = 262144 };
_Alignas(64) static uint8_t page[BYTES];
typedef uint32_t (*scan)(const uint8_t *, size_t);
extern uint32_t pryx_proof_zero_blocks(const uint8_t *, size_t);
__attribute__((noinline)) static uint32_t empty(const uint8_t *p, size_t n) {
    __asm__ volatile("" : : "r"(p), "r"(n) : "memory");
    return 1;
}

struct sample { uint64_t ns, cycles; int has_cycles; };

static uint64_t now(void) {
    struct timespec ts;
    if (clock_gettime(CLOCK_MONOTONIC_RAW, &ts)) { perror("clock"); exit(1); }
    return (uint64_t)ts.tv_sec * UINT64_C(1000000000) + ts.tv_nsec;
}

static struct sample time_calls(scan fn, int fd) {
    uint64_t values[3] = {0};
    struct sample s = {0};
    if (fd >= 0 && (ioctl(fd, PERF_EVENT_IOC_RESET, 0) || ioctl(fd, PERF_EVENT_IOC_ENABLE, 0))) {
        perror("perf start"); exit(1);
    }
    uint64_t start = now();
    for (unsigned i = 0; i < REPEATS; ++i) {
        if (fn(page, BYTES) != 1) { fputs("wrong zero predicate\n", stderr); exit(1); }
    }
    s.ns = now() - start;
    if (fd >= 0) {
        if (ioctl(fd, PERF_EVENT_IOC_DISABLE, 0) || read(fd, values, sizeof(values)) != sizeof(values)) {
            perror("perf read"); exit(1);
        }
        s.cycles = values[0];
        s.has_cycles = values[1] != 0 && values[1] == values[2];
    }
    return s;
}

int main(void) {
    if (!getenv("GITHUB_REPOSITORY") || strcmp(getenv("GITHUB_REPOSITORY"), "cubepals/pryx") ||
        !getenv("GITHUB_ACTIONS") || strcmp(getenv("GITHUB_ACTIONS"), "true")) {
        fputs("native measurements run in pryx CI\n", stderr); return 2;
    }
    cpu_set_t allowed;
    if (sched_getaffinity(0, sizeof(allowed), &allowed)) { perror("get affinity"); return 1; }
    int first = 0;
    while (first < CPU_SETSIZE && !CPU_ISSET(first, &allowed)) ++first;
    if (first == CPU_SETSIZE) return 1;
    CPU_ZERO(&allowed); CPU_SET(first, &allowed);
    if (sched_setaffinity(0, sizeof(allowed), &allowed)) { perror("set affinity"); return 1; }
    struct perf_event_attr event = {.type = PERF_TYPE_HARDWARE, .size = sizeof(event),
        .config = PERF_COUNT_HW_CPU_CYCLES, .disabled = 1, .exclude_kernel = 1, .exclude_hv = 1,
        .read_format = PERF_FORMAT_TOTAL_TIME_ENABLED | PERF_FORMAT_TOTAL_TIME_RUNNING};
    int fd = (int)syscall(__NR_perf_event_open, &event, 0, -1, -1, 0);
    if (fd < 0) fprintf(stderr, "PMU unavailable: %s\n", strerror(errno));
    printf("pair,iterations,loop_ns,call_ns,loop_cycles,call_cycles,pmu_valid\n");
    for (int pair = 0; pair < 15; ++pair) {
        struct sample full, base;
        if (pair % 2) { base = time_calls(empty, fd); full = time_calls(pryx_proof_zero_blocks, fd); }
        else { full = time_calls(pryx_proof_zero_blocks, fd); base = time_calls(empty, fd); }
        printf("%d,%u,%" PRIu64 ",%" PRIu64 ",%" PRIu64 ",%" PRIu64 ",%d\n", pair,
            REPEATS * (BYTES / WIDTH), full.ns, base.ns, full.cycles, base.cycles,
            full.has_cycles && base.has_cycles);
    }
    if (fd >= 0) close(fd);
    return 0;
}

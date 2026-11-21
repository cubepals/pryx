// SPDX-License-Identifier: FSL-1.1-ALv2
/** Sample the installed E0 loop on L1-resident zero bytes. The empty noinline
 * function supplies matched call overhead. PMU cycles are reported only when
 * the kernel exposes an uninterrupted hardware counter to this one thread. */
#define _GNU_SOURCE
#include "cycles.h"

enum { BYTES = 4096, WIDTH = 16, REPEATS = 262144 };
_Alignas(64) static uint8_t page[BYTES];
typedef uint32_t (*scan)(const uint8_t *, size_t);
extern uint32_t pryx_proof_zero_blocks(const uint8_t *, size_t);
__attribute__((noinline)) static uint32_t empty(const uint8_t *p, size_t n) {
    __asm__ volatile("" : : "r"(p), "r"(n) : "memory");
    return 1;
}

static struct sample time_calls(scan fn, int fd) {
    struct sample s = {0};
    start_sample(fd, &s);
    for (unsigned i = 0; i < REPEATS; ++i) {
        if (fn(page, BYTES) != 1) { fputs("wrong zero predicate\n", stderr); exit(1); }
    }
    end_sample(fd, &s);
    return s;
}

int main(void) {
    if (!getenv("GITHUB_REPOSITORY") || strcmp(getenv("GITHUB_REPOSITORY"), "cubepals/pryx") ||
        !getenv("GITHUB_ACTIONS") || strcmp(getenv("GITHUB_ACTIONS"), "true")) {
        fputs("native measurements run in pryx CI\n", stderr); return 2;
    }
    pin_cpu();
    int fd = open_cycles();
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

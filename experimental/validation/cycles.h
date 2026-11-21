// SPDX-License-Identifier: FSL-1.1-ALv2
/* Shared native throughput harness. Each thread reads its own user-cycle PMU.
 * A counter that did not run for its entire interval never supplies a cycle result. */
#ifndef PRYX_LAB_CYCLES_H
#define PRYX_LAB_CYCLES_H
#include <errno.h>
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

struct sample { uint64_t ns, cycles, enabled, running; int active, has_cycles; };

static int pin_cpu(void) {
    cpu_set_t allowed;
    if (sched_getaffinity(0, sizeof(allowed), &allowed)) { perror("get affinity"); exit(1); }
    int first = 0;
    while (first < CPU_SETSIZE && !CPU_ISSET(first, &allowed)) ++first;
    if (first == CPU_SETSIZE) { fputs("no allowed CPU\n", stderr); exit(1); }
    CPU_ZERO(&allowed); CPU_SET(first, &allowed);
    if (sched_setaffinity(0, sizeof(allowed), &allowed)) { perror("set affinity"); exit(1); }
    return first;
}

static int open_cycles(void) {
    struct perf_event_attr event = {.type = PERF_TYPE_HARDWARE, .size = sizeof(event),
        .config = PERF_COUNT_HW_CPU_CYCLES, .disabled = 1, .exclude_kernel = 1, .exclude_hv = 1,
        .read_format = PERF_FORMAT_TOTAL_TIME_ENABLED | PERF_FORMAT_TOTAL_TIME_RUNNING};
    int fd = (int)syscall(__NR_perf_event_open, &event, 0, -1, -1, 0);
    if (fd < 0) fprintf(stderr, "PMU unavailable: %s\n", strerror(errno));
    return fd;
}

static uint64_t now(void) {
    struct timespec ts;
    if (clock_gettime(CLOCK_MONOTONIC_RAW, &ts)) { perror("clock"); exit(1); }
    return (uint64_t)ts.tv_sec * UINT64_C(1000000000) + ts.tv_nsec;
}

static void start_sample(int fd, struct sample *s) {
    if (fd >= 0) {
        /* Reset clears the count but leaves time_enabled/time_running cumulative.
         * Save both so validity refers to this interval alone. */
        uint64_t before[3];
        if (read(fd, before, sizeof(before)) != sizeof(before) ||
            ioctl(fd, PERF_EVENT_IOC_RESET, 0) || ioctl(fd, PERF_EVENT_IOC_ENABLE, 0)) {
            perror("PMU start unavailable");
        } else {
            s->enabled = before[1];
            s->running = before[2];
            s->active = 1;
        }
    }
    s->ns = now();
}

static void end_sample(int fd, struct sample *s) {
    s->ns = now() - s->ns;
    if (s->active) {
        uint64_t after[3];
        if (ioctl(fd, PERF_EVENT_IOC_DISABLE, 0) || read(fd, after, sizeof(after)) != sizeof(after)) {
            s->enabled = s->running = 0;
            perror("PMU read unavailable"); return;
        }
        s->cycles = after[0];
        s->enabled = after[1] - s->enabled;
        s->running = after[2] - s->running;
        s->has_cycles = s->enabled != 0 && s->enabled == s->running;
    }
}
#endif

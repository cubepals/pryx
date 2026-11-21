/* SPDX-License-Identifier: FSL-1.1-ALv2 */
#ifndef PRYX_OBSERVATION_H
#define PRYX_OBSERVATION_H
#include <sys/types.h>

/* getpid takes no arguments. This marker in its unused first argument distinguishes
 * the fixture's calls from libc and the process policy's own setup calls. */
#include "mark.h"
#define WARM_CALLS 128
#define TIMED_CALLS 4096
#define UNAVAILABLE 77

int pryx_observe_floor(unsigned int supervisor);
long fixed_getpid(void);
int notify_install(int channel);
int notify_observe(pid_t child, int channel);
int trace_request(void);
int trace_observe(pid_t child);
#endif

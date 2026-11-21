// SPDX-License-Identifier: GPL-2.0-only OR BSD-2-Clause
// A private diagnostic map for the seven fixed stock socket-hook paths.
#include <linux/bpf.h>
struct witness_value { __u64 invocations, unexpected; };
struct {
    int (*type)[BPF_MAP_TYPE_PERCPU_ARRAY];
    int (*max_entries)[7];
    __u32 *key;
    struct witness_value *value;
} socket_witness __attribute__((section(".maps"), used));

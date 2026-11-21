// SPDX-License-Identifier: FSL-1.1-ALv2
/* Architecture-gated A1 entry points, run on the calling thread's pinned CPU.
 * --check runs correctness only. CI pairs 4 KiB calls with the same empty ABI. */
#define _GNU_SOURCE
#include "cycles.h"
#include <stdbool.h>
#if defined(__aarch64__)
#include <asm/hwcap.h>
#include <sys/auxv.h>
#endif

enum { BYTES = 4096, REPEATS = 4096, PAIRS = 15, FILL = 0x5a };
enum operation { DIFF, VALUE, HASH, COPY, CLEAR };
enum feature { BASE, SIMD, WIDE, ZVA, RUST_VECTOR };
typedef size_t compare_fn(const uint8_t *, const uint8_t *, size_t);
typedef size_t value_fn(const uint8_t *, uint8_t, size_t);
typedef void hash_fn(const uint8_t *, size_t, uint32_t *);
typedef void transfer_fn(uint8_t *, const uint8_t *, size_t, size_t);
union kernel {
    compare_fn *diff;
    value_fn *value;
    hash_fn *hash;
    transfer_fn *transfer;
};
struct path {
    const char *name;
    enum operation op;
    enum feature feature;
    unsigned width, hint, fill;
    union kernel fn;
};
_Alignas(BYTES) static uint8_t source[BYTES], destination[BYTES];
static uint32_t lanes[4];
extern bool a1_vector_available(void);
extern hash_fn pryx_page_hash_lanes;
extern transfer_fn pryx_page_copy_nt, pryx_page_clear_nt, a1_vector_copy, a1_vector_clear;
#if defined(__x86_64__)
extern compare_fn pryx_page_diff_avx2, pryx_page_diff_avx512;
extern value_fn pryx_page_value_avx2, pryx_page_value_avx512;
extern transfer_fn pryx_page_copy_gpr, pryx_page_clear_gpr, pryx_page_copy_rep, pryx_page_clear_rep;
#elif defined(__aarch64__)
extern compare_fn pryx_page_diff_neon;
extern value_fn pryx_page_value_neon;
extern size_t pryx_page_zva64(void);
extern transfer_fn pryx_page_clear_zva;
#endif

#define COMPARES(isa, feature, width) \
    {"diff_" #isa, DIFF, feature, width, 0, FILL, {.diff = pryx_page_diff_##isa}}, \
    {"value_" #isa, VALUE, feature, width, 0, 0, {.value = pryx_page_value_##isa}}, \
    {"value_" #isa, VALUE, feature, width, 0, FILL, {.value = pryx_page_value_##isa}}
#define TRANSFER(op, name, feature, width, hint) \
    {#name, op, feature, width, hint, FILL, {.transfer = pryx_page_##name}}
static const struct path paths[] = {
#if defined(__x86_64__)
    COMPARES(avx2, SIMD, 32), COMPARES(avx512, WIDE, 64),
    TRANSFER(COPY, copy_gpr, BASE, 8, 0), TRANSFER(COPY, copy_gpr, BASE, 8, 8),
    TRANSFER(CLEAR, clear_gpr, BASE, 8, 0),
    TRANSFER(COPY, copy_rep, BASE, BYTES, 0), TRANSFER(CLEAR, clear_rep, BASE, BYTES, 0),
#elif defined(__aarch64__)
    COMPARES(neon, SIMD, 16), TRANSFER(CLEAR, clear_zva, ZVA, 64, 0),
#endif
    {"hash_lanes", HASH, SIMD, 16, 0, FILL, {.hash = pryx_page_hash_lanes}},
    TRANSFER(COPY, copy_nt, SIMD, 32, 0), TRANSFER(COPY, copy_nt, SIMD, 32, 32),
    TRANSFER(CLEAR, clear_nt, SIMD, 32, 0),
    /* The compiler decides the Rust vector loop's unrolling. Emit units of one
     * complete page here. The model reader uses the observed emitted block width. */
    {"rust-copy", COPY, RUST_VECTOR, BYTES, 0, FILL, {.transfer = a1_vector_copy}},
    {"rust-clear", CLEAR, RUST_VECTOR, BYTES, 0, FILL, {.transfer = a1_vector_clear}},
};

static int supported(enum feature feature) {
    if (feature == BASE) return 1;
    if (feature == RUST_VECTOR) return a1_vector_available();
#if defined(__x86_64__)
    if (feature == SIMD) return !!__builtin_cpu_supports("avx2");
    return feature == WIDE && __builtin_cpu_supports("avx512f") && __builtin_cpu_supports("avx512bw");
#elif defined(__aarch64__)
    if (feature == ZVA) return pryx_page_zva64() != 0;
    return feature == SIMD && (getauxval(AT_HWCAP) & HWCAP_ASIMD) != 0;
#endif
}

/* Empty functions live behind the same indirect call with the same arguments.
 * The compiler barrier makes the call, pointer inputs and potential memory
 * effects observable for each ABI, without adding a memory transfer. */
static size_t empty_diff(const uint8_t *a, const uint8_t *b, size_t n) {
    __asm__ volatile("" :: "r"(a), "r"(b), "r"(n) : "memory"); return n;
}
static size_t empty_value(const uint8_t *a, uint8_t v, size_t n) {
    __asm__ volatile("" :: "r"(a), "r"(v), "r"(n) : "memory"); return n;
}
static void empty_hash(const uint8_t *a, size_t n, uint32_t *state) {
    __asm__ volatile("" :: "r"(a), "r"(n), "r"(state) : "memory");
}
static void empty_transfer(uint8_t *d, const uint8_t *s, size_t n, size_t hint) {
    __asm__ volatile("" :: "r"(d), "r"(s), "r"(n), "r"(hint) : "memory");
}
static union kernel control(enum operation op) {
    switch (op) {
    case DIFF: return (union kernel){.diff = empty_diff};
    case VALUE: return (union kernel){.value = empty_value};
    case HASH: return (union kernel){.hash = empty_hash};
    default: return (union kernel){.transfer = empty_transfer};
    }
}

#if __has_attribute(noclone)
__attribute__((noclone))
#endif
__attribute__((noinline)) static void calls(const struct path *p, union kernel fn, unsigned repeats) {
    switch (p->op) {
    case DIFF:
        for (unsigned i = 0; i < repeats; ++i)
            if (fn.diff(destination, source, BYTES) != BYTES) abort();
        break;
    case VALUE:
        for (unsigned i = 0; i < repeats; ++i)
            if (fn.value(source, p->fill, BYTES) != BYTES) abort();
        break;
    case HASH:
        for (unsigned i = 0; i < repeats; ++i) fn.hash(source, BYTES / 16, lanes);
        break;
    case COPY:
    case CLEAR: {
        const uint8_t *input = p->op == COPY ? source : NULL;
        for (unsigned i = 0; i < repeats; ++i) fn.transfer(destination, input, BYTES, p->hint);
        break;
    }
    }
}

static void prepare(const struct path *p) {
    memset(source, p->fill, BYTES);
    memset(destination, p->op == DIFF ? p->fill : 0xa5, BYTES);
    for (unsigned i = 0; i < 4; ++i) lanes[i] = UINT32_C(2166136261) + i;
}
static void validate(const struct path *p) {
    prepare(p);
    calls(p, p->fn, 1);
    if (p->op == HASH) {
        for (unsigned lane = 0; lane < 4; ++lane) {
            uint32_t expected = UINT32_C(2166136261) + lane;
            for (size_t j = 0; j < BYTES; j += 16) {
                uint32_t word;
                memcpy(&word, source + j + 4 * lane, 4);
                expected = (expected ^ word) * UINT32_C(16777619);
            }
            if (lanes[lane] != expected) abort();
        }
    } else if (p->op == COPY || p->op == CLEAR) {
        for (size_t i = 0; i < BYTES; ++i)
            if (destination[i] != (p->op == COPY ? p->fill : 0)) abort();
    } else {
        source[BYTES - 1] ^= 1;
        size_t mismatch = p->op == DIFF ? p->fn.diff(destination, source, BYTES) :
                                          p->fn.value(source, p->fill, BYTES);
        if (mismatch != BYTES - 1) abort();
    }
}

static struct sample measure(const struct path *p, union kernel fn, int fd) {
    struct sample s = {0};
    prepare(p);
    calls(p, fn, 8);
    start_sample(fd, &s);
    calls(p, fn, REPEATS);
    end_sample(fd, &s);
    return s;
}
static void print_sample(struct sample s) {
    printf(",%" PRIu64 ",%" PRIu64 ",%" PRIu64 ",%" PRIu64, s.ns, s.cycles, s.enabled, s.running);
}

int main(int argc, char **argv) {
    int check = argc == 2 && !strcmp(argv[1], "--check");
    if ((argc != 1 && !check) || (!check && (!getenv("GITHUB_ACTIONS") ||
        strcmp(getenv("GITHUB_ACTIONS"), "true") || !getenv("GITHUB_REPOSITORY") ||
        strcmp(getenv("GITHUB_REPOSITORY"), "cubepals/pryx")))) {
        fputs("use --check for functional checks; measurement runs on native pryx CI\n", stderr); return 2;
    }
    int cpu = pin_cpu();
    int fd = check ? -1 : open_cycles();
    if (!check)
        puts("kernel,hint,fill,unit_bytes,page_bytes,calls,cpu,pair,order,status,"
             "kernel_ns,kernel_cycles,kernel_enabled,kernel_running,call_ns,call_cycles,call_enabled,call_running");
    for (size_t i = 0; i < sizeof(paths) / sizeof(paths[0]); ++i) {
        const struct path *p = &paths[i];
        if (!supported(p->feature)) {
            if (!check) printf("%s,%u,%u,%u,%u,0,%d,,,unsupported ISA or ZVA geometry,,,,,,,,\n",
                               p->name, p->hint, p->fill, p->width, BYTES, cpu);
            else printf("%s hint=%u fill=%u unavailable\n", p->name, p->hint, p->fill);
            continue;
        }
        validate(p);
        if (check) {
            printf("%s hint=%u fill=%u passed\n", p->name, p->hint, p->fill); continue;
        }
        for (unsigned pair = 0; pair < PAIRS; ++pair) {
            struct sample full, empty;
            if (pair % 2) { empty = measure(p, control(p->op), fd); full = measure(p, p->fn, fd); }
            else { full = measure(p, p->fn, fd); empty = measure(p, control(p->op), fd); }
            printf("%s,%u,%u,%u,%u,%u,%d,%u,%s,%s", p->name, p->hint, p->fill, p->width,
                BYTES, REPEATS, cpu, pair, pair % 2 ? "call-first" : "kernel-first",
                fd < 0 ? "no PMU" : full.has_cycles && empty.has_cycles ? "counted" : "partial PMU");
            print_sample(full); print_sample(empty); putchar('\n'); fflush(stdout);
        }
    }
    if (fd >= 0) close(fd);
    return 0;
}

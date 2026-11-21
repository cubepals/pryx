// SPDX-License-Identifier: FSL-1.1-ALv2
/* Print the instruction families visible to a native or Memcheck process. */
#include <stdio.h>
#if defined(__aarch64__)
#include <asm/hwcap.h>
#include <sys/auxv.h>
#endif

int main(void) {
#if defined(__x86_64__)
    __builtin_cpu_init();
    printf("sse2=%u avx2=%u avx512f=%u avx512bw=%u\n",
           !!__builtin_cpu_supports("sse2"), !!__builtin_cpu_supports("avx2"),
           !!__builtin_cpu_supports("avx512f"), !!__builtin_cpu_supports("avx512bw"));
#elif defined(__aarch64__)
    printf("neon=%u\n", !!(getauxval(AT_HWCAP) & HWCAP_ASIMD));
#else
    puts("scalar");
#endif
    return 0;
}

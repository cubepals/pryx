// SPDX-License-Identifier: FSL-1.1-ALv2
// Freestanding C PID 1. The same ELF stamp precedes this entry and the
// production executable, so the clock event has the same meaning for both.
#include <asm/unistd.h>

__attribute__((noreturn)) void _start(void) {
    for (;;) {
#if defined(__x86_64__)
        register long fourth __asm__("r10") = 0;
        register long fifth __asm__("r8") = 0;
        long number = __NR_ppoll;
        __asm__ volatile ("syscall" : "+a"(number)
                          : "D"(0L), "S"(0L), "d"(0L), "r"(fourth), "r"(fifth)
                          : "rcx", "r11", "memory");
#elif defined(__aarch64__)
        register long first __asm__("x0") = 0;
        register long second __asm__("x1") = 0;
        register long third __asm__("x2") = 0;
        register long fourth __asm__("x3") = 0;
        register long fifth __asm__("x4") = 0;
        register long number __asm__("x8") = __NR_ppoll;
        __asm__ volatile ("svc #0" : "+r"(first)
                          : "r"(second), "r"(third), "r"(fourth), "r"(fifth), "r"(number)
                          : "memory", "cc");
#else
#error Unsupported guest architecture
#endif
    }
}

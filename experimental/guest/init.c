// SPDX-License-Identifier: FSL-1.1-ALv2
// Freestanding PID 1 timestamps its entry and blocks for the host to stop it.
#include <asm/unistd.h>
#include <linux/time.h>

static long call(long number, long first, long second, long third) {
#if defined(__x86_64__)
    register long fourth __asm__("r10") = 0;
    register long fifth __asm__("r8") = 0;
    __asm__ volatile ("syscall" : "+a"(number)
                      : "D"(first), "S"(second), "d"(third), "r"(fourth), "r"(fifth)
                      : "rcx", "r11", "memory");
    return number;
#elif defined(__aarch64__)
    register long arg1 __asm__("x0") = first;
    register long arg2 __asm__("x1") = second;
    register long arg3 __asm__("x2") = third;
    register long arg4 __asm__("x3") = 0;
    register long arg5 __asm__("x4") = 0;
    register long nr __asm__("x8") = number;
    __asm__ volatile ("svc #0" : "+r"(arg1)
                      : "r"(arg2), "r"(arg3), "r"(arg4), "r"(arg5), "r"(nr)
                      : "memory", "cc");
    return arg1;
#else
#error Unsupported guest architecture
#endif
}

__attribute__((noreturn)) void _start(void) {
    struct timespec stamp;
    if (call(__NR_getpid, 0, 0, 0) == 1 &&
        call(__NR_clock_gettime, CLOCK_BOOTTIME, (long)&stamp, 0) == 0) {
        unsigned long nanos = (unsigned long)stamp.tv_sec * 1000000000UL + stamp.tv_nsec;
        static char record[] = "pryx-lab-pid1 0000000000000000\n";
        char *end = record + sizeof(record) - 2;  // Just before the newline.
        for (unsigned int digit = 0; digit < 2 * sizeof(nanos); ++digit) {
            *--end = "0123456789abcdef"[nanos & 15];
            nanos >>= 4;
        }
        call(__NR_write, 1, (long)record, sizeof(record) - 1);
    }
    for (;;) call(__NR_ppoll, 0, 0, 0);
}

/* SPDX-License-Identifier: FSL-1.1-ALv2 */
/* One fixed benign syscall with an unused first argument identifying this fixture. */
#include <asm/unistd.h>
#include "mark.h"
.text
.global fixture_dispatch_start, fixture_dispatch_end
.global fixed_getpid
.type fixed_getpid,%function
fixed_getpid:
fixture_dispatch_start:
#if defined(__x86_64__)
    mov $FIXTURE_MARK, %edi
    mov $__NR_getpid, %eax
    syscall
    ret
#elif defined(__aarch64__)
    movz w0, #(FIXTURE_MARK & 0xffff)
    movk w0, #(FIXTURE_MARK >> 16), lsl #16
    mov x8, #__NR_getpid
    svc #0
    ret
#else
#error Unsupported architecture
#endif
fixture_dispatch_end:
.size fixed_getpid, .-fixed_getpid
.section .note.GNU-stack,"",%progbits

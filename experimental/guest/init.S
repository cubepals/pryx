/* SPDX-License-Identifier: FSL-1.1-ALv2 */
/* The host stops the isolated fixture after the ELF entry stamp is received. */
#include <asm/unistd.h>
.text
.globl _start
_start:
#if defined(__x86_64__)
    xor %edi,%edi
    xor %esi,%esi
    xor %edx,%edx
    xor %r10d,%r10d
    xor %r8d,%r8d
1:  mov $__NR_ppoll,%eax
    syscall
    jmp 1b
#elif defined(__aarch64__)
1:  mov x0,#0
    mov x1,#0
    mov x2,#0
    mov x3,#0
    mov x4,#0
    mov x8,#__NR_ppoll
    svc #0
    b 1b
#else
#error Unsupported guest architecture
#endif
.section .note.GNU-stack,"",%progbits

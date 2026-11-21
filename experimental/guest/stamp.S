/* SPDX-License-Identifier: FSL-1.1-ALv2 */
/* First instructions of the assembly init, also linked before the real agent's
 * normal ELF entry in the lab. No relocations through a runtime or loader.
 * CLOCK_BOOTTIME comes from linux/time.h via the caller's preprocessor. */
#include <asm/unistd.h>
.section .rodata
prefix: .ascii "pryx-lab-pid1 "
.set prefix_size, .-prefix
.text
.globl pryx_lab_entry
pryx_lab_entry:
#if defined(__x86_64__)
    push %rdx                   /* ELF startup ABI: dynamic linker finalizer */
    mov $__NR_getpid,%eax
    syscall
    cmp $1,%rax
    jne 3f
    sub $32,%rsp
    mov %rsp,%rsi
    mov $LAB_BOOT_CLOCK,%edi
    mov $__NR_clock_gettime,%eax
    syscall
    test %rax,%rax
    jne 2f
    imul $1000000000,(%rsp),%rax
    add 8(%rsp),%rax
    movb $10,16(%rsp)
    mov $16,%ecx
1:  mov %eax,%edx
    and $15,%edx
    add $48,%edx
    cmp $57,%edx
    jbe 4f
    add $39,%edx
4:  mov %dl,-1(%rsp,%rcx)
    shr $4,%rax
    loop 1b
    mov $__NR_write,%eax
    mov $1,%edi
    lea prefix(%rip),%rsi
    mov $prefix_size,%edx
    syscall
    mov $__NR_write,%eax
    mov %rsp,%rsi
    mov $17,%edx
    syscall
2:  add $32,%rsp
3:  pop %rdx
    jmp _start
#elif defined(__aarch64__)
    stp x0,x1,[sp,#-32]!
    mov x8,#__NR_getpid
    svc #0
    cmp x0,#1
    b.ne 3f
    mov x0,#LAB_BOOT_CLOCK
    add x1,sp,#16
    mov x8,#__NR_clock_gettime
    svc #0
    cbnz x0,3f
    ldp x2,x3,[sp,#16]
    movz x4,#0xca00
    movk x4,#0x3b9a,lsl #16     /* 10^9 nanoseconds per second */
    madd x2,x2,x4,x3
    sub sp,sp,#32
    mov w3,#10
    strb w3,[sp,#16]
    mov x4,#16
1:  sub x4,x4,#1
    and w3,w2,#15
    add w5,w3,#48
    add w6,w3,#87
    cmp w3,#10
    csel w3,w5,w6,lo
    strb w3,[sp,x4]
    lsr x2,x2,#4
    cbnz x4,1b
    mov x0,#1
    adrp x1,prefix
    add x1,x1,:lo12:prefix
    mov x2,#prefix_size
    mov x8,#__NR_write
    svc #0
    mov x0,#1
    mov x1,sp
    mov x2,#17
    mov x8,#__NR_write
    svc #0
    add sp,sp,#32
3:  ldp x0,x1,[sp],#32
    b _start
#else
#error Unsupported guest architecture
#endif
.section .note.GNU-stack,"",%progbits

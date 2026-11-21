/* SPDX-License-Identifier: FSL-1.1-ALv2 */
/* The link-instrumented fixture has one single-threaded child, one private
 * counter, and one fixed destination. The existing marked syscall returns
 * directly to its caller through the return address preserved by the tail branch. */
.bss
.p2align 2
.Llinked_calls:
    .zero 4

.text
.global __wrap_fixed_getpid
.hidden __wrap_fixed_getpid
.type __wrap_fixed_getpid,%function
__wrap_fixed_getpid:
#if defined(__x86_64__)
    incl .Llinked_calls(%rip)
    jmp __real_fixed_getpid
#elif defined(__aarch64__)
    adrp x9, .Llinked_calls
    add x9, x9, :lo12:.Llinked_calls
    ldr w10, [x9]
    add w10, w10, #1
    str w10, [x9]
    b __real_fixed_getpid
#else
#error Unsupported architecture
#endif
.size __wrap_fixed_getpid, .-__wrap_fixed_getpid

.global linked_count
.hidden linked_count
.type linked_count,%function
linked_count:
#if defined(__x86_64__)
    mov .Llinked_calls(%rip), %eax
#elif defined(__aarch64__)
    adrp x9, .Llinked_calls
    ldr w0, [x9, :lo12:.Llinked_calls]
#endif
    ret
.size linked_count, .-linked_count
.section .note.GNU-stack,"",%progbits

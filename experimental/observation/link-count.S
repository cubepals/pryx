/* SPDX-License-Identifier: FSL-1.1-ALv2 */
/* The link-instrumented fixture has one single-threaded child, one private
 * counter, and one fixed destination. The existing marked syscall returns
 * directly to its caller through the return address preserved by the tail branch. */
.bss
.p2align 2
.Llinked_calls:
    .zero 4

#ifdef PRYX_PREPARED_SLOT
/* Only this section of the self-built prepared fixtures can differ. */
.section .pryx_observe_count,"ax",%progbits
#else
.text
#endif
.global __wrap_fixed_getpid
.hidden __wrap_fixed_getpid
.type __wrap_fixed_getpid,%function
__wrap_fixed_getpid:
#if defined(__x86_64__)
#ifdef PRYX_PREPARED_SLOT
    movl .Llinked_calls(%rip), %r10d
    addl $PRYX_SLOT_INCREMENT, %r10d
    movl %r10d, .Llinked_calls(%rip)
#else
    incl .Llinked_calls(%rip)
#endif
    jmp __real_fixed_getpid
#elif defined(__aarch64__)
    adrp x9, .Llinked_calls
    add x9, x9, :lo12:.Llinked_calls
    ldr w10, [x9]
#ifdef PRYX_PREPARED_SLOT
    add w10, w10, #PRYX_SLOT_INCREMENT
#else
    add w10, w10, #1
#endif
    str w10, [x9]
    b __real_fixed_getpid
#else
#error Unsupported architecture
#endif
.size __wrap_fixed_getpid, .-__wrap_fixed_getpid

.text
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

# SPDX-License-Identifier: FSL-1.1-ALv2
.text
.macro scan name width count
.global \name
.hidden \name
.type \name,@function
\name:
.if \width == 32
    vpxor %ymm0,%ymm0,%ymm0
.else
    pxor %xmm0,%xmm0
.endif
1:
.set offset,0
.rept \count
.if \width == 32
    vpor offset(%rdi),%ymm0,%ymm0
.else
    movdqu offset(%rdi),%xmm1
    por %xmm1,%xmm0
.endif
.set offset,offset+\width
.endr
    add $(\width*\count),%rdi
    dec %rsi
    jne 1b
.if \width == 32
    vptest %ymm0,%ymm0
    sete %al
    vzeroupper
.else
    pxor %xmm1,%xmm1
    pcmpeqb %xmm1,%xmm0
    pmovmskb %xmm0,%eax
    cmp $65535,%eax
    sete %al
.endif
    movzbl %al,%eax
    ret
.size \name,.-\name
.endm
scan pryx_static_16_1,16,1
scan pryx_static_16_2,16,2
scan pryx_static_16_4,16,4
scan pryx_static_32_1,32,1
scan pryx_static_32_2,32,2
scan pryx_static_32_4,32,4
.section .note.GNU-stack,"",@progbits

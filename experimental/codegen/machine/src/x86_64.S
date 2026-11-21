# SPDX-License-Identifier: FSL-1.1-ALv2
# Static page scans for each detected vector width and unroll factor. Rust checks the tail.
.text
.macro scan name width count
.global \name
.hidden \name
.type \name,@function
\name:
.if \width == 64
    vpxord %zmm0,%zmm0,%zmm0
.elseif \width == 32
    vpxor %ymm0,%ymm0,%ymm0
.else
    pxor %xmm0,%xmm0
.endif
1:
.set offset,0
.rept \count
.if \width == 64
    vpord offset(%rdi),%zmm0,%zmm0
.elseif \width == 32
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
.if \width == 64
    vptestmb %zmm0,%zmm0,%k1
    kortestq %k1,%k1
    sete %al
    vzeroupper
.elseif \width == 32
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
scan pryx_static_64_1,64,1
scan pryx_static_64_2,64,2
scan pryx_static_64_4,64,4
.global pryx_static_copy_1_1
.hidden pryx_static_copy_1_1
.type pryx_static_copy_1_1,@function
pryx_static_copy_1_1:
    mov %rdx,%rcx
    rep movsb
    ret
.size pryx_static_copy_1_1,.-pryx_static_copy_1_1

.macro copy width count
.global pryx_static_copy_\width\()_\count
.hidden pryx_static_copy_\width\()_\count
.type pryx_static_copy_\width\()_\count,@function
pryx_static_copy_\width\()_\count:
1:
.set offset,0
.rept \count
.if \width == 64
    vmovdqu64 offset(%rsi),%zmm0
    vmovdqu64 %zmm0,offset(%rdi)
.elseif \width == 32
    vmovdqu offset(%rsi),%ymm0
    vmovdqu %ymm0,offset(%rdi)
.else
    movdqu offset(%rsi),%xmm0
    movdqu %xmm0,offset(%rdi)
.endif
.set offset,offset+\width
.endr
    add $(\width*\count),%rsi
    add $(\width*\count),%rdi
    dec %rdx
    jne 1b
.if \width >= 32
    vzeroupper
.endif
    ret
.size pryx_static_copy_\width\()_\count,.-pryx_static_copy_\width\()_\count
.endm
copy 16,1
copy 16,2
copy 16,4
copy 32,1
copy 32,2
copy 32,4
copy 64,1
copy 64,2
copy 64,4
.section .note.GNU-stack,"",@progbits

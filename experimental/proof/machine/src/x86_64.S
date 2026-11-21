/* SPDX-License-Identifier: FSL-1.1-ALv2 */
.text
.global pryx_proof_zero_blocks
.hidden pryx_proof_zero_blocks
.type pryx_proof_zero_blocks,@function
pryx_proof_zero_blocks:
    pxor %xmm1, %xmm1
.Lblocks:
# LLVM-MCA-BEGIN hot
# OSACA-BEGIN
    movdqu (%rdi), %xmm0
    por %xmm0, %xmm1
    addq $16, %rdi
    subq $16, %rsi
    jne .Lblocks
# OSACA-END
# LLVM-MCA-END hot
    pxor %xmm0, %xmm0
    pcmpeqb %xmm0, %xmm1
    pmovmskb %xmm1, %eax
    cmpl $65535, %eax
    sete %al
    movzbl %al, %eax
    ret
.size pryx_proof_zero_blocks, .-pryx_proof_zero_blocks
.section .note.GNU-stack,"",@progbits

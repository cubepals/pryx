/* SPDX-License-Identifier: FSL-1.1-ALv2 */
.text
.global pryx_proof_zero_blocks
.hidden pryx_proof_zero_blocks
.type pryx_proof_zero_blocks,%function
pryx_proof_zero_blocks:
    movi v1.16b, #0
.Lblocks:
// LLVM-MCA-BEGIN hot
// OSACA-BEGIN
    ldr q0, [x0], #16
    orr v1.16b, v1.16b, v0.16b
    subs x1, x1, #16
    b.ne .Lblocks
// OSACA-END
// LLVM-MCA-END hot
    umaxv b0, v1.16b
    umov w0, v0.b[0]
    cmp w0, #0
    cset w0, eq
    ret
.size pryx_proof_zero_blocks, .-pryx_proof_zero_blocks
.section .note.GNU-stack,"",%progbits

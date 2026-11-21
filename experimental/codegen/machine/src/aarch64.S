// SPDX-License-Identifier: FSL-1.1-ALv2
.text
.macro scan name count
.global \name
.hidden \name
.type \name,%function
\name:
    movi v0.16b,#0
1:
.set offset,0
.rept \count
    ldr q1,[x0,#offset]
    orr v0.16b,v0.16b,v1.16b
.set offset,offset+16
.endr
    add x0,x0,#(16*\count)
    subs x1,x1,#1
    b.ne 1b
    umaxv b0,v0.16b
    umov w0,v0.b[0]
    cmp w0,#0
    cset w0,eq
    ret
.size \name,.-\name
.endm
scan pryx_static_16_1,1
scan pryx_static_16_2,2
scan pryx_static_16_4,4

.macro copy count
.global pryx_static_copy_16_\count
.hidden pryx_static_copy_16_\count
.type pryx_static_copy_16_\count,%function
pryx_static_copy_16_\count:
1:
.set offset,0
.rept \count
    ldr q0,[x1,#offset]
    str q0,[x0,#offset]
.set offset,offset+16
.endr
    add x1,x1,#(16*\count)
    add x0,x0,#(16*\count)
    subs x2,x2,#1
    b.ne 1b
    ret
.size pryx_static_copy_16_\count,.-pryx_static_copy_16_\count
.endm
copy 1
copy 2
copy 4
.section .note.GNU-stack,"",%progbits

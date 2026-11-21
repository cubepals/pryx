/* Linux AArch64 page kernels. Vector compare finds a differing block, then
 * byte comparison identifies the first difference. Tails never read a full vector. */
.text
.macro compare name, paired
.global \name
.type \name,%function
\name:
    mov x3, #0
.if !\paired
    dup v1.16b, w1
.endif
.Lblock\@:
    cmp x2, #16
    b.lo .Ltail\@
    ldr q0, [x0, x3]
.if \paired
    ldr q1, [x1, x3]
.endif
    eor v2.16b, v0.16b, v1.16b
    umaxv b2, v2.16b
    fmov w4, s2
    cbnz w4, .Ltail\@
    add x3, x3, #16
    sub x2, x2, #16
    b .Lblock\@
.Ltail\@:
    cbz x2, .Ldone\@
    ldrb w4, [x0, x3]
.if \paired
    ldrb w5, [x1, x3]
    cmp w4, w5
.else
    and w5, w1, #255
    cmp w4, w5
.endif
    b.ne .Ldone\@
    add x3, x3, #1
    sub x2, x2, #1
    b .Ltail\@
.Ldone\@:
    mov x0, x3
    ret
.size \name,.-\name
.endm
compare pryx_page_diff_neon, 1
compare pryx_page_value_neon, 0

.global pryx_page_hash_lanes
.type pryx_page_hash_lanes,%function
pryx_page_hash_lanes:
    ldr q0, [x2]
    mov w3, #0x0193
    movk w3, #0x0100, lsl #16
    dup v1.4s, w3
    cbz x1, .Lhashdone
.Lhash:
    ldr q2, [x0], #16
    eor v0.16b, v0.16b, v2.16b
    mul v0.4s, v0.4s, v1.4s
    subs x1, x1, #1
    b.ne .Lhash
.Lhashdone:
    str q0, [x2]
    ret
.size pryx_page_hash_lanes,.-pryx_page_hash_lanes

.macro transfer name, copy
.global \name
.type \name,%function
\name:
    movi v0.16b, #0
    movi v1.16b, #0
.Lbulk\@:
    cmp x2, #32
    b.lo .Ltail\@
.if \copy
    cbz x3, .Lload\@
    cmp x2, x3
    b.ls .Lload\@
    prfm pldl1strm, [x1, x3]
.Lload\@:
    ldnp q0, q1, [x1]
    add x1, x1, #32
.endif
    stnp q0, q1, [x0]
    add x0, x0, #32
    sub x2, x2, #32
    b .Lbulk\@
.Ltail\@:
    cbz x2, .Ldone\@
.if \copy
    ldrb w4, [x1], #1
    strb w4, [x0], #1
.else
    strb wzr, [x0], #1
.endif
    sub x2, x2, #1
    b .Ltail\@
.Ldone\@:
    ret
.size \name,.-\name
.endm
transfer pryx_page_copy_nt, 1
transfer pryx_page_clear_nt, 0
/* Arm ARM DCZID_EL0 permits at most 2 KiB per zero block. Restrict DC ZVA
 * to whole, aligned 2 KiB regions even if the CPU changes within the call.
 * Ordinary stores cover both boundaries. Native runs pin workers before timing. */
.global pryx_page_zva64
.type pryx_page_zva64,%function
pryx_page_zva64:
    mrs x0, dczid_el0
    and x0, x0, #31
    cmp x0, #4
    cset x0, eq
    ret
.size pryx_page_zva64,.-pryx_page_zva64

.global pryx_page_clear_zva
.type pryx_page_clear_zva,%function
pryx_page_clear_zva:
    mrs x4, dczid_el0
    and x4, x4, #31
    cmp x4, #4
    b.ne pryx_page_clear_nt
.Lzva_head:
    cbz x2, .Lzva_done
    tst x0, #2047
    b.eq .Lzva_interior
    strb wzr, [x0], #1
    sub x2, x2, #1
    b .Lzva_head
.Lzva_interior:
    and x4, x2, #-2048
    cbz x4, pryx_page_clear_nt
    sub x2, x2, x4
.Lzva_loop:
    dc zva, x0
    add x0, x0, #64
    subs x4, x4, #64
    b.ne .Lzva_loop
    b pryx_page_clear_nt
.Lzva_done:
    ret
.size pryx_page_clear_zva,.-pryx_page_clear_zva

.section .note.GNU-stack,"",%progbits

/* Linux System V page kernels. Every wide access is preceded by a remaining-length
 * check. Streaming stores peel the destination to alignment and fence before return. */
.text
.macro compare name, width, paired
.global \name
.type \name, @function
\name:
    xor eax, eax
.if !\paired
    movzx esi, sil
    vmovd xmm0, esi
.if \width == 32
    vpbroadcastb ymm0, xmm0
.else
    vpbroadcastb zmm0, esi
.endif
.endif
.Lblock\@:
    cmp rdx, \width
    jb .Ltail\@
.if \width == 32
.if \paired
    vmovdqu ymm0, ymmword ptr [rsi + rax]
.endif
    vpcmpeqb ymm1, ymm0, ymmword ptr [rdi + rax]
    vpmovmskb ecx, ymm1
    not ecx
    test ecx, ecx
    jnz .Lfound\@
.else
.if \paired
    vmovdqu64 zmm0, zmmword ptr [rsi + rax]
.endif
    vpcmpneqb k1, zmm0, zmmword ptr [rdi + rax]
    kmovq rcx, k1
    test rcx, rcx
    jnz .Lfound\@
.endif
    add rax, \width
    sub rdx, \width
    jmp .Lblock\@
.Lfound\@:
    bsf rcx, rcx
    add rax, rcx
    jmp .Ldone\@
.Ltail\@:
    test rdx, rdx
    jz .Ldone\@
.if \paired
    mov cl, byte ptr [rsi + rax]
    cmp byte ptr [rdi + rax], cl
.else
    cmp byte ptr [rdi + rax], sil
.endif
    jne .Ldone\@
    inc rax
    dec rdx
    jmp .Ltail\@
.Ldone\@:
    vzeroupper
    ret
.size \name, .-\name
.endm
compare pryx_page_diff_avx2, 32, 1
compare pryx_page_value_avx2, 32, 0
compare pryx_page_diff_avx512, 64, 1
compare pryx_page_value_avx512, 64, 0

/* FNV-1a on four parallel little-endian word streams. The caller seeds and
 * folds the lanes and handles any final group shorter than sixteen bytes. */
.global pryx_page_hash_lanes
.type pryx_page_hash_lanes,@function
pryx_page_hash_lanes:
    vmovdqu xmm0, xmmword ptr [rdx]
    mov eax, 16777619
    vmovd xmm1, eax
    vpbroadcastd xmm1, xmm1
    test rsi, rsi
    jz .Lhashdone
.Lhash:
    vpxor xmm0, xmm0, xmmword ptr [rdi]
    vpmulld xmm0, xmm0, xmm1
    add rdi, 16
    dec rsi
    jnz .Lhash
.Lhashdone:
    vmovdqu xmmword ptr [rdx], xmm0
    ret
.size pryx_page_hash_lanes,.-pryx_page_hash_lanes

.macro transfer name, width, copy
.global \name
.type \name,@function
\name:
.if !\copy && \width > 8
    vpxor ymm0, ymm0, ymm0
.endif
    xor eax, eax
.Lalign\@:
    test rdx, rdx
    jz .Lfinish\@
    test rdi, \width - 1
    jz .Lbulk\@
.Lbyte\@:
.if \copy
    mov al, byte ptr [rsi]
    inc rsi
.endif
    mov byte ptr [rdi], al
    inc rdi
    dec rdx
    jmp .Lalign\@
.Lbulk\@:
    cmp rdx, \width
    jb .Lbyte\@
.if \copy
    test rcx, rcx
    jz .Lload\@
    cmp rdx, rcx
    jbe .Lload\@
    prefetchnta [rsi + rcx]
.Lload\@:
.if \width == 32
    vmovdqu ymm0, ymmword ptr [rsi]
.else
    mov rax, qword ptr [rsi]
.endif
    add rsi, \width
.endif
.if \width == 32
    vmovntdq ymmword ptr [rdi], ymm0
.else
    movnti qword ptr [rdi], rax
.endif
    add rdi, \width
    sub rdx, \width
    jmp .Lalign\@
.Lfinish\@:
    sfence
.if \width > 8
    vzeroupper
.endif
    ret
.size \name,.-\name
.endm
transfer pryx_page_copy_nt, 32, 1
transfer pryx_page_clear_nt, 32, 0
transfer pryx_page_copy_gpr, 8, 1
transfer pryx_page_clear_gpr, 8, 0

.global pryx_page_copy_rep
.type pryx_page_copy_rep,@function
pryx_page_copy_rep:
    mov rcx, rdx
    rep movsb
    ret
.size pryx_page_copy_rep,.-pryx_page_copy_rep
.global pryx_page_clear_rep
.type pryx_page_clear_rep,@function
pryx_page_clear_rep:
    mov rcx, rdx
    xor eax, eax
    rep stosb
    ret
.size pryx_page_clear_rep,.-pryx_page_clear_rep
.section .note.GNU-stack,"",@progbits

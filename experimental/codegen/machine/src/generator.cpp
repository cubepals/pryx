// SPDX-License-Identifier: FSL-1.1-ALv2
/** Assemble a bounded page scan selected by host features. Only fixed operations and
 * validated widths/unroll factors enter the generator; the Rust wrapper handles tails.
 * The handle owns the executable mapping. Every failed emission falls back at the caller. */
#include <asmjit/core.h>
#if defined(__x86_64__)
#include <asmjit/x86.h>
#elif defined(__aarch64__)
#include <asmjit/a64.h>
#endif
#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <new>

using namespace asmjit;
using Scan = uint32_t (*)(const uint8_t*, size_t);

struct Handler final : ErrorHandler {
    bool failed = false;
    void handle_error(Error, const char*, BaseEmitter*) override { failed = true; }
};

struct Kernel {
    void* image = nullptr;
    size_t mapped_size = 0;
    size_t code_size = 0;
    ~Kernel() {
        if (image) (void)VirtMem::release(image, mapped_size);
    }
};

#if defined(__x86_64__)
static void emit_scan(CodeHolder& code, unsigned width, unsigned count) {
    using namespace x86;
    Assembler a(&code);
    Label loop = a.new_label();
    if (width == 64) a.vpxord(zmm0, zmm0, zmm0);
    else if (width == 32) a.vpxor(ymm0, ymm0, ymm0);
    else a.pxor(xmm0, xmm0);
    a.bind(loop);
    for (unsigned i = 0; i < count; ++i) {
        auto memory = ptr(rdi, int(i * width));
        if (width == 64) a.vpord(zmm0, zmm0, memory);
        else if (width == 32) a.vpor(ymm0, ymm0, memory);
        else { a.movdqu(xmm1, memory); a.por(xmm0, xmm1); }
    }
    a.add(rdi, width * count);
    a.dec(rsi);
    a.jnz(loop);
    if (width == 64) { a.vptestmb(k1, zmm0, zmm0); a.kortestq(k1, k1); a.setz(al); a.vzeroupper(); }
    else if (width == 32) { a.vptest(ymm0, ymm0); a.setz(al); a.vzeroupper(); }
    else {
        a.pxor(xmm1, xmm1); a.pcmpeqb(xmm0, xmm1); a.pmovmskb(eax, xmm0);
        a.cmp(eax, 65535); a.setz(al);
    }
    a.movzx(eax, al);
    a.ret();
}
#elif defined(__aarch64__)
static void emit_scan(CodeHolder& code, unsigned, unsigned count) {
    using namespace a64;
    Assembler a(&code);
    Label loop = a.new_label();
    a.movi(v0.b16(), 0);
    a.bind(loop);
    for (unsigned i = 0; i < count; ++i) {
        a.ldr(q1, ptr(x0, int(16 * i)));
        a.orr(v0.b16(), v0.b16(), v1.b16());
    }
    a.add(x0, x0, 16 * count); a.subs(x1, x1, 1); a.b_ne(loop);
    a.umaxv(b0, v0.b16()); a.umov(w0, v0.b(0)); a.cmp(w0, 0);
    a.cset(w0, CondCode::kEQ); a.ret(x30);
}
#endif

#if defined(__x86_64__)
static void emit_copy(CodeHolder& code, unsigned width, unsigned count) {
    using namespace x86;
    Assembler a(&code);
    if (width == 1) {
        a.mov(rcx, rdx);
        a.rep().movsb();
    } else {
        Label loop = a.new_label();
        a.bind(loop);
        for (unsigned i = 0; i < count; ++i) {
            const auto src = ptr(rsi, int(i * width));
            const auto dst = ptr(rdi, int(i * width));
            if (width == 64) { a.vmovdqu64(zmm0, src); a.vmovdqu64(dst, zmm0); }
            else if (width == 32) { a.vmovdqu(ymm0, src); a.vmovdqu(dst, ymm0); }
            else { a.movdqu(xmm0, src); a.movdqu(dst, xmm0); }
        }
        a.add(rsi, width * count); a.add(rdi, width * count); a.dec(rdx); a.jnz(loop);
        if (width >= 32) a.vzeroupper();
    }
    a.ret();
}
#elif defined(__aarch64__)
static void emit_copy(CodeHolder& code, unsigned, unsigned count) {
    using namespace a64;
    Assembler a(&code);
    Label loop = a.new_label();
    a.bind(loop);
    for (unsigned i = 0; i < count; ++i) {
        a.ldr(q0, ptr(x1, int(16 * i))); a.str(q0, ptr(x0, int(16 * i)));
    }
    a.add(x1, x1, 16 * count); a.add(x0, x0, 16 * count);
    a.subs(x2, x2, 1); a.b_ne(loop); a.ret(x30);
}
#endif

static Kernel* generate(unsigned width, unsigned count, bool copy) noexcept {
    if (count != 1 && count != 2 && count != 4) return nullptr;
#if defined(__aarch64__)
    if (width != 16) return nullptr;
#else
    if (width != 16 && width != 32 && width != 64 && !(copy && width == 1 && count == 1)) return nullptr;
#endif
    void* storage = std::malloc(sizeof(Kernel));
    if (!storage) return nullptr;
    Kernel* kernel = new(storage) Kernel;
    Handler handler;
    CodeHolder code;
    if (code.init(Environment::host()) == Error::kOk) {
        code.set_error_handler(&handler);
        if (copy) emit_copy(code, width, count);
        else emit_scan(code, width, count);
        const size_t page = VirtMem::info().page_size;
        if (!handler.failed && code.flatten() == Error::kOk &&
            code.resolve_cross_section_fixups() == Error::kOk && code.code_size() <= page) {
            // The fixed emitter fits one page and has no externally supplied instructions.
            kernel->mapped_size = page;
            if (VirtMem::alloc(&kernel->image, page, VirtMem::MemoryFlags::kAccessRW) == Error::kOk &&
                code.relocate_to_base(reinterpret_cast<uintptr_t>(kernel->image)) == Error::kOk &&
                code.copy_flattened_data(kernel->image, page) == Error::kOk &&
                VirtMem::protect(kernel->image, page, VirtMem::MemoryFlags::kAccessRX) == Error::kOk) {
                kernel->code_size = code.code_size();
                VirtMem::flush_instruction_cache(kernel->image, kernel->code_size);
                return kernel;
            }
        }
    }
    kernel->~Kernel();
    std::free(kernel);
    return nullptr;
}

extern "C" Kernel* pryx_generate_scan(unsigned width, unsigned count) noexcept {
    return generate(width, count, false);
}
extern "C" Kernel* pryx_generate_copy(unsigned width, unsigned count) noexcept {
    return generate(width, count, true);
}
using Copy = void (*)(uint8_t*, const uint8_t*, size_t);
extern "C" Copy pryx_copy_entry(const Kernel* kernel) { return reinterpret_cast<Copy>(kernel->image); }
extern "C" Scan pryx_scan_entry(const Kernel* kernel) { return reinterpret_cast<Scan>(kernel->image); }
extern "C" const uint8_t* pryx_scan_bytes(const Kernel* kernel, size_t* size) {
    *size = kernel->code_size;
    return static_cast<const uint8_t*>(kernel->image);
}
extern "C" void pryx_free_scan(Kernel* kernel) {
    kernel->~Kernel();
    std::free(kernel);
}

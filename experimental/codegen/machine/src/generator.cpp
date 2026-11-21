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
    JitAllocator::CreateParams params{.options = JitAllocatorOptions::kUseDualMapping};
    JitRuntime runtime{&params};
    Scan scan = nullptr;
};

#if defined(__x86_64__)
static void emit(CodeHolder& code, unsigned width, unsigned count) {
    using namespace x86;
    Assembler a(&code);
    Label loop = a.new_label();
    if (width == 32) a.vpxor(ymm0, ymm0, ymm0);
    else a.pxor(xmm0, xmm0);
    a.bind(loop);
    for (unsigned i = 0; i < count; ++i) {
        auto memory = ptr(rdi, int(i * width));
        if (width == 32) a.vpor(ymm0, ymm0, memory);
        else { a.movdqu(xmm1, memory); a.por(xmm0, xmm1); }
    }
    a.add(rdi, width * count);
    a.dec(rsi);
    a.jnz(loop);
    if (width == 32) { a.vptest(ymm0, ymm0); a.setz(al); a.vzeroupper(); }
    else {
        a.pxor(xmm1, xmm1); a.pcmpeqb(xmm0, xmm1); a.pmovmskb(eax, xmm0);
        a.cmp(eax, 65535); a.setz(al);
    }
    a.movzx(eax, al);
    a.ret();
}
#elif defined(__aarch64__)
static void emit(CodeHolder& code, unsigned, unsigned count) {
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

extern "C" Kernel* pryx_generate_scan(unsigned width, unsigned count) noexcept {
    if (count != 1 && count != 2 && count != 4) return nullptr;
#if defined(__aarch64__)
    if (width != 16) return nullptr;
#else
    if (width != 16 && width != 32) return nullptr;
#endif
    void* storage = std::malloc(sizeof(Kernel));
    if (!storage) return nullptr;
    Kernel* kernel = new(storage) Kernel;
    Handler handler;
    CodeHolder code;
    if (code.init(kernel->runtime.environment()) == Error::kOk) {
        code.set_error_handler(&handler);
        emit(code, width, count);
        if (!handler.failed && kernel->runtime.add(&kernel->scan, &code) == Error::kOk) return kernel;
    }
    kernel->~Kernel();
    std::free(kernel);
    return nullptr;
}

extern "C" Scan pryx_scan_entry(const Kernel* kernel) { return kernel->scan; }
extern "C" void pryx_free_scan(Kernel* kernel) {
    kernel->~Kernel();
    std::free(kernel);
}

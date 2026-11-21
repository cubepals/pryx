// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Linux vector loads for complete blocks. The caller handles the scalar tail.
//! Parts (`machine/src/`): `lib.rs` dispatch, `x86_64.S` SSE2, `aarch64.S` NEON.

const WIDTH: usize = 16;

#[cfg(all(target_os = "linux", not(miri), target_arch = "x86_64"))]
core::arch::global_asm!(include_str!("x86_64.S"), options(att_syntax));
#[cfg(all(target_os = "linux", not(miri), target_arch = "aarch64"))]
core::arch::global_asm!(include_str!("aarch64.S"));

pub fn all_zero(bytes: &[u8]) -> Option<bool> {
    #[cfg(all(target_os = "linux", not(miri), any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        #[cfg(target_arch = "x86_64")]
        let available = std::is_x86_feature_detected!("sse2");
        #[cfg(target_arch = "aarch64")]
        let available = std::arch::is_aarch64_feature_detected!("neon");
        let blocks = bytes.len() / WIDTH * WIDTH;
        if available && blocks > 0 {
            unsafe extern "C" {
                fn pryx_proof_zero_blocks(data: *const u8, len: usize) -> u32;
            }
            // Each load lies in the first `blocks` bytes of this live slice.
            // The kernel expects a positive byte count divisible by WIDTH.
            let head_zero = unsafe { pryx_proof_zero_blocks(bytes.as_ptr(), blocks) } != 0;
            return Some(head_zero && bytes[blocks..].iter().all(|&b| b == 0));
        }
    }
    let _ = (bytes, WIDTH);
    None
}

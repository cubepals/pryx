// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Caps checked at construction keep unsupported ISA variants out of safe callers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Scalar,
    Library,
    #[cfg(all(
        not(miri),
        target_os = "linux",
        target_endian = "little",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    Vector,
    #[cfg(all(not(miri), target_os = "linux", target_arch = "x86_64"))]
    Avx2,
    #[cfg(all(not(miri), target_os = "linux", target_arch = "x86_64"))]
    Avx512,
    #[cfg(all(not(miri), target_os = "linux", target_endian = "little", target_arch = "aarch64"))]
    Neon,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Backend(pub(crate) Kind);

impl Backend {
    pub const SCALAR: Self = Self(Kind::Scalar);
    pub const LIBRARY: Self = Self(Kind::Library);

    pub fn available() -> Vec<Self> {
        #[allow(unused_mut)]
        let mut found = vec![Self::SCALAR, Self::LIBRARY];
        #[cfg(all(not(miri), target_os = "linux", target_arch = "x86_64"))]
        if std::is_x86_feature_detected!("avx2") {
            found.extend([Self(Kind::Vector), Self(Kind::Avx2)]);
            if std::is_x86_feature_detected!("avx512f") && std::is_x86_feature_detected!("avx512bw") {
                found.push(Self(Kind::Avx512));
            }
        }
        #[cfg(all(not(miri), target_os = "linux", target_endian = "little", target_arch = "aarch64"))]
        if std::arch::is_aarch64_feature_detected!("neon") {
            found.extend([Self(Kind::Vector), Self(Kind::Neon)]);
        }
        found
    }
    pub fn name(self) -> &'static str {
        match self.0 {
            Kind::Scalar => "scalar",
            Kind::Library => "library",
            #[cfg(all(
                not(miri),
                target_os = "linux",
                target_endian = "little",
                any(target_arch = "x86_64", target_arch = "aarch64")
            ))]
            Kind::Vector => "rust-vector",
            #[cfg(all(not(miri), target_os = "linux", target_arch = "x86_64"))]
            Kind::Avx2 => "asm-avx2",
            #[cfg(all(not(miri), target_os = "linux", target_arch = "x86_64"))]
            Kind::Avx512 => "asm-avx512-scan",
            #[cfg(all(not(miri), target_os = "linux", target_endian = "little", target_arch = "aarch64"))]
            Kind::Neon => "asm-neon",
        }
    }
}

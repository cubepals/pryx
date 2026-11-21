// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Shared paired-file harness for the standalone candidate and the pinned A1 baselines.

#[cfg(feature = "experimental")]
mod sleep;
#[cfg(feature = "experimental")]
pub use sleep::{Baseline, sample};

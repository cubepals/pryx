// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Coverage-guided differential checks on native scan inputs.

#![no_main]
libfuzzer_sys::fuzz_target!(|bytes: &[u8]| pryx_lab_proof::check_input(bytes));

// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Runs the installed bounded scan self-check only when named.

fn main() -> std::process::ExitCode {
    if !std::env::args_os().skip(1).eq([std::ffi::OsString::from("self-check")]) {
        eprintln!("usage: pryx-lab self-check");
        return std::process::ExitCode::from(2);
    }
    let mut page = [0; 4097];
    pryx_lab_proof::check_input(&page[1..]);
    for index in [0, 15, 16, 2048, 4096] {
        page[index] = 0x80;
        pryx_lab_proof::check_input(&page);
        page[index] = 0;
    }
    println!("self-check passed");
    std::process::ExitCode::SUCCESS
}

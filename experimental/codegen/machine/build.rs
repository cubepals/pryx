// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Build the explicitly selected, pinned assembler for this native CI probe.

use std::{env, path::PathBuf, process::Command};

fn main() {
    for part in ["src", "CMakeLists.txt"] {
        println!("cargo:rerun-if-changed={part}");
    }
    println!("cargo:rerun-if-env-changed=ASMJIT_SOURCE");
    if env::var_os("CARGO_CFG_MIRI").is_some() {
        return;
    }
    assert_eq!(env::var("HOST"), env::var("TARGET"), "build on a native runner");
    let source = PathBuf::from(env::var_os("ASMJIT_SOURCE").expect("set ASMJIT_SOURCE to the pinned checkout"))
        .canonicalize()
        .unwrap();
    let tip = Command::new("git").arg("-C").arg(&source).args(["rev-parse", "HEAD"]).output().unwrap();
    assert!(tip.status.success());
    assert_eq!(String::from_utf8(tip.stdout).unwrap().trim(), "677a10bac2c24f7e068cf1f9e675bf51c7b534ea");
    let clean = Command::new("git").arg("-C").arg(&source).args(["status", "--porcelain"]).output().unwrap();
    assert!(clean.status.success() && clean.stdout.is_empty(), "assembler checkout must be pristine");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let cmake = Command::new("cmake")
        .args(["-S", ".", "-B"])
        .arg(&out)
        .arg(format!("-DASMJIT_SOURCE={}", source.display()))
        .arg("-DCMAKE_BUILD_TYPE=Release")
        .status()
        .unwrap();
    assert!(cmake.success());
    assert!(Command::new("cmake").arg("--build").arg(&out).args(["--parallel", "2"]).status().unwrap().success());
    println!("cargo:rustc-link-search=native={}", out.join("lib").display());
    println!("cargo:rustc-link-lib=static=pryx_page_machine");
    println!("cargo:rustc-link-lib=static=asmjit");
}

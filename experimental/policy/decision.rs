// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2
//! A two-key classic BPF decision tree for the workload's measured getpid and bind.
//! A shortcut is eligible only when the original action is derivable from architecture
//! and syscall number alone. Every unmatched input flows into the unchanged original body.

use seccompiler::{BpfProgram, sock_filter};
use std::mem::offset_of;

const LOAD: u16 = (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16;
const RET: u16 = (libc::BPF_RET | libc::BPF_K) as u16;
const EQ: u16 = (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16;
const ARCH: u32 = offset_of!(libc::seccomp_data, arch) as u32;
const NR: u32 = offset_of!(libc::seccomp_data, nr) as u32;
const JA: u16 = (libc::BPF_JMP | libc::BPF_JA) as u16;
const GT: u16 = (libc::BPF_JMP | libc::BPF_JGT | libc::BPF_K) as u16;
const GE: u16 = (libc::BPF_JMP | libc::BPF_JGE | libc::BPF_K) as u16;
const AND: u16 = (libc::BPF_ALU | libc::BPF_AND | libc::BPF_K) as u16;

fn evaluate(program: &[sock_filter], word: impl Fn(u32) -> Option<u32>) -> Option<u32> {
    let mut pc = 0usize;
    let mut accumulator = 0;
    loop {
        let op = program.get(pc)?;
        let jump = match op.code {
            LOAD => {
                accumulator = word(op.k)?;
                0
            }
            AND => {
                accumulator &= op.k;
                0
            }
            RET => return Some(op.k),
            JA => op.k,
            EQ | GT | GE => {
                let yes = match op.code {
                    EQ => accumulator == op.k,
                    GT => accumulator > op.k,
                    _ => accumulator >= op.k,
                };
                u32::from(if yes { op.jt } else { op.jf })
            }
            _ => return None,
        };
        pc = pc.checked_add(1)?.checked_add(jump as usize)?;
    }
}

pub(super) fn tree(stock: &[sock_filter]) -> Result<BpfProgram, String> {
    let [arch_load, native, foreign, nr_load, ..] = stock else {
        return Err("unrecognized filter preamble".into());
    };
    if !(arch_load.code == LOAD
        && arch_load.k == ARCH
        && native.code == EQ
        && native.jt == 1
        && native.jf == 0
        && foreign.code == RET
        && foreign.k == libc::SECCOMP_RET_KILL_PROCESS
        && nr_load.code == LOAD
        && nr_load.k == NR)
    {
        return Err("unrecognized native architecture guard".into());
    }
    let mut result = stock[..4].to_vec();
    for nr in [libc::SYS_getpid as u32, libc::SYS_bind as u32] {
        let action = evaluate(stock, |offset| match offset {
            NR => Some(nr),
            ARCH => Some(native.k),
            _ => None,
        });
        if let Some(action) = action {
            result.push(sock_filter { code: EQ, jt: 0, jf: 1, k: nr });
            result.push(sock_filter { code: RET, jt: 0, jf: 0, k: action });
        }
    }
    result.extend_from_slice(&stock[4..]);
    check(stock, &result, native.k)?;
    Ok(result)
}

fn check(stock: &[sock_filter], candidate: &[sock_filter], native: u32) -> Result<(), String> {
    // Include every syscall rule key emitted by this version of the compiler, neighbouring
    // numbers, common low syscall numbers and a deterministic range of high input words.
    let mut numbers: Vec<u32> = (0..1024).collect();
    numbers.extend([u32::MAX, i32::MAX as u32, 1 << 31]);
    for op in stock.iter().filter(|op| op.code == EQ) {
        numbers.extend([op.k.wrapping_sub(1), op.k, op.k.wrapping_add(1)]);
    }
    numbers.sort_unstable();
    numbers.dedup();
    let mut seed = 0x7dc8_2d19u32;
    for arch in [native, native ^ 1] {
        for &nr in &numbers {
            for fixed in [Some(0), Some(1), Some(u32::MAX), None] {
                let word = |offset| match offset {
                    NR => Some(nr),
                    ARCH => Some(arch),
                    _ if offset.is_multiple_of(4) && (offset as usize) < size_of::<libc::seccomp_data>() => {
                        Some(fixed.unwrap_or_else(|| seed.rotate_left(offset)))
                    }
                    _ => None,
                };
                let original = evaluate(stock, word).ok_or("unsupported reference instruction or load")?;
                if evaluate(candidate, word) != Some(original) {
                    return Err(format!("candidate action differs at syscall {nr}"));
                }
            }
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_distinguishes_two_argument_words() {
        let arg = offset_of!(libc::seccomp_data, args) as u32;
        let seed = 0x7dc8_2d19u32;
        let stmt = |code, jt, jf, k| sock_filter { code, jt, jf, k };
        let stock = [
            stmt(LOAD, 0, 0, arg),
            stmt(EQ, 0, 3, seed.rotate_left(arg)),
            stmt(LOAD, 0, 0, arg + 4),
            stmt(EQ, 0, 1, seed.rotate_left(arg + 4)),
            stmt(RET, 0, 0, libc::SECCOMP_RET_ERRNO),
            stmt(RET, 0, 0, libc::SECCOMP_RET_ALLOW),
        ];
        assert!(check(&stock, &stock[5..], 0).is_err());
    }

    #[test]
    fn conditional_shortcut_cannot_ignore_the_argument() {
        let stmt = |code, jt, jf, k| sock_filter { code, jt, jf, k };
        let stock = [
            stmt(LOAD, 0, 0, ARCH),
            stmt(EQ, 1, 0, 10),
            stmt(RET, 0, 0, libc::SECCOMP_RET_KILL_PROCESS),
            stmt(LOAD, 0, 0, NR),
            stmt(EQ, 0, 3, libc::SYS_bind as u32),
            stmt(LOAD, 0, 0, offset_of!(libc::seccomp_data, args) as u32),
            stmt(EQ, 0, 1, 99),
            stmt(RET, 0, 0, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32),
            stmt(RET, 0, 0, libc::SECCOMP_RET_ALLOW),
        ];
        let new = tree(&stock).unwrap();
        assert_eq!(new.len(), stock.len() + 2, "only getpid is independent of its arguments");
        let input = |offset| match offset {
            ARCH => Some(10),
            NR => Some(libc::SYS_bind as u32),
            _ => Some(99),
        };
        assert_eq!(evaluate(&new, input), evaluate(&stock, input));
        assert_eq!(evaluate(&new, input), Some(libc::SECCOMP_RET_ERRNO | libc::EPERM as u32));
    }
}

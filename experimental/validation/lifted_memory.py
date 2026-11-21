# SPDX-License-Identifier: FSL-1.1-ALv2
"""Check reachable Arm instructions against the memory effects of the unoptimized lift.

Only ordinary loads, stores and the non-memory instructions used by the scan and its controls
are accepted. This check compares direction and width. The executor still models addresses,
writeback and register semantics. An unfamiliar instruction makes the proof inconclusive.
"""
from collections import defaultdict
from functools import lru_cache

from capstone.arm64 import ARM64_OP_MEM, ARM64_OP_REG

READS = {"ldr", "ldrb", "ldur", "ldurb", "ldp"}
WRITES = {"str", "strb", "stur", "sturb", "stp"}
NO_DATA_ACCESS = {
    "add", "and", "ands", "b", "b.eq", "b.hs", "b.ne", "bl", "cbnz", "cbz", "cmp",
    "cset", "mov", "movi", "orr", "ret", "sub", "subs", "umaxv", "umov",
}


def transferred_bytes(insn):
    if not any(operand.type == ARM64_OP_MEM for operand in insn.operands):
        raise ValueError(f"unsupported Arm addressing form at pc {insn.address:#x}")
    if insn.mnemonic.endswith("b"):
        return 1
    count = 2 if insn.mnemonic in ("ldp", "stp") else 1
    widths = {"w": 4, "x": 8, "q": 16}
    total = 0
    for operand in insn.operands[:count]:
        if operand.type != ARM64_OP_REG:
            raise ValueError(f"unsupported Arm data operand at pc {insn.address:#x}")
        register = insn.reg_name(operand.reg)
        if register in ("fp", "lr"):
            total += 8
            continue
        if register[:1] not in widths:
            raise ValueError(f"unsupported Arm register width at pc {insn.address:#x}")
        total += widths[register[0]]
    return total


@lru_cache(maxsize=4096)
def check_block(project, address):
    block = project.factory.block(address, opt_level=0)
    effects = defaultdict(lambda: {"read": 0, "write": 0})
    lifted_sites = set()
    for stmt in block.vex.statements:
        if stmt.tag == "Ist_IMark":
            pc = stmt.addr + stmt.delta
            lifted_sites.add(pc)
        elif stmt.tag == "Ist_Store":
            effects[pc]["write"] += stmt.data.result_size(block.vex.tyenv) // 8
        elif stmt.tag in ("Ist_LoadG", "Ist_StoreG", "Ist_CAS", "Ist_LLSC"):
            raise ValueError(f"unsupported memory operation in the Arm lift at pc {pc:#x}")
        for expr in stmt.expressions:
            if expr.tag == "Iex_Load":
                effects[pc]["read"] += expr.result_size(block.vex.tyenv) // 8
    decoded = block.capstone.insns
    if not decoded or {insn.address for insn in decoded} != lifted_sites:
        raise ValueError(f"unmatched Arm instruction boundaries in block {address:#x}")
    for insn in decoded:
        expected = {"read": 0, "write": 0}
        if insn.mnemonic in READS:
            expected["read"] = transferred_bytes(insn)
        elif insn.mnemonic in WRITES:
            expected["write"] = transferred_bytes(insn)
        elif insn.mnemonic not in NO_DATA_ACCESS or any(op.type == ARM64_OP_MEM for op in insn.operands):
            raise ValueError(f"unsupported Arm instruction at pc {insn.address:#x}: {insn.mnemonic}")
        for direction, count in expected.items():
            if count != effects[insn.address][direction]:
                raise ValueError(f"unmodeled Arm memory {direction} at pc {insn.address:#x}: {insn.mnemonic}")

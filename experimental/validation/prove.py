#!/usr/bin/env python3
"""Check compiled zero-test return values and memory bounds for lengths 0 through 64.

Every input byte is an unconstrained bit-vector. Every branch must return, and
the solver covers the union of all return conditions. The model permits stack
access for ordinary call/return and forbids writes to the input. It covers the
chosen ELF, calling convention and pointer placements, independently per target.
"""
import argparse
import hashlib
import json
from pathlib import Path

import angr
import claripy

from lifted_memory import check_block

BASE = 0x300000
STACK = 0x7FEF0000
STOP = 0x70001110


def model_low_byte():
    """Complete one VEX operation as specified in libvex_ir.h."""
    from angr.engines.vex.claripy import irop

    class PreserveLowByte:
        _float = False

        def calculate(self, value):
            if value.size() != 128:
                raise ValueError("unexpected VEX operand width")
            return claripy.ZeroExt(120, value[7:0])

    irop.operations.setdefault("Iop_ZeroHI120ofV128", PreserveLowByte())


def check(project, entry, length, alignment):
    address_base = BASE + alignment
    opts = {angr.options.ZERO_FILL_UNCONSTRAINED_REGISTERS}
    state = project.factory.call_state(entry, address_base, length, ret_addr=STOP, stack_base=STACK,
                                       prototype="unsigned int scan(char*, size_t)", add_options=opts)
    data = [claripy.BVS(f"b{i}", 8) for i in range(length)]
    for index, byte in enumerate(data):
        state.memory.store(address_base + index, byte)
    predicate = claripy.And(*(byte == 0 for byte in data))
    expected = claripy.If(predicate, claripy.BVV(1, 32), claripy.BVV(0, 32))

    def access(s, write):
        address = s.inspect.mem_write_address if write else s.inspect.mem_read_address
        size = s.inspect.mem_write_length if write else s.inspect.mem_read_length
        # Both permitted intervals are small, so subtraction cannot admit wraparound.
        def inside(start, end):
            return claripy.And(address.UGE(start), address.ULE(end), (end - address).UGE(size))
        if size is None and write and s.inspect.mem_write_expr is not None:
            bits = s.inspect.mem_write_expr.size()
            if bits % 8 == 0:
                size = bits // 8
        if size is None:
            raise ValueError("unbounded memory operation")
        safe = inside(STACK - 65536, STACK + 64)
        if not write:
            safe = claripy.Or(safe, inside(address_base, address_base + length))
            if project.arch.name == "AMD64" and not address.symbolic:
                # The PIE uses linker slots for intra-ELF calls. Exempt only a
                # RIP-relative indirect call/jump through its actual GOT.
                section = project.loader.find_section_containing(s.solver.eval(address))
                if section and section.name in (".got", ".got.plt"):
                    insn = project.factory.block(s.addr, num_inst=1).capstone.insns[0]
                    if insn.mnemonic in ("call", "jmp") and "[rip" in insn.op_str and size == 8:
                        return
        if s.solver.satisfiable(extra_constraints=[claripy.Not(safe)]):
            raise ValueError(f"memory bounds at pc {s.addr:#x}, length {length}")

    state.inspect.b("mem_read", when=angr.BP_BEFORE, action=lambda s: access(s, False))
    state.inspect.b("mem_write", when=angr.BP_BEFORE, action=lambda s: access(s, True))
    manager = project.factory.simulation_manager(state, save_unconstrained=True)
    for _ in range(512):
        manager.move(from_stash="active", to_stash="returned", filter_func=lambda s: s.addr == STOP)
        if not manager.active:
            break
        if project.arch.name == "AARCH64":
            for active in manager.active:
                check_block(project, active.addr)
            manager.step(opt_level=0)
        else:
            manager.step()
    paths = manager.stashes.get("returned", [])
    if manager.errored:
        raise RuntimeError(manager.errored[0].error)
    if manager.active or manager.deadended or manager.unconstrained or not paths:
        raise RuntimeError(f"unfinished execution at length {length}")
    covered = []
    for path in paths:
        result = path.regs.eax if project.arch.name == "AMD64" else path.regs.w0
        if path.solver.satisfiable(extra_constraints=[result != expected]):
            raise ValueError(f"wrong predicate at length {length}")
        covered.append(claripy.And(*path.solver.constraints))
    if claripy.Solver().satisfiable(extra_constraints=[claripy.Not(claripy.Or(*covered))]):
        raise RuntimeError(f"incomplete input coverage for length {length}")
    return len(paths)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("elf", type=Path)
    parser.add_argument("--alignment", type=int, choices=range(16), default=1)
    parser.add_argument("--stock-vex", action="store_true", help="disable the explicit low-byte IR model")
    args = parser.parse_args()
    if not args.stock_vex:
        model_low_byte()
    project = angr.Project(str(args.elf), auto_load_libs=False)
    if project.arch.name not in ("AMD64", "AARCH64"):
        raise ValueError("supported ELF architectures are AMD64 and AARCH64")
    symbol = project.loader.main_object.get_symbol("pryx_prove_scan")
    if not symbol:
        raise ValueError("missing pryx_prove_scan entry")
    paths = [check(project, symbol.rebased_addr, length, args.alignment) for length in range(65)]
    print(json.dumps({"architecture": project.arch.name, "angr": angr.__version__, "lengths": len(paths),
                      "pointer_mod_16": args.alignment, "return_paths": sum(paths), "stock_vex": args.stock_vex,
                      "checker_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                      "arm_memory_audit_sha256": hashlib.sha256(Path(__file__).with_name("lifted_memory.py").read_bytes()).hexdigest(),
                      "elf_sha256": hashlib.sha256(args.elf.read_bytes()).hexdigest()}))


if __name__ == "__main__":
    main()

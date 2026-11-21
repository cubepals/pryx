# SPDX-License-Identifier: FSL-1.1-ALv2
"""Add scratch-word block coverage exclusively to the two pinned test-run packet hooks.

Retain every original instruction and map reference, relocate intra-section jumps and
preserve both scratch registers around each update. A unique 64-bit path rank describes
exact visit order for the acyclic graph. This module never loads or attaches a program.
"""
import hashlib
import struct
import subprocess

DIGEST = "4f0d730fbd73c819fbf494c6ac76905da1be30c41bd0f0c00dfd6a7e540d933b"
HOOKS = ("ingress", "egress")
INSN = struct.Struct("<BBhi")
SECTION = struct.Struct("<IIQQQQIIQQ")
SYMBOL = struct.Struct("<IBBHQQ")
RELOC = struct.Struct("<QQ")


def cfg(code):
    instructions = {}
    starts = {0}
    n = len(code) // INSN.size
    pc = 0
    while pc < n:
        op, registers, offset, immediate = INSN.unpack_from(code, pc * 8)
        width = 2 if op == 0x18 else 1
        successors = [pc + width]
        if op & 7 in (5, 6):
            jump = op & 0xf0
            if jump == 0x80:
                if registers >> 4:
                    raise ValueError("unexpected subprogram call")
            elif jump == 0x90:
                successors = []
            else:
                target = pc + 1 + offset
                successors = [target] if jump == 0 or target == pc + 1 else [pc + 1, target]
                starts.update(successors)
            if jump not in (0x80,) and pc + 1 < n:
                starts.add(pc + 1)
        instructions[pc] = (width, successors)
        pc += width
    starts = sorted(starts)
    blocks = []
    for index, begin in enumerate(starts):
        stop = starts[index + 1] if index + 1 < len(starts) else n
        pcs = list(pc for pc in instructions if begin <= pc < stop)
        if not pcs or pcs[0] != begin:
            raise ValueError("branch outside a real instruction")
        blocks.append({"entry": begin, "pcs": pcs, "next": instructions[pcs[-1]][1]})
    graph = {b["entry"]: b["next"] for b in blocks}
    done, active = set(), set()
    def visit(node):
        if node in active:
            raise ValueError("a cyclic packet program needs per-visit counters")
        if node in done:
            return
        active.add(node)
        for other in graph[node]:
            visit(other)
        active.remove(node)
        done.add(node)
    visit(0)
    return instructions, blocks


def path_counts(blocks):
    graph = {b["entry"]: b["next"] for b in blocks}
    count = {}
    def paths(node):
        if node not in count:
            count[node] = sum(paths(to) for to in graph[node]) or 1
        return count[node]
    if paths(0) > (1 << 64):
        raise ValueError("finite CFG path rank exceeds the kernel witness field")
    return count


def executed_path(blocks, rank):
    graph = {b["entry"]: b for b in blocks}
    limit = path_counts(blocks)
    if not 0 <= rank < limit[0]:
        raise ValueError("invalid kernel path rank")
    trace = []
    block = graph[0]
    while True:
        trace.append(block)
        successors = block["next"]
        if not successors:
            if rank != 0:
                raise ValueError("incomplete path rank")
            return trace
        if len(successors) == 1:
            next_pc = successors[0]
        else:
            fall, taken = successors
            if rank < limit[taken]:
                next_pc = taken
            else:
                rank -= limit[taken]
                next_pc = fall
        block = graph[next_pc]


def patch(code, cb, lookup, storage):
    original, blocks = cfg(code)
    counts = path_counts(blocks)
    output = bytearray()
    def emit(op, dst, src, off=0, imm=0):
        output.extend(INSN.pack(op, dst | src << 4, off, imm))
    def signed(word):
        return word if word < 0x80000000 else word - 0x100000000
    # The pinned programs reach -72 and -120 respectively. The diagnostic reserves
    # four disjoint spill slots below -480. The source CFG is acyclic.
    emit(0x7b, 10, 1, -512)
    emit(0x7a, 10, 0, -488, 0)
    emit(0xb7, 8, 0)
    emit(0xb7, 9, 0)
    branch_targets, kept = {}, {}
    for pc, (width, successors) in original.items():
        branch_targets[pc] = len(output) // 8
        if code[8*pc] == 0x95:
            # R0 still carries the original action. R8 and R9 are dead at exit.
            emit(0x79, 8, 10, -512)
            emit(0x79, 9, 10, -488)
            emit(0x63, 8, 9, cb)
            emit(0x77, 9, 0, 0, 32)
            emit(0x63, 8, 9, cb + 4)
            # A nonzero witness also distinguishes legitimate rank zero from missing data.
            emit(0x62, 8, 0, cb + 8, 1)
        kept[pc] = len(output) // 8
        output.extend(code[8*pc:8*(pc+width)])
        if len(successors) == 2:
            # Taken paths have ranks [0, count(taken)). Only the original fall-through
            # executes this addition. Both original branch targets remain intact.
            weight = counts[successors[1]]
            emit(0x7b, 10, 9, -496)
            emit(0x79, 9, 10, -488)
            if weight <= 0x7fffffff:
                emit(0x07, 9, 0, 0, weight)
            else:
                emit(0x7b, 10, 8, -504)
                emit(0x18, 8, 0, 0, signed(weight & 0xffffffff))
                emit(0, 0, 0, 0, signed(weight >> 32))
                emit(0x0f, 9, 8)
                emit(0x79, 8, 10, -504)
            emit(0x7b, 10, 9, -488)
            emit(0x79, 9, 10, -496)
    for pc in original:
        op, regs, off, imm = INSN.unpack_from(code, 8 * pc)
        if op & 7 in (5, 6) and (op & 0xf0) not in (0x80, 0x90):
            start = kept[pc]
            target = branch_targets[pc + 1 + off]
            INSN.pack_into(output, start * 8, op, regs, target - start - 1, imm)
    for block in blocks:
        calls = [INSN.unpack_from(code, 8*p)[3] for p in block["pcs"] if code[8*p] == 0x85]
        block["map_lookups"] = calls.count(lookup)
        block["socket_storage_gets"] = calls.count(storage)
    return output, kept, branch_targets, blocks


def build(objcopy, original, destination, cb, lookup, storage):
    if hashlib.sha256(original.read_bytes()).hexdigest() != DIGEST:
        raise ValueError("unexpected network object")
    # Kernel loading needs the original BTF map types. Compiler line tables belong to
    # the original PCs and are omitted in the counted test image.
    subprocess.run([objcopy, "--remove-section=.BTF.ext", "--remove-section=.rel.BTF.ext",
                    original, destination], check=True)
    blob = bytearray(destination.read_bytes())
    if blob[:6] != b"\x7fELF\x02\x01":
        raise ValueError("expected 64-bit little endian ELF")
    table = struct.unpack_from("<Q", blob, 40)[0]
    stride, count, names_idx = struct.unpack_from("<HHH", blob, 58)
    if stride != SECTION.size:
        raise ValueError("unexpected ELF section stride")
    sections = [list(SECTION.unpack_from(blob, table+i*stride)) for i in range(count)]
    strings = sections[names_idx][4]
    def name(s):
        start = strings + s[0]
        return bytes(blob[start:blob.index(0, start)]).decode()
    indexes = {name(s): i for i,s in enumerate(sections)}
    result = {}
    for hook in HOOKS:
        index = indexes["cgroup_skb/" + hook]
        section = sections[index]
        start, length = section[4:6]
        output, kept, targets, blocks = patch(blob[start:start+length], cb, lookup, storage)
        result[hook] = blocks
        # Append replacements at the original section alignment. Unchanged sections keep
        # their file offsets and indexes; relocations and symbols now point at the new code.
        blob.extend(bytes(-len(blob) % section[8]))
        section[4], section[5] = len(blob), len(output)
        blob.extend(output)
        SECTION.pack_into(blob, table+index*stride, *section)
        rel = sections[indexes[".relcgroup_skb/" + hook]]
        for pos in range(rel[4], rel[4]+rel[5], RELOC.size):
            offset, symbol = RELOC.unpack_from(blob, pos)
            RELOC.pack_into(blob, pos, 8*kept[offset//8], symbol)
        sym = sections[indexes[".symtab"]]
        for pos in range(sym[4], sym[4]+sym[5], SYMBOL.size):
            nm, info, other, sh, value, size = SYMBOL.unpack_from(blob, pos)
            if sh != index:
                continue
            if info & 15 == 2:  # sole global function in this hook's section
                if value or size != length:
                    raise ValueError("unexpected function layout")
                value, size = 0, len(output)
            elif value:
                value = 8*targets[value//8]
            SYMBOL.pack_into(blob, pos, nm, info, other, sh, value, size)
    destination.write_bytes(blob)
    return result

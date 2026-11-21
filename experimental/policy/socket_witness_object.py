# SPDX-License-Identifier: FSL-1.1-ALv2
"""Construct a diagnostic clone of the one signed policy ELF for seven fixed successful UDP paths.

Every deviation from each predeclared path sets a private stack flag. Only the original
exits consult the separate witness map. No hook or program image is accepted from the CLI.
"""
import hashlib
import os
from pathlib import Path
import struct
import subprocess

from counted_object import DIGEST, INSN, SECTION, SYMBOL, RELOC, cfg

HOOKS = ("sock_create", "connect4", "connect6", "sendmsg4", "sendmsg6", "recvmsg4", "recvmsg6")
V4 = ((0, 7), (17, 41), (280, 302), (418, 426), (313, 330), (542, 542))
V6 = ((0, 16), (26, 53), (292, 309), (432, 439), (541, 567), (630, 630))
SPANS = {"connect4": V4, "sendmsg4": V4, "connect6": V6, "sendmsg6": V6,
         "sock_create": ((0, 3), (17, 31), (41, 43)), "recvmsg4": ((0, 12), (26, 27)),
         "recvmsg6": ((0, 5), (32, 33))}
EXPECTED = dict(zip(HOOKS, (20, 82, 97, 82, 97, 14, 8)))
# The measured original verifier depths span 8..168 bytes. Two 4-byte
# slots near the end of the frame stay private and the disjoint key and flag are initialized before use.
FLAG, KEY = -496, -504


class Elf:
    def __init__(self, file):
        self.data = bytearray(file.read_bytes())
        if self.data[:6] != b'\x7fELF\x02\x01' or struct.unpack_from('<H', self.data, 18)[0] != 247:
            raise ValueError("expected a little-endian 64-bit BPF object")
        self.table, = struct.unpack_from('<Q', self.data, 40)
        stride, count, names = struct.unpack_from('<HHH', self.data, 58)
        if stride != SECTION.size:
            raise ValueError("ELF section stride")
        self.headers = [list(SECTION.unpack_from(self.data, self.table+i*stride)) for i in range(count)]
        def string(table, start):
            offset = self.headers[table][4] + start
            return bytes(self.data[offset:self.data.index(0, offset)]).decode()
        self.idx = {string(names, s[0]): i for i,s in enumerate(self.headers)}
        sym = self.headers[self.idx['.symtab']]
        self.syms = [list(SYMBOL.unpack_from(self.data, i)) for i in range(sym[4], sym[4]+sym[5], SYMBOL.size)]
        self.names = [string(sym[6], s[0]) for s in self.syms]

    def section(self, name):
        offset, size = self.headers[self.idx[name]][4:6]
        return bytes(self.data[offset:offset+size])

    def relocs(self, name):
        return list(RELOC.iter_unpack(self.section('.rel'+name)))

    def named_relocs(self, name):
        return [(offset, self.names[ref >> 32], ref & 0xffffffff) for offset, ref in self.relocs(name)]

    def replace(self, name, payload):
        index=self.idx[name]
        header=self.headers[index]
        self.data.extend(bytes(-len(self.data) % header[8]))
        header[4:6]=[len(self.data),len(payload)]
        self.data.extend(payload)
        SECTION.pack_into(self.data,self.table+index*SECTION.size,*header)


def instrument(stock, name, map_symbol):
    code=stock.section('cgroup/'+name)
    original,_=cfg(code)
    route=[p for lo,hi in SPANS[name] for p in original if lo<=p<=hi]
    if len(route)!=EXPECTED[name] or len(set(route))!=len(route) or route[0]!=0 or original[route[-1]][1]:
        raise ValueError("predeclared stock path changed")
    follows=dict(zip(route,route[1:]))
    for p,next_pc in follows.items():
        if next_pc not in original[p][1]: raise ValueError(f"{name}: invalid recorded edge {p} -> {next_pc}")
    code_out=bytearray()
    actions=[]
    targets,kept,probes={},{},[]
    def emit(op,dst=0,src=0,off=0,imm=0):
        pos=len(code_out)//8
        code_out.extend(INSN.pack(op,dst | src<<4,off,imm))
        return pos
    emit(0x62,10,off=FLAG)
    for pc,(width,successors) in original.items():
        op,regs,off,imm=INSN.unpack_from(code,8*pc)
        if op==0x85 and (imm==12 or regs>>4):
            raise ValueError("unsupported tail call or subprogram")
        targets[pc]=len(code_out)//8
        if op==0x95:
            # At a stock exit R0 is the original scalar action. R6 is callee-saved.
            emit(0xbf,6,0)
            emit(0x62,10,off=KEY,imm=HOOKS.index(name))
            probes.append((8*emit(0x18,1),map_symbol<<32|1))
            emit(0)
            emit(0xbf,2,10)
            emit(0x07,2,imm=KEY)
            emit(0x85,imm=1)                    # UAPI BPF_FUNC_map_lookup_elem
            emit(0x15,0,off=7)                  # missing map entry: preserve original action
            emit(0x79,1,0)
            emit(0x07,1,imm=1)
            emit(0x7b,0,1)
            emit(0x79,1,0,off=8)
            emit(0x61,2,10,off=FLAG)
            emit(0x0f,1,2)
            emit(0x7b,0,1,off=8)
            emit(0xbf,0,6)
        kept[pc]=len(code_out)//8
        code_out.extend(code[8*pc:8*(pc+width)])
        if op&7 not in (5,6) or op&0xf0 in (0x80,0x90):
            continue
        taken=pc+1+off
        if len(successors)==2 and pc in follows:
            if follows[pc]==taken:
                # Missing the expected branch executes this flag store, then the original fall-through.
                actions.append((kept[pc],taken))
                emit(0x62,10,off=FLAG,imm=1)
            else:
                # Only an unexpected taken edge visits the marker, then continues to its original target.
                INSN.pack_into(code_out,8*kept[pc],op,regs,1,imm)
                actions.append((emit(0x05),successors[0]))
                emit(0x62,10,off=FLAG,imm=1)
                actions.append((emit(0x05),taken))
        else:
            actions.append((kept[pc],taken))
    for ip,pc in actions:
        op,regs,_,imm=INSN.unpack_from(code_out,8*ip)
        INSN.pack_into(code_out,8*ip,op,regs,targets[pc]-ip-1,imm)
    return code_out,targets,kept,probes,route


def build(tool, clang, objcopy, image, scratch):
    if hashlib.sha256(image.read_bytes()).hexdigest()!=DIGEST:
        raise ValueError("only the pinned stock policy is supported")
    map_obj=scratch/'witness-map.o'
    linked=scratch/'linked.o'
    final=scratch/'socket-witness.o'
    include='/usr/include/'+os.uname().machine+'-linux-gnu'
    subprocess.run([clang,'-O2','-g','-target','bpfel','-I'+include,'-c',
                    Path(__file__).with_name('socket_witness.bpf.c'),'-o',map_obj],check=True)
    subprocess.run([tool,'gen','object',linked,image,map_obj],check=True)
    subprocess.run([objcopy,'--remove-section=.BTF.ext','--remove-section=.rel.BTF.ext',linked,final],check=True)
    stock,elf=Elf(image),Elf(final)
    new_map=elf.names.index('socket_witness')
    manifest={}
    for hook in (*HOOKS,'ingress','egress'):
        section=('cgroup/' if hook in HOOKS else 'cgroup_skb/')+hook
        if elf.section(section)!=stock.section(section) or elf.named_relocs(section)!=stock.named_relocs(section):
            raise ValueError("BTF link changed the original program or map references: "+hook)
        if hook not in HOOKS: continue
        code,targets,kept,probes,route=instrument(stock,hook,new_map)
        prior=elf.relocs(section)
        elf.replace(section,code)
        relocated=[(8*kept[p//8],sym) for p,sym in prior]
        elf.replace('.rel'+section,b''.join(RELOC.pack(*r) for r in sorted(relocated+probes)))
        for symbol in elf.syms:
            _,info,_,sh,value,size=symbol
            if sh!=elf.idx[section]: continue
            if info&15==2:
                if value or size!=len(stock.section(section)):raise ValueError("unexpected BPF program symbol")
                symbol[5]=len(code)
            elif value:
                symbol[4]=8*targets[value//8]
        manifest[hook]={'original_instructions':len(route),'original_pcs':route,'witness_key':HOOKS.index(hook),
                        'stock_section_sha256':hashlib.sha256(stock.section(section)).hexdigest(),
                        'witness_section_sha256':hashlib.sha256(code).hexdigest(),'stock_relocations':len(prior),
                        'witness_exits':len(probes)}
    elf.replace('.symtab',b''.join(SYMBOL.pack(*s) for s in elf.syms))
    final.write_bytes(elf.data)
    return final,manifest

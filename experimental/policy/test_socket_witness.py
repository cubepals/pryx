# SPDX-License-Identifier: FSL-1.1-ALv2
"""Offline edge/relocation assurance for the seven exact diagnostic sections. Never loads BPF."""
import argparse
import hashlib
from pathlib import Path
import tempfile

from counted_object import DIGEST, INSN, cfg
from socket_witness_object import HOOKS, FLAG, KEY, Elf, build, instrument


def edges(stock, diagnostic, name):
    section='cgroup/'+name
    base,actual=stock.section(section),diagnostic.section(section)
    source,_=cfg(base)
    generated,targets,kept,probes,route=instrument(stock,name,diagnostic.names.index('socket_witness'))
    if generated!=actual:
        raise ValueError("linked object does not contain reconstructed instrumented section")
    original_entry={pos:pc for pc,pos in targets.items()}
    expected=dict(zip(route,route[1:]))
    tested=0
    for pc,(width,successors) in source.items():
        ip=kept[pc]
        op,r,offset,imm=INSN.unpack_from(base,8*pc)
        newop,newreg,newoff,newimm=INSN.unpack_from(actual,8*ip)
        assert (newop,newreg,newimm)==(op,r,imm)
        branch=op&7 in (5,6) and op&0xf0 not in (0x80,0x90)
        if not branch: assert actual[8*ip:8*(ip+width)]==base[8*pc:8*(pc+width)]
        for to in successors:
            if branch and to==pc+1+offset:
                nxt=ip+1+newoff
            else:
                nxt=ip+width
            marked=False
            for _ in range(4):
                if nxt in original_entry:break
                o,regs,off,val=INSN.unpack_from(actual,8*nxt)
                if o==0x05 and regs==val==0:
                    nxt+=1+off
                elif (o,regs,off,val)==(0x62,10,FLAG,1):
                    marked=True
                    nxt+=1
                else:
                    raise ValueError(f'{name}:{pc}: non-witness effect on branch edge')
            assert original_entry[nxt]==to
            assert marked==(pc in expected and to!=expected[pc]),(name,pc,to)
            tested+=1

    original_relocations=[(8*kept[offset//8],symbol,kind) for offset,symbol,kind in stock.named_relocs(section)]
    added=[(offset,'socket_witness',1) for offset,_ in probes]
    assert sorted(diagnostic.named_relocs(section))==sorted(original_relocations+added)
    assert INSN.unpack_from(actual,0)==(0x62,10,FLAG,0)
    assert all(targets[pc]==kept[pc] for pc in source if source[pc][1])
    for pc,(_,successors) in source.items():
        if successors:continue
        for action in (0,1):
            for marked in (0,1):
                for have_map in (False,True):
                    exit_case(actual,targets[pc],kept[pc],HOOKS.index(name),action,marked,have_map)
    return tested


def exit_case(actual,ip,last,key,action,flag,have_map):
    # Dry-run both possible diagnostic helper results. Unknown opcodes or any access
    # outside known witness slots/map members fail the test.
    fp,map_ptr=0x10000,0x20000
    reg={0:action,10:fp}
    memory={fp+FLAG:flag,map_ptr:7,map_ptr+8:2}
    steps=0
    while ip<=last:
        assert steps<36
        steps+=1
        op,regs,off,imm=INSN.unpack_from(actual,ip*8)
        dst,src=regs&15,regs>>4
        if op==0x95:
            assert ip==last and reg[0]==action
            assert memory=={fp+KEY:key,fp+FLAG:flag,map_ptr:7+int(have_map),map_ptr+8:2+flag*int(have_map)}
            return
        if op==0xbf:reg[dst]=reg[src]
        elif op==0x62:
            assert dst==10 and off==KEY
            memory[fp+off]=imm
        elif op==0x18:
            assert dst==1 and src==0 and off==imm==0 and INSN.unpack_from(actual,(ip+1)*8)==(0,0,0,0)
            reg[dst]='private_map_fd';ip+=1
        elif op==0x07:reg[dst]+=imm
        elif op==0x85:
            assert imm==1 and reg[1]=='private_map_fd' and memory[reg[2]]==key
            for r in range(1,6):reg.pop(r,None)
            reg[0]=map_ptr if have_map else 0
        elif op==0x15:
            if reg[dst]==imm:ip+=off
        elif op in (0x79,0x61):reg[dst]=memory[reg[src]+off]
        elif op==0x0f:reg[dst]+=reg[src]
        elif op==0x7b:
            assert dst==0 and off in (0,8)
            memory[reg[dst]+off]=reg[src]
        else: raise ValueError("unmodeled exit probe instruction")
        ip+=1
    raise ValueError("missing original exit")


def verify(image,tool,clang,objcopy):
    if hashlib.sha256(image.read_bytes()).hexdigest()!=DIGEST:raise ValueError("unexpected stock object")
    with tempfile.TemporaryDirectory(prefix='pryx-socket-witness-check-') as tmp:
        result,_=build(tool,clang,objcopy,image,Path(tmp))
        stock,diagnostic=Elf(image),Elf(result)
        for hook in HOOKS: print(hook, edges(stock,diagnostic,hook), "checked original successor edges")
        for name in ('ingress','egress'):
            section='cgroup_skb/'+name
            assert stock.section(section)==diagnostic.section(section)
            assert stock.named_relocs(section)==diagnostic.named_relocs(section)


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--bpftool',required=True)
    p.add_argument('--clang',default='clang-18')
    p.add_argument('--objcopy',default='llvm-objcopy-18')
    args=p.parse_args()
    verify(Path(__file__).resolve().parents[2]/'bpf/network.bpf.o',args.bpftool,args.clang,args.objcopy)

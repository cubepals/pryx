# SPDX-License-Identifier: FSL-1.1-ALv2
"""Observe fixed socket paths under a fresh loopback-only cgroup. No candidate input or timing."""
import argparse
from contextlib import ExitStack
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import tempfile

from socket_hooks import PACKET_HOOKS, SOCKET_HOOKS, SocketUAPI, local_rules, run_child
from socket_witness_object import DIGEST, EXPECTED, HOOKS, Elf, build

ROUNDS = 16
STOCK_COUNTERS = 5  # PRYX_COUNTERS in the pinned network.h


def save(file, value):
    file.write_text(json.dumps(value, indent=2)+'\n')


def read_map(tool, pins, name):
    return json.loads(subprocess.check_output([tool,'-j','map','dump','pinned',pins/name]))


def totals(raw, count):
    result={}
    for row in raw:
        key,=struct.unpack('<I',bytes(int(v,16) for v in row['key']))
        values=[struct.unpack('<QQ',bytes(int(v,16) for v in cpu['value'])) for cpu in row['values']]
        if key in result or not values or len({r['cpu'] for r in row['values']})!=len(values):
            raise ValueError("malformed per-CPU map or duplicate key")
        result[key]=[sum(pair[i] for pair in values) for i in range(2)]
    if set(result)!=set(range(count)):raise ValueError("missing keys in per-CPU map")
    return result


def key_values(raw):
    entries = {}
    for row in raw:
        key = bytes(int(value, 16) for value in row['key'])
        if key in entries:
            raise ValueError("duplicate ordinary map key")
        entries[key] = bytes(int(value, 16) for value in row['value'])
    return entries


def fixed_maps(tool, maps):
    values = {name: read_map(tool, maps, name) for name in ('rules', 'config', 'peers')}
    expected = {
        struct.pack('<I', 128) + ipaddress.IPv6Address(address).packed: bytes([2]) + bytes(199)
        for address in ('::ffff:127.0.0.1', '::1')
    }
    if (key_values(values['rules']) != expected or key_values(values['config']) != {bytes(4): bytes(96)}
            or values['peers']):
        raise ValueError("expected the two stock OWNED rules, zero configuration and an empty peer map")
    return values


def program_info(response, fresh):
    if isinstance(response, list):
        response, = response
    counts = 'run_cnt' in response
    times = 'run_time_ns' in response
    if counts != times or (not counts and fresh is False):
        raise ValueError("expected complete BPF runtime stats after the socket workload")
    count = int(response.get('run_cnt', 0))
    duration = int(response.get('run_time_ns', 0))
    if ((fresh is True and (count != 0 or duration != 0))
            or (fresh is False and (count <= 0 or duration <= 0))
            or count < 0 or duration < 0):
        raise ValueError("unexpected observations on fresh or exercised diagnostic program")
    return {'id':response['id'],'tag':response['tag'],'run_cnt':count,'map_ids':response.get('map_ids',[])}


def capture(tool,image,out,diagnostic,references):
    api=SocketUAPI()
    out.mkdir()
    with tempfile.TemporaryDirectory(dir='/sys/fs/bpf',prefix='pryx-socket-path-') as scratch, ExitStack() as owned:
        pins,maps=Path(scratch)/'programs',Path(scratch)/'maps'
        maps.mkdir()
        with (out/'verifier.log').open('xb') as log:
            subprocess.run([tool,'--debug','prog','loadall',image,pins,'pinmaps',maps],
                           stdout=log,stderr=subprocess.STDOUT,check=True)
        expected_maps = {'rules', 'config', 'counters', 'owners', 'sockets', 'peers'}
        if diagnostic:
            expected_maps.add('socket_witness')
        if {p.name for p in maps.iterdir()} != expected_maps or {p.name for p in pins.iterdir()} != set(SOCKET_HOOKS + PACKET_HOOKS):
            raise ValueError("unexpected programs or policy maps")
        map_ids = {}
        for name in sorted(expected_maps):
            row = json.loads(subprocess.check_output([tool, '-j', 'map', 'show', 'pinned', maps/name]))
            if isinstance(row, list):
                row, = row
            if row['name'] != name or row['id'] <= 0 or row['id'] in map_ids.values():
                raise ValueError("unexpected private map identity: "+name)
            map_ids[name] = row['id']
        save(out/'map-ids.json', map_ids)
        local_rules(tool,maps/'rules')
        cgroup=Path(tempfile.mkdtemp(dir='/sys/fs/cgroup',prefix='pryx-socket-path-'))
        owned.callback(cgroup.rmdir)
        group=os.open(cgroup,os.O_RDONLY|os.O_DIRECTORY)
        owned.callback(os.close,group)
        for hook in SOCKET_HOOKS+PACKET_HOOKS:
            fd=api.open_program(pins/hook)
            owned.callback(os.close,fd)
            owned.callback(os.close,api.attach(hook,fd,group))
        owned.callback(os.close,api.runtime_stats())

        def observe(label):
            programs = {}
            for name in SOCKET_HOOKS + PACKET_HOOKS:
                raw = subprocess.check_output([tool,'-j','prog','show','pinned',pins/name])
                fresh = {'before': True, 'after': False, 'partial': None}[label]
                programs[name] = program_info(json.loads(raw), fresh)
                used = references[name] | ({'socket_witness'} if diagnostic and name in HOOKS else set())
                if set(programs[name]['map_ids']) != {map_ids[n] for n in used}:
                    raise ValueError("loaded program refers to unexpected map IDs: "+name)
            state={'programs':programs,'counters':read_map(tool,maps,'counters'),**fixed_maps(tool,maps)}
            if diagnostic:state['witness']=read_map(tool,maps,'socket_witness')
            save(out/(label+'.json'),state)
            return state
        before=observe('before')
        # The same existing stock UDP function checks payload/peer/connection and closes every fd.
        try:
            run_child(cgroup,ROUNDS)
        except (RuntimeError, OSError):
            try:
                observe('partial')
            except (ValueError, OSError, subprocess.CalledProcessError) as error:
                save(out/'partial-unavailable.json', {'reason': str(error)})
            raise
        after=observe('after')
    calls={}
    for name,a in after['programs'].items():
        b=before['programs'][name]
        if (a['id'],a['tag'])!=(b['id'],b['tag']):raise ValueError("pinned program identity changed")
        calls[name]=a['run_cnt']-b['run_cnt']
    save(out/'invocations.json', calls)
    if any(any(v) for v in totals(before['counters'],STOCK_COUNTERS).values()):
        raise ValueError("stock counter map was not fresh")
    return calls,after,before


def run(args):
    if (os.geteuid()!=0 or os.getenv('GITHUB_REPOSITORY')!='cubepals/pryx' or os.getenv('GITHUB_ACTIONS')!='true'
            or os.getenv('GITHUB_EVENT_NAME')!='workflow_dispatch'):
        raise ValueError("requires the manually dispatched native pryx CI worker")
    image=Path(__file__).resolve().parents[2]/'bpf/network.bpf.o'
    if hashlib.sha256(image.read_bytes()).hexdigest()!=DIGEST:raise ValueError("unexpected stock policy")
    for path,fs in (('/sys/fs/bpf','bpf'),('/sys/fs/cgroup','cgroup2')):
        if not any(line.split()[4]==path and line.split()[-3]==fs for line in Path('/proc/self/mountinfo').read_text().splitlines()):
            raise ValueError("missing expected private fixture mount "+path)
    args.output.mkdir(parents=True,exist_ok=False)
    out=args.output.resolve()
    try:
        os.unshare(os.CLONE_NEWNET)
        if socket.if_nameindex()!=[(1,'lo')]:raise ValueError("requires a new isolated loopback-only net namespace")
        subprocess.run(['ip','link','set','dev','lo','up'],check=True)
        os.sched_setaffinity(0,{min(os.sched_getaffinity(0))})
        with tempfile.TemporaryDirectory(prefix='pryx-socket-object-') as scratch:
            diagnostic,manifest=build(args.bpftool,args.clang,args.objcopy,image,Path(scratch))
            save(out/'build.json',{'stock_sha256':DIGEST,'diagnostic_sha256':hashlib.sha256(diagnostic.read_bytes()).hexdigest(),
                                   'hooks':manifest})
            stock_elf = Elf(image)
            references = {}
            for name in SOCKET_HOOKS + PACKET_HOOKS:
                section = ('cgroup/' if name in HOOKS else 'cgroup_skb/') + name
                references[name] = {symbol for _, symbol, _ in stock_elf.named_relocs(section)}
            stock,initial,_=capture(args.bpftool,image,out/'stock',False,references)
            observed,end,start=capture(args.bpftool,diagnostic,out/'diagnostic',True,references)
            if (any(stock[n] <= 0 or stock[n] != observed[n] for n in PACKET_HOOKS)
                    or totals(initial['counters'],STOCK_COUNTERS)!=totals(end['counters'],STOCK_COUNTERS)):
                raise ValueError("stock and diagnostic packet observables differ")
            old,new=totals(start['witness'],len(HOOKS)),totals(end['witness'],len(HOOKS))
            if any(any(v) for v in old.values()):raise ValueError("private witness map was not initially empty")
            results={}
            for key,hook in enumerate(HOOKS):
                exits,bad=new[key]
                expected=ROUNDS*(4 if hook=='sock_create' else 1)
                accepted=exits==observed[hook]==stock[hook]==expected and bad==0
                results[hook]={'stock_invocations':stock[hook],'diagnostic_invocations':observed[hook],'witnessed_exits':exits,'off_path_exits':bad,
                               'original_instructions_per_event':EXPECTED[hook] if accepted else None}
            report={'stock_sha256':DIGEST,'scope':'diagnostic clone of fixed stock ELF, positive OWNED loopback UDP only',
                    'denied_workload_cases':0,'socket_storage':'not enumerated after this worker closes its socket FDs',
                    'rounds':ROUNDS,'cpu':min(os.sched_getaffinity(0)),'hooks':results,
                    'stock_and_diagnostic_udp_results_and_stock_counters_equal':True}
            save(out/'socket-instructions.json',report)
            if any(v['original_instructions_per_event'] is None for v in results.values()):
                raise ValueError("unwitnessed invocation or observed divergent path; see per-hook evidence")
    except (OSError,subprocess.CalledProcessError,ValueError,RuntimeError) as exc:
        save(out/'unavailable.json',{'reason':str(exc),'stock_sha256':DIGEST})
        raise


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output',type=Path)
    parser.add_argument('--bpftool',required=True)
    parser.add_argument('--clang',default='clang-18')
    parser.add_argument('--objcopy',default='llvm-objcopy-18')
    run(parser.parse_args())

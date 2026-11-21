# SPDX-License-Identifier: FSL-1.1-ALv2
"""Compare only the named local-TCP egress candidate with the checked-in network policy.

The packet fixture runs both programs using the kernel's packet test API and private maps.
The socket fixture attaches the original and candidate to their own isolated workloads.
"""
import argparse
from contextlib import ExitStack, contextmanager
import hashlib
import json
import math
import os
from pathlib import Path
import socket
import statistics
import struct
import subprocess
import tempfile

from candidate_image import NAME, build_images
from corpus import packet, packets
from counted_object import HOOKS
from run_coverage import map_state


def save(path, value):
    path.write_text(json.dumps(value,indent=2)+"\n")


@contextmanager
def loaded(tool, image, folder):
    with tempfile.TemporaryDirectory(prefix="pryx-policy-pair-",dir="/sys/fs/bpf") as temporary:
        pins, maps = Path(temporary)/"programs", Path(temporary)/"maps"
        maps.mkdir()
        with (folder/"verifier.log").open("xb") as log:
            subprocess.run([tool,"--debug","prog","loadall",image,pins,"pinmaps",maps],
                           stdout=log,stderr=subprocess.STDOUT,check=True)
        yield pins, maps


def context_layout(folder):
    executable = folder/"skb-layout"
    subprocess.run(["cc","-Werror","-Wall",Path(__file__).with_name("layout.c"),"-o",executable],check=True)
    size = int(subprocess.check_output([executable],text=True).split()[0])
    offset = int(subprocess.check_output([executable,"ifindex"],text=True))
    executable.unlink()
    return size,offset


def extra_packets():
    for version in (4,6):
        for kind,flags in (("syn",0x02),("syn-ack",0x12),("fin-ack",0x11),("rst",0x04)):
            yield f"v{version}-{kind}",packet(version,socket.IPPROTO_TCP,b"",tcp_flags=flags)


def packet_tests(tool, images, output):
    output.mkdir()
    raw = list(packets())
    inputs = dict([*raw,*extra_packets()])
    size,offset = context_layout(output)
    # Select one real device in the native runner for a non-loopback skb test context.
    # BPF_PROG_TEST_RUN consumes fixture bytes without transmitting any packet.
    index,device = next(((i,n) for i,n in socket.if_nameindex() if n != "lo"),(None,None))
    if index is None:
        raise RuntimeError("no second interface for the registered off-loopback context")
    loopback = socket.if_nametoindex("lo")
    if loopback != 1 or index == loopback:
        raise ValueError("unexpected loopback or second-interface context")
    contexts = {"loopback":0,"other":index}
    save(output/"corpus.json",{"contexts":contexts,"nonloopback_interface":device,"pairs":31,"repeat":1000})
    for label,data in inputs.items():
        (output/(label+".frame")).write_bytes(data)
    for name,i in contexts.items():
        context = bytearray(size)
        struct.pack_into("=I",context,offset,i)
        (output/(name+".ctx")).write_bytes(context)
    samples=[]
    cases=[]
    with ExitStack() as stack:
        pairs = {}
        for label,image in zip(("stock",NAME),images):
            folder=output/label
            folder.mkdir()
            pairs[label]=stack.enter_context(loaded(tool,image,folder))
        for ctx in contexts:
            for case in inputs:
                for hook in HOOKS:
                    observed=[]
                    for variant,(pins,maps) in pairs.items():
                        folder=output/variant
                        cmd=[tool,"-j","prog","run","pinned",pins/hook,"data_in",output/(case+".frame"),
                             "data_out",folder/"packet.out","data_size_out","4096","ctx_in",output/(ctx+".ctx"),
                             "ctx_out",folder/"context.out","ctx_size_out",str(size),"repeat","1"]
                        result=json.loads(subprocess.check_output(cmd))
                        state={"return":result["retval"],"maps":map_state(tool,maps)}
                        for key in ("packet","context"):
                            state[key]=hashlib.sha256((folder/(key+".out")).read_bytes()).hexdigest()
                        actual = struct.unpack_from("=I",(folder/"context.out").read_bytes(),offset)[0]
                        if actual != (loopback if ctx == "loopback" else index):
                            raise ValueError(f"kernel returned {actual} for {ctx} interface context")
                        observed.append(state)
                    complete={"context":ctx,"case":case,"hook":hook,"stock":observed[0],NAME:observed[1]}
                    cases.append(complete)
                    # The fixture has no rules and owns no sockets in either kernel packet run.
                    # Egress accepts parsed local non-SYN TCP. All remaining controls drop.
                    non_syn_tcp = ("-tcp-" in case or case.endswith(("-fin-ack","-rst")))
                    expected_action = int(hook == "egress" and ctx == "loopback" and non_syn_tcp)
                    if observed[0]!=observed[1] or observed[0]["return"] != expected_action:
                        save(output/"case-mismatch.json",complete)
                        raise ValueError(f"policy observable difference: {ctx}/{case}/{hook}")
        if len(cases)!=96 or {r['stock']['return'] for r in cases if r['hook']=='egress'}!={0,1}:
            raise ValueError("positive and negative egress controls were not both exercised")
        save(output/"equivalence.json",cases)
        with (output/"batches.jsonl").open("x",buffering=1) as journal:
            for pair in range(31):
                row={"pair":pair+1}
                order=("stock",NAME) if pair%2==0 else (NAME,"stock")
                for variant in order:
                    pins,_=pairs[variant]
                    times={}
                    for case,_ in raw:
                        for hook in HOOKS:
                            result=json.loads(subprocess.check_output([tool,"-j","prog","run","pinned",pins/hook,
                                "data_in",output/(case+".frame"),"repeat","1000"]))
                            expected=next(r['stock']['return'] for r in cases if r['context']=='loopback' and
                                          r['case']==case and r['hook']==hook)
                            if result['retval']!=expected or result['duration']<=0:
                                raise ValueError("timed event disagrees with its fixed stock control")
                            times[hook+"/"+case]=result['duration']
                    row[variant]=times
                samples.append(row)
                print(json.dumps(row),file=journal)
        if map_state(tool,pairs['stock'][1])!=map_state(tool,pairs[NAME][1]):
            raise ValueError("timed-workload counter maps differ")
    groups={}
    for hook in ("all",*HOOKS):
        values={}
        for variant in ("stock",NAME):
            points=[statistics.mean(v for k,v in pair[variant].items() if hook=='all' or k.startswith(hook+'/'))
                    for pair in samples]
            values[variant]={"p50":statistics.median(points),"p95":sorted(points)[math.ceil(0.95*len(points))-1]}
        groups[hook]=values
    save(output/"paired-summary.json",groups)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output",type=Path)
    parser.add_argument("--experiment",choices=("stock",NAME),default="stock")
    parser.add_argument("--phase",choices=("packet","socket"),default="packet")
    parser.add_argument("--bpftool",required=True)
    args=parser.parse_args()
    if args.experiment=='stock':
        print("stock policy selected")
        return
    if (os.geteuid()!=0 or os.environ.get('GITHUB_REPOSITORY')!='cubepals/pryx' or
        os.environ.get('GITHUB_ACTIONS')!='true' or os.environ.get('GITHUB_EVENT_NAME')!='workflow_dispatch'):
        parser.error("requires the named candidate on a manually dispatched native pryx worker")
    cpu=min(os.sched_getaffinity(0))
    os.sched_setaffinity(0,{cpu})
    args.output.mkdir(parents=True,exist_ok=False)
    root=Path(__file__).resolve().parents[2]
    images=build_images(root,args.output/'images')
    if args.phase=='packet':
        packet_tests(args.bpftool,images,args.output/'paired')
    else:
        from socket_compare import compare_private
        compare_private(args.bpftool,*images,args.output/'socket')


if __name__=='__main__':
    main()

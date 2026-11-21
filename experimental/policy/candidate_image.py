# SPDX-License-Identifier: FSL-1.1-ALv2
"""Build only the registered local TCP egress reorder from the pinned source and release recipe.

The throwaway output includes the before/after source hashes, verifier input and LLVM
listing. Every other program section and map relocation must match the stock image.
"""
import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess

from counted_object import DIGEST, RELOC, SECTION, SYMBOL

NAME = "local-tcp-ack-first"
CANDIDATE_DIGEST = "1c7f2419490f4f270046fc056cdca0971834be7b2d5d8a31421fa17165164993"
INPUTS = {
    "network.bpf.c": "188ff7af67d04d81c75f75739580d94642234c50bf01360ceec9eb7d2717c71c",
    "network.h": "1acdfaef003083545a0872331322b7984039787c7e5dd766de322017a6daf412",
    "build.sh": "79c45de606a305f78b0ad9dbeff003801739ea9bc6a3398bd1da04cd4278f334",
}
BEFORE = b"else if (packet.protocol == IPPROTO_TCP && tcp_response(skb))"
AFTER = b"else if (packet.protocol == IPPROTO_TCP && ((local && !packet.syn) || tcp_response(skb)))"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def sections(blob):
    if blob[:6] != b"\x7fELF\x02\x01":
        raise ValueError("expected the ELF64 little-endian BPF build")
    table = struct.unpack_from("<Q", blob, 40)[0]
    stride, count, names = struct.unpack_from("<HHH", blob, 58)
    if stride != SECTION.size:
        raise ValueError("unexpected ELF section table")
    headers = [SECTION.unpack_from(blob, table+i*stride) for i in range(count)]
    strings = headers[names][4]
    result = {}
    for section in headers:
        off = strings + section[0]
        name = blob[off:blob.index(0, off)].decode()
        result[name] = blob[section[4]:section[4]+section[5]]
    return result


def relocations(parts, section):
    # Branch layout can renumber ELF symbols. Compare the identity and offset of
    # each relocated map, including its location within the unchanged maps section.
    result = []
    for offset, info in RELOC.iter_unpack(parts.get(".rel"+section, b"")):
        name, kind, other, sh, value, size = SYMBOL.unpack_from(parts[".symtab"], (info >> 32)*SYMBOL.size)
        strings = parts[".strtab"]
        label = strings[name:strings.index(0, name)].decode()
        result.append((offset, info & 0xffffffff, label, kind, other, sh, value, size))
    return result


def build_images(root: Path, directory: Path) -> tuple[Path, Path]:
    directory.mkdir(parents=True, exist_ok=True)
    if any(directory.iterdir()):
        raise ValueError("requires an empty private build directory")
    reference = root/"bpf"/"network.bpf.o"
    if sha(reference.read_bytes()) != DIGEST:
        raise ValueError("unrecognized checked-in release image")
    source = {}
    for name, want in INPUTS.items():
        source[name] = (root/"bpf"/name).read_bytes()
        if sha(source[name]) != want:
            raise ValueError(f"unrecognized BPF build input {name}")
    if source["network.bpf.c"].count(BEFORE) != 1:
        raise ValueError("stock conditional is missing or ambiguous")
    images = []
    for label in ("stock", NAME):
        folder = directory/label
        folder.mkdir()
        for name, data in source.items():
            if label == NAME and name == "network.bpf.c":
                data = data.replace(BEFORE, AFTER)
            (folder/name).write_bytes(data)
        subprocess.run(["sh", "build.sh"], cwd=folder, check=True,
                       env=os.environ | {"CLANG":"clang-18", "LLVM_STRIP":"llvm-strip-18"})
        image = folder/"network.bpf.o"
        if label == "stock" and image.read_bytes() != reference.read_bytes():
            raise ValueError("the native recipe failed to reproduce the shipped object byte-for-byte")
        if label == NAME and sha(image.read_bytes()) != CANDIDATE_DIGEST:
            raise ValueError("compiled candidate differs from the registered Ubuntu 24.04 ELF")
        images.append(image)
        with (folder/"llvm-objdump.txt").open("wb") as out:
            subprocess.run(["llvm-objdump-18","-dr",image],stdout=out,check=True)
    stock, candidate = (sections(i.read_bytes()) for i in images)
    hooks = [key for key in stock if key.startswith(("cgroup/", "cgroup_skb/"))]
    if len(hooks) != 9 or set(stock) != set(candidate):
        raise ValueError("BPF sections differ from the pinned release layout")
    for key in [".maps","license",*hooks]:
        if key == "cgroup_skb/egress":
            if stock[key] == candidate[key]:
                raise ValueError("the candidate was omitted by the compiler")
        elif stock[key] != candidate[key] or relocations(stock,key) != relocations(candidate,key):
            raise ValueError(f"unexpected change to {key}")
    (directory/"source-identity.json").write_text(json.dumps({"candidate":NAME,"stock_object":DIGEST,
        "candidate_object":sha(images[1].read_bytes()),"stock_source":INPUTS["network.bpf.c"],
        "candidate_source":sha(source["network.bpf.c"].replace(BEFORE,AFTER)),"unchanged_programs":8},indent=2)+"\n")
    return tuple(images)

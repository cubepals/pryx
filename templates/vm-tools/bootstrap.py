#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Build the trusted read-only VM bootstrap from the actual static agent executable.

Only fixed directories, the unchanged executable and its own advertised marker enter the
image. OCI layers, environment values, workload configuration and keys never enter it.
"""
import argparse
import hashlib
import io
import json
from pathlib import Path, PurePosixPath
import subprocess
import tarfile

from filesystem import command

# GNU readelf renders the ELF ABI's PT_INTERP segment with this documented type name.
ELF_INTERPRETER_SEGMENT = 'INTERP'


def static(executable):
    headers = subprocess.check_output(['readelf', '--wide', '--program-headers', str(executable)], text=True, timeout=10)
    if any(line.split()[:1] == [ELF_INTERPRETER_SEGMENT] for line in headers.splitlines()):
        raise ValueError('VM bootstrap requires a static agent, with no host ELF interpreter')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--agent', type=Path, required=True)
    parser.add_argument('--network-helper', type=Path, required=True)
    parser.add_argument('--forwarder', type=Path, required=True)
    parser.add_argument('--network-probe', type=Path)
    parser.add_argument('--filesystem-component', type=Path, required=True)
    parser.add_argument('--root', type=Path, required=True)
    args = parser.parse_args()
    agent = args.agent.resolve(strict=True)
    static(agent)
    network_helper = args.network_helper.resolve(strict=True)
    static(network_helper)
    forwarder = args.forwarder.resolve(strict=True)
    static(forwarder)
    metadata = json.loads(subprocess.check_output([str(agent), 'bootstrap-probe'], timeout=10))
    if not metadata.get('vmInitVersion') or not metadata.get('vmVsock'):
        raise ValueError('This actual agent does not support VM bootstrap and transport')
    marker_path = PurePosixPath(metadata['vmBootMarkerPath'])
    if not marker_path.is_absolute() or len(marker_path.parts) != 2:
        raise ValueError('VM agent marker must be one file at the bootstrap root')
    args.root.mkdir(mode=0o700, parents=True, exist_ok=False)
    archive = args.root / 'bootstrap.tar'
    with tarfile.open(archive, 'w') as output:
        for name in ['dev', 'proc', 'sys', 'write', 'run']:
            entry = tarfile.TarInfo(name)
            entry.type, entry.mode = tarfile.DIRTYPE, 0o755
            output.addfile(entry)
        programs = [('init', agent), (metadata['vmNetworkProgram'], network_helper),
                    (metadata['vmPacketProgram'], forwarder)]
        if args.network_probe is not None:
            static(args.network_probe)
            programs.append(('network-probe', args.network_probe))
        for name, executable in programs:
            if PurePosixPath(name).name != name:
                raise ValueError('Bootstrap executable names must be single components')
            entry = tarfile.TarInfo(name)
            entry.mode, entry.size = 0o755, executable.stat().st_size
            with executable.open('rb') as source:
                output.addfile(entry, source)
        data = metadata['vmBootMarker'].encode()
        entry = tarfile.TarInfo(marker_path.name)
        entry.mode, entry.size = 0o444, len(data)
        output.addfile(entry, io.BytesIO(data))
    image = args.root / 'bootstrap.erofs'
    subprocess.run(command(args.filesystem_component / 'tools', 'mkfs.erofs',
                           '--tar=f', '--quiet', image, archive), check=True, timeout=60)
    archive.unlink()
    report = {'scope': 'Trusted bootstrap image only; no VM runtime advertisement',
              'agentSha256': hashlib.sha256(agent.read_bytes()).hexdigest(),
              'networkHelperSha256': hashlib.sha256(network_helper.read_bytes()).hexdigest(),
              'packetForwarderSha256': hashlib.sha256(forwarder.read_bytes()).hexdigest(),
              'imageSha256': hashlib.sha256(image.read_bytes()).hexdigest(),
              'imageBytes': image.stat().st_size, 'metadata': metadata, 'networkProbe': 'network-probe' if args.network_probe else None}
    (args.root / 'bootstrap.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report), flush=True)


if __name__ == '__main__':
    main()

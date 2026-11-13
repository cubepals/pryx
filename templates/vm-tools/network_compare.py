#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Compare real ordinary-user Firecracker networking through vsock and a pasta TAP bridge.

Both paths use the same published kernel, guest image, CPU/memory, MTU and loopback HTTP
fixture. The guest proves DNS and management denial; no production network policy or VM
confinement is claimed. Reports survive later failures and retain all thirty raw samples.
"""
import argparse
from http import HTTPStatus
import http.client
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import ipaddress
import os
from pathlib import Path
import platform
import shutil
import socket
import subprocess
import sys
import threading
import time

from inventory import ROOT
import kernel


class Fixture(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    disable_nagle_algorithm = True

    def do_GET(self):
        if self.path == self.server.metadata['networkHelloPath']:
            block = self.server.metadata['networkHello'].encode()
            remaining = len(block)
        elif self.path == self.server.metadata['networkBulkPath']:
            block, remaining = b'x' * (1024 * 1024), self.server.metadata['networkBulkBytes']
        else:
            self.send_error(HTTPStatus.NOT_FOUND)
            return
        self.send_response(HTTPStatus.OK)
        self.send_header('Content-Length', str(remaining))
        self.end_headers()
        while remaining:
            data = block[:remaining]
            self.wfile.write(data)
            remaining -= len(data)

    def log_message(self, *_):
        pass


def unix_request(path, method, target):
    connection = http.client.HTTPConnection('localhost', timeout=2)
    connection.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    connection.sock.settimeout(2)
    connection.sock.connect(str(path))
    try:
        connection.request(method, target)
        response = connection.getresponse()
        response.read(64 * 1024)
        return response.status
    finally:
        connection.close()


def proxy_ready(process, guest, control, routes):
    deadline = time.monotonic() + 10
    while True:
        if process.poll() is not None:
            raise ValueError('Network proxy exited before readiness; retain its log')
        try:
            if unix_request(control, 'GET', routes['statsPath']) == HTTPStatus.OK:
                break
        except OSError:
            pass
        if time.monotonic() >= deadline:
            raise TimeoutError('Private network proxy management did not become ready')
        time.sleep(0.02)
    if unix_request(guest, 'GET', routes['statsPath']) != HTTPStatus.NOT_FOUND:
        raise ValueError('Guest transport exposes host management statistics')
    if unix_request(guest, 'POST', routes['exposePath']) != HTTPStatus.NOT_FOUND:
        raise ValueError('Guest transport exposes host listener management')


def stop(process):
    if process is not None and process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def compare(args, report, save):
    selection = json.loads((ROOT / 'kernel-selection.json').read_text())
    inspection = json.loads(args.inspection.read_text())
    monitor = kernel.selected(selection, inspection, args.staged, 'monitor')
    guest_kernel = kernel.selected(selection, inspection, args.staged, 'kernel')
    monitor.chmod(0o700)
    metadata = json.loads(kernel.command([args.configurator, '--metadata']))
    routes = json.loads(kernel.command([args.proxy, '--metadata']))
    server = ThreadingHTTPServer(('127.0.0.1', 0), Fixture)
    server.metadata = metadata
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        for mode in ['vsock', 'pasta']:
            path = args.work / mode
            path.mkdir(mode=0o700)
            image = path / 'image'
            image.mkdir()
            fixture = {**metadata['network'], 'mode': mode,
                       'url': f'http://{metadata["network"]["host"]}:{server.server_address[1]}'}
            configuration = path / 'network.json'
            configuration.write_text(json.dumps(fixture) + '\n')
            root = kernel.root_image(args.guest, image, args.mkfs, base=args.inputs / 'alpine.tar.gz',
                                     additions={'netprobe': args.network_probe, 'gvforwarder': args.forwarder,
                                                'spike-network.json': configuration})
            pristine = path / 'pristine.ext4'
            with pristine.open('xb') as output:
                output.truncate(kernel.DISK_BYTES)
            kernel.command(['/usr/sbin/mke2fs', '-q', '-t', 'ext4', '-F', pristine])
            cohort = {'path': mode, 'fixture': fixture, 'samples': [], 'status': 'in-progress'}
            report['paths'].append(cohort)
            save()
            try:
                for index in range(kernel.SAMPLES):
                    writes = path / 'writes.ext4'
                    shutil.copyfile(pristine, writes)
                    proxy = None
                    try:
                        launcher, tap, record, preparation = None, None, None, 0
                        with (path / f'helper-{index:02}.log').open('wb') as helper_log:
                            if mode == 'vsock':
                                guest_socket = Path(str(path / 'v.sock') + '_' + str(metadata['networkPort']))
                                control_socket = path / 'private-api.sock'
                                started = time.monotonic_ns()
                                proxy = subprocess.Popen([str(args.proxy), '--config', str(configuration),
                                                          '--guest-socket', str(guest_socket),
                                                          '--control-socket', str(control_socket)],
                                                         stdout=helper_log, stderr=subprocess.STDOUT)
                                proxy_ready(proxy, guest_socket, control_socket, routes)
                                preparation = (time.monotonic_ns() - started) / 1_000_000
                            else:
                                tap, record = 'fc0', path / 'namespace.json'
                                record.unlink(missing_ok=True)
                                launcher = [str(args.inputs / 'pasta'), '--foreground', '--no-pidns', '-4',
                                            '--ns-ifname', 'uplink', '--address', fixture['device'] + '/' +
                                            str(ipaddress.ip_network(fixture['subnet']).prefixlen),
                                            '--gateway', fixture['gateway'], '--mac-addr', fixture['gatewayMac'],
                                            '--map-host-loopback', fixture['host'], '--mtu', str(fixture['mtu']),
                                            '--dhcp-dns', '--dns-forward', fixture['gateway'],
                                            '-t', 'none', '-u', 'none', '-T', 'none', '-U', 'none', '--',
                                            sys.executable, str(ROOT / 'network_namespace.py'),
                                            '--host-netns', os.readlink('/proc/self/ns/net'),
                                            '--host-uid', str(os.geteuid()), '--record', str(record)]
                            sample = kernel.boot(args.configurator, monitor, guest_kernel, root, writes, path,
                                                 f'{mode}-{index:02}', 1, metadata['reportPort'], tap=tap,
                                                 launcher=launcher, timeout=60, monitor_record=record)
                            sample['helperPreparationMs'] = preparation
                            sample['helperAndGuestReportMs'] = preparation + sample['launchToFilesystemVsockReportMs']
                            if mode == 'vsock':
                                sample['proxyResident'] = next(line for line in
                                    (Path('/proc') / str(proxy.pid) / 'status').read_text().splitlines()
                                    if line.startswith('VmRSS:'))
                                if not all(sample['network'].get(key) is True for key in
                                           ['directManagementDenied', 'gatewayManagementDenied']):
                                    raise ValueError('Both guest management-path denials are mandatory')
                            else:
                                sample['namespace'] = json.loads(record.read_text())
                            cohort['samples'].append(sample)
                            save()
                            print(json.dumps({'path': mode, 'sample': index, 'observation': sample}), flush=True)
                    finally:
                        stop(proxy)
                    writes.unlink()
                cohort['status'] = 'success'
            except Exception as failure:
                cohort['status'], cohort['failure'] = 'failure', str(failure)
            finally:
                save()
        if any(cohort['status'] != 'success' for cohort in report['paths']):
            raise ValueError('Network comparison failed; retained both path results and logs')
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=10)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['inspection', 'staged', 'configurator', 'guest', 'mkfs', 'inputs',
                 'network-probe', 'forwarder', 'proxy', 'work', 'output']:
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    if os.geteuid() == 0:
        raise ValueError('Network comparison must launch as the ordinary runner user')
    args.work.mkdir(mode=0o700)
    report = {'scope': 'Minimal network fixtures only; no production VM/policy/confinement proof',
              'hostKernel': platform.release(), 'uid': os.geteuid(), 'paths': [], 'status': 'in-progress'}

    def save():
        args.output.write_text(json.dumps(report, indent=2) + '\n')

    save()
    try:
        compare(args, report, save)
        report['status'] = 'success'
    except Exception as failure:
        report['status'], report['failure'] = 'failure', str(failure)
        raise
    finally:
        save()


if __name__ == '__main__':
    main()

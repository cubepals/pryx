#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Bounded read-only engine metadata diagnostics. Never prints arbitrary response bodies,
request data, engine labels, environment variables or registry/proxy configuration.
"""
import http.client
import json
import os
import socket
import time


class UnixConnection(http.client.HTTPConnection):
    def connect(self):
        self.sock = socket.socket(socket.AF_UNIX)
        self.sock.settimeout(self.timeout)
        self.sock.connect(os.environ['PRYX_TEST_ENGINE_SOCKET'])


def query(path):
    started = time.monotonic()
    connection = UnixConnection('localhost', timeout=5)
    report = {'path': path}
    value = {}
    try:
        connection.request('GET', path)
        response = connection.getresponse()
        report['status'] = response.status
        value = json.loads(response.read())
        if isinstance(value, dict):
            report['keys'] = sorted(value)
            if path.endswith('/version'):
                report['apiVersion'] = value.get('ApiVersion')
                report['components'] = [{k: item.get(k) for k in ['Name', 'Version']}
                                        for item in value.get('Components', [])]
            elif '/libpod/' in path:
                host = value.get('host', {})
                report['host'] = {k: host.get(k) for k in ['networkBackend', 'cgroupManager', 'cgroupControllers']}
                report['rootless'] = host.get('security', {}).get('rootless')
            else:
                report['enforcement'] = {k: value.get(k) for k in
                                         ['CgroupDriver', 'CgroupVersion', 'MemoryLimit', 'CpuCfsQuota', 'PidsLimit']}
    except (OSError, ValueError, http.client.HTTPException) as error:
        report['errorType'] = type(error).__name__
    finally:
        connection.close()
    report['elapsedMs'] = round((time.monotonic() - started) * 1000, 2)
    print(json.dumps(report), flush=True)
    return value


version = query('/version')
api = version.get('ApiVersion')
if isinstance(api, str):
    query(f'/v{api}/info')
engine = os.environ['PRYX_TEST_ENGINE']
for component in version.get('Components', []):
    if engine == 'podman' and engine in component.get('Name', '').lower():
        query(f"/v{component['Version']}/libpod/info")

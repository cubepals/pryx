#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Serve only a disposable CI tool distribution over certificate-verified loopback HTTPS."""
import argparse
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import ssl

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--directory', required=True)
parser.add_argument('--certificate', required=True)
parser.add_argument('--key', required=True)
parser.add_argument('--port', type=int, default=9877)
options = parser.parse_args()
context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.load_cert_chain(options.certificate, options.key)
with ThreadingHTTPServer(('127.0.0.1', options.port),
                         partial(SimpleHTTPRequestHandler, directory=options.directory)) as server:
    server.socket = context.wrap_socket(server.socket, server_side=True)
    server.serve_forever()

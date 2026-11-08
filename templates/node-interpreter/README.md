# JavaScript interpreter image

Build: `docker build -t alpine:pryx-node-interpreter templates/node-interpreter`.
Register/warm with `pryx template create node-interpreter --image alpine:pryx-node-interpreter`.
Released image: `ghcr.io/cubepals/pryx-node-interpreter:VERSION`.

Layers: the official Node 24.22.1 slim bookworm image is digest-pinned and preserves its MIT and
bundled dependency notices plus Debian copyright files under `/usr/share/doc`. `npm ci` installs
the integrity-pinned IJavascript 5.2.1 (BSD-3-Clause) dependency graph, preserving upstream
LICENSE/NOTICE files. The final FSL-1.1-ALv2 launcher routes only jmp's legacy ZeroMQ import to
ZeroMQ.js 6.8.0's official v5 compatibility API; IJavascript/jp-kernel supplies the whole Jupyter
server. It never invokes the Jupyter CLI.

The complete [Node inventory](../../docs/checks/measurements/k17-node-licences.json) and
[base-package inventory](../../docs/checks/measurements/k17-base-licences.json) record licences
and versions. ZeroMQ.js is MIT AND MPL-2.0: its unmodified corresponding source, including native
libzmq notices and source retrieval/build instructions, is freely available in
[the exact npm package](https://registry.npmjs.org/zeromq/-/zeromq-6.8.0.tgz) and
[upstream](https://github.com/zeromq/zeromq.js/tree/v6.8.0), under
[MPL-2.0](https://www.mozilla.org/en-US/MPL/2.0/). The Rust agent's option-ext source/licence
availability is described in the [Python image notices](../python-interpreter/README.md).
No copyleft source is copied into pryx. An optional `proxy_ca` build secret enables verified
package downloads in a proxy environment without retaining that CA in the image.

[Interpreter usage and maintenance limits](../../docs/interpreter.md).

Charts use Chart.js 4.6.1 (MIT) and napi-rs/canvas 1.1.0 (MIT; bundled Skia and Chromium BSD-3-Clause notices). The image also ships the upstream canvas/Chromium and Skia licence texts in `/opt/pryx/notices`. The [native canvas dependency inventory](../../docs/checks/measurements/k17-canvas-licences.json) records its Rust build graph. A documented Node VM `breakOnSigint` preload gives NEL a non-destructive interrupt; no Jupyter message implementation is added.

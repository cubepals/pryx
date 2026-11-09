# Python interpreter image

Build: `docker build -t alpine:pryx-python-interpreter templates/python-interpreter`.
Register/warm with `pryx template create python-interpreter --image alpine:pryx-python-interpreter`.
Released image: `ghcr.io/cubepals/pryx-python-interpreter:VERSION`.
The node supplies the agent; this image supplies the native kernel and libraries.

Layers: the digest-pinned official CPython 3.12.14 slim bookworm image preserves CPython's PSF
and historical licence text and Debian package copyright files under `/usr/share/doc`.
`requirements.txt` installs hash-pinned wheels without modifying their source, retaining each
`*.dist-info` licence/NOTICE. The final layer contains pryx's FSL-1.1-ALv2 IPython configuration.
Agg and the inline backend produce PNG/SVG; pandas tables also have JSON rows.

[The interpreter reference](../../docs/interpreter.md) lists direct versions/licences.
[Every resolved Python wheel](../../docs/checks/measurements/k17-python-licences.json) and
[base package](../../docs/checks/measurements/k17-base-licences.json) is inventoried. NumPy/SciPy
bundled BLAS/runtime notices remain in their wheel licence files. Certifi 2026.10.23 is an
unmodified MPL-2.0 dependency; its corresponding source and notices are freely available from
[PyPI](https://pypi.org/project/certifi/2026.10.23/#files) and the
[upstream source](https://github.com/certifi/python-certifi). pryx copies no copyleft source.
The Rust agent's option-ext 0.2.0 dependency is MPL-2.0; its unmodified source is available at
[crates.io](https://crates.io/crates/option-ext/0.2.0) under the
[MPL-2.0 terms](https://www.mozilla.org/en-US/MPL/2.0/).

An optional build secret `proxy_ca` can supply a network environment's CA bundle. It is used
only during wheel download and is not installed in the output image. TLS verification stays on.

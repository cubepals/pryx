# SPDX-License-Identifier: FSL-1.1-ALv2
FROM mirror.gcr.io/library/python:3.12.14-slim-bookworm@sha256:392307d22300de8b5986851a12d9176dfc0fc073e65bf6523ebd7dcbeb23564e
WORKDIR /opt/pryx
COPY requirements.txt .
COPY ipython/profile_default/ipython_config.py /opt/pryx/ipython/profile_default/ipython_config.py
# Optional session CA is a build secret; it is never retained in the released layer.
RUN --mount=type=secret,id=proxy_ca \
    set -eu; chmod -R a+rX /opt/pryx/ipython; \
    if [ -f /run/secrets/proxy_ca ]; then export PIP_CERT=/run/secrets/proxy_ca; fi; \
    pip install --no-cache-dir --only-binary=:all: --require-hashes -r requirements.txt
ENV IPYTHONDIR=/opt/pryx/ipython MPLBACKEND=Agg PRYX_KERNEL_PRESTART=python
# The sandbox's user has no account here, so an engine gives it the read-only root as its home,
# and matplotlib warns and starts a new temporary configuration and font cache on every import.
# Its home is the sandbox's data, writable and kept, as the process tier gives it: a template's
# build fills the cache once and every sandbox made from it keeps it.
ENV HOME=/data
WORKDIR /data
CMD ["sleep", "infinity"]

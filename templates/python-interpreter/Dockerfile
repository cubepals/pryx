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
WORKDIR /data
CMD ["sleep", "infinity"]

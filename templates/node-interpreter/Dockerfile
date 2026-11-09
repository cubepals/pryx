# SPDX-License-Identifier: FSL-1.1-ALv2
FROM mirror.gcr.io/library/node:24.22.1-bookworm-slim@sha256:be8204d4bd327363357f532187b752ff62759c31bedf98e9e18fc08c9cbd605c
WORKDIR /opt/pryx
COPY package.json package-lock.json kernel.cjs legacy-socket.cjs interrupt.cjs ./
COPY notices /opt/pryx/notices
RUN --mount=type=secret,id=proxy_ca \
    set -eu; chmod -R a+rX /opt/pryx; \
    if [ -f /run/secrets/proxy_ca ]; then export NODE_EXTRA_CA_CERTS=/run/secrets/proxy_ca; fi; \
    npm ci --omit=dev --no-audit --no-fund && npm cache clean --force
ENV PRYX_KERNEL_PRESTART=javascript NODE_PATH=/opt/pryx/node_modules PATH=/opt/pryx/node_modules/.bin:$PATH
WORKDIR /data
CMD ["sleep", "infinity"]

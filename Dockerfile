# SPDX-License-Identifier: FSL-1.1-ALv2
# One release image: control, edge, node, static agent and a versioned process userland.
# /pryxd and the protocol/deployment assets retain their paths for fleet image consumers.
# The default entrypoint chooses process isolation without an engine socket; --version and
# explicit node subcommands delegate to /pryxd. All build bases are reviewed digest pins.
FROM mirror.gcr.io/library/rust:1.99.1-alpine3.24@sha256:27cd9befd4a05d6f6eb407c3e28365f5092f008bfc5946207cca76ae5179ef23 AS build
RUN apk add --no-cache musl-dev file
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src src
COPY agent agent
# The unit `pryxd join` installs, and the daemon.json `doctor` checks against, are compiled in.
COPY deploy deploy
RUN cargo build --locked --release \
 && ./target/release/pryxd --version \
 && ./target/release/pryx-agent --version \
 && file target/release/pryx-agent | grep -q 'static' \
 && file target/release/pryxd | grep -q 'static' || { file target/release/pryxd; exit 1; }

FROM oven/bun:1.4.4@sha256:3f23864d3e8c923975c61971cabe67ee3d5a728f1e25a559256b95af719a3478 AS javascript
WORKDIR /src
COPY package.json bun.lock ./
COPY control control
COPY edge edge
COPY cli cli
COPY sdk/typescript sdk/typescript
COPY integrations integrations
COPY openapi openapi
COPY agent/openapi.json agent/openapi.json
COPY Cargo.toml ./
COPY sdk/python/pyproject.toml sdk/python/pyproject.toml
COPY deploy deploy
RUN bun install --frozen-lockfile \
 && bun run release:build \
 && bun build --compile deploy/image.ts --outfile dist/pryx-image

FROM debian:trixie-slim@sha256:7378260c0a098d27ac206225db5e7d92b8c7c84ea7afccc86eeb513756bbd392 AS agentos
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates python3 \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /source
COPY scripts/fetch-agentos.py scripts/fetch-agentos.py
COPY deploy/agentos.json deploy/agentos.NOTICE deploy/
COPY LICENSES/Apache-2.0.txt LICENSES/Apache-2.0.txt
RUN python3 scripts/fetch-agentos.py /agentos

FROM node:24-bookworm-slim@sha256:c0a93797ee78814ae6d32fcca2561fdbbe423149c3f5389aad32d86e9fd85bd4 AS node-userland
FROM ghcr.io/rustfs/rustfs:1.0.0@sha256:8cc9801755448b71a786705ce76692c77e14936cccd87cf2fc31842e58f4d1ff AS object-store
FROM debian:trixie-slim@sha256:7378260c0a098d27ac206225db5e7d92b8c7c84ea7afccc86eeb513756bbd392
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates coreutils git curl python3 python3-pip build-essential postgresql-17 libnss-wrapper util-linux nftables docker-cli tini \
 && docker --version \
 && rm -rf /var/lib/apt/lists/*
COPY --from=agentos /agentos /usr/local/lib/pryx/agentos
COPY --from=build /src/target/release/pryxd /pryxd
COPY --from=build /src/target/release/pryx-agent /usr/local/lib/pryx/pryx-agent
COPY --from=node-userland /usr/local/bin/node /usr/local/bin/node
COPY --from=node-userland /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
 && ln -s ../lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx
COPY --from=object-store /usr/bin/rustfs /usr/local/bin/rustfs
COPY --from=javascript /src/dist/pryx /usr/local/bin/pryx
COPY --from=javascript /src/dist/pryx-control /opt/pryx/pryx-control
COPY --from=javascript /src/dist/pryx-edge /opt/pryx/pryx-edge
COPY --from=javascript /src/dist/pryx-image /opt/pryx/pryx-image
COPY --from=javascript /src/target/release-assets/control /opt/pryx/control
COPY openapi /openapi
COPY deploy/daemon.json deploy/pryxd.service /deploy/
COPY deploy/process-facts.py /opt/pryx/process-facts.py
COPY LICENSE.md /LICENSE.md
COPY LICENSES/CDLA-Permissive-2.0.txt /licenses/CDLA-Permissive-2.0.txt
COPY deploy/webpki-roots.NOTICE /licenses/webpki-roots.NOTICE
ENV PRYX_ASSET_DIR=/opt/pryx/control PRYX_STATE_DIR=/var/lib/pryx PRYX_PRYXD_BIN=/pryxd
EXPOSE 8080
RUN install -d -m 1777 /var/lib/pryx
VOLUME ["/var/lib/pryx"]
LABEL org.opencontainers.image.source=https://github.com/cubepals/pryx \
      org.opencontainers.image.licenses=FSL-1.1-ALv2 \
      org.opencontainers.image.description="pryx control, edge and sandbox runtimes"
ENTRYPOINT ["/usr/bin/tini", "--", "/opt/pryx/pryx-image"]

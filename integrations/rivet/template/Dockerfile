# SPDX-License-Identifier: FSL-1.1-ALv2
# The upstream versioned server is already installed. Pin the public multi-architecture
# manifest and retain its userland; pryxd supplies its own independent trusted agent mount.
FROM rivetdev/sandbox-agent:0.5.5@sha256:7c2b67f718f6ad734e459e5a765187f9411ed21b7535c976d4e1dd8bdac2c097
RUN sandbox-agent --version
LABEL org.opencontainers.image.source=https://github.com/cubepals/pryx

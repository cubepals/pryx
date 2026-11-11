# SPDX-License-Identifier: FSL-1.1-ALv2
FROM mcr.microsoft.com/playwright:v1.65.0-noble@sha256:bc4fbb329a25287ed23e9216c97063646abdd1bf2da669a9895fa3e2078b2302 AS prepared
WORKDIR /opt/pryx/browser
COPY package.json package-lock.json start.cjs sandbox.cjs screenshot.cjs ./
RUN --mount=type=secret,id=proxy_ca \
    if [ -f /run/secrets/proxy_ca ]; then export NODE_EXTRA_CA_CERTS=/run/secrets/proxy_ca; fi; \
    npm ci --omit=dev --ignore-scripts && \
    find /ms-playwright -mindepth 1 -maxdepth 1 ! -name 'chromium_headless_shell-1254' ! -name 'ffmpeg-1013' -exec rm -rf '{}' + && \
    chmod -R a+rX /opt/pryx/browser
# Flatten the sanitized donor: proprietary Widevine never enters a published layer.
FROM scratch
COPY --from=prepared / /
ENV PLAYWRIGHT_BROWSERS_PATH=/ms-playwright PRYX_TOOL_TEMPLATE=browser
WORKDIR /data
CMD ["node", "/opt/pryx/browser/start.cjs"]

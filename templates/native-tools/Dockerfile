# SPDX-License-Identifier: FSL-1.1-ALv2
FROM mirror.gcr.io/library/python:3.12.14-slim-bookworm@sha256:392307d22300de8b5986851a12d9176dfc0fc073e65bf6523ebd7dcbeb23564e
COPY templates/native-tools/*.py templates/native-tools/*.json /opt/pryx/native-tools/
COPY templates/native-tools/notices /opt/pryx/native-tools/notices
COPY templates/desktop /opt/pryx/desktop
RUN --mount=type=secret,id=proxy_ca --mount=type=bind,source=templates/native-tools/cache,target=/build-cache \
    if [ -f /run/secrets/proxy_ca ]; then export SSL_CERT_FILE=/run/secrets/proxy_ca PIP_CERT=/run/secrets/proxy_ca; fi; \
    python3 /opt/pryx/native-tools/install.py && \
    python3 /opt/pryx/native-tools/artifacts.py && \
    if [ -d /build-cache/wheels ]; then pip install --no-index --find-links /build-cache/wheels --require-hashes -r /opt/pryx/desktop/requirements.txt; else pip install --no-cache-dir --require-hashes -r /opt/pryx/desktop/requirements.txt; fi && \
    chmod -R a+rX /opt/pryx && chmod 1777 /mnt
ENV PRYX_TOOL_TEMPLATE=tools
CMD ["sleep", "infinity"]

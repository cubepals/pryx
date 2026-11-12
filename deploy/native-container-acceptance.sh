#!/bin/bash
# SPDX-License-Identifier: FSL-1.1-ALv2
# Disposable fixture inside the release image's default Docker profile. Preparation runs as
# root; installation and both SDKs run as a fresh ordinary account without any engine socket.
set -euo pipefail
cd /test
useradd --create-home --shell /bin/bash pryxtools
cp -a sdk/python /home/pryxtools/sdk-source
chown -R pryxtools:pryxtools /home/pryxtools/sdk-source
su -s /bin/bash pryxtools -c 'python3 -m pip install --user --break-system-packages /home/pryxtools/sdk-source' > /home/pryxtools/sdk-install.log 2>&1
python3 deploy/tool-artifact-server.py --directory /native-artifacts \
  --certificate /fixture-trust/server.crt --key /fixture-trust/server.key > /home/pryxtools/native-artifact-server.log 2>&1 &
pryx_container_tls=$!
python3 -m http.server 9876 --bind 127.0.0.1 --directory /artifacts > /home/pryxtools/native-release-server.log 2>&1 &
pryx_container_http=$!
trap 'kill "$pryx_container_tls" "$pryx_container_http"; wait "$pryx_container_tls" "$pryx_container_http" || true' EXIT
curl --retry 5 --retry-connrefused --retry-delay 1 -fsS http://127.0.0.1:9876/install.sh > /dev/null
curl --retry 5 --retry-connrefused --retry-delay 1 --head -fsS https://127.0.0.1:9877/pryx-process-tools-linux-x64.tar.gz > /dev/null
su -s /bin/bash pryxtools -c 'cd /test; env -i HOME=/home/pryxtools PATH=/usr/local/bin:/usr/bin:/bin PRYX_TEST_BUN=/test-bun PRYX_NATIVE_BROWSER=off /bin/sh deploy/native-user-acceptance.sh'

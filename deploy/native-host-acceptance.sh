#!/bin/bash
# SPDX-License-Identifier: FSL-1.1-ALv2
# Prepare a disposable hosted runner, then run every installer/tool call as a fresh ordinary
# account without engine access. No production install acquires root or changes host trust.
set -euo pipefail
pryx_native_artifacts="$RUNNER_TEMP/native-distribution"
pryx_native_trust="$RUNNER_TEMP/native-host-trust"
mkdir -m 700 "$pryx_native_trust"
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj '/CN=Pryx CI installed tools CA' \
  -keyout "$pryx_native_trust/ca.key" -out "$pryx_native_trust/ca.crt" > "$pryx_native_trust/openssl.log" 2>&1
openssl req -newkey rsa:2048 -nodes -subj '/CN=127.0.0.1' \
  -keyout "$pryx_native_trust/server.key" -out "$pryx_native_trust/server.csr" >> "$pryx_native_trust/openssl.log" 2>&1
printf '%s\n' 'subjectAltName=IP:127.0.0.1' 'extendedKeyUsage=serverAuth' > "$pryx_native_trust/extensions"
openssl x509 -req -in "$pryx_native_trust/server.csr" -CA "$pryx_native_trust/ca.crt" \
  -CAkey "$pryx_native_trust/ca.key" -CAcreateserial -days 1 -extfile "$pryx_native_trust/extensions" \
  -out "$pryx_native_trust/server.crt" >> "$pryx_native_trust/openssl.log" 2>&1
sudo install -m 644 "$pryx_native_trust/ca.crt" /usr/local/share/ca-certificates/pryx-ci-installed-tools.crt
sudo update-ca-certificates
if [ -f /proc/sys/kernel/apparmor_restrict_unprivileged_userns ]; then
  sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0
fi
python3 deploy/tool-artifact-server.py --directory "$pryx_native_artifacts" \
  --certificate "$pryx_native_trust/server.crt" --key "$pryx_native_trust/server.key" > "$RUNNER_TEMP/native-host-artifact-server.log" 2>&1 &
pryx_native_tls=$!
python3 -m http.server 9876 --bind 127.0.0.1 --directory "$PWD/host-artifacts" > "$RUNNER_TEMP/native-host-release-server.log" 2>&1 &
pryx_native_http=$!
trap 'kill "$pryx_native_tls" "$pryx_native_http"; wait "$pryx_native_tls" "$pryx_native_http" || true' EXIT
curl --retry 5 --retry-connrefused --retry-delay 1 -fsS http://127.0.0.1:9876/install.sh > /dev/null
curl --retry 5 --retry-connrefused --retry-delay 1 --head -fsS https://127.0.0.1:9877/pryx-process-tools-linux-x64.tar.gz > /dev/null
sudo useradd --create-home --shell /bin/bash pryxtools
install -m 755 "$(command -v bun)" "$RUNNER_TEMP/pryx-test-bun"
sudo -u pryxtools -H python3 -m venv /home/pryxtools/sdk-python
sudo cp -a sdk/python /home/pryxtools/sdk-source
sudo chown -R pryxtools:pryxtools /home/pryxtools/sdk-source
sudo -u pryxtools -H /home/pryxtools/sdk-python/bin/pip install /home/pryxtools/sdk-source > "$RUNNER_TEMP/native-python-sdk-install.log" 2>&1
sudo -u pryxtools -H env -i HOME=/home/pryxtools PATH=/usr/local/bin:/usr/bin:/bin \
  PRYX_TEST_BUN="$RUNNER_TEMP/pryx-test-bun" PRYX_PYTHON=/home/pryxtools/sdk-python/bin/python \
  PRYX_NATIVE_BROWSER=on PRYX_NATIVE_MEASUREMENTS=/home/pryxtools/native-process-measurements.json \
  /bin/sh deploy/native-user-acceptance.sh | tee "$RUNNER_TEMP/native-user-acceptance.log"
sudo cp /home/pryxtools/native-process-measurements.json "$RUNNER_TEMP/native-process-measurements.json"
sudo chown "$(id -u):$(id -g)" "$RUNNER_TEMP/native-process-measurements.json"

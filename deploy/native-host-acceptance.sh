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
cleanup() {
  result=$?
  trap - EXIT
  if sudo test -f /home/pryxtools/native-process-measurements.json; then
    sudo cp /home/pryxtools/native-process-measurements.json "$RUNNER_TEMP/native-process-measurements.json" || result=1
    sudo chown "$(id -u):$(id -g)" "$RUNNER_TEMP/native-process-measurements.json" || result=1
  fi
  if sudo test -f /home/pryxtools/sdk-install.log; then
    sudo cp /home/pryxtools/sdk-install.log "$RUNNER_TEMP/native-python-sdk-install.log" || result=1
    sudo chown "$(id -u):$(id -g)" "$RUNNER_TEMP/native-python-sdk-install.log" || result=1
  fi
  kill "$pryx_native_tls" "$pryx_native_http" 2>/dev/null || true
  wait "$pryx_native_tls" "$pryx_native_http" || true
  exit "$result"
}
trap cleanup EXIT
curl --retry 5 --retry-connrefused --retry-delay 1 -fsS http://127.0.0.1:9876/install.sh > /dev/null
curl --retry 5 --retry-connrefused --retry-delay 1 --head -fsS https://127.0.0.1:9877/pryx-process-tools-linux-x64.tar.gz > /dev/null
sudo useradd --create-home --shell /bin/bash pryxtools
pryx_native_fixture=$(mktemp -d /tmp/pryx-native-acceptance.XXXXXX)
chmod 755 "$pryx_native_fixture"
mkdir -p "$pryx_native_fixture/deploy" "$pryx_native_fixture/templates/process-tools" \
  "$pryx_native_fixture/sdk/typescript/test/fixtures" "$pryx_native_fixture/sdk"
cp deploy/native-user-acceptance.sh deploy/native-container-acceptance.sh \
  deploy/local-diagnostics.py deploy/tool-artifact-server.py "$pryx_native_fixture/deploy/"
cp templates/process-tools/artifacts.json "$pryx_native_fixture/templates/process-tools/"
cp sdk/typescript/test/fixtures/chart.cjs "$pryx_native_fixture/sdk/typescript/test/fixtures/"
cp -a sdk/python "$pryx_native_fixture/sdk/python"
find "$pryx_native_fixture" -type d -exec chmod 755 {} +
find "$pryx_native_fixture" -type f -exec chmod 644 {} +
bun build --compile deploy/native-tools-acceptance.ts --outfile "$pryx_native_fixture/acceptance"
chmod 755 "$pryx_native_fixture/acceptance"
printf '%s\n' "$pryx_native_fixture" > "$RUNNER_TEMP/native-fixture-path"
sudo cp -a sdk/python /home/pryxtools/sdk-source
sudo chown -R pryxtools:pryxtools /home/pryxtools/sdk-source
cd "$pryx_native_fixture"
sudo -u pryxtools -H env -i HOME=/home/pryxtools PATH=/usr/local/bin:/usr/bin:/bin \
  PRYX_NATIVE_ACCEPTANCE_BIN="$pryx_native_fixture/acceptance" PRYX_PYTHON=/home/pryxtools/sdk-python/bin/python \
  PRYX_NATIVE_BROWSER=on PRYX_NATIVE_MEASUREMENTS=/home/pryxtools/native-process-measurements.json \
  /bin/sh deploy/native-user-acceptance.sh | tee "$RUNNER_TEMP/native-user-acceptance.log"

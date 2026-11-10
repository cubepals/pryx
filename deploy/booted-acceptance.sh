#!/bin/bash
# SPDX-License-Identifier: FSL-1.1-ALv2
# Boot a distribution under Incus, then exercise the installer in a real ordinary-user
# login session. The host prepares test fixtures; pryx receives no extra kernel privileges.
set -euo pipefail
image=$1
sudo apt-get update -qq
sudo apt-get install -y incus=6.0.0-1ubuntu0.3
sudo incus admin init --minimal
sudo iptables -I DOCKER-USER -i incusbr0 -j ACCEPT
sudo iptables -I DOCKER-USER -o incusbr0 -j ACCEPT
sudo incus launch "images:$image" pryx --config security.nesting=true
trap 'sudo incus delete --force pryx' EXIT
pryx_network_deadline=$((SECONDS + 60))
until sudo incus query /1.0/instances/pryx/state | jq -e '.network.eth0.addresses[] | select(.family == "inet" and .scope == "global")' >/dev/null; do
  if ((SECONDS >= pryx_network_deadline)); then echo 'The booted test system did not acquire an IPv4 address' >&2; exit 1; fi
  sleep 1
done
sudo incus info pryx
sudo incus config get pryx volatile.base_image | tee -a "$GITHUB_STEP_SUMMARY"
# The test archive is public preview data; the signing key stays on the build runner.
sudo incus exec pryx -- mkdir /artifacts
sudo tar -C host-artifacts -cf - . | sudo incus exec pryx -- tar -xf - -C /artifacts
sudo incus file push deploy/local-acceptance.sh pryx/local-acceptance.sh
sudo incus file push deploy/container-acceptance.sh pryx/container-acceptance.sh
sudo incus file push deploy/local-diagnostics.py pryx/local-diagnostics.py
# Reuse distro prerequisite setup, replacing only its final su with a PAM login session
# for systemd distributions. Alpine uses its actual OpenRC/foreground path.
sudo incus exec pryx -- sh -c '
  if [ "$(cat /proc/1/comm)" = systemd ]; then
    while ! systemctl is-active --quiet systemd-logind; do sleep 1; done
    PRYX_PREPARE_ONLY=1 sh /container-acceptance.sh
    case "$(. /etc/os-release; echo "$ID")" in
      debian|ubuntu) apt-get install -y dbus-user-session libpam-systemd polkitd ;;
      fedora) dnf install -y systemd-pam dbus-broker polkit ;;
    esac
    python3 -m http.server 9876 --bind 127.0.0.1 --directory /artifacts >/tmp/artifact-server.log 2>&1 &
    curl --retry 5 --retry-connrefused --retry-delay 1 --max-time 10 -fsS http://127.0.0.1:9876/install.sh -o /dev/null
    systemd-run --quiet --wait --collect --pipe --unit=pryx-acceptance \
      --property=User=pryxtest --property=PAMName=login \
      --setenv=HOME=/home/pryxtest --setenv=XDG_RUNTIME_DIR=/run/user/1001 \
      /bin/sh /local-acceptance.sh
  else
    sh /container-acceptance.sh
  fi
' | tee acceptance.txt
cat acceptance.txt >> "$GITHUB_STEP_SUMMARY"

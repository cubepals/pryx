#!/bin/bash
# SPDX-License-Identifier: FSL-1.1-ALv2
# Boot a distribution under Incus, then exercise the installer in a real ordinary-user
# login session. The host prepares test fixtures; pryx receives no extra kernel privileges.
set -euo pipefail
image=$1
pryx_sources=$(mktemp "$RUNNER_TEMP/pryx-incus-XXXXXX.sources")
pryx_source_parts=$(mktemp -d "$RUNNER_TEMP/pryx-incus-sources-XXXXXX")
grep -q '^URIs:' /etc/apt/sources.list.d/ubuntu.sources
apt_options=(-o "Dir::Etc::sourcelist=$pryx_sources" -o "Dir::Etc::sourceparts=$pryx_source_parts" -o Acquire::ForceIPv4=true -o Acquire::Retries=3)
install_incus() {
  sudo apt-get "${apt_options[@]}" update -qq -o APT::Update::Error-Mode=any &&
    sudo apt-get "${apt_options[@]}" install -y incus=6.0.0-1ubuntu0.3
}
# The manifest's mirrors are tried in order until one serves every signed index and package.
mapfile -t pryx_ubuntu_mirrors < <(jq -r '.ubuntuMirrors[]' .github/engine-versions.json)
pryx_installed=false
for pryx_ubuntu_mirror in "${pryx_ubuntu_mirrors[@]}"; do
  sed "s|^URIs:.*|URIs: $pryx_ubuntu_mirror|" /etc/apt/sources.list.d/ubuntu.sources > "$pryx_sources"
  if install_incus; then pryx_installed=true; break; fi
done
[ "$pryx_installed" = true ] || { echo 'No Ubuntu mirror installed Incus' >&2; exit 1; }
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
fixture_incus() { sudo timeout --kill-after=5s 2m incus "$@"; }
# The test archive is public preview data; the signing key stays on the build runner.
echo 'Booted acceptance: staging the verified archive'
fixture_incus exec pryx -- mkdir /artifacts
sudo timeout --kill-after=5s 2m tar -C host-artifacts -cf - . |
  fixture_incus exec pryx -- tar -xf - -C /artifacts
echo 'Booted acceptance: pushing fixture scripts'
fixture_incus file push deploy/local-acceptance.sh pryx/local-acceptance.sh
fixture_incus file push deploy/container-acceptance.sh pryx/container-acceptance.sh
fixture_incus file push deploy/local-diagnostics.py pryx/local-diagnostics.py
# Reuse distro prerequisite setup, replacing only its final su with a PAM login session
# for systemd distributions, whose user service gives pryx memory, cpu and pids for process
# sandboxes' limits (PRYX_EXPECT_PROCESS_LIMITS). Alpine uses its actual OpenRC/foreground path.
pryx_guest_env=(--env "PRYX_TEST_UBUNTU_MIRRORS=$(jq -r '.ubuntuMirrors | join(" ")' .github/engine-versions.json)")
echo 'Booted acceptance: detecting guest init'
pryx_guest_init=$(fixture_incus exec pryx "${pryx_guest_env[@]}" -- cat /proc/1/comm)
if [ "$pryx_guest_init" = systemd ]; then
  # The host deadline also covers entering the guest. The inner phases allow six minutes for
  # base packages, two for login packages and one for logind, plus bounded checks and cleanup.
  echo 'Booted acceptance: entering guest preparation'
  sudo timeout --kill-after=5s 10m incus exec pryx "${pryx_guest_env[@]}" -- sh -c '
    set -eu
    echo "Booted acceptance: installing base guest packages"
    PRYX_PREPARE_ONLY=1 timeout --kill-after=5s 6m sh /container-acceptance.sh
    [ "$(id -u pryxtest)" = 1001 ]
    echo "Booted acceptance: installing guest login packages"
    case "$(. /etc/os-release; echo "$ID")" in
      debian|ubuntu) DEBIAN_FRONTEND=noninteractive timeout --kill-after=5s 2m apt-get install -y \
        -o Acquire::ForceIPv4=true -o Acquire::Retries=3 dbus-user-session libpam-systemd polkitd ;;
      fedora) timeout --kill-after=5s 2m dnf install -y systemd-pam dbus-broker polkit ;;
    esac
    echo "Booted acceptance: starting guest logind"
    if ! timeout --kill-after=5s 1m systemctl start systemd-logind.service ||
       ! timeout --kill-after=5s 5s systemctl is-active --quiet systemd-logind.service; then
      echo "Booted acceptance: guest logind did not become active" >&2
      timeout --kill-after=5s 5s systemctl show -p ActiveState -p SubState -p Result systemd-logind.service || true
      exit 1
    fi
  '
  pryx_guest_command=(sh -c '
    set -eu
    python3 -m http.server 9876 --bind 127.0.0.1 --directory /artifacts >/tmp/artifact-server.log 2>&1 &
    curl --retry 5 --retry-connrefused --retry-delay 1 --max-time 10 -fsS http://127.0.0.1:9876/install.sh -o /dev/null
    echo "Booted acceptance: running the ordinary-user PAM session"
    systemd-run --quiet --wait --collect --pipe --unit=pryx-acceptance \
      --property=User=pryxtest --property=PAMName=login \
      --setenv=HOME=/home/pryxtest --setenv=XDG_RUNTIME_DIR=/run/user/1001 --setenv=PRYX_EXPECT_PROCESS_LIMITS=on \
      /bin/sh /local-acceptance.sh
  ')
else
  pryx_guest_command=(sh /container-acceptance.sh)
fi
echo 'Booted acceptance: running guest acceptance'
sudo incus exec pryx "${pryx_guest_env[@]}" -- "${pryx_guest_command[@]}" | tee acceptance.txt
cat acceptance.txt >> "$GITHUB_STEP_SUMMARY"

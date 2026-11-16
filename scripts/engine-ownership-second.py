#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Bounded, diagnostic-only ownership candidates for the existing rootless Docker fixture.

No engine/host configuration or data ownership is changed. Same-user namespace entry uses
held namespace descriptors, with socket peer/start identity checked around engine operations.
Passing a diagnostic never enables a product capability or replaces the existing refusal tests.
"""
import argparse
import hashlib
from http import HTTPStatus
import json
import os
from pathlib import Path
import select
import shutil
import signal
import socket
import stat
import struct
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from urllib.parse import urlsplit
import uuid

LIMIT = 180
COMMAND_LIMIT = 12
MAX_HELPER_BYTES = 64 * 1024 * 1024
# util-linux mountpoint(1): 32 means not a mountpoint; 1 means a tool/system error.
NOT_MOUNTPOINT = 32
# proc_pid_stat(5): field 22; the tail after comm starts at field 3.
STARTTIME_TAIL_INDEX = 19


class HttpsRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, newurl):
        parsed = urlsplit(newurl)
        if parsed.scheme != 'https' or parsed.username or parsed.password:
            raise Violation('helper redirect left credential-free verified HTTPS')
        return super().redirect_request(request, fp, code, message, headers, newurl)


class Missing(Exception):
    """An unavailable prerequisite, not evidence of a working ownership strategy."""


class Violation(Exception):
    """A candidate violated the fixture's ownership or lifetime invariant."""


class Probe:
    def __init__(self):
        self.deadline = time.monotonic() + LIMIT
        self.results = []
        self.violation = False
        self.env = {'PATH': os.defpath, 'LANG': 'C'}

    def run(self, argv, timeout=COMMAND_LIMIT, fds=(), cleanup=False):
        remaining = timeout if cleanup else self.deadline - time.monotonic()
        if remaining <= 0:
            raise Missing('overall diagnostic deadline reached')
        started = time.monotonic()
        try:
            child = subprocess.Popen(argv, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                     stderr=subprocess.PIPE, env=self.env, pass_fds=fds,
                                     start_new_session=True)
        except OSError as problem:
            return {'exit': None, 'errno': problem.errno, 'timedOut': False,
                    'stdout': '', 'stderr': str(problem)[:2048]}
        timed_out = False
        try:
            stdout, stderr = child.communicate(timeout=min(timeout, remaining))
        except subprocess.TimeoutExpired:
            timed_out = True
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            stdout, stderr = child.communicate(timeout=3)
        return {'exit': child.returncode, 'timedOut': timed_out,
                'milliseconds': round((time.monotonic() - started) * 1000),
                'stdout': stdout.decode(errors='replace')[-2048:],
                'stderr': stderr.decode(errors='replace')[-2048:]}

    def record(self, approach, **fields):
        self.results.append({'approach': approach, **fields})


class Peer:
    def __init__(self, endpoint):
        if not endpoint.startswith('unix://'):
            raise Missing('ownership diagnostic requires a local Unix engine socket')
        self.path = endpoint.removeprefix('unix://')
        self.identity = self.read()
        self.pid = self.identity[0]
        self.start = self.start_time()
        self.fds = []
        self.pidfd = os.pidfd_open(self.pid)

    def read(self):
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
            connection.settimeout(3)
            connection.connect(self.path)
            return struct.unpack('3i', connection.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, struct.calcsize('3i')))

    def start_time(self):
        return Path(f'/proc/{self.pid}/stat').read_text().rsplit(') ', 1)[1].split()[STARTTIME_TAIL_INDEX]

    def check(self):
        if (self.read() != self.identity or self.start_time() != self.start
                or select.select([self.pidfd], [], [], 0)[0]):
            raise Violation('engine socket peer/start identity changed during diagnostic')

    def namespaces(self):
        self.check()
        if self.identity[1] != os.geteuid():
            raise Missing('socket peer belongs to another host UID; standard same-user path not run')
        if (mapped_id(self.pid, 'uid', 0) != os.geteuid()
                or mapped_id(self.pid, 'gid', 0) != os.getegid()):
            raise Missing('daemon namespace root does not map to caller ownership; standard path not run')
        for name in ('user', 'mnt'):
            self.fds.append(os.open(f'/proc/{self.pid}/ns/{name}', os.O_RDONLY | os.O_CLOEXEC))
        self.check()
        return ['nsenter', f'--user=/proc/self/fd/{self.fds[0]}',
                f'--mount=/proc/self/fd/{self.fds[1]}', '--setuid=0', '--setgid=0', '--']

    def close(self):
        for descriptor in self.fds + [self.pidfd]:
            os.close(descriptor)


def mapped_id(pid, kind, inner):
    for line in Path(f'/proc/{pid}/{kind}_map').read_text().splitlines():
        inside, outside, length = map(int, line.split())
        if inside <= inner < inside + length:
            return outside + inner - inside
    raise Missing(f'workload {kind} is not present in the daemon namespace mapping')


def require_helper(args, fixture, deadline):
    if time.monotonic() >= deadline:
        raise Missing('overall diagnostic deadline reached before helper preparation')
    pin = json.loads(Path(args.manifest).read_text())['fuseOverlayfs']
    path = Path(args.helper).resolve() if args.helper else fixture / 'fuse-overlayfs'
    if not args.helper:
        if not pin['url'].startswith('https://'):
            raise Violation('helper manifest does not use verified HTTPS')
        end = min(time.monotonic() + 45, deadline)
        for attempt in range(3):
            remaining = end - time.monotonic()
            if remaining <= 0:
                raise Missing('overall diagnostic deadline reached before helper download')
            try:
                opener = urllib.request.build_opener(HttpsRedirect())
                with opener.open(pin['url'], timeout=min(10, remaining)) as response:
                    if not response.url.startswith('https://'):
                        raise Violation('helper redirect left verified HTTPS')
                    chunks, length = [], 0
                    while True:
                        if time.monotonic() >= end:
                            raise TimeoutError('helper download deadline')
                        chunk = response.read1(min(1024*1024, MAX_HELPER_BYTES+1-length))
                        if not chunk:
                            break
                        chunks.append(chunk)
                        length += len(chunk)
                        if length > MAX_HELPER_BYTES:
                            raise Violation('helper archive exceeds diagnostic bound')
                    data = b''.join(chunks)
                if len(data) > MAX_HELPER_BYTES:
                    raise Violation('helper archive exceeds diagnostic bound')
                path.write_bytes(data)
                break
            except (OSError, urllib.error.URLError) as problem:
                if isinstance(problem, urllib.error.HTTPError) and problem.code != HTTPStatus.TOO_MANY_REQUESTS and problem.code < HTTPStatus.INTERNAL_SERVER_ERROR:
                    raise Missing('pinned helper download refused by origin') from None
                if attempt == 2 or time.monotonic() + (2 ** attempt) >= end:
                    raise Missing('pinned helper download unavailable within bound') from None
                time.sleep(2 ** attempt)
    if hashlib.sha256(path.read_bytes()).hexdigest() != pin['sha256']:
        raise Violation('pinned FUSE helper digest differs')
    if not args.helper:
        path.chmod(0o700)
    return path


def engine_prefix(args, fixture):
    executable = shutil.which('docker')
    if executable is None:
        raise Missing('Docker CLI is unavailable')
    config = fixture / 'client-config'
    config.mkdir(mode=0o700)
    return [executable, '--config', str(config), '--host', args.endpoint]


def docker(probe, peer, prefix, *arguments):
    peer.check()
    result = probe.run(prefix + list(arguments))
    peer.check()
    return result


def fixture_write(probe, peer, prefix, image, source, uid, gid, expected, value, mount_extra=''):
    if uid <= 0 or gid <= 0:
        raise Violation('ownership workload UID/GID must remain nonzero')
    name = 'pryx-k22-owner-' + uuid.uuid4().hex
    script = ('set -eu; test "$(id -u)" = "$1"; test "$(id -g)" = "$2"; '
              'test "$(cat /data/from-host)" = "$3"; umask 077; '
              'printf %s "$4" > /data/marker; chmod 600 /data/marker; '
              'test "$(stat -c %u /data/marker)" = "$1"; '
              'test "$(stat -c %a /data/marker)" = 600')
    arguments = ['run', '--rm', '--pull=never', '--name', name, '--network=none',
                 '--read-only', '--cap-drop=ALL', '--security-opt=no-new-privileges',
                 f'--user={uid}:{gid}', '--mount',
                 f'type=bind,source={source},target=/data{mount_extra}', image,
                 'sh', '-c', script, 'ownership', str(uid), str(gid), expected, value]
    try:
        result = docker(probe, peer, prefix, *arguments)
        if result['exit'] == 0:
            marker = source.parent / 'upper' / 'marker' if source.name == 'merged' else source / 'marker'
            try:
                info = marker.lstat()
                contents = marker.read_text() if stat.S_ISREG(info.st_mode) else None
            except OSError as problem:
                raise Violation(f'successful candidate backing is unreadable: errno {problem.errno}') from None
            if not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid() or info.st_gid != os.getegid():
                raise Violation('successful candidate changed original backing owner/type')
            if stat.S_IMODE(info.st_mode) != 0o600 or contents != value:
                raise Violation('successful candidate lost private mode or original backing bytes')
        return result
    finally:
        # The generated exact name is ours even if a timed-out create returned no ID.
        peer.check()
        cleanup = probe.run(prefix + ['rm', '--force', name], timeout=4, cleanup=True)
        remaining = probe.run(prefix + ['ps', '-a', '--filter', f'name=^/{name}$',
                                        '--format', '{{.ID}}'], timeout=4, cleanup=True)
        peer.check()
        if (cleanup['timedOut'] or remaining['exit'] != 0 or remaining['stdout'].strip()):
            raise Violation('fixture container cleanup could not prove removal within bound')


def engine_option(probe, peer, prefix, args, fixture):
    source = fixture / 'engine-bind'
    source.mkdir(mode=0o700)
    (source / 'from-host').write_text('host-first')
    (source / 'from-host').chmod(0o600)
    result = fixture_write(probe, peer, prefix, args.image, source, 1000, 1000,
                           'host-first', 'engine-candidate', ',idmap=true')
    probe.record('engine-owned-bind-idmap', result=result,
                 accepted=result['exit'] == 0, diagnosticOnly=True)


def kernel_mapping(probe, peer, prefix, ns, fixture):
    source, target = fixture / 'kernel-source', fixture / 'kernel-target'
    source.mkdir(mode=0o700)
    target.mkdir(mode=0o700)
    # Namespace propagation changes are confined to this newly cloned mount namespace.
    commands = [ns + ['unshare', '--mount', '--propagation', 'private', '--',
                     'sh', '-c', 'mount --bind "$1" "$2" && stat -c %u:%g "$2"', 'bind', str(source), str(target)],
                ns + ['unshare', '--mount', '--propagation', 'private', '--',
                      'sh', '-c', 'mount --bind -o X-mount.idmap=b:1000:0:1 "$1" "$2" && stat -c %u:%g "$2"',
                      'idmap', str(source), str(target)]]
    outcomes = [probe.run(command, fds=peer.fds) for command in commands]
    peer.check()
    probe.record('kernel-idmapped-bind', bindControl=outcomes[0], idmapped=outcomes[1],
                 filesystem=probe.run(['stat', '-f', '-c', '%T', str(source)]),
                 accepted=False, reason='mount success alone does not prove workload/store mapping',
                 diagnosticOnly=True)


def mount_view(probe, peer, ns, helper, base, target_uid, target_gid):
    for name in ('lower', 'upper', 'work', 'merged'):
        (base / name).mkdir(mode=0o700)
    (base / 'upper' / 'from-host').write_text('host-first')
    (base / 'upper' / 'from-host').chmod(0o600)
    prefix = ns or []
    # In the daemon user namespace backing UID/GID 0 is the ordinary host owner.
    uid = 0 if ns else os.geteuid()
    gid = 0 if ns else os.getegid()
    options = (f'lowerdir={base}/lower,upperdir={base}/upper,workdir={base}/work,'
               f'uidmapping={uid}:{target_uid}:1,gidmapping={gid}:{target_gid}:1,'
               'allow_other,timeout=0')
    descriptors = peer.fds if ns else ()
    for tool in ('fusermount3', 'mountpoint'):
        if probe.run(prefix+[tool, '--version'], fds=descriptors)['exit'] != 0:
            raise Missing(f'{tool} prerequisite is unavailable before FUSE setup')
    initial = probe.run(prefix+['mountpoint', '--quiet', str(base/'merged')], fds=descriptors)
    if initial['exit'] != NOT_MOUNTPOINT or initial['timedOut']:
        raise Missing('cannot establish an unmounted fixture before FUSE setup')
    log = tempfile.TemporaryFile()
    try:
        child = subprocess.Popen(prefix + [str(helper), '-f', '-o', options, str(base/'merged')],
                                 stdin=subprocess.DEVNULL, stdout=log, stderr=log,
                                 env=probe.env, pass_fds=descriptors, start_new_session=True)
    except BaseException:
        log.close()
        raise
    try:
        ready = False
        end = min(probe.deadline, time.monotonic()+5)
        while child.poll() is None and time.monotonic() < end:
            check = probe.run(prefix+['mountpoint', '--quiet', str(base/'merged')], timeout=1,
                              fds=descriptors)
            if check['exit'] == 0:
                ready = True
                break
            if check['exit'] != NOT_MOUNTPOINT or check['timedOut']:
                raise Missing('FUSE readiness cannot be observed')
            time.sleep(0.1)
        return child, log, ready
    except BaseException:
        # The caller cannot own resources until this function returns. Retire everything
        # already started even when readiness itself loses its deadline or subprocess.
        retire_view(probe, peer, ns, base, child, log)
        raise


def retire_view(probe, peer, ns, base, child, log):
    prefix = ns or []
    problems = []
    unmount = None
    try:
        unmount = probe.run(prefix + ['fusermount3', '-u', str(base/'merged')], timeout=3,
                            fds=peer.fds if ns else (), cleanup=True)
    except Exception as problem:
        problems.append('unmount: '+type(problem).__name__)
    try:
        if child.poll() is None:
            try:
                os.killpg(child.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                child.wait(timeout=2)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                child.wait(timeout=2)
        if child.poll() is None:
            problems.append('helper exit is unconfirmed')
    except Exception as problem:
        problems.append('helper termination: '+type(problem).__name__)
    try:
        gone = probe.run(prefix+['mountpoint', '--quiet', str(base/'merged')], timeout=1,
                         fds=peer.fds if ns else (), cleanup=True)
        if gone['exit'] != NOT_MOUNTPOINT or gone['timedOut']:
            problems.append('negative mount observation is unconfirmed')
    except Exception as problem:
        problems.append('mount observation: '+type(problem).__name__)
    text = ''
    try:
        log.seek(0)
        text = log.read(2048).decode(errors='replace')
    except Exception as problem:
        problems.append('helper log: '+type(problem).__name__)
    finally:
        try:
            log.close()
        except Exception as problem:
            problems.append('helper log close: '+type(problem).__name__)
    if problems:
        message = 'FUSE cleanup could not be confirmed: '+'; '.join(problems)
        # TemporaryDirectory teardown can also fail on a surviving mount. Latch this
        # before raising so a later filesystem error cannot downgrade it to inconclusive.
        probe.violation = True
        probe.record('fuse-view-retirement', invariantFailure=message)
        raise Violation(message)
    return {'unmount': unmount, 'helperExit': child.returncode, 'helperLog': text,
            'mountRetired': True}


def fuse_mapping(probe, peer, prefix, args, fixture, ns, helper, daemon_scope):
    location = 'daemon-namespace' if daemon_scope else 'caller-namespace'
    for uid in (1000, 1001):
        base = fixture / f'fuse-{location}-{uid}'
        base.mkdir(mode=0o700)
        target_uid = uid if ns else mapped_id(peer.pid, 'uid', uid)
        target_gid = uid if ns else mapped_id(peer.pid, 'gid', uid)
        child, log, ready = mount_view(probe, peer, ns, helper, base, target_uid, target_gid)
        result = {'mounted': ready, 'workloadUid': uid, 'diagnosticOnly': True}
        try:
            if ready:
                first = fixture_write(probe, peer, prefix, args.image, base/'merged', uid, uid,
                                      'host-first', 'sandbox-first')
                result['firstWrite'] = first
                if first['exit'] == 0:
                    (base/'upper'/'from-host').write_text('host-second')
                    second = fixture_write(probe, peer, prefix, args.image, base/'merged', uid, uid,
                                           'host-second', 'sandbox-second')
                    result['secondWrite'] = second
                    if second['exit'] == 0:
                        snapshot = fixture / f'snapshot-{location}-{uid}'
                        shutil.copytree(base/'upper', snapshot, symlinks=True)
                        snapshot_file = snapshot/'marker'
                        snapshot_info = snapshot_file.lstat()
                        if (snapshot_file.read_text() != 'sandbox-second'
                                or stat.S_IMODE(snapshot_info.st_mode) != 0o600
                                or snapshot_info.st_uid != os.geteuid()):
                            raise Violation('private backing snapshot lost bytes, mode or owner')
                        result['privateSnapshotReadable'] = True
                        result['candidateWriteProof'] = True
                    if child.poll() is not None:
                        raise Violation('FUSE helper exited while successful view was still in use')
        finally:
            result['retirement'] = retire_view(probe, peer, ns, base, child, log)
            probe.record('fuse-id-view-'+location, **result)
        if not ready:
            break


def measure(args):
    probe = Probe()
    peer = None
    report = {'schema': 1, 'diagnosticOnly': True, 'productEnabled': False,
              'callerUid': os.geteuid(), 'standardOwnershipEnabled': False,
              'workloadUids': [1000, 1001],
              'mappingSemantics': 'pinned fuse-overlayfs main.c read_mappings/find_mapping/get_uid: '
                                  'daemon-namespace backing0 -> workloadview UID, inverse write -> backing0; '
                                  'syntax/source inspection is not runtime proof',
              'notProven': ['live pryx store lease', 'existing-data snapshot/fork',
                            'helper restart and node orphan recovery']}
    try:
        if os.geteuid() == 0:
            raise Missing('run this rootless diagnostic as the ordinary engine user')
        if os.environ.get('PRYX_TEST_ENGINE', 'docker') != 'docker':
            raise Missing('standard Docker diagnostic; Podman retains its original measured path')
        peer = Peer(args.endpoint)
        report['socketPeerUid'] = peer.identity[1]
        report['sameUserSocketPeer'] = peer.identity[1] == os.geteuid()
        report['uidMap'] = Path(f'/proc/{peer.pid}/uid_map').read_text().splitlines()
        report['gidMap'] = Path(f'/proc/{peer.pid}/gid_map').read_text().splitlines()
        with tempfile.TemporaryDirectory(prefix='pryx-k22-owner-', dir=Path.home()) as temporary:
            fixture = Path(temporary)
            if any(character in str(fixture) for character in ',:\n'):
                raise Missing('temporary path cannot be represented safely in Docker/FUSE mount options')
            prefix = engine_prefix(args, fixture)
            report['tools'] = {tool: probe.run([tool, '--version']) for tool in ('nsenter', 'mount')}
            metadata = docker(probe, peer, prefix, 'info', '--format', '{{json .SecurityOptions}}')
            report['rootlessInfo'] = metadata
            if metadata['exit'] != 0 or 'rootless' not in metadata['stdout']:
                raise Missing('selected socket did not confirm rootless Docker')
            versions = json.loads(Path(args.manifest).with_name('engine-versions.json').read_text())
            version = docker(probe, peer, prefix, 'version', '--format', '{{.Server.Version}}')
            report['engineVersion'] = version
            if version['exit'] != 0 or version['stdout'].strip() != versions['docker']['version']:
                raise Missing('selected engine does not match the pinned standard Docker version')
            image = docker(probe, peer, prefix, 'image', 'inspect', args.image,
                           '--format', '{{json .RepoDigests}}')
            if image['exit'] != 0:
                raise Missing('fixture image is not present; diagnostic never pulls an image')
            expected = versions['image']['reference'].split('@', 1)[1]
            if not any(value.endswith('@'+expected) for value in json.loads(image['stdout']) or []):
                raise Violation('fixture image alias does not contain the pinned image digest')
            report['fixtureImageDigestVerified'] = True
            engine_option(probe, peer, prefix, args, fixture)
            try:
                ns = peer.namespaces()
                entry = probe.run(ns + ['sh', '-c', 'id -u; id -g; '
                                        'stat -Lc %i /proc/self/ns/user; '
                                        'stat -Lc %i /proc/self/ns/mnt'], fds=peer.fds)
                report['daemonNamespaceEntry'] = entry
                if entry['exit'] != 0:
                    raise Missing('same-user daemon namespace entry refused; see bounded entry result')
                identities = entry['stdout'].splitlines()
                expected = ['0', '0'] + [str(os.fstat(fd).st_ino) for fd in peer.fds]
                if identities != expected:
                    raise Violation('namespace-entry UID/GID or inode readback differs from held descriptors')
                report['daemonNamespaceIdentityVerified'] = True
            except (Missing, OSError) as problem:
                ns = None
                report['daemonNamespaceRefusal'] = str(problem)[:2048]
                probe.record('kernel-idmapped-bind', notRun=True,
                             reason='same-user daemon namespaces unavailable; kernel support unproved')
            if ns:
                kernel_mapping(probe, peer, prefix, ns, fixture)
            helper = require_helper(args, fixture, probe.deadline)
            report['fuseVersion'] = probe.run([str(helper), '--version'])
            fuse_mapping(probe, peer, prefix, args, fixture, None, helper, False)
            if ns:
                fuse_mapping(probe, peer, prefix, args, fixture, ns, helper, True)
            else:
                probe.record('fuse-id-view-daemon-namespace', notRun=True,
                             reason='same-user daemon namespace path not available; not disproved')
            peer.check()
    except Missing as problem:
        report['unavailable'] = str(problem)
    except (Violation, AssertionError) as problem:
        probe.violation = True
        report['invariantFailure'] = str(problem)
    except (OSError, subprocess.SubprocessError, ValueError, KeyError) as problem:
        report['inconclusive'] = f'{type(problem).__name__}: {problem}'[:2048]
    finally:
        if peer:
            peer.close()
    report['results'] = probe.results
    report['invariantViolation'] = probe.violation
    Path(args.output).write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(report, separators=(',', ':')))
    return int(probe.violation)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', required=True)
    parser.add_argument('--image', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--helper', help='existing digest-verified helper; otherwise fetch manifest asset')
    parser.add_argument('--endpoint', default=os.environ.get('DOCKER_HOST', 'unix:///var/run/docker.sock'))
    return measure(parser.parse_args())


if __name__ == '__main__':
    sys.exit(main())

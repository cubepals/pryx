#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Record immutable inputs for the VM spikes from primary release metadata and actual bytes.

This discovery step hashes published archives without executing or extracting them. Its
report supplies pins for subsequent boot experiments; it neither ships a bundle nor enables
a VM tier. The small metadata token, when supplied by CI, stays on the GitHub API origin.
"""
import argparse
import hashlib
import http.client
from http import HTTPStatus
import json
import os
from pathlib import Path
import re
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request

ROOT = Path(__file__).parent
API = 'https://api.github.com'
MAX_METADATA_BYTES = 2 * 1024 * 1024
MAX_ARCHIVE_BYTES = 4 * 1024 * 1024 * 1024
READ_BYTES = 1024 * 1024
ATTEMPTS = 3
RETRYABLE = {HTTPStatus.REQUEST_TIMEOUT, HTTPStatus.TOO_MANY_REQUESTS,
             HTTPStatus.INTERNAL_SERVER_ERROR, HTTPStatus.BAD_GATEWAY,
             HTTPStatus.SERVICE_UNAVAILABLE, HTTPStatus.GATEWAY_TIMEOUT}


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, url):
        return None


class HttpsRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, url):
        if urllib.parse.urlsplit(url).scheme != 'https':
            raise ValueError('VM inventory redirects require HTTPS')
        return super().redirect_request(request, response, code, message, headers, url)


def request(url, consume, authenticated=False):
    if urllib.parse.urlsplit(url).scheme != 'https':
        raise ValueError('VM inventory requires HTTPS')
    headers = {'User-Agent': 'pryx-vm-input-inventory'}
    if authenticated:
        if urllib.parse.urlsplit(url).netloc != urllib.parse.urlsplit(API).netloc:
            raise ValueError('Inventory token cannot leave the GitHub API origin')
        token = os.environ.get('GH_TOKEN')
        if token:
            headers['Authorization'] = 'Bearer ' + token
    opener = urllib.request.build_opener(NoRedirect() if authenticated else HttpsRedirect())
    for attempt in range(ATTEMPTS):
        try:
            with opener.open(urllib.request.Request(url, headers=headers), timeout=60) as response:
                return consume(response)
        except urllib.error.HTTPError as error:
            if error.code not in RETRYABLE or attempt + 1 == ATTEMPTS:
                raise
        except (urllib.error.URLError, TimeoutError, ConnectionError, http.client.IncompleteRead):
            if attempt + 1 == ATTEMPTS:
                raise
        time.sleep(2 ** attempt)
    raise AssertionError('Unreachable retry state')


def api(path):
    def read(response):
        raw = response.read(MAX_METADATA_BYTES + 1)
        if len(raw) > MAX_METADATA_BYTES:
            raise ValueError('VM release metadata exceeds its bound')
        return json.loads(raw)
    return request(API + path, read, authenticated=True)


def source_commit(repository, tag):
    value = api(f'/repos/{repository}/git/ref/tags/{urllib.parse.quote(tag, safe="")}')['object']
    for _ in range(4):
        if value['type'] == 'commit':
            return value['sha']
        if value['type'] != 'tag':
            raise ValueError('Release tag does not identify a source commit')
        value = api(f'/repos/{repository}/git/tags/{value["sha"]}')['object']
    raise ValueError('Release tag indirection exceeds its bound')


def archive_digest(asset, expected_digest, cache=None):
    return acquire(asset['browser_download_url'], asset['size'], expected_digest, asset.get('digest'), cache)


def acquire(url, expected_size, expected_digest, declared_digest=None, cache=None):
    if not isinstance(expected_size, int) or not 0 < expected_size <= MAX_ARCHIVE_BYTES:
        raise ValueError('VM candidate archive size exceeds its bound')

    def read(response, destination=None):
        digest, size = hashlib.sha256(), 0
        while block := response.read(READ_BYTES):
            size += len(block)
            if size > expected_size:
                raise ValueError('VM candidate archive exceeds its declared size')
            digest.update(block)
            if destination is not None:
                destination.write(block)
        if size != expected_size:
            raise ValueError('VM candidate archive is incomplete')
        actual = digest.hexdigest()
        if actual != expected_digest:
            raise ValueError('VM candidate differs from its committed archive digest')
        if declared_digest is not None and declared_digest != f'{digest.name}:{actual}':
            raise ValueError('VM candidate differs from its upstream release digest')
        return actual
    if cache is not None:
        cache.mkdir(parents=True, exist_ok=True, mode=0o700)
        cached = cache / (expected_digest + '.archive')
        if cached.exists():
            with cached.open('rb') as source:
                return read(source)

        def save(response):
            temporary = None
            try:
                with tempfile.NamedTemporaryFile(dir=cache, delete=False) as output:
                    temporary = Path(output.name)
                    actual = read(response, output)
                temporary.replace(cached)
                return actual
            finally:
                if temporary is not None:
                    temporary.unlink(missing_ok=True)
        return request(url, save)
    # No API credential is attached to the public archive request or its redirects.
    return request(url, read)


def inventory(candidate, cache=None):
    repository, tag = candidate['repository'], candidate['tag']
    if not re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', repository):
        raise ValueError('Invalid upstream repository identity')
    release = api(f'/repos/{repository}/releases/tags/{urllib.parse.quote(tag, safe="")}')
    if release['tag_name'] != tag or release['draft'] or release['prerelease']:
        raise ValueError('VM inventory requires the named published stable release')
    assets = [asset for asset in release['assets'] if re.fullmatch(candidate['assetPattern'], asset['name'])]
    if len(assets) != 1:
        names = [asset['name'] for asset in release['assets']]
        raise ValueError(f'Candidate must select one published archive: {names}')
    asset = assets[0]
    commit = source_commit(repository, tag)
    if commit != candidate['sourceCommit']:
        raise ValueError('Release source differs from its committed identity')
    result = {**candidate, 'sourceCommit': commit,
              'releaseUrl': release['html_url'], 'assetId': asset['id'],
              'assetName': asset['name'], 'url': asset['browser_download_url'],
              'bytes': asset['size'], 'upstreamDigest': asset.get('digest'),
              'sha256': archive_digest(asset, candidate['sha256'], cache), 'executed': False}
    print(json.dumps(result), flush=True)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--catalog', type=Path, default=ROOT / 'candidates.json')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--archive-cache', type=Path)
    args = parser.parse_args()
    report = {'scope': 'Primary published candidate archives, hashed but never executed; not VM-tier proof',
              'candidates': []}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    for candidate in json.loads(args.catalog.read_text()):
        report['candidates'].append(inventory(candidate, args.archive_cache))
        args.output.write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    main()

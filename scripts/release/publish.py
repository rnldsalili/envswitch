#!/usr/bin/env python3
"""Single publisher. Called only by release.yml's gated contents:write job."""
import argparse
import json
import re
import subprocess
import tempfile
from pathlib import Path

from release import digest, directory_files, guard_latest, verify_assets, verify_checksums, version_number


def gh(*args, raw=False, optional=False):
    result = subprocess.run(['gh', *args], capture_output=True, check=False, timeout=180)
    if result.returncode:
        if optional and b'(HTTP 404)' in result.stderr:
            return None
        raise RuntimeError(f'gh failed: {result.stderr.decode(errors="replace").strip()}')
    return result.stdout if raw else json.loads(result.stdout)


def tag_sha(base, tag):
    ref = gh('api', f'{base}/git/ref/tags/{tag}', optional=True)
    if ref is None:
        return None
    obj = ref['object']
    for _ in range(10):
        if obj['type'] == 'commit':
            return obj['sha']
        if obj['type'] != 'tag':
            break
        obj = gh('api', f'{base}/git/tags/{obj["sha"]}')['object']
    raise ValueError('tag does not resolve to a commit')


def asset_list(base, release_id):
    pages = gh('api', f'{base}/releases/{release_id}/assets?per_page=100', '--paginate', '--slurp')
    return [asset for page in pages for asset in page]


def verify_remote(base, release_id, directory, kind, version, complete):
    assets = asset_list(base, release_id)
    names = [asset['name'] for asset in assets]
    wanted = {p.name for p in directory_files(directory)}
    if len(names) != len(set(names)) or not set(names) <= wanted:
        raise ValueError('unexpected or duplicate remote release assets')
    if complete:
        verify_assets(kind, version, names)
    with tempfile.TemporaryDirectory(prefix='envswitch-release-') as temp:
        downloaded = Path(temp)
        for asset in assets:
            if asset['state'] != 'uploaded':
                raise ValueError(f'asset upload not complete: {asset["name"]}')
            data = gh('api', f'{base}/releases/assets/{int(asset["id"])}',
                      '-H', 'Accept: application/octet-stream', raw=True)
            path = downloaded / asset['name']
            path.write_bytes(data)
            if digest(path) != digest(directory / asset['name']):
                raise ValueError(f'remote asset conflicts with this run: {asset["name"]}')
        if complete:
            verify_checksums(downloaded)
    return set(names)


def publish(repo, directory, kind, version, tag, sha):
    version_number(version)
    if tag != f'v{version}' or not re.fullmatch(r'[0-9a-f]{40}', sha):
        raise ValueError('invalid planned tag or SHA')
    if not re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', repo):
        raise ValueError('invalid repository')
    verify_assets(kind, version, [p.name for p in directory_files(directory)])
    verify_checksums(directory)
    base = f'repos/{repo}'
    existing_sha = tag_sha(base, tag)
    if existing_sha is not None and existing_sha != sha:
        raise ValueError(f'{tag} already points at another SHA')
    release = gh('api', f'{base}/releases/tags/{tag}', optional=True)
    if release is not None:
        if release['tag_name'] != tag or release['prerelease']:
            raise ValueError('existing release identity conflicts')
        if not release['draft']:
            if existing_sha != sha:
                raise ValueError('published release has no matching tag')
            verify_remote(base, release['id'], directory, kind, version, complete=True)
            print(f'{tag}: identical complete published release; no-op')
            return

    def check_latest():
        pages = gh('api', f'{base}/releases?per_page=100', '--paginate', '--slurp')
        guard_latest(version, pages)

    check_latest()
    # Check any partial draft before creating tags or uploading missing files.
    present = (verify_remote(base, release['id'], directory, kind, version, complete=False)
               if release is not None else set())
    if existing_sha is None:
        gh('api', f'{base}/git/refs', '--method', 'POST',
           '-f', f'ref=refs/tags/{tag}', '-f', f'sha={sha}')
    if release is None:
        body = (f'Source commit: {sha}\nRelease kind: {kind}\n\n'
                'Light migration: pre-bridge Bun installations must reinstall using the README instructions.\n')
        release = gh('api', f'{base}/releases', '--method', 'POST',
                     '-f', f'tag_name={tag}', '-f', f'target_commitish={sha}', '-f', f'name={tag}',
                     '-F', 'draft=true', '-F', 'prerelease=false', '-f', f'body={body}')
    for path in directory_files(directory):
        if path.name not in present:
            gh('release', 'upload', tag, str(path.resolve()), '--repo', repo, raw=True)
    verify_remote(base, release['id'], directory, kind, version, complete=True)
    if tag_sha(base, tag) != sha:
        raise ValueError('tag changed while preparing release')
    check_latest()
    result = gh('api', f'{base}/releases/{release["id"]}', '--method', 'PATCH',
                '-F', 'draft=false', '-F', 'prerelease=false', '-f', 'make_latest=true')
    if result['draft'] or result['prerelease'] or result['tag_name'] != tag:
        raise ValueError('promotion did not return the planned published release')
    print(f'{tag}: verified and published as latest')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('repo')
    parser.add_argument('directory', type=Path)
    parser.add_argument('kind', choices=['source', 'native'])
    parser.add_argument('version')
    parser.add_argument('tag')
    parser.add_argument('sha')
    args = parser.parse_args()
    publish(args.repo, args.directory, args.kind, args.version, args.tag, args.sha)

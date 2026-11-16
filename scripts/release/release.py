#!/usr/bin/env python3
"""Protocol v1 packaging and validation. Python 3.11+, standard library only."""
import argparse
import gzip
import hashlib
import io
import re
import shutil
import subprocess
import tarfile
import tomllib
import zipfile
from pathlib import Path

TARGETS = (
    'aarch64-apple-darwin', 'x86_64-apple-darwin',
    'x86_64-unknown-linux-musl', 'aarch64-unknown-linux-musl',
    'x86_64-pc-windows-msvc',
)


def version_number(version):
    if not re.fullmatch(r'0\.0\.(0|[1-9][0-9]*)', version):
        raise ValueError(f'invalid release version: {version!r}')
    return int(version[4:])


def archive_name(version, target):
    version_number(version)
    if target not in (*TARGETS, 'source'):
        raise ValueError(f'unsupported target: {target}')
    suffix = 'zip' if target.endswith('windows-msvc') else 'tar.gz'
    return f'envswitch_{version}_{target}.{suffix}'


def expected_assets(kind, version):
    if kind not in ('source', 'native'):
        raise ValueError(f'unsupported release kind: {kind}')
    return [archive_name(version, t) for t in (TARGETS if kind == 'native' else ('source',))] + [
        'install.sh', 'install.ps1', 'SHA256SUMS',
    ]


def verify_assets(kind, version, names):
    expected = set(expected_assets(kind, version))
    if len(names) != len(set(names)) or set(names) != expected:
        raise ValueError(f'asset set mismatch: expected {sorted(expected)}, got {sorted(names)}')


def stamp(root, version):
    """Edit only the local package version; never ask Cargo to update dependencies."""
    version_number(version)
    manifest = root / 'crates/envswitch/Cargo.toml'
    lock = root / 'Cargo.lock'
    manifest_text, lock_text = manifest.read_text(), lock.read_text()
    manifest_before = tomllib.loads(manifest_text)
    lock_before = tomllib.loads(lock_text)
    package = manifest_before['package']
    if package['name'] != 'envswitch':
        raise ValueError('expected the envswitch package')
    local = [p for p in lock_before['package'] if p['name'] == 'envswitch' and 'source' not in p]
    if len(local) != 1 or local[0]['version'] != package['version']:
        raise ValueError('manifest/lockfile local package mismatch')

    def replace_version(block):
        updated, count = re.subn(r'(?m)^version\s*=\s*"[^"\n]+"', f'version = "{version}"', block)
        if count != 1:
            raise ValueError('expected exactly one package version')
        return updated

    sections = re.split(r'(?m)(?=^\[)', manifest_text)
    sections = [replace_version(s) if s.startswith('[package]\n') else s for s in sections]
    stamped_manifest = ''.join(sections)
    entries = lock_text.split('[[package]]')
    changed = 0
    for index, entry in enumerate(entries[1:], 1):
        data = tomllib.loads(entry)
        if data.get('name') == 'envswitch' and 'source' not in data:
            entries[index] = replace_version(entry)
            changed += 1
    stamped_lock = '[[package]]'.join(entries)
    manifest_after = tomllib.loads(stamped_manifest)
    lock_after = tomllib.loads(stamped_lock)
    if changed != 1 or manifest_after['package']['version'] != version:
        raise ValueError('package stamping failed')
    manifest_after['package']['version'] = package['version']
    for entry in lock_after['package']:
        if entry['name'] == 'envswitch' and 'source' not in entry:
            entry['version'] = package['version']
    if manifest_before != manifest_after or lock_before != lock_after:
        raise ValueError('dependency graph changed during stamping')
    manifest.write_bytes(stamped_manifest.encode())
    lock.write_bytes(stamped_lock.encode())


def regular_file(path):
    if path.is_symlink() or not path.is_file():
        raise ValueError(f'expected a regular file: {path}')
    return path


def digest(path):
    with regular_file(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def directory_files(directory):
    files = sorted(directory.iterdir())
    for path in files:
        regular_file(path)
        if not re.fullmatch(r'[A-Za-z0-9_][A-Za-z0-9_.-]*', path.name):
            raise ValueError(f'unsafe asset basename: {path.name!r}')
    return files


def parse_checksums(data):
    if not data or not data.endswith(b'\n'):
        raise ValueError('SHA256SUMS requires LF lines and a final LF')
    entries = {}
    for line in data.splitlines(keepends=True):
        match = re.fullmatch(rb'([0-9a-f]{64})  ([A-Za-z0-9_][A-Za-z0-9_.-]*)\n', line)
        if not match:
            raise ValueError('malformed SHA256SUMS entry')
        checksum, name = (part.decode('ascii') for part in match.groups())
        if name == 'SHA256SUMS' or name in entries:
            raise ValueError('duplicate or self-entry in SHA256SUMS')
        entries[name] = checksum
    return entries


def checksums(directory):
    files = [p for p in directory_files(directory) if p.name != 'SHA256SUMS']
    if not files:
        raise ValueError('cannot checksum an empty release')
    data = ''.join(f'{digest(p)}  {p.name}\n' for p in files)
    (directory / 'SHA256SUMS').write_bytes(data.encode('ascii'))


def verify_checksums(directory):
    entries = parse_checksums(regular_file(directory / 'SHA256SUMS').read_bytes())
    files = [p for p in directory_files(directory) if p.name != 'SHA256SUMS']
    if set(entries) != {p.name for p in files}:
        raise ValueError('SHA256SUMS does not cover exactly the release files')
    for path in files:
        if entries[path.name] != digest(path):
            raise ValueError(f'SHA-256 mismatch: {path.name}')


def write_archive(path, files):
    for source, name, mode in files:
        regular_file(source)
    if path.suffix == '.zip':
        with zipfile.ZipFile(path, 'w', compression=zipfile.ZIP_DEFLATED) as archive:
            for source, name, mode in files:
                info = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0))
                info.create_system = 3
                info.external_attr = (0o100000 | mode) << 16
                archive.writestr(info, source.read_bytes(), compress_type=zipfile.ZIP_DEFLATED)
    else:
        with path.open('wb') as raw, gzip.GzipFile(filename='', mode='wb', fileobj=raw, mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode='w', format=tarfile.USTAR_FORMAT) as archive:
                for source, name, mode in files:
                    data = source.read_bytes()
                    info = tarfile.TarInfo(name)
                    info.size, info.mode = len(data), mode
                    archive.addfile(info, io.BytesIO(data))
    return path


def package_native(root, output, version, target, binary):
    if target not in TARGETS:
        raise ValueError(f'unsupported native target: {target}')
    files = [(binary, 'envswitch.exe' if target.endswith('windows-msvc') else 'envswitch', 0o755)]
    files += [(root / name, name, 0o644) for name in ('scripts/install.sh', 'scripts/install.ps1', 'README.md')]
    return write_archive(output / archive_name(version, target), files)


def package_source(root, output, version):
    names = ['package.json', 'bun.lock', 'README.md', 'tsconfig.json']
    for folder in ('src', 'scripts'):
        for path in sorted((root / folder).rglob('*')):
            relative = path.relative_to(root)
            if path.is_symlink() or any(p == '.git' or p == 'node_modules' or p == '.env' or p.startswith('.env.')
                                        or p == '__pycache__' for p in relative.parts):
                raise ValueError(f'unsafe source archive member: {relative}')
            if not path.is_dir():
                names.append(relative.as_posix())
    files = [(root / name, f'envswitch/{name}', 0o755 if name.endswith('.sh') else 0o644) for name in names]
    return write_archive(output / archive_name(version, 'source'), files)


def collect(artifacts, output, kind, version):
    expected = {'release-common'}
    if kind == 'native':
        expected.update(f'release-native-{t}' for t in TARGETS)
    containers = list(artifacts.iterdir())
    if {p.name for p in containers} != expected or any(p.is_symlink() or not p.is_dir() for p in containers):
        raise ValueError('unexpected or missing artifact containers')
    files = []
    for container in containers:
        members = directory_files(container)
        wanted = (['install.sh', 'install.ps1'] + ([archive_name(version, 'source')] if kind == 'source' else [])
                  if container.name == 'release-common' else
                  [archive_name(version, container.name.removeprefix('release-native-'))])
        if sorted(p.name for p in members) != sorted(wanted):
            raise ValueError(f'unexpected files in {container.name}')
        files.extend(members)
    verify_assets(kind, version, [p.name for p in files] + ['SHA256SUMS'])
    output.mkdir()  # refuse to mix this run with previous outputs
    for path in files:
        shutil.copyfile(path, output / path.name)
    checksums(output)
    verify_checksums(output)


def guard_latest(version, pages):
    candidate = version_number(version)
    for page in pages:
        for release in page:
            tag = release['tag_name']
            if not release['draft'] and re.fullmatch(r'v0\.0\.(0|[1-9][0-9]*)', tag):
                if version_number(tag[1:]) > candidate:
                    raise ValueError(f'refusing to regress latest from {tag} to v{version}')


def check_version(binary, version, target):
    version_number(version)
    if target not in TARGETS:
        raise ValueError(f'unsupported target: {target}')
    result = subprocess.run([str(binary.resolve()), '--version'], capture_output=True, check=False)
    if result.returncode or result.stderr or result.stdout != f'envswitch {version} {target}\n'.encode():
        raise ValueError(f'binary --version does not match envswitch {version} {target}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    p = sub.add_parser('stamp')
    p.add_argument('root', type=Path)
    p.add_argument('version')
    for command in ('checksums', 'verify-checksums'):
        p = sub.add_parser(command)
        p.add_argument('directory', type=Path)
    p = sub.add_parser('collect')
    p.add_argument('artifacts', type=Path)
    p.add_argument('output', type=Path)
    p.add_argument('kind', choices=['source', 'native'])
    p.add_argument('version')
    for command in ('package', 'check-version'):
        p = sub.add_parser(command)
        p.add_argument('version')
        p.add_argument('target', choices=[*TARGETS, 'source'] if command == 'package' else TARGETS)
        p.add_argument('--binary', type=Path)
        if command == 'package':
            p.add_argument('--root', type=Path, default=Path('.'))
            p.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if args.command == 'stamp':
        stamp(args.root, args.version)
    elif args.command == 'checksums':
        checksums(args.directory)
    elif args.command == 'verify-checksums':
        verify_checksums(args.directory)
    elif args.command == 'collect':
        collect(args.artifacts, args.output, args.kind, args.version)
    elif args.command == 'check-version':
        if args.binary is None:
            parser.error('--binary is required')
        check_version(args.binary, args.version, args.target)
    elif args.command == 'package':
        args.output.mkdir(parents=True, exist_ok=True)
        if args.target == 'source':
            print(package_source(args.root, args.output, args.version))
        else:
            if args.binary is None:
                parser.error('--binary is required for native packages')
            print(package_native(args.root, args.output, args.version, args.target, args.binary))


if __name__ == '__main__':
    main()

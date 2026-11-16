"""Release helper contracts; run with python3 -m unittest discover -s test/release."""
import hashlib
import importlib.util
import os
from pathlib import Path
import shutil
import sys
import tarfile
import tempfile
import tomllib
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[2]
HELPER = ROOT / 'scripts/release/release.py'
sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('release', HELPER)
r = importlib.util.module_from_spec(spec)
# Missing helpers are an expected failure until implementation lands.
if HELPER.exists():
    spec.loader.exec_module(r)

NATIVE = [
    'envswitch_0.0.42_aarch64-apple-darwin.tar.gz',
    'envswitch_0.0.42_x86_64-apple-darwin.tar.gz',
    'envswitch_0.0.42_x86_64-unknown-linux-musl.tar.gz',
    'envswitch_0.0.42_aarch64-unknown-linux-musl.tar.gz',
    'envswitch_0.0.42_x86_64-pc-windows-msvc.zip',
    'install.sh', 'install.ps1', 'SHA256SUMS',
]
SOURCE = ['envswitch_0.0.42_source.tar.gz', 'install.sh', 'install.ps1', 'SHA256SUMS']


class Helpers(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)

    def test_stamp_changes_only_workspace_package_version_in_temp_copy(self):
        if not (ROOT / 'Cargo.toml').exists():
            self.skipTest('M1a source-only checkout has no Rust workspace')
        for name in ['Cargo.toml', 'Cargo.lock', 'crates/envswitch/Cargo.toml']:
            dest = self.root / name
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, dest)
        before = tomllib.loads((self.root / 'Cargo.lock').read_text())
        r.stamp(self.root, '0.0.42')
        after = tomllib.loads((self.root / 'Cargo.lock').read_text())
        self.assertEqual(tomllib.loads((self.root / 'crates/envswitch/Cargo.toml').read_text())['package']['version'], '0.0.42')
        local = [p for p in after['package'] if p['name'] == 'envswitch' and 'source' not in p]
        self.assertEqual(len(local), 1)
        self.assertEqual(local[0]['version'], '0.0.42')
        local[0]['version'] = '0.0.0'
        self.assertEqual(before, after)
        r.stamp(self.root, '0.0.42')  # rerun is safe

    def test_stamp_refuses_invalid_identity_before_mutation(self):
        for version in ['v0.0.42', '0.0.042', '1.0.0', '0.0.-1', '0.0.42\n']:
            with self.subTest(version=version), self.assertRaises(ValueError):
                r.stamp(self.root, version)

    def test_asset_sets_reject_missing_extra_duplicate_for_both_kinds(self):
        for kind, names in [('source', SOURCE), ('native', NATIVE)]:
            r.verify_assets(kind, '0.0.42', names)
            for invalid in [names[:-1], names + ['other.zip'], names + [names[0]]]:
                with self.subTest(kind=kind, names=invalid), self.assertRaises(ValueError):
                    r.verify_assets(kind, '0.0.42', invalid)
        with self.assertRaises(ValueError):
            r.verify_assets('native', '0.0.42', NATIVE + [SOURCE[0]])

    def test_checksums_are_canonical_and_verify_bytes(self):
        (self.root / 'z.zip').write_bytes(b'abc')
        (self.root / 'install.sh').write_bytes(b'')
        r.checksums(self.root)
        digest = hashlib.sha256(b'abc').hexdigest()
        self.assertEqual((self.root / 'SHA256SUMS').read_bytes(),
                         ('e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  install.sh\n'
                          + digest + '  z.zip\n').encode())
        r.verify_checksums(self.root)
        r.checksums(self.root)  # excludes the old manifest
        r.verify_checksums(self.root)
        (self.root / 'z.zip').write_bytes(b'bad')
        with self.assertRaises(ValueError):
            r.verify_checksums(self.root)

    def test_checksums_reject_noncanonical_duplicate_missing_and_self_entries(self):
        line = b'0' * 64 + b'  install.sh\n'
        for data in [b'', line[:-1], line.replace(b'\n', b'\r\n'), line.replace(b'  ', b' '),
                     b'A' * 64 + b'  install.sh\n', line + line,
                     b'0' * 64 + b'  ../install.sh\n', b'0' * 64 + b'  SHA256SUMS\n']:
            with self.subTest(data=data), self.assertRaises(ValueError):
                r.parse_checksums(data)
        (self.root / 'SHA256SUMS').write_bytes(line)
        with self.assertRaises(ValueError):
            r.verify_checksums(self.root)
        (self.root / 'install.sh').write_bytes(b'')
        r.checksums(self.root)
        (self.root / 'extra').write_bytes(b'')
        with self.assertRaises(ValueError):
            r.verify_checksums(self.root)

    def payload(self):
        (self.root / 'scripts').mkdir()
        for name in ['scripts/install.sh', 'scripts/install.ps1', 'README.md']:
            (self.root / name).write_text(name + '\n')
        binary = self.root / 'envswitch'
        binary.write_bytes(b'fixture binary')
        return binary

    def test_native_archives_have_exact_protocol_layout_and_executable_mode(self):
        binary = self.payload()
        out = self.root / 'out'
        out.mkdir()
        archive = r.package_native(self.root, out, '0.0.42', 'aarch64-apple-darwin', binary)
        with tarfile.open(archive) as tf:
            self.assertEqual(sorted(tf.getnames()), ['README.md', 'envswitch', 'scripts/install.ps1', 'scripts/install.sh'])
            self.assertTrue(all(m.isfile() for m in tf))
            self.assertEqual(tf.getmember('envswitch').mode, 0o755)
            self.assertEqual(tf.extractfile('scripts/install.sh').read(), b'scripts/install.sh\n')
        archive = r.package_native(self.root, out, '0.0.42', 'x86_64-pc-windows-msvc', binary)
        with zipfile.ZipFile(archive) as zf:
            self.assertEqual(sorted(zf.namelist()), ['README.md', 'envswitch.exe', 'scripts/install.ps1', 'scripts/install.sh'])
            self.assertEqual(zf.read('envswitch.exe'), b'fixture binary')

    def test_source_layout_and_symlink_rejection(self):
        self.payload()
        (self.root / 'src').mkdir()
        (self.root / 'src/cli.ts').write_text('console.log("fixture");\n')
        for name in ['package.json', 'bun.lock', 'tsconfig.json']:
            (self.root / name).write_text('{}\n')
        out = self.root / 'out'
        out.mkdir()
        archive = r.package_source(self.root, out, '0.0.42')
        with tarfile.open(archive) as tf:
            self.assertIn('envswitch/src/cli.ts', tf.getnames())
            self.assertNotIn('envswitch/envswitch', tf.getnames())
            self.assertEqual(tf.extractfile('envswitch/scripts/install.ps1').read(), b'scripts/install.ps1\n')
        if os.name != 'nt':
            (self.root / 'src/secret').symlink_to('/etc/passwd')
            with self.assertRaises(ValueError):
                r.package_source(self.root, out, '0.0.42')

    def test_artifact_collection_requires_exact_container_names_and_payloads(self):
        artifacts = self.root / 'artifacts'
        common = artifacts / 'release-common'
        common.mkdir(parents=True)
        for name in SOURCE[:-1]:
            (common / name).write_bytes(b'x')
        out = self.root / 'release'
        r.collect(artifacts, out, 'source', '0.0.42')
        r.verify_assets('source', '0.0.42', [p.name for p in out.iterdir()])
        r.verify_checksums(out)
        (artifacts / 'release-unexpected').mkdir()
        with self.assertRaises(ValueError):
            r.collect(artifacts, self.root / 'other', 'source', '0.0.42')

    def test_native_artifact_collection_rejects_missing_target_and_wrong_container_payload(self):
        artifacts = self.root / 'artifacts'
        common = artifacts / 'release-common'
        common.mkdir(parents=True)
        for name in ['install.sh', 'install.ps1']:
            (common / name).write_bytes(b'installer')
        for name in NATIVE[:5]:
            target = name.removeprefix('envswitch_0.0.42_').removesuffix('.tar.gz').removesuffix('.zip')
            container = artifacts / ('release-native-' + target)
            container.mkdir()
            (container / name).write_bytes(b'archive')
        r.collect(artifacts, self.root / 'release', 'native', '0.0.42')
        r.verify_checksums(self.root / 'release')
        (container / name).rename(container / 'wrong.zip')
        with self.assertRaises(ValueError):
            r.collect(artifacts, self.root / 'wrong', 'native', '0.0.42')
        shutil.rmtree(container)
        with self.assertRaises(ValueError):
            r.collect(artifacts, self.root / 'missing', 'native', '0.0.42')

    def test_latest_guard_handles_pagination_numeric_order_drafts_and_equal_versions(self):
        pages = [[{'tag_name': 'v0.0.9', 'draft': False}],
                 [{'tag_name': 'v0.0.100', 'draft': True}]]
        r.guard_latest('0.0.10', pages)
        r.guard_latest('0.0.9', pages)
        pages[1][0]['draft'] = False
        with self.assertRaises(ValueError):
            r.guard_latest('0.0.10', pages)
        r.guard_latest('0.0.100', pages)
        with self.assertRaises(ValueError):
            r.guard_latest('0.0.010', pages)

    def test_version_check_compares_exact_bytes_and_stderr(self):
        if os.name == 'nt':
            self.skipTest('fixture uses Unix executable shebang')
        binary = self.root / 'envswitch'
        binary.write_text('#!/bin/sh\nprintf "envswitch 0.0.42 aarch64-apple-darwin\\n"\n')
        binary.chmod(0o755)
        r.check_version(binary, '0.0.42', 'aarch64-apple-darwin')
        with self.assertRaises(ValueError):
            r.check_version(binary, '0.0.43', 'aarch64-apple-darwin')
        binary.write_text('#!/bin/sh\nprintf "envswitch 0.0.42 aarch64-apple-darwin\\n\\n"\n')
        with self.assertRaises(ValueError):
            r.check_version(binary, '0.0.42', 'aarch64-apple-darwin')

        for extra in ['printf error >&2', 'exit 1']:
            binary.write_text('#!/bin/sh\nprintf "envswitch 0.0.42 aarch64-apple-darwin\\n"\n' + extra + '\n')
            with self.assertRaises(ValueError):
                r.check_version(binary, '0.0.42', 'aarch64-apple-darwin')


if __name__ == '__main__':
    unittest.main()

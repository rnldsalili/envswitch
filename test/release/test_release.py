"""Release helper contracts; run with python3 -m unittest discover -s test/release."""
import hashlib
import io
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
sys.dont_write_bytecode = True
sys.path.insert(0, str(ROOT / 'scripts/release'))
import release as r

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
        for name in ['Cargo.toml', 'Cargo.lock', 'crates/envswitch/Cargo.toml']:
            dest = self.root / name
            dest.parent.mkdir(parents=True, exist_ok=True)
            if (ROOT / name).exists():
                shutil.copyfile(ROOT / name, dest)
            else:
                fixtures = {
                    'Cargo.toml': '[workspace]\nmembers = ["crates/envswitch"]\n',
                    'crates/envswitch/Cargo.toml': '[package]\nname = "envswitch"\nversion = "0.0.0"\ndescription = "café"\n',
                    'Cargo.lock': 'version = 4\n\n[[package]]\nname = "envswitch"\nversion = "0.0.0"\n',
                }
                dest.write_text(fixtures[name], encoding='utf-8')
        before = tomllib.loads((self.root / 'Cargo.lock').read_text(encoding='utf-8'))
        r.stamp(self.root, '0.0.42')
        after = tomllib.loads((self.root / 'Cargo.lock').read_text(encoding='utf-8'))
        self.assertEqual(tomllib.loads((self.root / 'crates/envswitch/Cargo.toml').read_text(encoding='utf-8'))['package']['version'], '0.0.42')
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
            (self.root / name).write_text('{"name": "envswitch"}\n', encoding='utf-8')
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

    def artifacts(self, kind):
        fixture = Path(tempfile.mkdtemp(dir=self.root))
        payload = fixture / 'payload'
        (payload / 'scripts').mkdir(parents=True)
        (payload / 'src').mkdir()
        for name in ['scripts/install.sh', 'scripts/install.ps1', 'README.md', 'src/cli.ts', 'bun.lock', 'tsconfig.json']:
            (payload / name).write_bytes(name.encode())
        (payload / 'package.json').write_bytes(b'{"name":"envswitch"}')
        binary = payload / 'envswitch'
        binary.write_bytes(b'fixture binary')
        artifacts = fixture / 'artifacts'
        common = artifacts / 'release-common'
        common.mkdir(parents=True)
        for name in ['install.sh', 'install.ps1']:
            shutil.copyfile(payload / 'scripts' / name, common / name)
        if kind == 'source':
            r.package_source(payload, common, '0.0.42')
        else:
            for target in r.TARGETS:
                container = artifacts / ('release-native-' + target)
                container.mkdir()
                r.package_native(payload, container, '0.0.42', target, binary)
        return artifacts, common

    def test_artifact_collection_requires_exact_container_names_and_payloads(self):
        for kind in ['source', 'native']:
            artifacts, common = self.artifacts(kind)
            out = artifacts.parent / 'release'
            r.collect(artifacts, out, kind, '0.0.42')
            r.verify_assets(kind, '0.0.42', [p.name for p in out.iterdir()])
            r.verify_checksums(out)
            (artifacts / 'release-unexpected').mkdir()
            with self.assertRaises(ValueError):
                r.collect(artifacts, artifacts.parent / 'other', kind, '0.0.42')

    def rewrite_tar(self, path, mutation):
        with tarfile.open(path) as archive:
            entries = [(m, archive.extractfile(m).read() if m.isfile() else b'') for m in archive]
        mutation(entries)
        with tarfile.open(path, 'w:gz') as archive:
            for info, data in entries:
                info.size = len(data)
                archive.addfile(info, io.BytesIO(data) if info.isfile() else None)

    def test_collect_rejects_bad_native_tar_contents_types_modes_and_script_bytes(self):
        for mutation in ['extra', 'missing', 'link', 'mode', 'script', 'duplicate', 'path']:
            with self.subTest(mutation=mutation):
                artifacts, common = self.artifacts('native')
                path = next((artifacts / 'release-native-aarch64-apple-darwin').iterdir())
                def change(entries):
                    if mutation == 'extra':
                        entries.append((tarfile.TarInfo('extra.txt'), b''))
                    elif mutation == 'missing':
                        entries.pop()
                    elif mutation == 'link':
                        entries[0][0].type = tarfile.SYMTYPE
                        entries[0][0].linkname = '/etc/passwd'
                    elif mutation == 'mode':
                        entries[0][0].mode = 0o644
                    elif mutation == 'script':
                        entries[1] = (entries[1][0], b'wrong script')
                    elif mutation == 'duplicate':
                        entries.append(entries[0])
                    else:
                        entries[0][0].name = '../envswitch'
                self.rewrite_tar(path, change)
                with self.assertRaises(ValueError):
                    r.collect(artifacts, artifacts.parent / 'release', 'native', '0.0.42')

    def test_collect_rejects_bad_native_zip_modes_types_layout_and_script_bytes(self):
        for mutation in ['extra', 'link', 'reparse', 'mode', 'script']:
            with self.subTest(mutation=mutation):
                artifacts, common = self.artifacts('native')
                path = next((artifacts / 'release-native-x86_64-pc-windows-msvc').iterdir())
                with zipfile.ZipFile(path) as archive:
                    entries = [(m, archive.read(m)) for m in archive.infolist()]
                if mutation == 'extra':
                    entries.append((zipfile.ZipInfo('extra'), b''))
                elif mutation == 'link':
                    entries[0][0].external_attr = 0o120755 << 16
                elif mutation == 'reparse':
                    entries[0][0].external_attr |= 0x400
                elif mutation == 'mode':
                    entries[0][0].external_attr = 0o100644 << 16
                else:
                    entries[1] = (entries[1][0], b'wrong script')
                with zipfile.ZipFile(path, 'w') as archive:
                    for info, data in entries:
                        archive.writestr(info, data)
                with self.assertRaises(ValueError):
                    r.collect(artifacts, artifacts.parent / 'release', 'native', '0.0.42')

    def test_collect_rejects_source_missing_required_files_wrong_root_package_and_scripts(self):
        for mutation in ['missing', 'root', 'package', 'script', 'unsafe', 'link']:
            with self.subTest(mutation=mutation):
                artifacts, common = self.artifacts('source')
                path = common / 'envswitch_0.0.42_source.tar.gz'
                def change(entries):
                    if mutation == 'missing':
                        entries[:] = [(m, d) for m, d in entries if m.name != 'envswitch/src/cli.ts']
                    elif mutation == 'root':
                        entries[0][0].name = 'package.json'
                    elif mutation == 'unsafe':
                        entries.append((tarfile.TarInfo('envswitch/src/.env'), b''))
                    elif mutation == 'link':
                        entries[0][0].type = tarfile.LNKTYPE
                        entries[0][0].linkname = 'elsewhere'
                    else:
                        name = 'envswitch/package.json' if mutation == 'package' else 'envswitch/scripts/install.sh'
                        entries[:] = [(m, b'{"name":"wrong"}' if mutation == 'package' else b'wrong script')
                                      if m.name == name else (m, d) for m, d in entries]
                self.rewrite_tar(path, change)
                with self.assertRaises(ValueError):
                    r.collect(artifacts, artifacts.parent / 'release', 'source', '0.0.42')

    def test_collect_accepts_protocol_directory_entries(self):
        for kind in ['source', 'native']:
            artifacts, common = self.artifacts(kind)
            path = (common / 'envswitch_0.0.42_source.tar.gz' if kind == 'source' else
                    next((artifacts / 'release-native-aarch64-apple-darwin').iterdir()))
            def add_directory(entries):
                info = tarfile.TarInfo('envswitch/' if kind == 'source' else 'scripts/')
                info.type, info.mode = tarfile.DIRTYPE, 0o755
                entries.append((info, b''))
            self.rewrite_tar(path, add_directory)
            r.collect(artifacts, artifacts.parent / 'release', kind, '0.0.42')

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

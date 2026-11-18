"""The release workflow is the only identity and publication authority."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.dont_write_bytecode = True
sys.path.insert(0, str(ROOT / 'scripts/release'))
import release as r


class Planning(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.env = dict(GITHUB_RUN_NUMBER='42', GITHUB_SHA='a' * 40,
                        GITHUB_EVENT_NAME='push', GITHUB_REF='refs/heads/main',
                        DISPATCH_KIND='', PUBLISH_REQUESTED='')

    def rust_workspace(self):
        manifest = self.root / 'crates/envswitch/Cargo.toml'
        manifest.parent.mkdir(parents=True)
        manifest.touch()
        (self.root / 'rust-toolchain.toml').touch()

    def test_push_selects_native_with_both_rust_files_and_source_only_with_the_ts_app(self):
        with self.assertRaises(ValueError):  # neither Rust files nor package.json
            r.plan(self.root, self.env)
        (self.root / 'package.json').write_bytes(b'{}')
        self.assertEqual(r.plan(self.root, self.env)['kind'], 'source')
        self.rust_workspace()
        result = r.plan(self.root, self.env)
        self.assertEqual(result['kind'], 'native')
        self.assertEqual((result['version'], result['tag'], result['sha']), ('0.0.42', 'v0.0.42', 'a' * 40))
        (self.root / 'package.json').unlink()
        self.assertEqual(r.plan(self.root, self.env)['kind'], 'native')
        (self.root / 'rust-toolchain.toml').unlink()
        with self.assertRaises(ValueError):
            r.plan(self.root, self.env)

    def test_pushes_publish_only_on_main(self):
        self.rust_workspace()
        for ref, expected in (('refs/heads/main', 'true'), ('refs/heads/topic', 'false'), ('refs/tags/v0.0.42', 'false')):
            self.env['GITHUB_REF'] = ref
            self.assertEqual(r.plan(self.root, self.env)['publish'], expected)
        self.env.update(GITHUB_REF='refs/heads/main', GITHUB_EVENT_NAME='pull_request')
        self.assertEqual(r.plan(self.root, self.env)['publish'], 'false')

    def test_dispatch_kind_is_explicit_and_publication_requires_main_and_true(self):
        self.rust_workspace()
        (self.root / 'package.json').write_bytes(b'{}')
        self.env.update(GITHUB_EVENT_NAME='workflow_dispatch')
        for kind in ('source', 'native'):
            self.env['DISPATCH_KIND'] = kind
            for ref in ('refs/heads/main', 'refs/heads/topic', 'refs/tags/v0.0.42'):
                self.env.update(GITHUB_REF=ref, PUBLISH_REQUESTED='false')
                self.assertEqual(r.plan(self.root, self.env)['publish'], 'false')
                self.env['PUBLISH_REQUESTED'] = 'true'
                if ref == 'refs/heads/main':
                    self.assertEqual(r.plan(self.root, self.env)['publish'], 'true')
                else:
                    with self.assertRaises(ValueError):
                        r.plan(self.root, self.env)
        self.env.update(DISPATCH_KIND='', PUBLISH_REQUESTED='false')
        with self.assertRaises(ValueError):
            r.plan(self.root, self.env)

    def test_source_dispatch_is_refused_without_the_ts_app(self):
        self.rust_workspace()
        self.env.update(GITHUB_EVENT_NAME='workflow_dispatch', DISPATCH_KIND='source', PUBLISH_REQUESTED='false')
        with self.assertRaisesRegex(ValueError, 'TypeScript app'):
            r.plan(self.root, self.env)

    def test_plan_cli_writes_workflow_outputs(self):
        self.rust_workspace()
        output = self.root / 'outputs'
        subprocess.run([sys.executable, str(ROOT / 'scripts/release/release.py'), 'plan', str(self.root)],
                       env=dict(os.environ, **self.env, GITHUB_OUTPUT=str(output)), check=True, capture_output=True)
        outputs = dict(line.split('=', 1) for line in output.read_text().splitlines())
        self.assertEqual((outputs['kind'], outputs['tag'], outputs['publish']), ('native', 'v0.0.42', 'true'))
        self.assertNotIn('bun', outputs)

    def test_linkage_checks_reject_non_system_macos_libs_and_windows_dynamic_crt(self):
        r.check_linkage_output('aarch64-apple-darwin', 'envswitch:\n\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0)\n')
        r.check_linkage_output('x86_64-apple-darwin', 'envswitch:\n\t/System/Library/Frameworks/Security.framework/Security (compatibility version 1.0.0)\n')
        for library in ('/opt/homebrew/lib/libcustom.dylib', '@rpath/libcustom.dylib'):
            with self.assertRaises(ValueError):
                r.check_linkage_output('aarch64-apple-darwin', 'envswitch:\n\t' + library + ' (compatibility version 1.0.0)\n')
        r.check_linkage_output('x86_64-pc-windows-msvc', 'Dump of file envswitch.exe\n KERNEL32.dll\n')
        for library in ('VCRUNTIME140.dll', 'vcruntime140_1.dll'):
            with self.assertRaises(ValueError):
                r.check_linkage_output('x86_64-pc-windows-msvc', 'Dump of file envswitch.exe\n ' + library)
        with self.assertRaises(ValueError):
            r.check_linkage_output('aarch64-apple-darwin', '')


if __name__ == '__main__':
    unittest.main()

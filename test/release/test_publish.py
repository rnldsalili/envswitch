"""Publication sequencing with an in-memory GitHub transport; never calls the network."""
import copy
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.dont_write_bytecode = True
sys.path.insert(0, str(ROOT / 'scripts/release'))
import release as r

spec = importlib.util.spec_from_file_location('publish', ROOT / 'scripts/release/publish.py')
p = importlib.util.module_from_spec(spec)
if Path(spec.origin).exists():
    spec.loader.exec_module(p)

SHA = 'a' * 40


class GitHub:
    """Only the REST/CLI boundary is replaced; release validation stays real."""
    def __init__(self):
        self.tag = None
        self.release = None
        self.assets = {}
        self.events = []
        self.published = []
        self.annotated = {}

    def gh(self, *args, raw=False, optional=False):
        if args[:2] == ('release', 'upload'):
            path = Path(args[3])
            self.assets[path.name] = path.read_bytes()
            self.events.append('upload')
            return b''
        path = args[1]
        method = args[args.index('--method') + 1] if '--method' in args else 'GET'
        fields = dict(args[i + 1].split('=', 1) for i, arg in enumerate(args) if arg in ('-f', '-F'))
        if '/git/ref/tags/' in path:
            return None if self.tag is None else {'object': copy.deepcopy(self.tag)}
        if '/git/tags/' in path:
            return {'object': self.annotated[path.rsplit('/', 1)[-1]]}
        if path.endswith('/git/refs'):
            self.tag = {'type': 'commit', 'sha': fields['sha']}
            self.events.append('tag')
            return {}
        if '/releases/tags/' in path:
            return copy.deepcopy(self.release)
        if '/assets?per_page=' in path:
            return [[{'id': i + 1, 'name': n, 'state': 'uploaded'} for i, n in enumerate(self.assets)]]
        if '/releases/assets/' in path:
            return list(self.assets.values())[int(path.rsplit('/', 1)[-1]) - 1]
        if path.endswith('/releases?per_page=100'):
            return [copy.deepcopy(self.published)]
        if path.endswith('/releases') and method == 'POST':
            self.release = {'id': 7, 'draft': True, 'prerelease': False, 'tag_name': fields['tag_name']}
            self.events.append('draft')
            return copy.deepcopy(self.release)
        if path.endswith('/releases/7'):
            if method == 'PATCH':
                self.release['draft'] = False
                self.events.append('promote')
            return copy.deepcopy(self.release)
        raise AssertionError(f'unexpected GitHub operation: {args}')


class Publisher(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.directory = Path(self.tmp.name)
        for name in ['envswitch_0.0.42_source.tar.gz', 'install.sh', 'install.ps1']:
            (self.directory / name).write_bytes(name.encode())
        r.checksums(self.directory)
        self.remote = GitHub()

    def publish(self):
        with patch.object(p, 'gh', self.remote.gh):
            return p.publish('demo-suite/envswitch', self.directory, 'source', '0.0.42', 'v0.0.42', SHA)

    def existing(self, draft=False):
        self.remote.tag = {'type': 'commit', 'sha': SHA}
        self.remote.release = {'id': 7, 'draft': draft, 'prerelease': False, 'tag_name': 'v0.0.42'}
        self.remote.assets = {f.name: f.read_bytes() for f in self.directory.iterdir()}

    def test_new_release_is_verified_in_draft_before_promotion(self):
        self.publish()
        self.assertFalse(self.remote.release['draft'])
        self.assertEqual(self.remote.events[:2], ['tag', 'draft'])
        self.assertEqual(self.remote.events[-1], 'promote')
        self.assertEqual(len(self.remote.assets), 4)

    def test_identical_published_release_is_a_noop_even_after_newer_publication(self):
        self.existing()
        self.remote.published = [{'tag_name': 'v0.0.100', 'draft': False}]
        self.publish()
        self.assertEqual(self.remote.events, [])

    def test_conflicting_tag_or_public_payload_fails_without_writes(self):
        self.existing()
        self.remote.tag['sha'] = 'b' * 40
        with self.assertRaises(ValueError):
            self.publish()
        self.assertEqual(self.remote.events, [])
        self.remote.tag['sha'] = SHA
        self.remote.assets['install.sh'] = b'conflict'
        with self.assertRaises(ValueError):
            self.publish()
        self.assertEqual(self.remote.events, [])

    def test_higher_published_version_blocks_tag_creation(self):
        self.remote.published = [{'tag_name': 'v0.0.100', 'draft': False}]
        with self.assertRaises(ValueError):
            self.publish()
        self.assertEqual(self.remote.events, [])

    def test_incomplete_published_release_is_never_repaired(self):
        self.existing()
        del self.remote.assets['install.sh']
        with self.assertRaises(ValueError):
            self.publish()
        self.assertEqual(self.remote.events, [])

    def test_partial_draft_resumes_without_overwriting_existing_assets(self):
        self.existing(draft=True)
        del self.remote.assets['install.sh']
        self.publish()
        self.assertEqual(self.remote.events, ['upload', 'promote'])

    def test_extra_duplicate_or_corrupt_draft_assets_cannot_be_promoted(self):
        self.existing(draft=True)
        self.remote.assets['unexpected'] = b'extra'
        with self.assertRaises(ValueError):
            self.publish()
        self.assertEqual(self.remote.events, [])
        del self.remote.assets['unexpected']
        self.remote.assets['install.ps1'] = b'bad'
        with self.assertRaises(ValueError):
            self.publish()
        self.assertEqual(self.remote.events, [])

    def test_corrupt_uploaded_asset_leaves_release_in_draft(self):
        original = self.remote.gh
        def corrupt_upload(*args, **kwargs):
            result = original(*args, **kwargs)
            if args[:2] == ('release', 'upload') and Path(args[3]).name == 'install.sh':
                self.remote.assets['install.sh'] = b'corrupted in transit'
            return result
        with patch.object(p, 'gh', corrupt_upload), self.assertRaises(ValueError):
            p.publish('demo-suite/envswitch', self.directory, 'source', '0.0.42', 'v0.0.42', SHA)
        self.assertTrue(self.remote.release['draft'])
        self.assertNotIn('promote', self.remote.events)

    def test_duplicate_remote_metadata_fails_before_writes(self):
        self.existing(draft=True)
        original = self.remote.gh
        def duplicate_assets(*args, **kwargs):
            result = original(*args, **kwargs)
            if '/assets?per_page=' in args[1]:
                result[0].append(result[0][0])
            return result
        with patch.object(p, 'gh', duplicate_assets), self.assertRaises(ValueError):
            p.publish('demo-suite/envswitch', self.directory, 'source', '0.0.42', 'v0.0.42', SHA)
        self.assertEqual(self.remote.events, [])

    def test_annotated_tag_is_peeled_to_commit(self):
        self.existing()
        self.remote.tag = {'type': 'tag', 'sha': 'c' * 40}
        self.remote.annotated['c' * 40] = {'type': 'commit', 'sha': SHA}
        self.publish()
        self.assertEqual(self.remote.events, [])

    def test_promotion_rechecks_latest_after_upload(self):
        original = self.remote.gh
        def concurrent_release(*args, **kwargs):
            result = original(*args, **kwargs)
            if args[:2] == ('release', 'upload'):
                self.remote.published = [{'tag_name': 'v0.0.100', 'draft': False}]
            return result
        with patch.object(p, 'gh', concurrent_release), self.assertRaises(ValueError):
            p.publish('demo-suite/envswitch', self.directory, 'source', '0.0.42', 'v0.0.42', SHA)
        self.assertTrue(self.remote.release['draft'])
        self.assertNotIn('promote', self.remote.events)


if __name__ == '__main__':
    unittest.main()

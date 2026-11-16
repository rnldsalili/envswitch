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
        self.states = {}
        self.ids = {}

    def gh(self, *args, raw=False, optional=False):
        path = args[1]
        method = args[args.index('--method') + 1] if '--method' in args else 'GET'
        fields = dict(args[i + 1].split('=', 1) for i, arg in enumerate(args) if arg in ('-f', '-F'))
        if path.startswith('https://uploads.github.com/'):
            assert '/releases/7/assets?name=' in path
            name = path.split('?name=', 1)[1]
            file = Path(args[args.index('--input') + 1])
            self.assets[name] = file.read_bytes()
            self.states[name] = 'uploaded'
            self.events.append('upload')
            return {}
        if '/git/ref/tags/' in path:
            return None if self.tag is None else {'object': copy.deepcopy(self.tag)}
        if '/git/tags/' in path:
            return {'object': self.annotated[path.rsplit('/', 1)[-1]]}
        if path.endswith('/git/refs'):
            self.tag = {'type': 'commit', 'sha': fields['sha']}
            self.events.append('tag')
            return {}
        if '/releases/tags/' in path:
            if self.release is None or self.release['draft']:
                if optional:
                    return None
                raise RuntimeError('gh failed: HTTP 404')
            return copy.deepcopy(self.release)
        if '/assets?per_page=' in path:
            for name in self.assets:
                self.ids.setdefault(name, len(self.ids) + 1)
            return [[{'id': self.ids[n], 'name': n, 'state': self.states.get(n, 'uploaded')} for n in self.assets]]
        if '/releases/assets/' in path:
            name, = [n for n, asset_id in self.ids.items() if asset_id == int(path.rsplit('/', 1)[-1])]
            if method == 'DELETE':
                del self.assets[name]
                self.events.append('delete')
                return b''
            return self.assets[name]
        if path.endswith('/releases?per_page=100'):
            return [copy.deepcopy(self.published), [copy.deepcopy(self.release)] if self.release else []]
        if path.endswith('/releases') and method == 'POST':
            self.release = {'id': 7, 'draft': True, 'prerelease': False, 'tag_name': fields['tag_name'], 'body': fields['body']}
            self.events.append('draft')
            return copy.deepcopy(self.release)
        if path.endswith('/releases/7'):
            if method == 'PATCH':
                self.release['draft'] = False
                self.release['body'] = fields['body']
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

    def test_extra_draft_assets_are_refused_and_conflicting_draft_bytes_can_be_replaced(self):
        self.existing(draft=True)
        self.remote.assets['unexpected'] = b'extra'
        with self.assertRaises(ValueError):
            self.publish()
        self.assertEqual(self.remote.events, [])
        del self.remote.assets['unexpected']
        self.remote.assets['install.ps1'] = b'bad'
        self.publish()
        self.assertEqual(self.remote.events, ['delete', 'upload', 'promote'])
        self.assertEqual(self.remote.assets['install.ps1'], (self.directory / 'install.ps1').read_bytes())

    def test_corrupt_uploaded_asset_leaves_release_in_draft(self):
        original = self.remote.gh
        def corrupt_upload(*args, **kwargs):
            result = original(*args, **kwargs)
            if args[1].startswith('https://uploads.github.com/') and args[1].endswith('?name=install.sh'):
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

    def test_starter_asset_is_removed_and_uploaded_again_only_in_draft(self):
        self.existing(draft=True)
        self.remote.states['install.sh'] = 'starter'
        self.publish()
        self.assertEqual(self.remote.events, ['delete', 'upload', 'promote'])
        self.existing(draft=False)
        self.remote.events.clear()
        self.remote.states['install.sh'] = 'starter'
        with self.assertRaises(ValueError):
            self.publish()
        self.assertEqual(self.remote.events, [])

    def test_duplicate_tag_matches_from_listing_fail_without_writes(self):
        self.existing(draft=True)
        self.remote.published.append(copy.deepcopy(self.remote.release))
        with self.assertRaises(ValueError):
            self.publish()
        self.assertEqual(self.remote.events, [])

    def test_source_release_notes_do_not_instruct_users_to_reinstall(self):
        self.publish()
        self.assertNotIn('reinstall', self.remote.release['body'])

    def test_resumed_source_draft_gets_source_release_notes(self):
        self.existing(draft=True)
        self.remote.release['body'] = 'must reinstall'
        self.publish()
        self.assertNotIn('reinstall', self.remote.release['body'])

    def test_published_during_draft_inspection_cannot_have_assets_deleted(self):
        self.existing(draft=True)
        self.remote.assets['install.sh'] = b'conflict'
        original = self.remote.gh
        def published_during_read(*args, **kwargs):
            result = original(*args, **kwargs)
            if '/releases/assets/' in args[1] and '--method' not in args:
                self.remote.release['draft'] = False
            return result
        with patch.object(p, 'gh', published_during_read), self.assertRaises(ValueError):
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
            if args[1].startswith('https://uploads.github.com/'):
                self.remote.published = [{'tag_name': 'v0.0.100', 'draft': False}]
            return result
        with patch.object(p, 'gh', concurrent_release), self.assertRaises(ValueError):
            p.publish('demo-suite/envswitch', self.directory, 'source', '0.0.42', 'v0.0.42', SHA)
        self.assertTrue(self.remote.release['draft'])
        self.assertNotIn('promote', self.remote.events)


if __name__ == '__main__':
    unittest.main()

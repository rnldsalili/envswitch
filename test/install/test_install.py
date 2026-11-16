import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from fixtures import REPO, Server, release
from harness import set_root, assert_owned


class InstallTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='.suite-', dir=REPO/'test/install')
        self.home = Path(self.tmp.name).resolve()
        set_root(self.home)
        self.env = {k: v for k, v in os.environ.items() if k not in ('GITHUB_TOKEN','GH_TOKEN','REPO','APP_ROOT','INSTALL_DIR','ENVS_SWITCH_INSTALL_TEST_MODE') and not k.startswith('ENVSWITCH_')}
        for name in ('HOME','USERPROFILE','LOCALAPPDATA','APPDATA','XDG_DATA_HOME','XDG_CONFIG_HOME','XDG_CACHE_HOME','XDG_STATE_HOME','TMPDIR','TEMP','TMP','BUN_INSTALL','GH_CONFIG_DIR'):
            p = self.home / name; p.mkdir(); self.env[name] = str(p)
        self.bin = Path(self.env['HOME']) / 'tools'; self.bin.mkdir()
        # Never put the user's PATH or Bun/gh on the test PATH.
        self.env.update(PYTHONDONTWRITEBYTECODE='1', HTTPS_PROXY='http://127.0.0.1:9', NO_PROXY='127.0.0.1,localhost,::1', PATH=str(self.bin) + ':/usr/bin:/bin:/usr/sbin:/sbin', SHELL='/bin/zsh', REPO='demo/suite',
                        APP_ROOT=str(self.home / 'app'), INSTALL_DIR=str(self.home / 'launchers'), ENVSWITCH_INSTALL_TEST_MODE='install')
        for tool in ('python3',):
            (self.bin / tool).symlink_to(sys.executable)
        self.fake('bun', 'echo "$*" >> "$HOME/bun.log"\ncase "$1" in --version) echo 1.4.0;; install) :;; *) exit 0;; esac')
        self.fake('op', 'echo forbidden >> "$HOME/op.log"; exit 99')
        self.fake('gh', 'exit 1')
        (self.bin/'curl').symlink_to('/usr/bin/curl')
        self.server = None
    def tearDown(self):
        if self.server: self.server.close()
        self.tmp.cleanup()
        set_root(None)
    def fake(self, name, code):
        p = self.bin / name
        assert_owned(p.parent)
        if p.is_symlink(): p.unlink()
        p.write_text('#!/bin/bash\n' + code + '\n'); p.chmod(0o755)
    def serve(self, **kw):
        if self.server: self.server.close()
        fault = kw.get('fault')
        self.server = Server(release(**kw), fault=fault)
        self.env['ENVSWITCH_INSTALL_TEST_API_BASE_URL'] = self.server.base
    def run_install(self, ok=True, **env):
        result = subprocess.run([os.environ.get('INSTALL_TEST_BASH','/bin/bash'), str(REPO / 'scripts/install.sh')], env=dict(self.env, **env), capture_output=True, text=True)
        self.assertEqual(result.returncode, 0 if ok else 1, result.stdout + result.stderr)
        self.assertFalse((Path(self.env['HOME']) / 'op.log').exists())
        return result
    @property
    def current(self): return Path(self.env['APP_ROOT']) / 'current'
    def test_fresh_native_and_unchanged(self):
        self.serve(); r = self.run_install()
        self.assertIn('success: installed envswitch v0.0.42\n', r.stdout)
        receipt = json.loads((self.current / 'install.json').read_text())
        self.assertEqual(receipt['kind'], 'native')
        self.assertEqual((self.current / '.version').read_bytes(), b'v0.0.42\n')
        self.assertFalse((Path(self.env['HOME']) / 'bun.log').exists())
        before = (self.current / 'install.json').stat().st_mtime_ns
        r = self.run_install(); self.assertIn('success: envswitch is already up to date (v0.0.42)', r.stdout)
        self.assertEqual(before, (self.current / 'install.json').stat().st_mtime_ns)
        self.assertEqual(sum(p.endswith('/latest') for p, _ in self.server.requests), 2)
    def test_source_then_native(self):
        self.serve(kind='source'); self.run_install()
        self.assertEqual(json.loads((self.current / 'install.json').read_text())['target'], None)
        sentinel = self.current / 'src' / '.env'; sentinel.write_text('preserve')
        self.serve(); self.run_install(); self.assertEqual(sentinel.read_text(), 'preserve')
    def test_integrity_and_metadata_fail_closed(self):
        for fault in ('missing-sums','malformed','no-final-lf','self-entry','missing-archive-entry','duplicate-sum','missing-entry','mismatch','script-hash','partial','mixed','duplicate','bad-json','wrong-version','wrong-target','newline','crlf','version-fails','stderr','help-fails','traversal','link','duplicate-path','extra'):
            with self.subTest(fault=fault):
                self.serve(fault=fault); r = self.run_install(ok=False)
                self.assertIn('error:', r.stderr)
                self.assertFalse((Path(self.env['INSTALL_DIR']) / 'envswitch').exists())
    def test_pin(self):
        self.serve(); self.run_install(ENVSWITCH_VERSION='0.0.42')
        self.assertTrue(any('/tags/v0.0.42' in p for p, _ in self.server.requests))
        self.assertFalse(any(p.endswith('/latest') for p, _ in self.server.requests))
    def test_missing_metadata_repair(self):
        self.serve(); self.run_install()
        for name in ('.version', 'scripts/install.sh', 'install.json'):
            with self.subTest(name=name):
                (self.current / name).unlink()
                self.assertIn('success: repaired envswitch v0.0.42', self.run_install(ENVSWITCH_CURRENT_VERSION='v0.0.42').stdout)
    def test_rollback_boundaries(self):
        self.serve(); self.run_install()
        paths = [self.current / '.version', self.current / 'install.json', Path(self.env['INSTALL_DIR']) / 'envswitch']
        before = [p.read_bytes() for p in paths]
        self.serve(ver='0.0.43'); self.server.tag = 'v0.0.43'
        for boundary in ('after-payload','after-launcher','after-metadata'):
            with self.subTest(boundary=boundary):
                self.run_install(ok=False, ENVSWITCH_INSTALL_TEST_FAIL_AT=boundary)
                self.assertEqual([p.read_bytes() for p in paths], before)
    def test_lock_nonowner(self):
        self.serve(); root = Path(self.env['APP_ROOT']); root.mkdir()
        lock = root / '.install.lock'; lock.write_text('someone else\n')
        self.assertIn('another envswitch installer', self.run_install(ok=False).stderr)
        self.assertEqual(lock.read_text(), 'someone else\n'); self.assertEqual(self.server.requests, [])
    def test_paths_and_arguments(self):
        self.env.update(APP_ROOT=str(self.home / "app '雪$`&!"), INSTALL_DIR=str(self.home / "bin '雪$`&!"))
        self.serve(); self.run_install()
        args = ['hello world', '雪', '$`&!%']
        r = subprocess.run([str(Path(self.env['INSTALL_DIR']) / 'envswitch'), *args], env=self.env, capture_output=True, text=True)
        self.assertEqual(r.returncode, 23); self.assertEqual(r.stdout.splitlines(), args)
    def test_hostile_launcher(self):
        self.serve(); p = Path(self.env['INSTALL_DIR']); p.mkdir(); (p/'envswitch').write_text('user file')
        self.run_install(ok=False); self.assertEqual((p/'envswitch').read_text(), 'user file')
        self.assertEqual(self.server.requests, [])

    def test_detection_matrix(self):
        for osname, arch, capability, target in (
            ('Linux','x86_64','0','x86_64-unknown-linux-musl'),
            ('Linux','aarch64','0','aarch64-unknown-linux-musl'),
            ('Darwin','arm64','1','aarch64-apple-darwin'),
            ('Darwin','x86_64','0','x86_64-apple-darwin'),
            ('Darwin','x86_64','1','aarch64-apple-darwin')):
            with self.subTest(os=osname, arch=arch, rosetta=capability):
                self.fake('uname', 'if [[ "$1" == "-s" ]]; then echo '+osname+'; else echo '+arch+'; fi')
                self.fake('sysctl', 'echo '+capability)
                self.env['APP_ROOT']=str(self.home/target/capability/'app')
                self.env['INSTALL_DIR']=str(self.home/target/capability/'bin')
                self.serve(); self.assertIn('selected target: '+target,self.run_install().stdout)
        self.fake('uname','if [[ "$1" == "-s" ]]; then echo Linux; else echo i686; fi')
        self.serve(); self.assertIn('unsupported architecture',self.run_install(ok=False).stderr)
        self.assertEqual(self.server.requests,[])

    def test_invalid_inputs_no_requests(self):
        self.serve()
        for key, value in (('ENVSWITCH_VERSION','v0.0.01'),('ENVSWITCH_CURRENT_VERSION','0.0.1\n'),('REPO','../suite'),('APP_ROOT',self.env['HOME']),('INSTALL_DIR',str(self.current/'bin')),('ENVSWITCH_VERIFY_ATTESTATIONS','true'),('ENVSWITCH_INSTALL_TEST_API_BASE_URL','http://example.com')):
            with self.subTest(key=key): self.run_install(ok=False,**{key:value})
        self.assertEqual(self.server.requests,[])

    def test_gh_fallback_and_attestations(self):
        for mode in ('unauthenticated','metadata-fails','download-fails'):
            self.fake('gh', 'echo "$*" >> "$HOME/gh.log"\n'+('exit 1' if mode=='unauthenticated' else '[[ "$1" == auth ]] && exit 0; exit 1'))
            self.env['APP_ROOT']=str(self.home/mode/'app'); self.env['INSTALL_DIR']=str(self.home/mode/'bin')
            self.serve(); self.run_install()
        self.fake('gh','[[ "$1" == auth ]] && exit 0; [[ "$1" == attestation ]] && exit 0; exit 1')
        self.serve(); self.run_install(ENVSWITCH_VERIFY_ATTESTATIONS='1')
        self.fake('gh','[[ "$1" == auth ]] && exit 0; exit 1')
        self.assertIn('attestation verification failed',self.run_install(ok=False,ENVSWITCH_VERIFY_ATTESTATIONS='1').stderr)
        (self.bin/'gh').unlink()
        self.assertIn('gh is required',self.run_install(ok=False,ENVSWITCH_VERIFY_ATTESTATIONS='1').stderr)
        self.run_install()

    def test_profile_update_escape_and_preserve(self):
        profile=Path(self.env['HOME'])/'.zshrc'; profile.write_bytes(b'# before\n')
        self.env['INSTALL_DIR']=str(self.home/'old-bin'); self.serve(); self.run_install()
        profile.write_bytes(profile.read_bytes()+b'# after\n')
        self.env['INSTALL_DIR']=str(self.home/'new $`"bin')
        self.run_install()
        data=profile.read_bytes(); self.assertTrue(data.startswith(b'# before\n')); self.assertTrue(data.endswith(b'# after\n'))
        self.assertEqual(data.count(b'# >>> envswitch PATH >>>'),1)
        self.assertIn(b'new \\$\\`\\"bin',data)
        self.assertNotIn(b'old-bin',data)

    def test_broken_profile_rolls_back(self):
        self.serve(); self.run_install(); marker=self.current/'.version'; before=marker.read_bytes()
        profile=Path(self.env['HOME'])/'.zshrc'; profile.write_text('# >>> envswitch PATH >>>\n')
        self.serve(ver='0.0.43'); self.server.tag='v0.0.43'
        self.assertIn('invalid envswitch PATH block',self.run_install(ok=False).stderr)
        self.assertEqual(marker.read_bytes(),before)

    def test_invalid_receipt_and_symlink(self):
        self.serve(); self.run_install(); r=self.current/'install.json'; old=r.read_bytes()
        data=json.loads(old); data['payloadPath']=str(self.home/'outside'); r.write_text(json.dumps(data))
        self.assertIn('invalid envswitch install receipt',self.run_install(ok=False).stderr)
        r.write_bytes(old)
        other=self.home/'other'; other.mkdir(); alias=self.home/'alias'; alias.symlink_to(other,target_is_directory=True)
        self.assertIn('unsafe installation path',self.run_install(ok=False,APP_ROOT=str(alias/'app')).stderr)

    def test_immutable_conflict_preserves_bytes(self):
        self.serve(); self.run_install(); receipt=json.loads((self.current/'install.json').read_text())
        p=Path(receipt['payloadPath']); p.write_bytes(b'corrupted')
        self.assertIn('existing native payload differs',self.run_install(ok=False).stderr)
        self.assertEqual(p.read_bytes(),b'corrupted')

    def test_missing_payload_repaired(self):
        self.serve(); self.run_install(); receipt=json.loads((self.current/'install.json').read_text()); Path(receipt['payloadPath']).unlink()
        self.assertIn('success: repaired',self.run_install().stdout)

    def test_launcher_replace_failure_preserves_old(self):
        self.serve(); self.run_install(); p=Path(self.env['INSTALL_DIR'])/'envswitch'; before=p.read_bytes()
        real_mv=shutil.which('mv')
        self.fake('mv','for last; do :; done\nif [[ "$last" == "$INSTALL_DIR/envswitch" ]]; then exit 9; fi\nexec "'+real_mv+'" "$@"')
        self.serve(ver='0.0.43'); self.server.tag='v0.0.43'
        self.assertIn('installation failed: could not replace a managed path', self.run_install(ok=False).stderr)
        self.assertEqual(p.read_bytes(),before)

    def test_source_adoption_and_missing_file_repair(self):
        self.serve(kind='source'); self.run_install()
        for name in ('install.json','src/cli.ts','bun.lock','package.json'):
            with self.subTest(name=name):
                (self.current/name).unlink()
                self.assertIn('success: repaired',self.run_install(ENVSWITCH_CURRENT_VERSION='0.0.42').stdout)
        (self.current/'unknown.txt').write_text('preserve'); (self.current/'.version').unlink()
        self.assertIn('refusing to replace an unmanaged path',self.run_install(ok=False).stderr)
        self.assertEqual((self.current/'unknown.txt').read_text(),'preserve')

    def test_source_missing_checksums_and_bun_failures(self):
        self.serve(kind='source',fault='missing-sums')
        self.assertIn('is missing SHA256SUMS; refusing unverified installation',self.run_install(ok=False).stderr)
        self.assertFalse((Path(self.env['HOME'])/'bun.log').exists())
        self.serve(kind='source'); self.fake('bun','exit 9')
        self.assertIn('Bun verification failed',self.run_install(ok=False).stderr)
        self.fake('bun','[[ "$1" == --version ]] && exit 0; exit 9')
        self.assertIn('bun install failed',self.run_install(ok=False).stderr)

    def test_shadowed_path_does_not_verify_shadow(self):
        self.fake('envswitch','echo forbidden >> "$HOME/shadow.log"; exit 0')
        self.env['PATH']+=':'+self.env['INSTALL_DIR']; self.serve(); self.run_install()
        self.assertFalse((Path(self.env['HOME'])/'shadow.log').exists())
        self.assertFalse((Path(self.env['HOME'])/'.zshrc').exists())

    def test_concurrent_install_and_signal_cleanup(self):
        import time
        self.fake('gh','[[ "$1" == auth ]] && exit 0; if [[ "$1" == api ]]; then sleep 20; fi; exit 1')
        self.serve()
        child=subprocess.Popen(['/bin/bash',str(REPO/'scripts/install.sh')],env=self.env,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
        try:
            lock=Path(self.env['APP_ROOT'])/'.install.lock'
            for _ in range(300):
                if lock.exists(): break
                time.sleep(.01)
            self.assertTrue(lock.exists()); old=lock.read_bytes()
            self.assertIn('another envswitch installer',self.run_install(ok=False).stderr); self.assertEqual(old,lock.read_bytes())
            child.terminate(); child.communicate(timeout=10)
            self.assertEqual(child.returncode,143); self.assertFalse(lock.exists())
        finally:
            if child.poll() is None: child.kill(); child.communicate()

    def test_http_retry_and_missing_pin(self):
        self.serve(); self.server.statuses=[500,429]; self.run_install()
        self.assertEqual(sum(p.endswith('/latest') for p,_ in self.server.requests),3)
        self.serve(); self.server.statuses=[404]
        self.assertIn('GitHub Release not found or unavailable: demo/suite@v0.0.42',self.run_install(ok=False,ENVSWITCH_VERSION='0.0.42').stderr)
        self.assertEqual(len(self.server.requests),1)

    def test_redirect_does_not_forward_token(self):
        self.serve(); other=Server(self.server.files)
        try:
            self.server.redirect=other.base; self.run_install(GITHUB_TOKEN='synthetic-test-token')
            self.assertTrue(all(auth=='Bearer synthetic-test-token' for _,auth in self.server.requests))
            self.assertTrue(other.requests); self.assertTrue(all(auth is None for _,auth in other.requests))
        finally: other.close()

    def test_changed_latest_is_not_resolved_again(self):
        self.serve(); self.server.change_latest=True
        self.assertIn('success: installed envswitch v0.0.42',self.run_install().stdout)
        self.assertEqual(sum(p.endswith('/latest') for p,_ in self.server.requests),1)

    def test_authenticated_gh_success(self):
        self.serve(); home=Path(self.env['HOME']); assets=home/'gh-assets'; assets.mkdir()
        for name,data in self.server.files.items(): (assets/name).write_bytes(data)
        (home/'gh-release.json').write_text(json.dumps({'tag_name':'v0.0.42','id':42,'draft':False,'prerelease':False,'assets':[{'id':i,'name':name,'url':self.server.base+'/repos/demo/suite/releases/assets/'+str(i)} for i,name in enumerate(self.server.files,1)]}))
        self.fake('gh','exec python3 "'+str(REPO/'test/install/fake-gh.py')+'" "$@"')
        self.run_install(ENVSWITCH_VERIFY_ATTESTATIONS='1')
        log=[json.loads(line) for line in (home/'gh.log').read_text().splitlines()]
        self.assertEqual(sum(args[0]=='api' for args in log),1); self.assertEqual(self.server.requests,[])
        self.assertTrue(any(args[:2]==['attestation','verify'] for args in log))

    def test_source_archive_hostile_types(self):
        import base64
        import hashlib
        fixtures=json.loads((REPO/'test/install/archive-fixtures.json').read_text())
        for name in ('source-traversal','source-link','source-hardlink','source-fifo','source-dot'):
            with self.subTest(archive=name):
                self.serve(kind='source')
                asset='envswitch_0.0.42_source.tar.gz'; data=base64.b64decode(fixtures[name])
                self.server.files[asset]=data
                sums=self.server.files['SHA256SUMS'].decode().splitlines()
                self.server.files['SHA256SUMS']=('\n'.join(hashlib.sha256(data).hexdigest()+'  '+asset if line.endswith('  '+asset) else line for line in sums)+'\n').encode()
                self.assertIn('unsafe or invalid archive: '+asset,self.run_install(ok=False).stderr)

    def test_profile_selection_and_deprecated_alias(self):
        for shell, profile in (('/bin/zsh','.zshrc'),('/bin/bash','.bashrc'),('/bin/fish','.profile'),('','.profile')):
            env=dict(self.env,SHELL=shell,ENVSWITCH_INSTALL_TEST_MODE='shell-profile-path')
            r=subprocess.run(['/bin/bash',str(REPO/'scripts/install.sh')],env=env,capture_output=True,text=True)
            self.assertEqual(r.returncode,0,r.stderr); self.assertEqual(r.stdout,str(Path(self.env['HOME'])/profile)+'\n')
        self.assertIn('invalid installer setting',self.run_install(ok=False,ENVS_SWITCH_INSTALL_TEST_MODE='shell-profile-path').stderr)

    def test_piped_helper_and_full_install_without_python(self):
        self.serve()
        tools=self.home/'no-python-tools'; tools.mkdir()
        for name in ('bash','curl','tar','awk','sed','mktemp','find','uname','sysctl','shasum','sha256sum','od','tr','cat','cp','mv','rm','rmdir','mkdir','chmod','cmp','sort','grep','cut','tail','date','sleep','readlink','wc','head','gzip','kill','basename','dirname'):
            tool=shutil.which(name)
            if tool: (tools/name).symlink_to(tool)
        self.env['PATH']=str(tools)
        code=(REPO/'scripts/install.sh').read_bytes()
        helper=dict(self.env,ENVSWITCH_INSTALL_TEST_MODE='shell-profile-path')
        shell=os.environ.get('INSTALL_TEST_BASH','/bin/bash')
        result=subprocess.run([shell],input=code,env=helper,capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertEqual(result.stdout,(str(Path(self.env['HOME'])/'.zshrc')+'\n').encode())
        result=subprocess.run([shell],input=code,env=self.env,capture_output=True)
        self.assertEqual(result.returncode,0,result.stdout+result.stderr)
        self.assertIn(b'success: installed envswitch v0.0.42',result.stdout)

    def test_install_hook_requires_loopback_url(self):
        self.env.pop('ENVSWITCH_INSTALL_TEST_API_BASE_URL',None)
        self.assertIn('invalid installer setting: ENVSWITCH_INSTALL_TEST_API_BASE_URL',self.run_install(ok=False).stderr)
        self.assertFalse(Path(self.env['APP_ROOT']).exists())

    def test_unset_home_is_clear_error(self):
        self.serve(); env=dict(self.env); env.pop('HOME')
        r=subprocess.run(['/bin/bash',str(REPO/'scripts/install.sh')],env=env,capture_output=True,text=True)
        self.assertEqual(r.returncode,1); self.assertIn('error:',r.stderr); self.assertNotIn('Traceback',r.stderr)

    def test_symlinked_profile_preserves_link_and_target(self):
        self.serve(); target=self.home/'dotfiles'/'zshrc'; target.parent.mkdir(); target.write_bytes(b'# stowed\n')
        profile=Path(self.env['HOME'])/'.zshrc'; profile.symlink_to(target)
        self.run_install(); self.assertTrue(profile.is_symlink())
        self.assertTrue(target.read_bytes().startswith(b'# stowed\n'))
        self.assertIn(b'# >>> envswitch PATH >>>',target.read_bytes())

    def test_harness_rejects_writes_through_tool_symlinks(self):
        link=self.bin/'awk'; link.symlink_to('/usr/bin/awk')
        with self.assertRaisesRegex(AssertionError,'escapes isolated test root'): link.write_text('forbidden')

    def test_token_never_appears_in_curl_arguments(self):
        self.serve(); self.fake('curl','printf "%s\\n" "$@" >> "$HOME/curl-args.log"\nexec /usr/bin/curl "$@"')
        self.run_install(GITHUB_TOKEN='synthetic-test-token')
        log=(Path(self.env['HOME'])/'curl-args.log').read_text()
        self.assertNotIn('synthetic-test-token',log); self.assertIn('--config',log)

    def test_asset_repo_case_is_insensitive(self):
        self.serve(); self.server.repo='DeMo/SuItE'; self.run_install()

    def test_retry_after_is_respected(self):
        self.serve(); self.server.statuses=[429]; self.server.retry_after=2
        self.run_install(); self.assertGreaterEqual(self.server.request_times[1]-self.server.request_times[0],1.9)

    def test_bun_failure_preserves_diagnostic_output(self):
        self.serve(kind='source'); self.fake('bun','[[ "$1" == --version ]] && exit 0\necho dependency-diagnostic; echo dependency-error >&2; exit 9')
        r=self.run_install(ok=False); self.assertIn('dependency-diagnostic',r.stdout); self.assertIn('dependency-error',r.stderr)

if __name__ == '__main__': unittest.main(verbosity=2)

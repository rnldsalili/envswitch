"""Windows-only end-to-end suite; persistent User PATH always goes to the mock file."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from fixtures import REPO, Server, release


class WindowsInstall(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if os.name != 'nt': raise unittest.SkipTest('Windows process and .cmd semantics')
        cls.pwsh = os.environ.get('ENVSWITCH_TEST_POWERSHELL') or shutil.which('pwsh')
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='envswitch-suite-')
        self.home = Path(self.temp.name).resolve()
        self.env = {k:v for k,v in os.environ.items() if k not in ('GITHUB_TOKEN','GH_TOKEN') and not k.startswith('ENVSWITCH_')}
        for key in ('HOME','USERPROFILE','LOCALAPPDATA','APPDATA','XDG_DATA_HOME','XDG_CONFIG_HOME','XDG_CACHE_HOME','XDG_STATE_HOME','TMPDIR','TEMP','TMP','BUN_INSTALL','GH_CONFIG_DIR'):
            p=self.home/key; p.mkdir(); self.env[key]=str(p)
        self.tools=Path(self.env['USERPROFILE'])/'tools'; self.tools.mkdir()
        self.env.update(ENVSWITCH_INSTALL_TEST_MODE='install', ENVSWITCH_INSTALL_TEST_USER_PATH_FILE=str(Path(self.env['USERPROFILE'])/'user-path.txt'), SHELL='powershell')
        self.env['PATH']=str(self.tools)+';'+str(Path(self.env['SystemRoot'])/'System32')+';'+str(Path(self.env['SystemRoot'])/'System32/WindowsPowerShell/v1.0')
        (self.tools/'gh.cmd').write_text('@echo off\nexit /b 1\n')
        (self.tools/'bun.cmd').write_text('@echo off\necho %*>>"%USERPROFILE%\\bun.log"\nexit /b 0\n')
        self.root=self.home/'app'; self.dest=self.home/'launchers'; self.server=None
        self.payload=self.build_binary()
    def tearDown(self):
        if self.server: self.server.close()
        self.temp.cleanup()
    def build_binary(self, ver='0.0.42', target='x86_64-pc-windows-msvc', fault=None):
        # Windows PowerShell's compiler emits a real .NET Framework EXE; no SDK/download required.
        path=self.home/('payload-'+ver+'.exe')
        if path.exists(): path.unlink()
        line=f'envswitch {ver} {target}'+('\\n\\n' if fault=='newline' else '\\n')
        code='public class Payload { public static int Main(string[] args) { if(args[0]=="--hold") { System.Console.ReadLine(); return 0; } if(args[0]=="--version") { System.Console.Write("'+line+'"); return 0; } if(args[0]=="--help") return '+('9' if fault=='help-fails' else '0')+'; foreach(var arg in args) System.Console.Write(arg+"\\n"); return 23; }}'
        script=self.home/'build.ps1'; script.write_text("Add-Type -TypeDefinition '"+code+"' -OutputAssembly '"+str(path).replace("'","''")+"' -OutputType ConsoleApplication\n")
        r=subprocess.run(['powershell.exe','-NoProfile','-File',str(script)],env=self.env,capture_output=True,text=True)
        self.assertEqual(r.returncode,0,r.stdout+r.stderr); return path.read_bytes()
    def serve(self, **kw):
        if self.server: self.server.close()
        fault=kw.get('fault'); payload=self.payload
        if kw.get('ver') or fault in ('wrong-version','wrong-target','newline','help-fails'):
            payload=self.build_binary('0.0.99' if fault=='wrong-version' else kw.get('ver','0.0.42'), 'wrong-target' if fault=='wrong-target' else 'x86_64-pc-windows-msvc',fault)
        self.server=Server(release(windows_binary=payload,**kw),tag='v'+kw.get('ver','0.0.42'),fault=fault)
        self.env['ENVSWITCH_INSTALL_TEST_API_BASE_URL']=self.server.base
    def invoke(self,*params,ok=True,**env):
        r=subprocess.run([self.pwsh,'-NoProfile','-File',str(REPO/'scripts/install.ps1'),'-Repo','demo/suite','-AppRoot',str(self.root),'-InstallDir',str(self.dest),*params],env=dict(self.env,**env),capture_output=True,text=True,encoding='utf-8')
        self.assertEqual(r.returncode,0 if ok else 1,r.stdout+r.stderr); return r
    def test_native_unchanged_repair(self):
        self.serve(); self.assertIn('success: installed envswitch v0.0.42',self.invoke().stdout)
        self.assertFalse((Path(self.env['USERPROFILE'])/'bun.log').exists())
        self.assertIn('already up to date (v0.0.42)',self.invoke().stdout)
        for name in ('.version','scripts/install.ps1','install.json'):
            (self.root/'current'/name).unlink(); self.assertIn('success: repaired',self.invoke('-CurrentVersion','0.0.42').stdout)
    def test_source_to_native(self):
        self.serve(kind='source'); self.invoke()
        sentinel=self.root/'current/src/.env'; sentinel.write_text('preserve')
        self.serve(); self.invoke(); self.assertEqual(sentinel.read_text(),'preserve')
    def test_bad_releases(self):
        for fault in ('missing-sums','malformed','duplicate-sum','missing-entry','mismatch','script-hash','partial','mixed','duplicate','bad-json','wrong-version','wrong-target','newline','help-fails','traversal','extra'):
            with self.subTest(fault=fault):
                self.serve(fault=fault); self.assertIn('error:',self.invoke(ok=False).stderr)
                self.assertFalse((self.dest/'envswitch.cmd').exists())
    def test_pin_and_whatif(self):
        self.serve(); before={str(p):p.read_bytes() for p in self.home.rglob('*') if p.is_file()}
        r=self.invoke('-WhatIf','-Version','0.0.42','-VerifyAttestations')
        self.assertEqual(len(r.stdout.splitlines()),6); self.assertEqual(self.server.requests,[])
        self.assertEqual(before,{str(p):p.read_bytes() for p in self.home.rglob('*') if p.is_file()})
        self.invoke('-Version','v0.0.42'); self.assertFalse(any(p.endswith('/latest') for p,_ in self.server.requests))
    def test_rollback(self):
        self.serve(); self.invoke()
        paths=[self.root/'current/.version',self.root/'current/install.json',self.dest/'envswitch.cmd',Path(self.env['ENVSWITCH_INSTALL_TEST_USER_PATH_FILE'])]
        before=[p.read_bytes() for p in paths]
        self.serve(ver='0.0.43')
        for boundary in ('after-payload','after-launcher','after-metadata'):
            with self.subTest(boundary=boundary):
                self.invoke(ok=False,ENVSWITCH_INSTALL_TEST_FAIL_AT=boundary); self.assertEqual(before,[p.read_bytes() for p in paths])
    def test_lock_and_immutable_conflict(self):
        self.serve(); self.root.mkdir(); lock=self.root/'.install.lock'; lock.write_text('owner')
        self.assertIn('another envswitch installer',self.invoke(ok=False).stderr); self.assertEqual(lock.read_text(),'owner')
        lock.unlink(); self.invoke(); r=json.loads((self.root/'current/install.json').read_text()); Path(r['payloadPath']).write_bytes(b'corrupt')
        self.assertIn('existing native payload differs',self.invoke(ok=False).stderr)
    def test_unicode_metacharacters_and_status(self):
        self.root=self.home/'app 雪 %!&'; self.dest=self.home/'bin 雪 %!&'
        self.serve(); self.invoke()
        # Invoke through cmd exactly as installer does; this also checks nonzero forwarding.
        command='""'+str(self.dest/'envswitch.cmd')+'" "hello world" "雪""'
        r=subprocess.run([self.env['ComSpec'],'/d','/s','/c',command],env=self.env,capture_output=True)
        self.assertEqual(r.returncode,23)
    def test_running_old_exe_is_preserved(self):
        self.serve(); self.invoke()
        receipt=json.loads((self.root/'current/install.json').read_text()); old=Path(receipt['payloadPath']); before=old.read_bytes()
        child=subprocess.Popen([str(old),'--hold'],stdin=subprocess.PIPE,env=self.env)
        try:
            self.serve(ver='0.0.43'); self.invoke(); self.assertEqual(old.read_bytes(),before)
            self.assertIsNone(child.poll())
        finally: child.communicate(input=b'\n',timeout=10)

    def test_gh_success_fallback_and_attestation(self):
        self.serve(); home=Path(self.env['HOME']); assets=home/'gh-assets'; assets.mkdir()
        for name,data in self.server.files.items(): (assets/name).write_bytes(data)
        (home/'gh-release.json').write_text(json.dumps({'tag_name':'v0.0.42','id':42,'draft':False,'prerelease':False,'assets':[{'name':name,'url':self.server.base+'/repos/demo/suite/releases/assets/'+str(i)} for i,name in enumerate(self.server.files,1)]}))
        (self.tools/'gh.cmd').write_text('@echo off\r\n"'+sys.executable+'" "'+str(REPO/'test/install/fake-gh.py')+'" %*\r\nexit /b %ERRORLEVEL%\r\n')
        self.invoke('-VerifyAttestations'); self.assertEqual(self.server.requests,[])
        for mode in ('unauthenticated','metadata-fails','download-fails'):
            self.invoke(FAKE_GH_MODE=mode)
        self.assertIn('attestation verification failed',self.invoke('-VerifyAttestations',ok=False,FAKE_GH_MODE='attestation-fails').stderr)
        (self.tools/'gh.cmd').unlink()
        self.assertIn('gh is required',self.invoke('-VerifyAttestations',ok=False).stderr)

    def test_launcher_sharing_violation(self):
        self.serve(); self.invoke(); self.serve(ver='0.0.43')
        # Hold the old launcher without delete sharing; the old bytes must remain intact.
        holder=self.home/'hold.ps1'; ready=self.home/'ready'
        holder.write_text("$f=[IO.File]::Open('"+str(self.dest/'envswitch.cmd')+"','Open','Read','Read'); [IO.File]::WriteAllText('"+str(ready)+"','ready'); [Console]::ReadLine(); $f.Dispose()")
        child=subprocess.Popen([self.pwsh,'-NoProfile','-File',str(holder)],stdin=subprocess.PIPE,env=self.env)
        try:
            import time
            for _ in range(100):
                if ready.exists(): break
                time.sleep(.05)
            self.assertTrue(ready.exists()); r=self.invoke(ok=False); self.assertIn('could not replace envswitch launcher',r.stderr)
        finally: child.communicate(input=b'\n',timeout=10)
        self.assertEqual((self.root/'current/.version').read_bytes(),b'v0.0.42\n')

if __name__=='__main__': unittest.main(verbosity=2)

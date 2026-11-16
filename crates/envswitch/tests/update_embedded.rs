//! Every subprocess has isolated homes and a symlink-only PATH. Never write tools.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

const HOMES: &[&str] = &[
    "HOME",
    "USERPROFILE",
    "LOCALAPPDATA",
    "APPDATA",
    "XDG_DATA_HOME",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "XDG_STATE_HOME",
    "TMPDIR",
    "TEMP",
    "TMP",
    "BUN_INSTALL",
    "GH_CONFIG_DIR",
];
struct Sandbox {
    temp: tempfile::TempDir,
    root: PathBuf,
    tools: PathBuf,
}
impl Sandbox {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let mut root = temp.path().canonicalize().unwrap();
        #[cfg(windows)]
        {
            root = PathBuf::from(root.to_str().unwrap().trim_start_matches("\\\\?\\"));
        }
        #[cfg(not(windows))]
        let _ = &mut root;
        let tools = root.join("tools");
        fs::create_dir(&tools).unwrap();
        for home in HOMES {
            let p = root.join(home);
            assert!(p.starts_with(&root));
            fs::create_dir(p).unwrap();
        }
        let s = Self { temp, root, tools };
        s.write("sitecustomize.py", AUDIT);
        s
    }
    fn write(&self, name: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> PathBuf {
        let p = self.root.join(name);
        assert!(
            p.starts_with(&self.root)
                && !p.components().any(|c| c == std::path::Component::ParentDir)
        );
        assert!(!fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink()));
        assert!(
            p.parent()
                .unwrap()
                .canonicalize()
                .unwrap()
                .starts_with(self.temp.path().canonicalize().unwrap())
        );
        fs::write(&p, bytes).unwrap();
        p
    }
    fn command(&self, exe: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut c = Command::new(exe);
        c.env_clear()
            .current_dir(&self.root)
            .env("PATH", &self.tools)
            .env("ENVSWITCH_D3_SANDBOX", &self.root)
            .env("PYTHONPATH", &self.root)
            .env("PYTHONDONTWRITEBYTECODE", "1");
        for home in HOMES {
            c.env(home, self.root.join(home));
        }
        #[cfg(windows)]
        {
            if let Some(system_root) = std::env::var_os("SystemRoot") {
                c.env("SystemRoot", &system_root).env("WINDIR", system_root);
            }
            c.env("ComSpec", self.tools.join("cmd.exe"))
                .env("PATHEXT", ".COM;.EXE;.BAT;.CMD");
        }
        c
    }
    #[cfg(windows)]
    fn resolve(&self, name: &str) -> PathBuf {
        self.try_resolve(name)
            .unwrap_or_else(|| panic!("required fixture tool unavailable: {name}"))
    }
    fn try_resolve(&self, name: &str) -> Option<PathBuf> {
        let names = if cfg!(windows) {
            vec![
                format!("{name}.exe"),
                format!("{name}.com"),
                name.to_owned(),
            ]
        } else {
            vec![name.to_owned()]
        };
        for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
            for name in &names {
                if let Ok(path) = dir.join(name).canonicalize()
                    && path.is_file()
                {
                    return Some(path);
                }
            }
        }
        None
    }
    fn link(&self, name: &str, target: &Path) {
        let target = target.canonicalize().unwrap();
        let p = self.tools.join(name);
        assert!(p.parent() == Some(self.tools.as_path()));
        if p.symlink_metadata().is_ok() {
            assert!(p.symlink_metadata().unwrap().file_type().is_symlink());
            fs::remove_file(&p).unwrap();
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &p).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&target, &p).unwrap();
        assert!(p.symlink_metadata().unwrap().file_type().is_symlink());
    }
    fn python(&self) -> PathBuf {
        // Check discovered interpreters without ever opening them for writing.
        for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
            for name in if cfg!(windows) {
                &["python3.exe", "python.exe"][..]
            } else {
                &["python3"][..]
            } {
                if let Ok(path) = dir.join(name).canonicalize()
                    && self
                        .command(&path)
                        .arg("--version")
                        .output()
                        .is_ok_and(|o| o.status.success() && o.stdout.starts_with(b"Python 3"))
                {
                    return path;
                }
            }
        }
        panic!("Python 3 is required only for the D2b loopback fixtures");
    }
    fn driver(&self, source: &str) -> Output {
        let script = self.write("driver.py", source);
        let python = self.python();
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let output = self
            .command(python)
            .arg("-B")
            .arg(script)
            .arg(repo)
            .arg(env!("CARGO_BIN_EXE_envswitch"))
            .arg(std::env::current_exe().unwrap())
            .arg(env!("CARGO_PKG_VERSION"))
            .arg(env!("TARGET"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        for p in fs::read_dir(&self.tools).unwrap() {
            assert!(p.unwrap().file_type().unwrap().is_symlink());
        }
        output
    }
}

// Python fixture and legacy-installer writes are checked *after* resolving links.
// This rejects a write through tools/python3 to a host interpreter.
const AUDIT: &str = r#"
import os, sys
from pathlib import Path
root = Path(os.environ['ENVSWITCH_D3_SANDBOX']).resolve()
def owned(path, entry=False):
    if isinstance(path, int): return
    p=Path(os.fsdecode(path))
    p=(p.parent.resolve()/p.name) if entry else p.resolve()
    assert p == root or root in p.parents, 'write escaped sandbox: '+str(p)
def audit(event,args):
    if event == 'open' and args[2] & (os.O_WRONLY|os.O_RDWR|os.O_CREAT|os.O_TRUNC|os.O_APPEND): owned(args[0])
    elif event in ('os.mkdir','os.chmod','os.truncate'): owned(args[0])
    elif event in ('os.remove','os.rmdir'): owned(args[0], True)
    elif event in ('os.rename','os.link'): owned(args[0],True); owned(args[1],True)
    elif event == 'os.symlink': owned(args[1], True)
sys.addaudithook(audit)
"#;

#[test]
fn runner_child() {
    use envswitch::{
        install::EmbeddedInstaller,
        ports::{InstallPlan, InstallerRunner},
    };
    let Some(result_path) = std::env::var_os("ENVSWITCH_D3_RESULT") else {
        return;
    };
    let root = PathBuf::from(std::env::var_os("ENVSWITCH_D3_SANDBOX").unwrap());
    let result_path = PathBuf::from(result_path);
    assert!(result_path.starts_with(&root));
    assert!(!fs::symlink_metadata(&result_path).is_ok_and(|m| m.file_type().is_symlink()));
    let plan = InstallPlan {
        repo: "demo/suite".into(),
        app_root: root.join("app '雪$`&!"),
        install_dir: Some(root.join("bin '雪$`&!")),
        version: std::env::var("ENVSWITCH_D3_PIN").ok(),
        current_version: "v0.0.0".into(),
        verify_attestations: false,
    };
    struct Broken;
    impl std::io::Write for Broken {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut stdout: Box<dyn std::io::Write> = if std::env::var_os("ENVSWITCH_D3_BROKEN").is_some() {
        Box::new(Broken)
    } else {
        Box::new(Vec::new())
    };
    let result = if let Some(exe) = std::env::var_os("ENVSWITCH_D3_CLI_BROKEN") {
        let mut deps = envswitch::ports::Deps::real().unwrap();
        deps.current_exe = Some(exe.into());
        deps.out = Box::new(Broken);
        serde_json::json!({"code":envswitch::run(vec!["update".into()], &mut deps)})
    } else {
        match EmbeddedInstaller.run_with_output(&plan, stdout.as_mut()) {
            Ok((code, output)) => serde_json::json!({"code":code,"output":output}),
            Err(e) => serde_json::json!({"code":e.exit_code(),"error":e.to_string()}),
        }
    };
    fs::write(result_path, result.to_string()).unwrap();
}

#[cfg(unix)]
#[test]
fn runner_handoff_signals_warnings_permissions_and_cleanup() {
    use std::os::unix::fs::PermissionsExt;
    let s = Sandbox::new();
    let python = s.python();
    let fake = s.write(
        "fake-bash",
        format!("#!{}\n{}", python.display(), FAKE_BASH),
    );
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    s.link("bash", &fake);
    s.driver(RUNNER_TEST);
}

#[cfg(unix)]
const FAKE_BASH: &str = r#"
import json, os, signal, stat, sys, time, shutil
from pathlib import Path
root=Path(os.environ['ENVSWITCH_D3_SANDBOX']); script=Path(sys.argv[1])
assert root in script.resolve().parents
record=dict(pid=os.getpid(),argv=sys.argv[1:],cwd=os.getcwd(),script=str(script),script_mode=stat.S_IMODE(script.stat().st_mode),dir_mode=stat.S_IMODE(script.parent.stat().st_mode),env={k:os.environ.get(k) for k in ['REPO','APP_ROOT','INSTALL_DIR','ENVSWITCH_VERSION','ENVSWITCH_CURRENT_VERSION','ENVSWITCH_VERIFY_ATTESTATIONS','GITHUB_TOKEN']})
(root/'log.json').write_text(json.dumps(record))
action=os.environ['ENVSWITCH_D3_ACTION']
if action in ('int','term'):
    def finish(number, frame):
        (root/'child-cleaned').write_text('cleaned')
        print('child cleanup complete',flush=True)
        sys.exit(128+number)
    signal.signal(signal.SIGINT,finish); signal.signal(signal.SIGTERM,finish)
    (root/'ready').write_text('ready')
    time.sleep(15)
    raise AssertionError('signal was not delivered')
if action=='signal': os.kill(os.getpid(),signal.SIGTERM)
if action=='cleanup':
    shutil.rmtree(script.parent)
    script.parent.write_text('inert cleanup failure fixture')
print('installer stdout',flush=True)
sys.exit(7 if action=='7' else 0)
"#;

#[cfg(unix)]
const RUNNER_TEST: &str = r#"
import json, os, signal, subprocess, sys, time
from pathlib import Path
root=Path(os.environ['ENVSWITCH_D3_SANDBOX']); env=dict(os.environ)
env.update(ENVSWITCH_D3_RESULT=str(root/'result.json'), REPO='wrong/repo',APP_ROOT='wrong',INSTALL_DIR='wrong',ENVSWITCH_VERSION='v0.0.77',ENVSWITCH_CURRENT_VERSION='v0.0.77',ENVSWITCH_VERIFY_ATTESTATIONS='1',GITHUB_TOKEN='synthetic-token')
for action,pin,code in [('0',None,0),('7','v0.0.42',7),('signal',None,143),('int',None,130),('term',None,143),('broken',None,0),('cleanup',None,0),('spawn',None,2)]:
    child_env=dict(env,ENVSWITCH_D3_ACTION=action)
    if pin: child_env['ENVSWITCH_D3_PIN']=pin
    if action=='broken': child_env['ENVSWITCH_D3_BROKEN']='1'
    if action=='spawn': (Path(env['PATH'])/'bash').unlink()
    proc=subprocess.Popen([sys.argv[3],'--exact','runner_child','--nocapture'],env=child_env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,start_new_session=True)
    if action in ('int','term'):
        deadline=time.monotonic()+10
        while not (root/'ready').exists():
            assert time.monotonic()<deadline, 'child never became ready'
            time.sleep(.01)
        if action=='int':
            os.kill(proc.pid,signal.SIGINT)
            time.sleep(.05)
            assert proc.poll() is None, 'parent died before child cleanup'
            os.kill(json.loads((root/'log.json').read_text())['pid'],signal.SIGINT) # terminal also delivers INT to child
        else: os.kill(proc.pid,signal.SIGTERM) # only the parent receives TERM
    stdout,stderr=proc.communicate(timeout=20)
    assert proc.returncode==0,stdout+stderr
    result=json.loads((root/'result.json').read_text()); assert result['code']==code,result
    if action=='spawn': assert result['error'].startswith('update failed: could not run installer:'),result
    else:
        record=json.loads((root/'log.json').read_text())
        assert record['argv']==[record['script']] and record['cwd']==str(Path(record['script']).parent),record
        assert record['script_mode']==0o600 and record['dir_mode']==0o700,record
        assert record['env']==dict(REPO='demo/suite',APP_ROOT=str(root/"app '雪$`&!"),INSTALL_DIR=str(root/"bin '雪$`&!"),ENVSWITCH_VERSION=pin,ENVSWITCH_CURRENT_VERSION='v0.0.0',ENVSWITCH_VERIFY_ATTESTATIONS=None,GITHUB_TOKEN='synthetic-token'),record
        if action in ('int','term'):
            assert (root/'child-cleaned').read_text()=='cleaned'
            assert result['output']=='child cleanup complete\n',result
            (root/'child-cleaned').unlink(); (root/'ready').unlink()
        if action=='broken': assert 'warning: could not forward installer stdout:' in stderr,stderr
        if action=='cleanup':
            assert 'warning: could not remove installer temporary directory' in stderr,stderr
            assert Path(record['script']).parent.is_file()
            Path(record['script']).parent.unlink()
        else: assert not Path(record['script']).parent.exists(),record
    assert list(Path(env['TMPDIR']).iterdir())==[]
"#;

#[cfg(unix)]
#[test]
fn embedded_installer_uses_hermetic_d2b_release() {
    hermetic_release();
}

#[cfg(windows)]
#[test]
fn windows_powershell_51_embedded_installer_end_to_end() {
    hermetic_release();
}

fn hermetic_release() {
    let s = Sandbox::new();
    #[cfg(unix)]
    {
        // Same allowlist the D2b installer suite uses (test/install/harness.py
        // UNIX_TOOLS); the installer needs no python3. Tools absent on this
        // host (e.g. sha256sum on macOS, sysctl on Linux) are skipped.
        for name in [
            "bash",
            "curl",
            "tar",
            "awk",
            "sed",
            "mktemp",
            "find",
            "uname",
            "sysctl",
            "shasum",
            "sha256sum",
            "od",
            "tr",
            "cat",
            "cp",
            "mv",
            "rm",
            "rmdir",
            "mkdir",
            "chmod",
            "cmp",
            "sort",
            "grep",
            "cut",
            "tail",
            "date",
            "sleep",
            "readlink",
            "wc",
            "head",
            "gzip",
            "kill",
            "basename",
            "dirname",
        ] {
            if let Some(target) = s.try_resolve(name) {
                s.link(name, &target);
            }
        }
    }
    #[cfg(windows)]
    {
        s.link("powershell.exe", &s.resolve("powershell"));
        s.link("cmd.exe", &s.resolve("cmd"));
        s.link("chcp.com", &s.resolve("chcp"));
    }
    s.driver(E2E);
}

const E2E: &str = r#"
import json, os, shutil, subprocess, sys
from pathlib import Path
sys.path.insert(0,str(Path(sys.argv[1])/'test/install'))
from fixtures import Server,release
root=Path(os.environ['ENVSWITCH_D3_SANDBOX']); env=dict(os.environ)
app=root/"app '雪$`&!"; launchers=root/"bin '雪$`&!"; current=app/'current'
version,target=sys.argv[4:6]
windows=os.name=='nt'
env.update(ENVSWITCH_INSTALL_TEST_MODE='install',ENVSWITCH_VERSION='v0.0.77',ENVSWITCH_CURRENT_VERSION='v0.0.77',ENVSWITCH_VERIFY_ATTESTATIONS='1',REPO='wrong/repo',APP_ROOT=str(root/'wrong-app'),INSTALL_DIR=str(root/'wrong-bin'),SHELL='zsh')
if windows:
    import ctypes
    prior_cp=ctypes.windll.kernel32.GetConsoleOutputCP()
    env['ENVSWITCH_INSTALL_TEST_USER_PATH_FILE']=str(Path(env['USERPROFILE'])/'user-path.txt')
    # Compile a tiny real Windows payload using D2b's Framework compiler fixture.
    payload=root/'fixture.exe'
    code='public class Payload { public static int Main(string[] args) { if(args[0]=="--version") { System.Console.Write("envswitch 0.0.42 x86_64-pc-windows-msvc\\n"); return 0; } return args[0]=="--help" ? 0 : 23; } }'
    build=root/'build.ps1'; build.write_text("Add-Type -TypeDefinition '"+code+"' -OutputAssembly '"+str(payload).replace("'","''")+"' -OutputType ConsoleApplication\n")
    built=subprocess.run([str(Path(env['PATH'])/'powershell.exe'),'-NoProfile','-File',str(build)],env=env,capture_output=True)
    assert built.returncode==0,built.stderr
    files=release(windows_binary=payload.read_bytes())
else:
    files=release()
server=Server(files); env['ENVSWITCH_INSTALL_TEST_API_BASE_URL']=server.base
try:
    if windows:
        # Direct EmbeddedInstaller exercises Windows PowerShell 5.1 -File binding.
        env['ENVSWITCH_D3_RESULT']=str(root/'result.json')
        first=subprocess.run([sys.argv[3],'--exact','runner_child','--nocapture'],env=env,capture_output=True)
        assert first.returncode==0,first.stdout+first.stderr
        result=json.loads((root/'result.json').read_text()); assert result['code']==0,result
        assert 'success: installed envswitch v0.0.42' in result['output'],result
        assert ctypes.windll.kernel32.GetConsoleOutputCP()==prior_cp
        env.pop('ENVSWITCH_D3_RESULT')
    else:
        (current/'bin').mkdir(parents=True); launchers.mkdir()
    old=current/'bin'/('envswitch_'+version+'_'+target+('.exe' if windows else ''))
    shutil.copy2(sys.argv[2],old)
    if not windows:
        launcher=launchers/'envswitch'
        launcher.write_text('#!/usr/bin/env bash\nset -euo pipefail\nexec '+"'"+str(old).replace("'","'\\''")+"'"+' "$@"\n'); launcher.chmod(0o755)
        receipt=dict(schemaVersion=1,kind='native',repo='demo/suite',appRoot=str(app),installDir=str(launchers),launcherPath=str(launcher),target=target,tag='v'+version,payloadPath=str(old))
        (current/'install.json').write_text(json.dumps(receipt)); (current/'.version').write_text('v0.0.999\n')
    for attempt in range(3):
        if attempt==2:
            (current/'.version').unlink(); (current/'scripts'/('install.ps1' if windows else 'install.sh')).unlink()
        result=subprocess.run([str(old),'update'],env=env,capture_output=True)
        assert result.returncode==0,result.stdout+result.stderr
        if windows: assert ctypes.windll.kernel32.GetConsoleOutputCP()==prior_cp
        assert result.stdout.endswith(('updated envswitch v'+version+' -> v0.0.42\n').encode()),result.stdout
        after=json.loads((current/'install.json').read_text())
        assert after['repo']=='demo/suite' and after['installDir']==str(launchers),after
        assert (current/'.version').read_bytes()==b'v0.0.42\n' and old.exists()
        assert not (app/'.install.lock').exists()
        assert not list(Path(env['TMPDIR']).glob('envswitch-update-*'))
    if not windows:
        # A broken stdout must still reach and pass the CLI's metadata checks.
        broken_env=dict(env,ENVSWITCH_D3_RESULT=str(root/'result.json'),ENVSWITCH_D3_CLI_BROKEN=str(old))
        broken=subprocess.run([sys.argv[3],'--exact','runner_child','--nocapture'],env=broken_env,capture_output=True)
        assert broken.returncode==0,broken.stdout+broken.stderr
        assert json.loads((root/'result.json').read_text())['code']==0,broken.stderr
        assert b'warning: could not forward installer stdout:' in broken.stderr,broken.stderr
    assert sum(p.endswith('/latest') for p,_ in server.requests)==4,server.requests
    assert not any('/tags/' in p for p,_ in server.requests)
    assert all(p.startswith('/repos/demo/suite/') and auth is None for p,auth in server.requests)
    assert not (root/'wrong-app').exists() and not (root/'wrong-bin').exists()
finally: server.close()
"#;

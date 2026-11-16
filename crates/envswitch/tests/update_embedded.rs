//! Process isolation keeps all installer homes, PATH edits and hooks off the host.

#[cfg(unix)]
#[test]
fn embedded_installer_updates_real_managed_binary_using_hermetic_release() {
    let temp = tempfile::tempdir().unwrap();
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let output = std::process::Command::new("/usr/bin/python3")
        .args(["-B", "-c", r#"
import json, os, shutil, subprocess, sys
from pathlib import Path
sys.path.insert(0, str(Path(sys.argv[1]) / 'test/install'))
from fixtures import Server, release
root = Path(sys.argv[3]).resolve()
app = root / "app '雪$`&!"
launchers = root / "bin '雪$`&!"
current = app / 'current'
(current / 'bin').mkdir(parents=True)
launchers.mkdir()
env = {}
for key in ('HOME','USERPROFILE','LOCALAPPDATA','APPDATA','XDG_DATA_HOME','XDG_CONFIG_HOME','XDG_CACHE_HOME','XDG_STATE_HOME','TMPDIR','TEMP','TMP','BUN_INSTALL','GH_CONFIG_DIR'):
    path = root / key
    path.mkdir()
    env[key] = str(path)
tools = Path(env['HOME']) / 'tools'
tools.mkdir()
(tools / 'python3').symlink_to(sys.executable)
for name in ('gh', 'op', 'bun', 'curl'):
    path = tools / name
    path.write_text('#!/bin/bash\necho forbidden >> "$HOME/'+name+'.log"\nexit 98\n')
    path.chmod(0o755)
env.update(PATH=str(tools)+':/usr/bin:/bin:/usr/sbin:/sbin', SHELL='/bin/zsh',
           ENVSWITCH_INSTALL_TEST_MODE='install', ENVSWITCH_VERSION='v0.0.77',
           ENVSWITCH_CURRENT_VERSION='v0.0.77', ENVSWITCH_VERIFY_ATTESTATIONS='1',
           REPO='wrong/repo', APP_ROOT=str(root/'wrong-app'), INSTALL_DIR=str(root/'wrong-bin'))
version, target = sys.argv[4:6]
payload = current / 'bin' / ('envswitch_'+version+'_'+target)
shutil.copy2(sys.argv[2], payload)
launcher = launchers / 'envswitch'
launcher.write_text('#!/usr/bin/env bash\nset -euo pipefail\nexec '+("'"+str(payload).replace("'","'\\''")+"'")+' "$@"\n')
launcher.chmod(0o755)
receipt = dict(schemaVersion=1, kind='native', repo='demo/suite', appRoot=str(app),
               installDir=str(launchers), launcherPath=str(launcher), target=target,
               tag='v'+version, payloadPath=str(payload))
(current/'install.json').write_text(json.dumps(receipt))
(current/'.version').write_text('v0.0.999\n')
# Scripts deliberately missing: the updater must use its embedded copy.
server = Server(release())
env['ENVSWITCH_INSTALL_TEST_API_BASE_URL'] = server.base
try:
    for attempt in range(3):
        if attempt == 2:
            (current/'.version').unlink()
            (current/'scripts/install.sh').unlink()
        result = subprocess.run([str(payload), 'update'], env=env, capture_output=True, text=True)
        assert result.returncode == 0, result.stdout+result.stderr
        assert result.stdout.endswith('updated envswitch v'+version+' -> v0.0.42\n'), result.stdout
        assert result.stderr == 'envswitch [info] updating envswitch v'+version+' -> latest\n', result.stderr
        terminal = ['success: installed envswitch v0.0.42', 'success: envswitch is already up to date (v0.0.42)', 'success: repaired envswitch v0.0.42'][attempt]
        assert terminal+'\n' in result.stdout, result.stdout
        after = json.loads((current/'install.json').read_text())
        assert after['repo'] == 'demo/suite' and after['installDir'] == str(launchers)
        assert (current/'.version').read_bytes() == b'v0.0.42\n'
        assert payload.exists(), 'running old payload must be preserved'
        assert not (app/'.install.lock').exists()
        assert not list(Path(env['TMPDIR']).glob('envswitch-update-*')), 'private scripts must be cleaned'
    assert sum(p.endswith('/latest') for p,_ in server.requests) == 3, server.requests
    assert not any('/tags/' in p for p,_ in server.requests), server.requests
    assert all(p.startswith('/repos/demo/suite/') for p,_ in server.requests)
    assert all(auth is None for _,auth in server.requests)
    assert not (Path(env['HOME'])/'op.log').exists()
    assert not (Path(env['HOME'])/'bun.log').exists()
    assert not (root/'wrong-app').exists() and not (root/'wrong-bin').exists()
finally:
    server.close()
"#])
        .arg(repo).arg(env!("CARGO_BIN_EXE_envswitch")).arg(temp.path())
        .arg(env!("CARGO_PKG_VERSION")).arg(env!("TARGET"))
        .env("PYTHONDONTWRITEBYTECODE", "1").output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
#[test]
fn runner_child() {
    use envswitch::install::EmbeddedInstaller;
    use envswitch::ports::{InstallPlan, InstallerRunner};
    let Some(result_path) = std::env::var_os("ENVSWITCH_D3_RESULT") else {
        return;
    };
    let plan = InstallPlan {
        repo: "demo/suite".into(),
        app_root: std::env::var_os("ENVSWITCH_D3_ROOT").unwrap().into(),
        install_dir: Some(std::env::var_os("ENVSWITCH_D3_BIN").unwrap().into()),
        version: std::env::var("ENVSWITCH_D3_PIN").ok(),
        current_version: "v0.0.0".into(),
        verify_attestations: false,
    };
    let mut stdout = Vec::new();
    let result = match EmbeddedInstaller.run_with_output(&plan, &mut stdout) {
        Ok((code, output)) => serde_json::json!({"code":code,"output":output}),
        Err(error) => serde_json::json!({"code":error.exit_code(),"error":error.to_string()}),
    };
    std::fs::write(result_path, result.to_string()).unwrap();
}

#[cfg(unix)]
#[test]
fn runner_handoff_clears_stale_variables_forwards_pin_maps_signal_and_cleans_up() {
    let temp = tempfile::tempdir().unwrap();
    let output = std::process::Command::new("/usr/bin/python3")
        .args(["-B", "-c", r#"
import json, os, stat, subprocess, sys
from pathlib import Path
root = Path(sys.argv[2]).resolve()
tools = root/'tools'
tools.mkdir()
tmp = root/'tmp'
tmp.mkdir()
bash = tools/'bash'
bash.write_text('#!'+sys.executable+'\n'+'''
import json, os, stat, sys
from pathlib import Path
script = Path(sys.argv[1])
record = dict(argv=sys.argv[1:], cwd=os.getcwd(), script=str(script),
              script_mode=stat.S_IMODE(script.stat().st_mode),
              dir_mode=stat.S_IMODE(script.parent.stat().st_mode),
              env={k:os.environ.get(k) for k in ['REPO','APP_ROOT','INSTALL_DIR','ENVSWITCH_VERSION','ENVSWITCH_CURRENT_VERSION','ENVSWITCH_VERIFY_ATTESTATIONS','GITHUB_TOKEN']})
Path(os.environ['ENVSWITCH_D3_LOG']).write_text(json.dumps(record))
if os.environ.get('ENVSWITCH_D3_ACTION') == 'signal': os.kill(os.getpid(),15)
print('installer stdout',flush=True)
sys.exit(int(os.environ.get('ENVSWITCH_D3_ACTION','0')))
''')
bash.chmod(0o755)
env = dict(PATH=str(tools), TMPDIR=str(tmp), HOME=str(root),
           ENVSWITCH_D3_ROOT=str(root/"app '雪$`&!"), ENVSWITCH_D3_BIN=str(root/"bin '雪$`&!"),
           ENVSWITCH_D3_RESULT=str(root/'result.json'), ENVSWITCH_D3_LOG=str(root/'log.json'),
           REPO='wrong/repo', APP_ROOT='wrong', INSTALL_DIR='wrong', ENVSWITCH_VERSION='v0.0.77',
           ENVSWITCH_CURRENT_VERSION='v0.0.77', ENVSWITCH_VERIFY_ATTESTATIONS='1', GITHUB_TOKEN='synthetic-token')
for action,pin,code in [('0',None,0),('7','v0.0.42',7),('signal',None,143),('spawn',None,2)]:
    child_env = dict(env,ENVSWITCH_D3_ACTION=action)
    if pin: child_env['ENVSWITCH_D3_PIN']=pin
    if action=='spawn': bash.unlink()
    proc = subprocess.run([sys.argv[1],'--exact','runner_child','--nocapture'],env=child_env,capture_output=True,text=True)
    assert proc.returncode == 0, proc.stdout+proc.stderr
    result = json.loads((root/'result.json').read_text())
    assert result['code'] == code, result
    if action=='spawn':
        assert result['error'].startswith('update failed: could not run installer:'), result
    else:
        record = json.loads((root/'log.json').read_text())
        assert record['argv'] == [record['script']], record
        assert record['cwd'] == str(Path(record['script']).parent), record
        assert record['script_mode'] == 0o600 and record['dir_mode'] == 0o700, record
        assert not Path(record['script']).parent.exists(), record
        assert record['env'] == dict(REPO='demo/suite',APP_ROOT=env['ENVSWITCH_D3_ROOT'],INSTALL_DIR=env['ENVSWITCH_D3_BIN'],
            ENVSWITCH_VERSION=pin,ENVSWITCH_CURRENT_VERSION='v0.0.0',ENVSWITCH_VERIFY_ATTESTATIONS=None,GITHUB_TOKEN='synthetic-token'), record
        if action != 'signal': assert result['output'] == 'installer stdout\n', result
    assert list(tmp.iterdir()) == [], list(tmp.iterdir())
"#]).arg(std::env::current_exe().unwrap()).arg(temp.path()).output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

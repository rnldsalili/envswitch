#!/usr/bin/env bash
set -euo pipefail
if ! command -v python3 >/dev/null 2>&1; then
  printf 'error: required installer tool not found: python3\n' >&2
  exit 1
fi
# Python's standard library validates JSON and archive metadata before writing any members.
exec python3 - "${BASH_SOURCE[0]}" "$@" <<'PY'
import contextlib
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import signal
import stat
import subprocess
import sys
import tarfile
import time
import urllib.error
import urllib.parse
import urllib.request

E = os.environ
ROLLBACK_MESSAGES=[]
os.umask(0o077)
BEGIN, END = '# >>> envswitch PATH >>>', '# <<< envswitch PATH <<<'
TARGETS = ('aarch64-apple-darwin', 'x86_64-apple-darwin', 'x86_64-unknown-linux-musl',
           'aarch64-unknown-linux-musl', 'x86_64-pc-windows-msvc')
VERSION = r'0\.0\.(?:0|[1-9][0-9]*)'

class Failure(Exception): pass

def fail(*lines):
    raise Failure('\n'.join('error: ' + s for s in lines))

def info(s): print('info: ' + s, flush=True)

def setting(name): fail('invalid installer setting: ' + name)

def tag(value, name):
    if not value: return None
    if not re.fullmatch('v?' + VERSION, value):
        fail('invalid ' + name + ': ' + value + '; expected 0.0.N or v0.0.N')
    return 'v' + value.lstrip('v')

def repo_ok(s):
    return bool(re.fullmatch(r'[a-zA-Z0-9_.-]+/[a-zA-Z0-9_.-]+', s)) and all(c not in ('.','..') for c in s.split('/'))

def unsafe(p):
    fail('unsafe installation path: ' + str(p), 'choose absolute, non-overlapping envswitch paths without symlinks or reparse points')

def check_links(p):
    for c in (p, *p.parents):
        if c.is_symlink(): unsafe(c)

def directory(value, name):
    if not value.startswith('/') or '..' in value:
        fail('invalid ' + name + ': ' + value, name + " must be an absolute path without '..'")
    if ':' in value or any(ord(c) < 32 or 127 <= ord(c) <= 159 for c in value): unsafe(value)
    p = Path(os.path.normpath(value)); check_links(p)
    if p == Path('/') or (p.exists() and not p.is_dir()): unsafe(p)
    return p

def inside(p, root): return p == root or root in p.parents

def quote(p): return "'" + str(p).replace("'", "'\\''") + "'"

def escaped(p):
    return re.sub(r'([\\"$`])', r'\\\1', str(p))

def profile():
    return Path(E['HOME']) / {'zsh':'.zshrc','bash':'.bashrc'}.get(Path(E.get('SHELL','')).name, '.profile')

def profile_bytes(install, p):
    check_links(p)
    old = p.read_bytes() if p.exists() else b''
    block = (BEGIN + '\nexport PATH="' + escaped(install) + ':$PATH"\n' + END + '\n').encode()
    lines = old.splitlines(keepends=True)
    starts = [i for i,s in enumerate(lines) if BEGIN.encode() in s]
    ends = [i for i,s in enumerate(lines) if END.encode() in s]
    if starts or ends:
        if len(starts) != 1 or len(ends) != 1 or ends[0] != starts[0]+2 or lines[starts[0]].rstrip(b'\r\n') != BEGIN.encode() or lines[ends[0]].rstrip(b'\r\n') != END.encode() or not re.fullmatch(rb'export PATH=".*:\$PATH"\r?\n', lines[starts[0]+1]):
            fail('invalid envswitch PATH block in ' + str(p) + '; repair the block and rerun the installer')
        a,b = starts[0],ends[0]
        if b''.join(lines[a:b+1]) == block: return old, 'PATH block already exists in ' + str(p)
        return b''.join(lines[:a])+block+b''.join(lines[b+1:]), 'updated envswitch PATH block in ' + str(p)
    return old + (b'\n' if not old or old.endswith(b'\n') else b'\n\n') + block, 'added ' + str(install) + ' to PATH in ' + str(p)

def run(args, **kw):
    return subprocess.run([str(a) for a in args], stdout=subprocess.PIPE, stderr=subprocess.PIPE, **kw)

def json_read(data):
    def pairs(items):
        d = {}
        for k,v in items:
            if k in d: raise ValueError('duplicate JSON key')
            d[k]=v
        return d
    return json.loads(data.decode('utf-8'), object_pairs_hook=pairs)

def launcher(kind, payload):
    head = '#!/usr/bin/env bash\nset -euo pipefail\n'
    if kind == 'source':
        head += '''if command -v bun >/dev/null 2>&1; then
  BUN_BIN="$(command -v bun)"
elif [[ -x "${HOME}/.bun/bin/bun" ]]; then
  BUN_BIN="${HOME}/.bun/bin/bun"
else
  printf 'envswitch: bun is not installed or not on PATH\\n' >&2
  exit 127
fi
exec "$BUN_BIN" '''
    else: head += 'exec '
    return (head + quote(payload) + ' "$@"\n').encode()

def unmanaged(p):
    fail('refusing to replace an unmanaged path: ' + str(p), 'preserve user files and reinstall using the README instructions with explicit install paths')

def payload_for(root, kind, version, target):
    return root / 'current' / ('src/cli.ts' if kind == 'source' else 'bin/envswitch_' + version[1:] + '_' + target)

def receipt_read(p):
    if not p.exists(): return None
    try:
        r = json_read(p.read_bytes())
        assert isinstance(r,dict) and type(r['schemaVersion']) is int and r['schemaVersion'] == 1
        assert r['kind'] in ('source','native') and repo_ok(r['repo'])
        assert re.fullmatch('v'+VERSION,r['tag'])
        root = directory(r['appRoot'], 'APP_ROOT'); dest = directory(r['installDir'], 'INSTALL_DIR')
        assert str(root) == r['appRoot'] and str(dest) == r['installDir']
        assert root != dest and not inside(dest,root/'current') and not any(c.startswith('.install-') for c in dest.parts)
        assert root not in [Path(E[k]) for k in ('HOME','USERPROFILE') if E.get(k)] and root == p.parent.parent
        assert r['launcherPath'] == str(dest/'envswitch')
        assert r['target'] is None if r['kind'] == 'source' else r['target'] in TARGETS and not r['target'].endswith('msvc')
        assert r['payloadPath'] == str(payload_for(root,r['kind'],r['tag'],r['target']))
        for k in ('launcherPath','payloadPath'): check_links(Path(r[k]))
        return r
    except (ValueError,KeyError,TypeError,AssertionError,Failure):
        fail('invalid envswitch install receipt: ' + str(p), 'reinstall envswitch with the install script from the README after checking the install paths')

def source_tree(p, dependencies=False, required=True):
    allowed = {'src','scripts','package.json','bun.lock','README.md','tsconfig.json'}
    if dependencies: allowed.add('node_modules')
    for f in p.rglob('*'):
        rel = f.relative_to(p)
        if rel.parts[0] not in allowed or f.is_symlink() or not (f.is_file() or f.is_dir()) or any(c in ('.git','.env') or c.startswith('.env.') for c in rel.parts): unmanaged(f)
    try:
        if required or (p/'package.json').exists(): assert json_read((p/'package.json').read_bytes())['name'] == 'envswitch'
        if required: assert (p/'src/cli.ts').is_file() and (p/'bun.lock').is_file()
    except (ValueError,KeyError,AssertionError,OSError): fail('archive did not contain an envswitch source package')

class NoTokenRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        url = urllib.parse.urlsplit(newurl)
        allowed = url.scheme=='https' and url.hostname in ('api.github.com','release-assets.githubusercontent.com','objects.githubusercontent.com','github-releases.githubusercontent.com') and url.port in (None,443)
        allowed = allowed or (TEST and url.scheme=='http' and url.hostname in ('127.0.0.1','localhost','::1'))
        if not allowed or url.username or url.password: raise ValueError('unsafe redirect')
        redirected = super().redirect_request(req,fp,code,msg,headers,newurl)
        if urllib.parse.urlsplit(req.full_url).netloc != url.netloc: redirected.remove_header('Authorization')
        return redirected

def request_timeout(signum, frame): raise TimeoutError()

def http(url, accept):
    previous=signal.signal(signal.SIGALRM,request_timeout)
    signal.setitimer(signal.ITIMER_REAL,60)
    try: return request_http(url,accept)
    finally:
        signal.setitimer(signal.ITIMER_REAL,0); signal.signal(signal.SIGALRM,previous)

def request_http(url, accept):
    headers = {'Accept':accept}
    if E.get('GITHUB_TOKEN'): headers['Authorization']='Bearer '+E['GITHUB_TOKEN']
    opener=urllib.request.build_opener(NoTokenRedirect())
    deadline=time.monotonic()+60
    for attempt in range(3):
        try:
            with opener.open(urllib.request.Request(url,headers=headers),timeout=max(0.1,deadline-time.monotonic())) as response:
                return response.read()
        except urllib.error.HTTPError as e:
            if e.code not in (408,429) and e.code < 500: raise
            if attempt == 2: raise
        except (urllib.error.URLError,TimeoutError,OSError):
            if attempt == 2: raise
        if time.monotonic() >= deadline: raise TimeoutError()

class Installer:
    def __init__(self):
        self.stage=None; self.token=secrets.token_hex(16); self.locked=False; self.journal=[]; self.created=[]; self.commit=False; self.new_payload=None
    def mkdir(self,p):
        if p.exists():
            check_links(p)
            if not p.is_dir(): unsafe(p)
            return
        self.mkdir(p.parent); p.mkdir(mode=0o700); self.created.append(p)
    def replace(self,p,data=None,source=None,mode=0o600):
        check_links(p); self.mkdir(p.parent)
        backup=self.stage/('backup-'+str(len(self.journal)))
        existed=p.exists()
        if existed:
            if p.is_dir(): shutil.copytree(p,backup,symlinks=True)
            else: shutil.copy2(p,backup)
        self.journal.append((p,backup if existed else None))
        tmp=p.parent/('.install-'+self.token+'-'+str(len(self.journal)))
        moved=False
        try:
            if source:
                if source.is_dir(): shutil.copytree(source,tmp)
                else:
                    with tmp.open('xb') as f: f.write(source.read_bytes())
                    tmp.chmod(mode)
            else:
                with tmp.open('xb') as f: f.write(data)
                tmp.chmod(mode)
            if p.is_dir():
                os.replace(p,self.stage/('moved-'+str(len(self.journal)))); moved=True
            check_links(p)
            try: os.replace(tmp,p)
            except OSError:
                if not moved: self.journal.pop()
                raise
        finally:
            if tmp.exists():
                if tmp.is_dir(): shutil.rmtree(tmp)
                else: tmp.unlink()
    def boundary(self,s):
        if TEST and E.get('ENVSWITCH_INSTALL_TEST_FAIL_AT') == s: fail('injected installer test failure: '+s)
    def cleanup(self):
        rollback_failed=False
        if not self.commit:
            try:
                for p,b in reversed(self.journal):
                    if b:
                        if p.is_dir(): shutil.rmtree(p)
                        os.replace(b,p)
                    elif p.exists():
                        if p.is_dir(): shutil.rmtree(p)
                        else: p.unlink()
            except OSError:
                ROLLBACK_MESSAGES.append('error: rollback incomplete; preserve '+str(self.stage)+' and reinstall using the README instructions')
                rollback_failed=True
        if not self.commit and not rollback_failed and self.new_payload:
            try:
                check_links(self.new_payload); self.new_payload.unlink()
            except FileNotFoundError: pass
            except OSError: info('deferred cleanup: '+str(self.new_payload))
        if self.locked:
            try:
                if not LOCK.is_symlink() and LOCK.read_text() == self.token+' '+str(os.getpid())+'\n': LOCK.unlink()
            except FileNotFoundError: pass
        if not rollback_failed and self.stage and self.stage.exists():
            try: shutil.rmtree(self.stage)
            except OSError: info('deferred cleanup: '+str(self.stage))
        for p in reversed(self.created):
            with contextlib.suppress(OSError): p.rmdir()
    def download(self,name):
        dst=self.stage/name
        if GH:
            info('downloading '+name+' with GitHub CLI')
            try: result=run([GH,'release','download',TAG,'--repo',REPO,'--pattern',name,'--dir',self.stage],timeout=60)
            except subprocess.TimeoutExpired: result=None
            if result is not None and result.returncode == 0 and dst.is_file(): return dst
            if dst.exists(): dst.unlink()
            info('GitHub CLI download did not find '+name+'; trying direct download')
        else: info('downloading '+name)
        try: dst.write_bytes(http(ASSETS[name],'application/octet-stream'))
        except (OSError,ValueError,urllib.error.URLError): fail('release asset not found or unavailable: '+name)
        return dst
    def verify(self,p,name):
        info('verifying SHA-256 for '+name)
        args=([HASH] if Path(HASH).name == 'sha256sum' else [HASH,'-a','256'])+[str(p)]
        r=run(args)
        if r.returncode or r.stdout.decode().split()[0].lower() != SUMS[name]: fail('SHA-256 mismatch for '+name)
    def check(self,p,kind,version,target,staged=False):
        try:
            if kind == 'native':
                result=run([p,'--version'])
                if result.returncode or result.stderr or result.stdout != ('envswitch '+version+' '+target+'\n').encode():
                    if staged: fail('native payload verification failed; expected envswitch '+version+' '+target)
                    return False
            return run([p,'--help']).returncode == 0
        except OSError:
            if staged: fail('native payload verification failed; expected envswitch '+version+' '+target)
            return False


def extract(archive,out,kind):
    try:
        with tarfile.open(archive,'r:gz') as tar:
            seen=set(); members=[]
            for m in tar.getmembers():
                name=m.name
                if name.startswith('./'): name=name[2:]
                if m.isdir(): name=name.rstrip('/')
                parts=name.split('/')
                if not name or '\\' in name or ':' in name or any(ord(c)<32 or 127<=ord(c)<=159 for c in name) or any(c in ('','.','..') for c in parts) or name in seen or not (m.isfile() or m.isdir()): raise ValueError()
                seen.add(name)
                if kind == 'native':
                    if name not in ('envswitch','scripts','scripts/install.sh','scripts/install.ps1','README.md') or (m.isdir() != (name == 'scripts')): raise ValueError()
                else:
                    if parts[0] != 'envswitch' or (len(parts)>1 and parts[1] not in ('package.json','bun.lock','README.md','tsconfig.json','src','scripts')) or any(c in ('.git','node_modules','.env') or c.startswith('.env.') for c in parts): raise ValueError()
                    if len(parts)>2 and parts[1] not in ('src','scripts'): raise ValueError()
                    if m.isdir() and len(parts)>1 and parts[1] not in ('src','scripts'): raise ValueError()
                members.append((m,name))
            expected={'envswitch','scripts/install.sh','scripts/install.ps1','README.md'} if kind=='native' else {'envswitch/package.json','envswitch/bun.lock','envswitch/src/cli.ts','envswitch/scripts/install.sh','envswitch/scripts/install.ps1'}
            if not expected.issubset({n for m,n in members if m.isfile()}): raise ValueError()
            out.mkdir(mode=0o700)
            for m,n in members:
                p=out/n; check_links(p)
                if m.isdir(): p.mkdir(mode=0o700,parents=True,exist_ok=True)
                else:
                    p.parent.mkdir(mode=0o700,parents=True,exist_ok=True)
                    with p.open('xb') as f: shutil.copyfileobj(tar.extractfile(m),f)
                    p.chmod(0o600)
            for p in out.rglob('*'):
                if p.is_symlink() or not (p.is_file() or p.is_dir()): raise ValueError()
    except (ValueError,OSError,tarfile.TarError): fail('unsafe or invalid archive: '+archive.name)


def main():
    global TEST, REPO, TAG, ASSETS, HASH, SUMS, GH, LOCK
    old=E.get('ENVS_SWITCH_INSTALL_TEST_MODE',''); mode=E.get('ENVSWITCH_INSTALL_TEST_MODE','')
    if old and mode: setting('ENVSWITCH_INSTALL_TEST_MODE')
    if old and old not in ('ensure-profile-path','shell-profile-path'): setting('ENVS_SWITCH_INSTALL_TEST_MODE')
    mode=mode or old
    if mode not in ('','install','ensure-profile-path','shell-profile-path'): setting('ENVSWITCH_INSTALL_TEST_MODE')
    if mode == 'shell-profile-path':
        if len(sys.argv)!=2: fail('unexpected installer argument: '+sys.argv[2])
        print(profile()); return
    if mode == 'ensure-profile-path':
        if len(sys.argv)!=4: setting('ENVSWITCH_INSTALL_TEST_MODE')
        p=Path(sys.argv[3]); data,msg=profile_bytes(Path(sys.argv[2]),p)
        if not p.exists() or p.read_bytes()!=data: p.parent.mkdir(parents=True,exist_ok=True); p.write_bytes(data)
        info(msg); return
    if len(sys.argv)>2: fail('unexpected installer argument: '+sys.argv[2])
    TEST=mode=='install'
    if TEST and E.get('ENVSWITCH_INSTALL_TEST_FAIL_AT','') not in ('','after-payload','after-launcher','after-metadata'): setting('ENVSWITCH_INSTALL_TEST_FAIL_AT')
    base='https://api.github.com'
    if TEST and E.get('ENVSWITCH_INSTALL_TEST_API_BASE_URL'):
        base=E['ENVSWITCH_INSTALL_TEST_API_BASE_URL']
        u=urllib.parse.urlsplit(base)
        if u.scheme!='http' or u.hostname not in ('127.0.0.1','localhost','::1') or u.username or u.password or u.path or u.query or u.fragment: setting('ENVSWITCH_INSTALL_TEST_API_BASE_URL')
    REPO=E.get('REPO') or 'rnldsalili/envswitch'
    if not repo_ok(REPO): fail('invalid repository format: '+REPO,'expected owner/repo')
    pin=tag(E.get('ENVSWITCH_VERSION'),'Version'); current=tag(E.get('ENVSWITCH_CURRENT_VERSION'),'CurrentVersion')
    attest=E.get('ENVSWITCH_VERIFY_ATTESTATIONS','')
    if attest not in ('','0','1'): setting('ENVSWITCH_VERIFY_ATTESTATIONS')
    root=directory(E.get('APP_ROOT') or str(Path(E.get('XDG_DATA_HOME') or (E['HOME']+'/.local/share'))/'envswitch'),'APP_ROOT')
    dest=directory(E.get('INSTALL_DIR') or E['HOME']+'/.local/bin','INSTALL_DIR')
    app=root/'current'; launch=dest/'envswitch'; LOCK=root/'.install.lock'
    if root in [Path(E[k]) for k in ('HOME','USERPROFILE') if E.get(k)] or dest==root or inside(dest,app) or inside(root,launch) or any(c.startswith('.install-') for c in (*root.parts,*dest.parts)): unsafe(root)
    for p in (app, launch, app/'bin', app/'scripts', app/'scripts/install.sh', app/'scripts/install.ps1', app/'install.json', app/'.version'): check_links(p)
    if app.exists() and not app.is_dir(): unsafe(app)
    for p in (root/'.git',app/'.git'):
        if p.exists(): unmanaged(p)
    for tool in ('uname','bash'):
        if not shutil.which(tool): fail('required installer tool not found: '+tool)
    osname=run(['uname','-s']).stdout.decode().strip().lower(); arch=run(['uname','-m']).stdout.decode().strip()
    if osname not in ('darwin','linux'): fail('unsupported OS for this installer: '+osname,'use scripts/install.ps1 on Windows')
    if osname=='darwin' and shutil.which('sysctl') and run(['sysctl','-n','hw.optional.arm64']).stdout.strip()==b'1': arch='arm64'
    cpu={'aarch64':'aarch64','arm64':'aarch64','x86_64':'x86_64','amd64':'x86_64'}.get(arch)
    if not cpu: fail('unsupported architecture for this installer: '+arch)
    target=cpu+('-apple-darwin' if osname=='darwin' else '-unknown-linux-musl')
    info('detected macOS' if osname=='darwin' else 'detected WSL/Linux' if Path('/proc/version').exists() and 'microsoft' in Path('/proc/version').read_text().lower() else 'detected Linux')
    previous=receipt_read(app/'install.json')
    if launch.exists():
        content=launch.read_bytes(); recognized=False
        if previous: recognized=content==launcher(previous['kind'],Path(previous['payloadPath']))
        if content==launcher('source',app/'src/cli.ts'): recognized=True
        # Historical launcher differs only in blank lines and a double-quoted literal path.
        historical=launcher('source',app/'src/cli.ts').replace(quote(app/'src/cli.ts').encode(), ('"'+str(app/'src/cli.ts')+'"').encode())
        if content.replace(b'\n\n',b'\n')==historical: recognized=True
        for p in (app/'bin').glob('envswitch_*') if (app/'bin').is_dir() else []:
            if re.fullmatch('envswitch_'+VERSION+'_('+ '|'.join(TARGETS[:-1])+')',p.name) and content==launcher('native',p): recognized=True
        # Missing immutable payload still has a recognizable exact launcher.
        if not recognized:
            for t in TARGETS[:-1]:
                match=re.search(rb"envswitch_(0\.0\.(?:0|[1-9][0-9]*))_"+t.encode(),content)
                if match and content==launcher('native',app/'bin'/('envswitch_'+match[1].decode()+'_'+t)): recognized=True
        if not recognized: unmanaged(launch)
    if app.exists() and any(app.iterdir()) and not previous and not launch.exists():
        if not (app/'package.json').is_file(): unmanaged(app)
        try:
            if json_read((app/'package.json').read_bytes()).get('name')!='envswitch': unmanaged(app)
        except ValueError: unmanaged(app)
    GH=shutil.which('gh')
    if GH and TEST and not inside(Path(GH),Path(E['HOME'])): GH=None
    verifier=GH
    if GH and run([GH,'auth','status'],timeout=60).returncode: GH=None
    if attest=='1' and not verifier: fail('gh is required when attestation verification is requested')
    HASH=shutil.which('sha256sum') or shutil.which('shasum')
    if not HASH: fail('required installer tool not found: sha256sum or shasum')
    tx=Installer()
    try:
        tx.mkdir(root)
        try:
            fd=os.open(LOCK,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
        except FileExistsError: fail('another envswitch installer may be running: '+str(LOCK),'retry after it exits; remove the lock only after confirming no installer is running')
        with os.fdopen(fd,'w') as f: f.write(tx.token+' '+str(os.getpid())+'\n')
        tx.locked=True
        previous=receipt_read(app/'install.json')
        tx.stage=root/('.install-stage-'+tx.token); tx.stage.mkdir(mode=0o700)
        if Path(sys.argv[1]).is_file(): shutil.copyfile(sys.argv[1],tx.stage/'executing-install.sh')
        endpoint='repos/'+REPO+'/releases/'+('tags/'+pin if pin else 'latest')
        data=None
        if GH:
            try: r=run([GH,'api',endpoint],timeout=60)
            except subprocess.TimeoutExpired: r=None
            if r is not None and not r.returncode: data=r.stdout
        if data is None:
            try: data=http(base+'/'+endpoint,'application/vnd.github+json')
            except (OSError,ValueError,urllib.error.URLError):
                fail('GitHub Release not found or unavailable: '+REPO+'@'+pin if pin else 'no GitHub Release found for '+REPO+', or access is denied')
        bad='invalid envswitch release metadata for '+REPO
        try:
            release=json_read(data); TAG=release['tag_name']
            assert re.fullmatch('v'+VERSION,TAG) and (not pin or TAG==pin)
            assert type(release['id']) is int and release['id']>0 and release['draft'] is False and release['prerelease'] is False
            ASSETS={}; duplicates=set()
            for a in release['assets']:
                n=a['name']; url=a['url']
                assert isinstance(n,str) and isinstance(url,str)
                assert re.fullmatch(re.escape(base+'/repos/'+REPO+'/releases/assets/')+r'[0-9]+',url)
                if n in ASSETS: duplicates.add(n)
                ASSETS[n]=url
        except (KeyError,TypeError,ValueError,AssertionError): fail(bad)
        ver=TAG[1:]; source='envswitch_'+ver+'_source.tar.gz'
        natives=['envswitch_'+ver+'_'+t+('.zip' if t.endswith('msvc') else '.tar.gz') for t in TARGETS]
        kind='native' if any(n in ASSETS for n in natives) else 'source'
        required=natives if kind=='native' else [source]
        if (kind=='native' and source in ASSETS) or any(n not in ASSETS for n in required) or duplicates.intersection(required+['SHA256SUMS','install.sh','install.ps1']): fail(bad)
        if 'SHA256SUMS' not in ASSETS: fail('release '+TAG+' is missing SHA256SUMS; refusing unverified installation')
        if any(n not in ASSETS for n in ('install.sh','install.ps1')): fail(bad)
        name=natives[TARGETS.index(target)] if kind=='native' else source
        info('resolved envswitch '+TAG+' ('+kind+')')
        if kind=='native': info('selected target: '+target)
        manifest=tx.download('SHA256SUMS'); archive=tx.download(name)
        try:
            raw=manifest.read_bytes(); assert raw.endswith(b'\n'); SUMS={}
            for line in raw.split(b'\n')[:-1]:
                m=re.fullmatch(rb'([0-9a-f]{64})  ([^/\\\x00-\x20\x7f]+)',line); assert m
                n=m[2].decode('utf-8'); assert not any(127<=ord(c)<=159 for c in n); assert n not in SUMS and n not in ('SHA256SUMS','.','..'); SUMS[n]=m[1].decode()
            assert all(n in SUMS for n in required+['install.sh','install.ps1'])
        except (AssertionError,UnicodeError): fail('invalid SHA256SUMS for release '+TAG)
        tx.verify(archive,name)
        if attest=='1':
            info('verifying GitHub attestation for '+name)
            if run([verifier,'attestation','verify',archive,'--repo',REPO],timeout=60).returncode: fail('GitHub attestation verification failed for '+name)
        info('extracting '+kind+' archive'); extract(archive,tx.stage/'extracted',kind)
        tree=tx.stage/'extracted'/('envswitch' if kind=='source' else '')
        for n in ('install.sh','install.ps1'): tx.verify(tree/'scripts'/n,n)
        payload=payload_for(root,kind,TAG,target)
        if kind=='native':
            staged=tx.stage/payload.name; shutil.copyfile(tree/'envswitch',staged); staged.chmod(0o755)
            if not tx.check(staged,kind,ver,target,True): fail('envswitch launcher verification failed: '+str(launch))
        else:
            source_tree(tree)
            bun=shutil.which('bun') or (str(Path(E['HOME'])/'.bun/bin/bun') if os.access(Path(E['HOME'])/'.bun/bin/bun',os.X_OK) else None)
            if not bun:
                if not shutil.which('curl'): fail('curl is required to install Bun automatically','install Bun manually from https://bun.com/docs/installation, then rerun this installer')
                info('Bun is not installed; installing Bun')
                bootstrap=run(['curl','-fsSL','https://bun.com/install'])
                if bootstrap.returncode or run(['bash'],input=bootstrap.stdout).returncode: fail('Bun installation finished, but bun was not found on PATH')
                E['BUN_INSTALL']=E.get('BUN_INSTALL') or E['HOME']+'/.bun'; E['PATH']=E['BUN_INSTALL']+'/bin:'+E['PATH']
                bun=shutil.which('bun')
                if not bun: fail('Bun installation finished, but bun was not found on PATH')
            info('using Bun at '+bun)
            if run([bun,'--version']).returncode: fail('Bun verification failed')
            info('installing production dependencies')
            if run([bun,'install','--production','--frozen-lockfile'],cwd=tree).returncode: fail('bun install failed')
            check=tx.stage/'source-check'; check.write_bytes(launcher('source',tree/'src/cli.ts')); check.chmod(0o755)
            if not tx.check(check,kind,ver,target): fail('envswitch launcher verification failed: '+str(launch))
        receipt={'schemaVersion':1,'kind':kind,'repo':REPO,'appRoot':str(root),'installDir':str(dest),'launcherPath':str(launch),'target':target if kind=='native' else None,'tag':TAG,'payloadPath':str(payload)}
        healthy=previous==receipt and (app/'.version').is_file() and (app/'.version').read_bytes()==(TAG+'\n').encode() and launch.is_file() and launch.read_bytes()==launcher(kind,payload) and os.access(launch,os.X_OK)
        healthy=healthy and all((app/'scripts'/n).is_file() and (app/'scripts'/n).read_bytes()==(tree/'scripts'/n).read_bytes() for n in ('install.sh','install.ps1'))
        healthy=healthy and payload.is_file()
        if kind=='native': healthy=healthy and payload.read_bytes()==staged.read_bytes()
        else:
            if healthy:
                # Source shape was validated at staging; installed required files must survive.
                healthy=healthy and all((app/n).is_file() for n in ('package.json','bun.lock','src/cli.ts'))
                if healthy:
                    try: healthy=json_read((app/'package.json').read_bytes()).get('name')=='envswitch'
                    except (ValueError,UnicodeError): healthy=False
        if healthy and tx.check(launch,kind,ver,target):
            tx.commit=True; tx.cleanup(); tx.stage=None; tx.locked=False
            print('success: envswitch is already up to date ('+TAG+')'); return
        oldtag=previous['tag'] if previous else None
        if not oldtag and (app/'.version').is_file():
            with contextlib.suppress(ValueError,UnicodeError):
                s=(app/'.version').read_text(encoding='utf-8-sig').strip('\r\n')
                if re.fullmatch('v?'+VERSION,s): oldtag='v'+s.lstrip('v')
        outcome='repaired' if (oldtag or current)==TAG and (not previous or (previous['kind']==kind and previous['target']==receipt['target'])) else 'installed'
        tx.mkdir(app); tx.mkdir(dest)
        if kind=='native':
            tx.mkdir(app/'bin'); check_links(payload)
            if payload.exists():
                if not payload.is_file() or payload.read_bytes()!=staged.read_bytes(): fail('existing native payload differs from verified release: '+str(payload),'close envswitch processes and reinstall using the README instructions; preserve the existing file until recovery is complete')
            else:
                info('installing binary to '+str(payload)); os.link(staged,payload); tx.new_payload=payload
        else:
            for f in app.iterdir():
                if f.name not in ('src','node_modules','package.json','bun.lock','README.md','tsconfig.json','scripts','bin','.version','install.json'): unmanaged(f)
            # Refuse to replace any old source entry containing user-owned or linked files.
            existing=tx.stage/'old-source'; existing.mkdir()
            for n in ('src','node_modules','package.json','bun.lock','README.md','tsconfig.json'):
                p=app/n
                if p.is_symlink(): unmanaged(p)
                if p.exists():
                    if p.is_dir():
                        for f in p.rglob('*'):
                            if f.is_symlink() or not (f.is_file() or f.is_dir()) or f.name=='.env' or f.name.startswith('.env.') or f.name=='.git': unmanaged(f)
                        shutil.copytree(p,existing/n)
                    else: shutil.copy2(p,existing/n)
            if any(existing.iterdir()): source_tree(existing,True,False)
            for n in ('src','node_modules','package.json','bun.lock','README.md','tsconfig.json'):
                if (tree/n).exists(): tx.replace(app/n,source=tree/n)
                elif (app/n).exists():
                    tx.journal.append((app/n,tx.stage/('removed-'+n))); os.replace(app/n,tx.journal[-1][1])
        tx.boundary('after-payload')
        info('installing launcher to '+str(launch)); tx.replace(launch,launcher(kind,payload),mode=0o755)
        info('verifying envswitch launcher')
        if not os.access(launch,os.X_OK): fail('launcher was not installed or is not executable: '+str(launch))
        if not tx.check(launch,kind,ver,target): fail('envswitch launcher verification failed: '+str(launch))
        tx.boundary('after-launcher')
        for n in ('install.sh','install.ps1'): tx.replace(app/'scripts'/n,source=tree/'scripts'/n)
        tx.replace(app/'install.json',(json.dumps(receipt,ensure_ascii=False,indent=2)+'\n').encode())
        tx.replace(app/'.version',(TAG+'\n').encode())
        if receipt_read(app/'install.json')!=receipt or (app/'.version').read_bytes()!=(TAG+'\n').encode(): fail('envswitch installation failed: metadata verification')
        tx.boundary('after-metadata')
        onpath=str(dest) in E.get('PATH','').split(':')
        if not onpath:
            p=profile(); data,msg=profile_bytes(dest,p)
            if not p.exists() or p.read_bytes()!=data: tx.replace(p,data,mode=stat.S_IMODE(p.stat().st_mode) if p.exists() else 0o600)
            info(msg)
        tx.commit=True; tx.cleanup(); tx.stage=None; tx.locked=False
        print('success: '+outcome+' envswitch '+TAG)
        info(('binary: '+str(payload)) if kind=='native' else 'source: '+str(app)); info('launcher: '+str(launch))
        if onpath: info("run 'envswitch --help' to verify the installation")
        else:
            info('restart your terminal, or run this command now:'); info('export PATH="'+escaped(dest)+':$PATH"'); info("then run 'envswitch --help' to verify the installation")
    finally: tx.cleanup()


def interrupted(signum, frame): raise KeyboardInterrupt(signum)
signal.signal(signal.SIGTERM,interrupted); signal.signal(signal.SIGINT,interrupted)
try: main()
except Failure as e:
    print(str(e),file=sys.stderr)
    for message in ROLLBACK_MESSAGES: print(message,file=sys.stderr)
    sys.exit(1)
except KeyboardInterrupt as e:
    for message in ROLLBACK_MESSAGES: print(message,file=sys.stderr)
    sys.exit(128+(e.args[0] if e.args else 2))
except (OSError,ValueError,TypeError,subprocess.SubprocessError) as e:
    # Never print remote URLs, headers, or subprocess diagnostics containing credentials.
    print('error: envswitch installation failed: '+type(e).__name__,file=sys.stderr)
    for message in ROLLBACK_MESSAGES: print(message,file=sys.stderr)
    sys.exit(1)
PY

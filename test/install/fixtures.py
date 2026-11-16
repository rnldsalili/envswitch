"""Loopback-only releases; no GitHub, Bun bootstrap, op, or host authentication."""
import hashlib
import http.server
import io
import json
import os
from pathlib import Path
import tarfile
import threading
import zipfile

TARGETS = ['aarch64-apple-darwin', 'x86_64-apple-darwin', 'x86_64-unknown-linux-musl',
           'aarch64-unknown-linux-musl', 'x86_64-pc-windows-msvc']
REPO = Path(__file__).resolve().parents[2]


def archive(entries, zip_format=False):
    out = io.BytesIO()
    if zip_format:
        with zipfile.ZipFile(out, 'w') as z:
            for name, data in (entries.items() if isinstance(entries, dict) else entries):
                z.writestr(name, data)
    else:
        with tarfile.open(fileobj=out, mode='w:gz') as t:
            for name, data in (entries.items() if isinstance(entries, dict) else entries):
                h = tarfile.TarInfo(name)
                if isinstance(data, tuple):
                    h.type, h.linkname = data
                    data = b''
                h.size = len(data)
                h.mode = 0o755 if name == 'envswitch' else 0o644
                t.addfile(h, io.BytesIO(data))
    return out.getvalue()


def release(kind='native', ver='0.0.42', fault=None, windows_binary=None):
    files = {n: (REPO / 'scripts' / n).read_bytes() for n in ('install.sh', 'install.ps1')}
    bundled = {'scripts/' + n: d for n, d in files.items()}
    if fault == 'script-hash':
        bundled['scripts/install.sh'] += b'\n# tampered\n'
    if kind == 'native':
        for target in TARGETS:
            version = '0.0.99' if fault == 'wrong-version' else ver
            actual_target = 'wrong-target' if fault == 'wrong-target' else target
            line = f'envswitch {version} {actual_target}'
            payload = (f'#!/usr/bin/env bash\ncase "$1" in\n--version) printf "%s\\n" "{line}";;\n'
                       '--help) exit 0;;\n*) printf "%s\\n" "$@"; exit 23;;\nesac\n').encode()
            if fault == 'newline':
                payload = payload.replace(b'%s\\n', b'%s\\n\\n')
            if fault == 'crlf':
                payload = payload.replace(b'%s\\n', b'%s\\r\\n')
            if fault == 'version-fails':
                payload = payload.replace(b'--version) printf', b'--version) exit 7; printf')
            if fault == 'stderr':
                payload = payload.replace(b'case "$1"', b'echo unexpected >&2\ncase "$1"')
            if fault == 'help-fails':
                payload = payload.replace(b'--help) exit 0', b'--help) exit 9')
            win = target.endswith('msvc')
            entries = dict(bundled, **{'README.md': b'demo\n', 'envswitch.exe' if win else 'envswitch': windows_binary or payload})
            if fault == 'traversal':
                entries['../escape'] = b'evil'
            if fault == 'link':
                entries['README.md'] = (tarfile.SYMTYPE, '/tmp/escape') if not win else b'demo'
            if fault == 'extra':
                entries['.env'] = b'SECRET=bad'
            name = f'envswitch_{ver}_{target}.' + ('zip' if win else 'tar.gz')
            if fault == 'duplicate-path': entries = list(entries.items()) + [('./README.md', b'duplicate')]
            files[name] = archive(entries, win)
    else:
        entries = dict(bundled, **{'package.json': b'{"name":"envswitch"}', 'bun.lock': b'lock',
                                 'src/cli.ts': b'// fixture', 'README.md': b'demo'})
        files[f'envswitch_{ver}_source.tar.gz'] = archive({'envswitch/' + n: d for n, d in entries.items()})
    sums = ''.join(hashlib.sha256(d).hexdigest() + '  ' + n + '\n' for n, d in files.items())
    if fault == 'malformed': sums = sums.replace('  ', ' ', 1)
    if fault == 'no-final-lf': sums = sums.rstrip('\n')
    if fault == 'self-entry': sums += ('0'*64)+'  SHA256SUMS\n'
    if fault == 'missing-archive-entry': sums = ''.join(line+'\n' for line in sums.splitlines() if '  envswitch_' not in line)
    if fault == 'duplicate-sum': sums += sums.splitlines()[0] + '\n'
    if fault == 'missing-entry': sums = '\n'.join(sums.splitlines()[1:]) + '\n'
    if fault == 'mismatch':
        for n,d in files.items():
            if n.startswith('envswitch_'): sums = sums.replace(hashlib.sha256(d).hexdigest(), '0' * 64)
    files['SHA256SUMS'] = sums.encode()
    if fault == 'missing-sums': del files['SHA256SUMS']
    if fault == 'partial': del files[f'envswitch_{ver}_{TARGETS[0]}.tar.gz']
    if fault == 'mixed': files[f'envswitch_{ver}_source.tar.gz'] = b'bad'
    return files


class Server:
    def __init__(self, files, tag='v0.0.42', fault=None):
        self.files, self.tag, self.fault, self.requests = files, tag, fault, []
        self.statuses=[]; self.redirect=None; self.change_latest=False
        owner = self
        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_): pass
            def do_GET(self):
                owner.requests.append((self.path, self.headers.get('Authorization')))
                base = f'http://127.0.0.1:{self.server.server_port}'
                if owner.statuses:
                    self.send_error(owner.statuses.pop(0)); return
                if owner.redirect and '/assets/' in self.path:
                    self.send_response(302); self.send_header('Location',owner.redirect+self.path); self.end_headers(); return
                if self.path.endswith('/latest') or '/tags/' in self.path:
                    assets = [{'name': n, 'url': base + '/repos/demo/suite/releases/assets/' + str(i)} for i, n in enumerate(owner.files, 1)]
                    if owner.fault == 'duplicate': assets.append(assets[0])
                    data = json.dumps({'assets': assets, 'id': 42, 'draft': False, 'prerelease': False, 'tag_name': owner.tag}).encode()
                    if owner.fault == 'bad-json': data = b'{"tag_name":"v0.0.42",'
                else:
                    try: data = list(owner.files.values())[int(self.path.rsplit('/', 1)[1])-1]
                    except (ValueError, IndexError):
                        self.send_error(404); return
                self.send_response(200); self.end_headers(); self.wfile.write(data)
                if owner.change_latest: owner.tag='v0.0.99'
        self.http = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.base = f'http://127.0.0.1:{self.http.server_port}'
        self.thread = threading.Thread(target=self.http.serve_forever, daemon=True)
        self.thread.start()
    def close(self):
        self.http.shutdown(); self.http.server_close(); self.thread.join()

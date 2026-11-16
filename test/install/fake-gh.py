#!/usr/bin/env python3
"""Suite-owned gh; all bytes come from a fixture directory, never from a real account."""
import json
import os
from pathlib import Path
import shutil
import sys

root=Path(os.environ['HOME'])
args=sys.argv[1:]
with (root/'gh.log').open('a') as f: f.write(json.dumps(args)+'\n')
mode=os.environ.get('FAKE_GH_MODE','ok')
if args[:2]==['auth','status']: sys.exit(1 if mode=='unauthenticated' else 0)
if args[0]=='api':
    if mode=='metadata-fails': sys.exit(1)
    print((root/'gh-release.json').read_text()); sys.exit(0)
if args[:2]==['release','download']:
    if mode=='download-fails': sys.exit(1)
    name=args[args.index('--pattern')+1]; dest=Path(args[args.index('--dir')+1])
    shutil.copyfile(root/'gh-assets'/name,dest/name); sys.exit(0)
if args[:2]==['attestation','verify']: sys.exit(1 if mode=='attestation-fails' else 0)
sys.exit(1)

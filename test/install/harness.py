"""Guard every Path write in the fixture harness; never follow a tool link for writing."""
from pathlib import Path

UNIX_TOOLS = ('bash','curl','tar','awk','sed','mktemp','find','uname','sysctl',
              'shasum','sha256sum','od','tr','cat','cp','mv','rm','rmdir','mkdir',
              'chmod','cmp','sort','grep','cut','tail','date','sleep','readlink',
              'wc','head','gzip','kill','basename','dirname')

_active_root = None
_original_text = Path.write_text
_original_bytes = Path.write_bytes


def assert_owned(path):
    if _active_root is None:
        raise AssertionError('no active isolated test root')
    resolved = Path(path).resolve()
    if resolved != _active_root and _active_root not in resolved.parents:
        raise AssertionError('harness write escapes isolated test root: ' + str(path))


def set_root(root):
    global _active_root
    _active_root = Path(root).resolve() if root is not None else None


def write_text(path, *args, **kwargs):
    assert_owned(path)
    return _original_text(path, *args, **kwargs)


def write_bytes(path, *args, **kwargs):
    assert_owned(path)
    return _original_bytes(path, *args, **kwargs)


Path.write_text = write_text
Path.write_bytes = write_bytes

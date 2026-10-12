"""Keep Cargo's cached fingerprints usable for unchanged checkout files.

The snapshot travels with the Rust build cache. Content and permission hashes
must match before restoring a source timestamp; changed/new files retain their
fresh checkout timestamps and therefore invalidate Cargo normally. Every test
still executes. This is not a cache of test results.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import subprocess


def source_path(root, name):
    path = PurePosixPath(name)
    if path.is_absolute() or '..' in path.parts or '\\' in name:
        raise ValueError('source path must stay inside the workspace')
    target = root / name
    target.resolve().relative_to(root.resolve())
    return target


def signature(path):
    with path.open('rb') as source:
        digest = hashlib.file_digest(source, 'sha256').hexdigest()
    return {'sha256': digest, 'mode': path.stat().st_mode & 0o777}


def snapshot(root, names):
    root = Path(root)
    result = {}
    for name in names:
        path = source_path(root, name)
        if path.is_file() and not path.is_symlink():
            result[name] = dict(signature(path), mtime_ns=path.stat().st_mtime_ns)
    return result


def restore(root, saved):
    root = Path(root)
    restored = 0
    for name, entry in saved.items():
        path = source_path(root, name)
        if not path.is_file() or path.is_symlink():
            continue
        actual = signature(path)
        if all(entry.get(key) == value for key, value in actual.items()):
            os.utime(path, ns=(path.stat().st_atime_ns, entry['mtime_ns']))
            restored += 1
    return restored


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('snapshot', nargs='?', default='.cache/ci-source-times.json')
    args = parser.parse_args()
    root = Path.cwd().resolve()
    saved = Path(args.snapshot)
    restored = restore(root, json.loads(saved.read_text())) if saved.is_file() else 0
    names = subprocess.check_output(['git', 'ls-files', '-z'], text=True).split('\0')
    current = snapshot(root, [name for name in names if name])
    saved.parent.mkdir(parents=True, exist_ok=True)
    saved.write_text(json.dumps(current, sort_keys=True) + '\n')
    print(f'Restored timestamps for {restored} byte-identical inputs; recorded {len(current)} files.')


if __name__ == '__main__':
    main()

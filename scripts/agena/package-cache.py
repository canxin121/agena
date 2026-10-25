#!/usr/bin/env python3
"""Content-based package reuse. A hit also verifies the archive and checksum."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile


def digest(path):
    value = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            value.update(block)
    return value.hexdigest()


def fingerprint(binary, web, version, target, recipe):
    assets = []
    for path in sorted(web.rglob('*')):
        relative = path.relative_to(web).as_posix()
        if path.is_symlink():
            assets.append((relative, 'symlink', os.readlink(path)))
        elif path.is_file():
            assets.append((relative, 'file', digest(path)))
        elif path.is_dir():
            assets.append((relative, 'directory'))
    if not (web / 'index.html').is_file():
        raise ValueError('web-dist/index.html is missing')
    inputs = {'schema': 1, 'binary': digest(binary), 'web': assets,
              'version': version, 'target': target,
              'recipe': digest(recipe), 'cache_recipe': digest(__file__)}
    return hashlib.sha256(json.dumps(inputs, sort_keys=True).encode()).hexdigest()


def checksum_matches(archive, sha):
    return Path(str(archive) + '.sha256').read_text().strip() == f'{sha}  {archive.name}'


def check(archive, key):
    try:
        previous = json.loads(Path(str(archive) + '.build.json').read_text())
        return (previous['input'] == key and previous['archive'] == digest(archive)
                and checksum_matches(archive, previous['archive']))
    except (OSError, ValueError, KeyError, TypeError):
        return False


def record(archive, key):
    sha = digest(archive)
    if not checksum_matches(archive, sha):
        raise ValueError('package checksum does not match the completed archive')
    marker = Path(str(archive) + '.build.json')
    fd, temporary = tempfile.mkstemp(prefix=marker.name + '.', dir=marker.parent)
    try:
        with os.fdopen(fd, 'w') as stream:
            json.dump({'input': key, 'archive': sha}, stream)
            stream.write('\n')
        os.replace(temporary, marker)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    key = commands.add_parser('fingerprint')
    key.add_argument('binary', type=Path)
    key.add_argument('web', type=Path)
    key.add_argument('version')
    key.add_argument('target')
    key.add_argument('recipe', type=Path)
    for name in ['check', 'record']:
        command = commands.add_parser(name)
        command.add_argument('archive', type=Path)
        command.add_argument('key')
    args = parser.parse_args()
    if args.command == 'fingerprint':
        print(fingerprint(args.binary, args.web, args.version, args.target, args.recipe))
    elif args.command == 'check':
        raise SystemExit(0 if check(args.archive, args.key) else 1)
    else:
        record(args.archive, args.key)


if __name__ == '__main__':
    main()

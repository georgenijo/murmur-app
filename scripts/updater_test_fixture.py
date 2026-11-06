#!/usr/bin/env python3
"""Serve an isolated signed OTA fixture. Never launches an app or reads any
production key. Requires minisign on PATH; generates a throwaway key per run.
Use only a synthetic com.localdictation.shutdown-test bundle built by the QA
operator. Output and private throwaway key remain under the requested cache dir.
"""
import argparse
import base64
import functools
import http.server
import json
from pathlib import Path
import plistlib
import shutil
import subprocess
import tarfile

IDENTIFIER = 'com.localdictation.shutdown-test'


def prepare(bundle: Path, directory: Path, port: int, failure: str = 'none') -> dict:
    bundle = bundle.resolve(strict=True)
    info = plistlib.loads((bundle / 'Contents/Info.plist').read_bytes())
    if info.get('CFBundleIdentifier') != IDENTIFIER or info.get('CFBundleExecutable') != 'ui':
        raise ValueError('Only a shutdown-test ui bundle can be served')
    version = info['CFBundleShortVersionString']
    # Require an operator-owned cache scratch directory, never /tmp or a daily app.
    directory = directory.expanduser().resolve()
    cache = (Path.home() / '.cache').resolve()
    if cache not in directory.parents or directory.exists():
        raise ValueError('Use a new directory beneath ~/.cache/')
    directory.mkdir(parents=True, mode=0o700)
    secrets = directory / 'throwaway-key'
    secrets.mkdir(mode=0o700)
    public = secrets / 'test.pub'
    private = secrets / 'test.key'
    subprocess.run(['minisign', '-G', '-W', '-p', str(public), '-s', str(private)], check=True, stdout=subprocess.DEVNULL)
    served = directory / 'feed'
    served.mkdir()
    archive = served / 'Murmur.app.tar.gz'
    with tarfile.open(archive, 'w:gz') as output:
        output.add(bundle, arcname='Murmur.app')
    if failure == 'archive':
        archive.write_bytes(b'Synthetic invalid archive')
    signature = directory / 'update.sig'
    subprocess.run(['minisign', '-S', '-s', str(private), '-m', str(archive), '-x', str(signature)], check=True, stdout=subprocess.DEVNULL)
    if failure == 'signature':
        with archive.open('ab') as output:
            output.write(b'Synthetic tamper')
    url = f'http://127.0.0.1:{port}'
    manifest = {'version': version, 'notes': 'Synthetic updater QA fixture', 'platforms': {
        'darwin-aarch64': {'url': url + '/Murmur.app.tar.gz', 'signature': base64.b64encode(signature.read_bytes()).decode('ascii')}
    }}
    (served / 'latest-v2.json').write_text(json.dumps(manifest))
    config = {'feed': url + '/latest-v2.json', 'publicKey': base64.b64encode(public.read_bytes()).decode('ascii')}
    (directory / 'fixture.json').write_text(json.dumps(config))
    # Erase the test private key immediately after signing. The server exposes
    # only feed/, never the config or its key-generation directory.
    shutil.rmtree(secrets)
    return config


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bundle', type=Path, required=True)
    parser.add_argument('--directory', type=Path, required=True)
    parser.add_argument('--port', type=int, default=18990)
    parser.add_argument('--failure', choices=['none', 'signature', 'archive'], default='none')
    args = parser.parse_args()
    prepare(args.bundle, args.directory, args.port, args.failure)
    print(f'Fixture: {args.directory / "fixture.json"}', flush=True)
    handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=str(args.directory / 'feed'))
    http.server.ThreadingHTTPServer(('127.0.0.1', args.port), handler).serve_forever()


if __name__ == '__main__':
    main()

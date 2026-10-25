"""Offline packaging contracts, including a real warm Cargo build when available."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import unittest

import test_installer_storage as installer_fixture

SCRIPTS = Path(__file__).resolve().parents[1]
HOST = 'x86_64-unknown-linux-gnu'
CROSS = 'aarch64-unknown-linux-gnu'
FAKE_CARGO = r'''#!/usr/bin/env python3
import json, os, pathlib, sys
args = sys.argv[1:]
root = pathlib.Path(os.environ['TEST_CHECKOUT'])
target_dir = pathlib.Path(os.environ.get('CARGO_TARGET_DIR', str(root / 'target')))
if args[0] == 'metadata':
    print(json.dumps({'packages': [{'name': 'agena', 'version': '0.1.0'}], 'target_directory': str(target_dir)}))
elif args[0] == 'build':
    with open(os.environ['TEST_CARGO_CALLS'], 'a') as f:
        f.write(json.dumps({'args': args, 'cwd': os.getcwd(), 'rustflags': os.environ.get('RUSTFLAGS'), 'encoded': os.environ.get('CARGO_ENCODED_RUSTFLAGS')}) + '\n')
    if os.environ.get('TEST_BUILD_FAIL'): sys.exit(42)
    triple = args[args.index('--target') + 1] if '--target' in args else ''
    triple = os.environ.get('TEST_ARTIFACT_TARGET', triple)
    binary = target_dir / triple / 'release/agena'
    binary.parent.mkdir(parents=True, exist_ok=True)
    binary.write_text(os.environ['TEST_BINARY'])
    binary.chmod(0o755)
    if not os.environ.get('TEST_NO_ARTIFACT'):
        print(json.dumps({'reason': 'compiler-artifact', 'target': {'name': 'agena', 'kind': ['bin']}, 'executable': str(binary), 'fresh': True}))
'''


class PackageBuildTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='agena-package-contract-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.checkout = self.root / 'checkout with spaces'
        self.scripts = self.checkout / 'scripts/agena'
        self.scripts.mkdir(parents=True)
        for name in ['package-backend.sh', 'package-cache.py', 'build-web.mjs']:
            shutil.copy2(SCRIPTS / name, self.scripts / name)
        (self.checkout / 'Cargo.toml').write_text('[workspace]\nmembers=[]\n')
        self.web = self.checkout / 'packages/agena-web'
        (self.web / 'dist').mkdir(parents=True)
        (self.web / 'package.json').write_text('{}\n')
        (self.web / 'dist/index.html').write_text('fixture web')
        self.bin = self.root / 'fake bin'
        self.bin.mkdir()
        (self.bin / 'cargo').write_text(FAKE_CARGO)
        (self.bin / 'cargo').chmod(0o755)
        (self.bin / 'rustc').write_text('#!/bin/sh\nprintf "%s\\n" "x86_64-unknown-linux-gnu"\n')
        (self.bin / 'rustc').chmod(0o755)
        self.calls = self.root / 'cargo calls.jsonl'
        self.env = dict(os.environ)
        for name in ['RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_BUILD_TARGET', 'CARGO_TARGET_DIR', 'AGENA_FORCE_EXPLICIT_TARGET']:
            self.env.pop(name, None)
        self.env.update(PATH=str(self.bin) + os.pathsep + os.environ['PATH'],
                        AGENA_WEB_DIST_PREBUILT='1', TEST_CHECKOUT=str(self.checkout),
                        TEST_CARGO_CALLS=str(self.calls), TEST_BINARY=installer_fixture.STUB,
                        TEST_SANDBOX_ROOT=str(self.root), TEST_CALLS_PATH=str(self.root / 'lifecycle.tsv'))

    def package(self, *args, env=None, success=True):
        result = subprocess.run(['bash', str(self.scripts / 'package-backend.sh'), *args],
                                cwd=self.root, env=self.env | (env or {}), text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=30)
        self.assertEqual(result.returncode == 0, success, result.stdout)
        return result

    def build_calls(self):
        return [json.loads(line) for line in self.calls.read_text().splitlines()]

    def archive(self, triple=HOST):
        return self.checkout / f'artifacts/agena/agena-backend-{triple}-v0.1.0.tar.gz'

    def test_host_shares_native_cache_and_reuses_verified_archive(self):
        stale = self.checkout / f'target/{HOST}/release/agena'
        stale.parent.mkdir(parents=True)
        stale.write_text('stale legacy binary')
        self.package(HOST)
        archive = self.archive()
        before = archive.stat().st_mtime_ns
        result = self.package(HOST)
        self.assertIn('reusing verified archive', result.stdout)
        self.assertEqual(before, archive.stat().st_mtime_ns)
        for call in self.build_calls():
            self.assertNotIn('--target', call['args'])
            self.assertEqual(call['cwd'], str(self.checkout))
            self.assertIsNone(call['rustflags'])
        with tarfile.open(archive) as package:
            self.assertEqual(package.extractfile('./bin/agena').read().decode(), installer_fixture.STUB)

    def test_target_directory_and_flags_match_ordinary_cargo(self):
        target = self.root / 'custom target directory'
        self.package(HOST, env={'CARGO_TARGET_DIR': str(target), 'RUSTFLAGS': '--cfg custom_build'})
        call = self.build_calls()[-1]
        self.assertEqual(call['args'][call['args'].index('--target-dir') + 1], str(target))
        self.assertEqual(call['rustflags'], '--cfg custom_build')
        self.assertNotIn('--target', call['args'])

    def test_cross_and_target_specific_flags_keep_explicit_target(self):
        self.package(CROSS)
        self.assertEqual(self.build_calls()[-1]['args'][self.build_calls()[-1]['args'].index('--target') + 1], CROSS)
        self.package(HOST, 'false', '-C target-cpu=generic', env={'RUSTFLAGS': '--cfg original'})
        call = self.build_calls()[-1]
        self.assertIn('--target', call['args'])
        self.assertEqual(call['rustflags'], '--cfg original -C target-cpu=generic')

    def test_encoded_flags_are_preserved_and_extended(self):
        encoded = '--cfg\x1fencoded'
        self.package(HOST, env={'CARGO_ENCODED_RUSTFLAGS': encoded})
        self.assertEqual(self.build_calls()[-1]['encoded'], encoded)
        self.assertIsNone(self.build_calls()[-1]['rustflags'])
        self.package(HOST, 'false', '-C target-cpu=generic', env={'CARGO_ENCODED_RUSTFLAGS': encoded})
        self.assertEqual(self.build_calls()[-1]['encoded'], encoded + '\x1f-C\x1ftarget-cpu=generic')

    def test_build_std_and_forced_target_remain_explicit(self):
        ci = self.checkout / 'scripts/ci'
        ci.mkdir()
        (ci / 'run-build-std-cargo.sh').write_text('#!/bin/bash\nshift\nexec cargo "$@"\n')
        self.package(HOST, 'true')
        self.assertIn('--target', self.build_calls()[-1]['args'])
        self.package(HOST, env={'AGENA_FORCE_EXPLICIT_TARGET': '1'})
        self.assertIn('--target', self.build_calls()[-1]['args'])

    def test_failure_or_wrong_artifact_never_uses_stale_binary(self):
        self.package(HOST, env={'TEST_BUILD_FAIL': '1'}, success=False)
        self.assertFalse(self.archive().exists())
        self.package(HOST, env={'TEST_NO_ARTIFACT': '1'}, success=False)
        self.assertFalse(self.archive().exists())
        self.package(HOST, env={'TEST_ARTIFACT_TARGET': CROSS}, success=False)
        self.package(CROSS, env={'TEST_ARTIFACT_TARGET': ''}, success=False)

    def test_package_corruption_and_input_changes_force_repackaging(self):
        self.package(HOST)
        self.archive().write_bytes(b'corrupt')
        result = self.package(HOST)
        self.assertNotIn('reusing verified archive', result.stdout)
        checksum = Path(str(self.archive()) + '.sha256')
        checksum.write_text('invalid checksum')
        self.assertNotIn('reusing verified archive', self.package(HOST).stdout)
        (self.web / 'dist/index.html').write_text('changed asset')
        self.assertNotIn('reusing verified archive', self.package(HOST).stdout)
        self.assertNotIn('reusing verified archive', self.package(HOST, env={'TEST_BINARY': installer_fixture.STUB + '\n# changed binary\n'}).stdout)

    def test_real_installer_reinstall_source_uses_same_native_cache(self):
        install = self.root / 'installed application'
        data = self.root / 'instance data'
        env = self.env | {'HOME': str(self.root / 'home')}
        for action in ['install', 'reinstall', 'reinstall']:
            command = ['bash', str(SCRIPTS / 'install.sh'), action, '--source', str(self.checkout),
                       '--install-dir', str(install), '--data-dir', str(data),
                       '--workspace', str(self.root / 'workspace'), '--service-mode', 'detached', '--no-path-update']
            result = subprocess.run(command, cwd=self.root, env=env, text=True,
                                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=30)
            self.assertEqual(result.returncode, 0, result.stdout)
            if action == 'reinstall': self.assertIn('reusing verified archive', result.stdout)
        self.assertEqual(len(self.build_calls()), 3)
        self.assertTrue(all('--target' not in call['args'] for call in self.build_calls()))

    def test_real_cargo_warm_cache_and_config_flags_survive_packaging(self):
        rustup = shutil.which('rustup')
        if not rustup: self.skipTest('rustup is unavailable')
        found = subprocess.run([rustup, 'which', '--toolchain', '1.97.0', 'cargo'], text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        if found.returncode: self.skipTest('pinned Rust toolchain is unavailable')
        real_cargo = found.stdout.strip()
        (self.checkout / 'src').mkdir()
        (self.checkout / 'src/main.rs').write_text('#![allow(unexpected_cfgs)]\n#[cfg(not(cache_fixture))]\ncompile_error!("Cargo config was suppressed");\nfn main() {}\n')
        (self.checkout / 'Cargo.toml').write_text('[package]\nname="agena"\nversion="0.1.0"\nedition="2024"\n')
        config = self.checkout / '.cargo'
        config.mkdir()
        (config / 'config.toml').write_text('[build]\nrustflags=["--cfg", "cache_fixture"]\ntarget-dir="custom build output"\n')
        env = self.env | {'PATH': os.environ['PATH'], 'RUSTUP_TOOLCHAIN': '1.97.0', 'RUSTC_WRAPPER': '', 'CARGO_NET_OFFLINE': 'true'}
        warm = subprocess.run([real_cargo, 'build', '--release'], cwd=self.checkout, env=env,
                              text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=30)
        self.assertEqual(warm.returncode, 0, warm.stdout)
        binary = self.checkout / 'custom build output/release/agena'
        stamp = binary.stat().st_mtime_ns
        for _ in range(2):
            result = subprocess.run(['bash', str(self.scripts / 'package-backend.sh')], cwd=self.root, env=env,
                                    text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=30)
            self.assertEqual(result.returncode, 0, result.stdout)
            self.assertNotIn('Compiling agena', result.stdout)
            self.assertEqual(binary.stat().st_mtime_ns, stamp)


if __name__ == '__main__':
    unittest.main()

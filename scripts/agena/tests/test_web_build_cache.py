"""Exercise frontend cache invalidation with a real, dependency-free Bun build."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parents[1]
BUN = shutil.which('bun')


@unittest.skipUnless(BUN, 'Bun is required for frontend build cache contracts')
class WebBuildCacheTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='agena-web-cache-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.project = self.root / 'web project'
        (self.project / 'src').mkdir(parents=True)
        (self.project / 'src/view.ts').write_text('first')
        (self.project / 'package.json').write_text(json.dumps({'name': 'cache-fixture', 'scripts': {'build': 'bun ./build.mjs'}}))
        self.source = '''import {readFileSync, writeFileSync, mkdirSync, existsSync} from 'node:fs';
const counter=process.env.TEST_COUNTER;
const n=existsSync(counter)?Number(readFileSync(counter,'utf8'))+1:1;
writeFileSync(counter,String(n)); mkdirSync('dist',{recursive:true});
writeFileSync('dist/index.html',String(n));
'''
        (self.project / 'build.mjs').write_text(self.source)
        self.counter = self.root / 'counter'
        self.cache = self.project / 'node_modules/.cache/agena-web-build.json'
        self.env = os.environ | {'TEST_COUNTER': str(self.counter)}

    def build(self, env=None, success=True):
        result = subprocess.run([BUN, str(SCRIPTS / 'build-web.mjs'), str(self.project)], env=self.env | (env or {}),
                                text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=30)
        self.assertEqual(result.returncode == 0, success, result.stdout)
        return result

    def count(self):
        return int(self.counter.read_text())

    def test_unchanged_and_touched_sources_reuse_successful_output(self):
        self.build()
        self.assertIn('reusing verified dist', self.build().stdout)
        os.utime(self.project / 'src/view.ts', None)
        self.build()
        self.assertEqual(self.count(), 1)

    def test_changed_added_deleted_and_linked_inputs_invalidate(self):
        self.build()
        source = self.project / 'src/view.ts'
        stamp = source.stat()
        source.write_text('other')
        os.utime(source, ns=(stamp.st_atime_ns, stamp.st_mtime_ns))
        self.build()
        self.assertEqual(self.count(), 2)
        added = self.project / 'src/new.ts'
        added.write_text('new')
        self.build()
        added.unlink()
        self.build()
        linked = self.root / 'linked source'
        linked.mkdir()
        (linked / 'entry.ts').write_text('linked')
        (self.project / 'src/linked').symlink_to(linked, target_is_directory=True)
        self.build()
        (linked / 'entry.ts').write_text('changed linked source')
        self.build()
        self.assertEqual(self.count(), 6)

    def test_corrupt_deleted_or_extra_outputs_trigger_a_real_build(self):
        self.build()
        (self.project / 'dist/index.html').write_text('corrupt')
        self.build()
        (self.project / 'dist/index.html').unlink()
        self.build()
        (self.project / 'dist/extra.js').write_text('unexpected')
        self.build()
        self.assertEqual(self.count(), 4)

    def test_environment_and_forced_build_invalidate(self):
        self.build()
        self.build({'VITE_FIXTURE': 'changed'})
        self.build({'VITE_FIXTURE': 'changed'})
        self.assertEqual(self.count(), 2)
        self.build({'VITE_FIXTURE': 'changed', 'AGENA_WEB_FORCE_BUILD': '1'})
        self.build({'VITE_FIXTURE': 'changed'})
        self.assertEqual(self.count(), 3)

    def test_failed_build_does_not_preserve_a_success_cache(self):
        self.build()
        (self.project / 'build.mjs').write_text('throw new Error("fixture failure");\n')
        self.build(success=False)
        self.assertFalse(self.cache.exists())
        (self.project / 'build.mjs').write_text(self.source)
        self.build()
        self.assertEqual(self.count(), 2)

    def test_corrupt_cache_record_rebuilds(self):
        self.build()
        self.cache.write_text('broken json')
        self.build()
        self.assertEqual(self.count(), 2)

    def test_only_root_build_directories_are_excluded_from_inputs(self):
        self.build()
        nested = self.project / 'src/.cache'
        nested.mkdir()
        (nested / 'authored.ts').write_text('an authored source file')
        self.build()
        self.assertEqual(self.count(), 2)
        (self.project / 'node_modules/irrelevant-cache').write_text('generated')
        self.build()
        self.assertEqual(self.count(), 2)
        (self.project / 'dist/unexpected.tsbuildinfo').write_text('output must be verified')
        self.build()
        self.assertEqual(self.count(), 3)


if __name__ == '__main__':
    unittest.main()

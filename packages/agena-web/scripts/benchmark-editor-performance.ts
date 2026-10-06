/** CPU-only comparison of the Git HEAD diff line-map path and the current index. */
import assert from 'node:assert/strict'
import { cpus } from 'node:os'
import { resolve } from 'node:path'
import ts from 'typescript'
import { createDisplayLineResolver } from '../src/components/editor/displayLineResolver'

const root = resolve(import.meta.dir, '../../..')
const baseline = Bun.spawnSync(['git', 'rev-parse', 'HEAD'], { cwd: root }).stdout.toString().trim()
const source = Bun.spawnSync(['git', 'show', `${baseline}:packages/agena-web/src/components/MonacoDiffEditor.vue`], {
  cwd: root,
})
if (source.exitCode) throw new Error(source.stderr.toString())
const script = source.stdout.toString().match(/<script setup lang="ts">([\s\S]*?)<\/script>/)![1]!
const ast = ts.createSourceFile('baseline.ts', script, ts.ScriptTarget.Latest, true)
const names = new Set([
  'clamp',
  'normalizePositiveInteger',
  'normalizeLineNumberMap',
  'resolveModelLineFromDisplayLine',
])
const functions = ast.statements.filter(
  (statement) => ts.isFunctionDeclaration(statement) && names.has(statement.name?.text ?? ''),
)
assert.equal(functions.length, names.size)
const moduleSource =
  functions.map((statement) => statement.getText(ast)).join('\n') +
  `
export { normalizeLineNumberMap };
export function locateHunks(lineCount, map, hunks) {
  const locate = (anchor) => resolveModelLineFromDisplayLine(anchor, lineCount, normalizeLineNumberMap(map, lineCount), null);
  return [...hunks].sort((a, b) => locate(a) - locate(b)).map(locate);
}`
const old = await import(
  `data:text/javascript;base64,${Buffer.from(new Bun.Transpiler({ loader: 'ts' }).transformSync(moduleSource)).toString('base64')}`
)
const samples = 3
let consumed: unknown
function median(run: () => unknown) {
  consumed = run()
  const times = Array.from({ length: samples }, () => {
    const started = performance.now()
    consumed = run()
    return performance.now() - started
  })
  return times.sort((a, b) => a - b)[1]!
}
const results = []
for (const [lineCount, hunkCount] of [
  [2_000, 200],
  [10_000, 300],
  [50_000, 500],
]) {
  const map = Array.from({ length: lineCount! }, (_, index) => (index % 7 === 0 ? null : 100_000 + index))
  const hunks = Array.from({ length: hunkCount! }, (_, index) => 100_000 + ((index * 7919) % lineCount!))
  const before = () => old.locateHunks(lineCount, map, hunks)
  const after = () => {
    const locate = createDisplayLineResolver(lineCount!, old.normalizeLineNumberMap(map, lineCount), null)
    return hunks.map((anchor) => locate(anchor)).sort((a, b) => a - b)
  }
  assert.deepEqual(after(), before())
  const before_ms = median(before),
    after_ms = median(after)
  const row = { lines: lineCount, hunks: hunkCount, before_ms, after_ms, speedup: before_ms / after_ms }
  results.push(row)
  console.log(JSON.stringify(row))
}
const report = {
  captured_at: new Date().toISOString(),
  baseline_commit: baseline,
  runtime: { bun: Bun.version, platform: process.platform, arch: process.arch, cpu: cpus()[0]?.model },
  methodology: {
    statistic: 'median',
    samples,
    warmups: 1,
    isolated_cpu_only: true,
    browser_layout_and_Monaco_diff_excluded: true,
  },
  results,
}
const output = process.argv.find((argument) => argument.startsWith('--output='))?.slice('--output='.length)
if (output) await Bun.write(resolve(output), JSON.stringify(report, null, 2) + '\n')
void consumed

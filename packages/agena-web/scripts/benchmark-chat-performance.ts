/** Run from agena-web: bun scripts/benchmark-chat-performance.ts --output=/path/results.json
 * Baselines are bundled from Git HEAD, so both versions run on the same machine
 * in the same process. These are CPU microbenchmarks, not browser FPS claims.
 */
import { cpus } from 'node:os'
import { resolve, relative } from 'node:path'
import { computed, reactive, ref } from 'vue'
import { projectTranscriptBlocks, createTranscriptProjector } from '../src/pages/chat/transcriptProjection'
import type { MessageLike } from '../src/components/chat/messageList.types'
import { SseFrames } from '../src/lib/sseFrames'
import { highlightCodeToHtml } from '../src/lib/highlight'
import { mergeActivityLog, type ActivityLog } from '../src/types/activity'

const root = resolve(import.meta.dir, '../../..')
const head = Bun.spawnSync(['git', 'rev-parse', 'HEAD'], { cwd: root }).stdout.toString().trim()
const samples = 9
let consumed: unknown

async function baselineModule(path: string, exposeVue = false) {
  const entry = resolve(root, path)
  const build = await Bun.build({
    entrypoints: [entry],
    target: 'bun',
    format: 'esm',
    plugins: [
      {
        name: 'source-at-head',
        setup(builder) {
          builder.onLoad({ filter: /\.[tj]s$/ }, ({ path: file }) => {
            if (!file.startsWith(`${root}/packages/agena-web/src/`)) return
            const result = Bun.spawnSync(['git', 'show', `${head}:${relative(root, file)}`], { cwd: root })
            if (result.exitCode) throw new Error(result.stderr.toString())
            return {
              loader: file.endsWith('.ts') ? 'ts' : 'js',
              contents:
                result.stdout.toString() +
                (exposeVue && file === entry
                  ? '\nexport { reactive as benchReactive, computed as benchComputed } from "vue";'
                  : ''),
            }
          })
        },
      },
    ],
  })
  if (!build.success) throw new Error(build.logs.map(String).join('\n'))
  return import(`data:text/javascript;base64,${Buffer.from(await build.outputs[0]!.text()).toString('base64')}`)
}

const old = await baselineModule('packages/agena-web/src/pages/chat/transcriptProjection.ts', true)
const oldHighlight = await baselineModule('packages/agena-web/src/lib/highlight.ts')
const oldActivity = await baselineModule('packages/agena-web/src/types/activity.ts')

function median(run: () => unknown) {
  for (let i = 0; i < 3; i++) consumed = run()
  const times: number[] = []
  for (let i = 0; i < samples; i++) {
    const before = performance.now()
    consumed = run()
    times.push(performance.now() - before)
  }
  return times.sort((a, b) => a - b)[Math.floor(times.length / 2)]!
}

const results: Array<{
  name: string
  input: Record<string, number | string>
  before_ms: number
  after_ms: number
  speedup: number
}> = []
function compare(name: string, input: Record<string, number | string>, before: () => unknown, after: () => unknown) {
  const before_ms = median(before)
  const after_ms = median(after)
  const row = { name, input, before_ms, after_ms, speedup: before_ms / after_ms }
  results.push(row)
  console.log(
    `${name} ${JSON.stringify(input)}: ${before_ms.toFixed(3)} → ${after_ms.toFixed(3)} ms (${row.speedup.toFixed(2)}x)`,
  )
}

const message = (id: number, role: string): MessageLike => ({
  info: { id: String(id), role },
  parts: [{ id: String(id + 1), agenaKind: 'text', text: `message ${id}` }],
})
for (const count of [500, 1000, 2000]) {
  const messages = Array.from({ length: count }, (_, i) => message(i * 2 + 1, 'assistant'))
  compare(
    'fold_consecutive_assistants',
    { messages: count },
    () => old.projectTranscriptBlocks(messages, { showReasoning: true }),
    () => projectTranscriptBlocks(messages, { showReasoning: true }),
  )
}
for (const count of [1000, 2000, 4000]) {
  const one = message(1, 'assistant')
  one.parts = Array.from({ length: count }, (_, i) => ({ id: String(i + 2), agenaKind: 'text', text: `fragment ${i}` }))
  one.parts.push({
    id: String(count + 2),
    agenaKind: 'tool_call',
    agenaContent: { name: 'shell.run', input: {}, call_id: 1 },
  })
  compare(
    'classify_answers_before_tool',
    { parts: count },
    () => old.projectTranscriptBlocks([one], { showReasoning: true }),
    () => projectTranscriptBlocks([one], { showReasoning: true }),
  )
}
for (const count of [500, 2000, 6000]) {
  const source = Array.from({ length: count }, (_, i) => message(i * 2 + 1, i % 2 ? 'assistant' : 'user'))
  const previous = old.benchReactive(structuredClone(source))
  const before = old.benchComputed(() => old.projectTranscriptBlocks(previous, { showReasoning: true }))
  const messages = reactive(structuredClone(source))
  const project = createTranscriptProjector(() => ({ showReasoning: true }))
  const after = computed(() => project(messages))
  consumed = before.value
  consumed = after.value
  compare(
    'stream_one_tail_preserving_history',
    { messages: count },
    () => {
      previous.at(-1).parts[0].text += 'x'
      return before.value
    },
    () => {
      messages.at(-1)!.parts[0]!.text += 'x'
      return after.value
    },
  )
}
for (const count of [500, 1000, 2000]) {
  const keys = Array.from({ length: count }, (_, i) => String(i))
  const hits = ref(
    keys.flatMap((key) => [
      { key, offset: 0 },
      { key, offset: 10 },
    ]),
  )
  const indexed = reactive(new Set(keys))
  compare(
    'search_row_membership',
    { rows: count, matches: count * 2 },
    () => keys.reduce((total, key) => total + Number(hits.value.some((hit) => hit.key === key)), 0),
    () => keys.reduce((total, key) => total + Number(indexed.has(key)), 0),
  )
}
const body = `data: ${'x'.repeat(2 * 1024 * 1024)}\n\n`
const chunks = Array.from({ length: Math.ceil(body.length / 4096) }, (_, i) => body.slice(i * 4096, (i + 1) * 4096))
compare(
  'SSE_frame_delimiters',
  { bytes: body.length, chunk_bytes: 4096 },
  () => {
    let buffer = ''
    const frames: string[] = []
    for (const chunk of chunks) {
      buffer += chunk.replace(/\r\n/g, '\n').replace(/\r/g, '\n')
      const split = buffer.split('\n\n')
      buffer = split.pop() ?? ''
      frames.push(...split)
    }
    return frames
  },
  () => {
    const frames = new SseFrames()
    return chunks.flatMap((chunk) => frames.push(chunk))
  },
)
const code = 'const result = items.map(item => item.value);\n'.repeat(2000)
let serial = 0
compare(
  'large_code_explicit_language',
  { characters: code.length, language: 'javascript', policy: 'plain escaped source above 16 KiB' },
  () => oldHighlight.highlightCodeToHtml(code + serial++, 'javascript'),
  () => highlightCodeToHtml(code + serial++, 'javascript'),
)
const log: ActivityLog = {
  activity_id: 'task_1',
  status: 'running',
  lines: [{ seq: 1, stream: 'message', text: '中😀'.repeat(400_000) }],
  last_seq: 1,
  has_more: false,
  dropped_lines: 0,
}
compare(
  'activity_log_large_tail',
  { source_characters: log.lines[0]!.text.length, budget_bytes: 128 * 1024 },
  () => oldActivity.mergeActivityLog(null, log),
  () => mergeActivityLog(null, log),
)

const report = {
  captured_at: new Date().toISOString(),
  baseline_commit: head,
  runtime: { bun: Bun.version, platform: process.platform, arch: process.arch, cpu: cpus()[0]?.model },
  methodology: {
    statistic: 'median',
    samples,
    warmups: 3,
    isolated_cpu_only: true,
    JSON_parsing_and_browser_layout_excluded: true,
  },
  results,
}
const output = process.argv.find((arg) => arg.startsWith('--output='))?.slice('--output='.length)
if (output) await Bun.write(resolve(output), JSON.stringify(report, null, 2) + '\n')
void consumed

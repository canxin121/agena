// Reuse only a successful build whose sources and complete output still match.
import { createHash } from 'node:crypto'
import { existsSync, mkdirSync, readFileSync, readdirSync, readlinkSync, realpathSync, renameSync, rmSync, statSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const recipe = fileURLToPath(import.meta.url)
const ignored = new Set(['node_modules', 'dist', 'dist-rust-debug', '.git', '.cache', '.DS_Store', '.prettier-cache'])

function treeDigest(root, source = false) {
  const hash = createHash('sha256')
  function walk(directory, ancestors = new Set()) {
    const real = realpathSync(directory)
    if (ancestors.has(real)) throw new Error(`Directory cycle in frontend inputs: ${directory}`)
    const parents = new Set([...ancestors, real])
    for (const entry of readdirSync(directory, { withFileTypes: true }).sort((a, b) => a.name < b.name ? -1 : a.name > b.name ? 1 : 0)) {
      if (source && directory === root && (ignored.has(entry.name) || entry.name.endsWith('.tsbuildinfo'))) continue
      const filename = path.join(directory, entry.name)
      if (entry.isSymbolicLink()) hash.update(readlinkSync(filename)).update('\0')
      if (entry.isDirectory() || (entry.isSymbolicLink() && statSync(filename).isDirectory())) walk(filename, parents)
      else {
        const relative = path.relative(root, filename).split(path.sep).join('/')
        hash.update(relative).update('\0').update(readFileSync(filename)).update('\0')
      }
    }
  }
  walk(root)
  return hash.digest('hex')
}

export function buildWeb(project) {
  project = path.resolve(project)
  const dist = path.join(project, 'dist')
  const cache = path.join(project, 'node_modules/.cache/agena-web-build.json')
  const bun = process.execPath
  const inputs = () => {
    const environment = Object.fromEntries(Object.entries(process.env)
      .filter(([key]) => /^(VITE_|NODE_|BUN_|BROWSERSLIST)/.test(key) || ['TZ', 'SOURCE_DATE_EPOCH'].includes(key))
      .sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0))
    return createHash('sha256').update(JSON.stringify({
      schema: 1, project, bun: Bun.version, platform: process.platform, arch: process.arch, environment,
      source: treeDigest(project, true),
    })).update(readFileSync(recipe)).digest('hex')
  }
  const output = () => {
    if (!existsSync(path.join(dist, 'index.html'))) return null
    return treeDigest(dist)
  }
  const started = Date.now()
  const initial = inputs()
  if (process.env.AGENA_WEB_FORCE_BUILD !== '1') {
    try {
      const previous = JSON.parse(readFileSync(cache, 'utf8'))
      if (previous.input === initial && previous.output === output()) {
        console.log('Web frontend unchanged; reusing verified dist/.')
        return
      }
    } catch {
      // A missing, corrupt or incomplete cache always causes a real build.
    }
  }
  rmSync(cache, { force: true })
  function run(args) {
    const result = spawnSync(bun, args, { cwd: project, env: process.env, stdio: 'inherit' })
    if (result.error) throw result.error
    if (result.status !== 0) throw new Error(`bun ${args.join(' ')} failed (${result.status ?? result.signal})`)
  }
  console.log('Building Web frontend...')
  run(['install', '--frozen-lockfile'])
  const before = inputs()
  run(['run', 'build'])
  const result = output()
  if (!result || inputs() !== before) throw new Error('Frontend inputs changed during build or dist/index.html is missing; rerun the build.')
  mkdirSync(path.dirname(cache), { recursive: true })
  const temporary = `${cache}.${process.pid}.tmp`
  writeFileSync(temporary, JSON.stringify({ input: before, output: result }) + '\n')
  renameSync(temporary, cache)
  console.log(`Web frontend ready in ${((Date.now() - started) / 1000).toFixed(1)}s.`)
}

if (import.meta.main) {
  try {
    buildWeb(process.argv[2])
  } catch (error) {
    console.error(`ERROR: ${error.message}`)
    process.exitCode = 1
  }
}

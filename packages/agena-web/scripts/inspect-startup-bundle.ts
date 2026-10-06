/** Inspect emitted static imports, without treating async route chunks as startup JS. */
import { readFileSync, existsSync } from 'node:fs'
import { resolve, dirname, relative } from 'node:path'
import { gzipSync } from 'node:zlib'
import ts from 'typescript'

const root = resolve(import.meta.dir, '..')
const dist = resolve(root, 'dist')
const html = readFileSync(resolve(dist, 'index.html'), 'utf8')
const entry = html.match(/<script\s[^>]*type="module"[^>]*src="([^"]+)"/)?.[1]
if (!entry) throw new Error('The build has no module entry')
const visited = new Set<string>()
const modules: Array<{ path: string; bytes: number; gzipBytes: number; iconComponents: number }> = []
function visit(path: string) {
  if (visited.has(path)) return
  visited.add(path)
  const input = readFileSync(path)
  const source = ts.createSourceFile(path, input.toString(), ts.ScriptTarget.Latest, true, ts.ScriptKind.JS)
  let icons = 0
  const countIcons = (node: ts.Node) => {
    if (ts.isPropertyAssignment(node) && node.name.getText(source) === 'name') {
      const value = node.initializer
      if ((ts.isStringLiteral(value) || ts.isNoSubstitutionTemplateLiteral(value)) && /^Ri[A-Z0-9]/.test(value.text))
        icons++
    }
    ts.forEachChild(node, countIcons)
  }
  countIcons(source)
  modules.push({
    path: relative(dist, path),
    bytes: input.length,
    gzipBytes: gzipSync(input).length,
    iconComponents: icons,
  })
  for (const statement of source.statements) {
    if (!ts.isImportDeclaration(statement) && !ts.isExportDeclaration(statement)) continue
    if (!statement.moduleSpecifier || !ts.isStringLiteral(statement.moduleSpecifier)) continue
    const module = statement.moduleSpecifier.text
    if (!module.endsWith('.js')) continue
    visit(module.startsWith('/') ? resolve(dist, module.slice(1)) : resolve(dirname(path), module))
  }
}
visit(resolve(dist, entry.replace(/^\//, '')))
const snapshot = {
  captured_at: new Date().toISOString(),
  initial_static_js_bytes: modules.reduce((sum, module) => sum + module.bytes, 0),
  initial_static_js_gzip_bytes: modules.reduce((sum, module) => sum + module.gzipBytes, 0),
  initial_icon_components: modules.reduce((sum, module) => sum + module.iconComponents, 0),
  initial_static_js_modules: modules.length,
  modules,
}
const label = process.argv.find((argument) => argument.startsWith('--label='))?.slice('--label='.length) || 'current'
const output = process.argv.find((argument) => argument.startsWith('--output='))?.slice('--output='.length)
if (output) {
  const path = resolve(output)
  const report = existsSync(path)
    ? JSON.parse(readFileSync(path, 'utf8'))
    : {
        methodology: {
          emitted_entry_static_import_graph: true,
          dynamic_routes_css_fonts_and_worker_assets_excluded: true,
          gzip_each_module_independently: true,
          browser_timings_measured: false,
        },
      }
  report[label] = snapshot
  await Bun.write(path, JSON.stringify(report, null, 2) + '\n')
}
console.log(JSON.stringify({ label, ...snapshot, modules: undefined }, null, 2))

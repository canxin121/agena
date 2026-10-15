import MarkdownIt from 'markdown-it'

export type AppService = {
  name: string
  command: string
  args?: string[]
  cwd?: string
  port?: number
  healthPath?: string
}
export type AppManifest = {
  id: string
  title: string
  description?: string
  html: string
  height?: number
  services?: AppService[]
}
export type ChartSpec = {
  type: 'line' | 'area' | 'bar' | 'scatter' | 'pie'
  title?: string
  xLabel?: string
  yLabel?: string
  labels: string[]
  series: { name: string; values: (number | null)[] }[]
}
export type ReplySegment =
  | { kind: 'markdown'; key: string; content: string; error?: string }
  | { kind: 'app'; key: string; manifest: AppManifest }
  | { kind: 'chart'; key: string; spec: ChartSpec; source: string }

const parser = new MarkdownIt({ html: false })
const idPattern = /^[a-zA-Z0-9_-]{1,80}$/
const record = (value: unknown): value is Record<string, unknown> =>
  !!value && typeof value === 'object' && !Array.isArray(value)
const text = (value: unknown, max: number): value is string => typeof value === 'string' && value.length <= max

export function parseAppManifest(raw: unknown): AppManifest {
  if (
    !record(raw) ||
    typeof raw.id !== 'string' ||
    !idPattern.test(raw.id) ||
    !text(raw.title, 200) ||
    !raw.title.trim() ||
    !text(raw.html, 512 * 1024) ||
    !raw.html ||
    (raw.description !== undefined && !text(raw.description, 2000)) ||
    (raw.height !== undefined &&
      (!Number.isInteger(raw.height) || Number(raw.height) < 160 || Number(raw.height) > 1200))
  ) {
    throw new Error(
      'Invalid app: use id, title, html, optional description, height (160–1200), and up to four services.',
    )
  }
  if (raw.services !== undefined && (!Array.isArray(raw.services) || raw.services.length > 4))
    throw new Error('An app supports up to four services.')
  const names = new Set<string>()
  for (const service of (raw.services || []) as unknown[]) {
    if (
      !record(service) ||
      typeof service.name !== 'string' ||
      !idPattern.test(service.name) ||
      names.has(service.name) ||
      !text(service.command, 4096) ||
      !service.command.trim() ||
      service.command.includes('\0') ||
      (service.cwd !== undefined && !text(service.cwd, 4096)) ||
      (service.port !== undefined &&
        (!Number.isInteger(service.port) || Number(service.port) < 0 || Number(service.port) > 65535)) ||
      (service.args !== undefined &&
        (!Array.isArray(service.args) ||
          service.args.length > 128 ||
          service.args.some((a) => !text(a, 16384) || a.includes('\0')))) ||
      (service.healthPath !== undefined &&
        (typeof service.healthPath !== 'string' || !isServicePath(service.healthPath)))
    ) {
      throw new Error('Invalid app service: use a unique name, command, argv array, workspace cwd, and loopback port.')
    }
    names.add(service.name)
  }
  return raw as AppManifest
}

export function parseChartSpec(raw: unknown): ChartSpec {
  if (
    !record(raw) ||
    !['line', 'area', 'bar', 'scatter', 'pie'].includes(String(raw.type)) ||
    !Array.isArray(raw.labels) ||
    !raw.labels.length ||
    raw.labels.length > 512 ||
    raw.labels.some((v) => !text(v, 100)) ||
    !Array.isArray(raw.series) ||
    !raw.series.length ||
    raw.series.length > 8 ||
    ['title', 'xLabel', 'yLabel'].some((k) => raw[k] !== undefined && !text(raw[k], 200))
  )
    throw new Error('Invalid chart schema or size limits exceeded.')
  for (const series of raw.series) {
    if (
      !record(series) ||
      !text(series.name, 100) ||
      !Array.isArray(series.values) ||
      series.values.length !== raw.labels.length ||
      series.values.some((v) => v !== null && (typeof v !== 'number' || !Number.isFinite(v) || Math.abs(v) > 1e100))
    )
      throw new Error('Chart series must match labels and contain finite numbers or null.')
  }
  if (!raw.series.some((s) => (s as { values: unknown[] }).values.some((v) => v !== null)))
    throw new Error('Chart has no numeric data.')
  if (
    raw.type === 'pie' &&
    (raw.series.length !== 1 ||
      raw.series[0].values.some((v: number | null) => v !== null && v < 0) ||
      !raw.series[0].values.some((v: number | null) => v !== null && v > 0))
  )
    throw new Error('Pie charts need one nonnegative series with a positive total.')
  return raw as ChartSpec
}

/** Only top-level, explicitly closed CommonMark fences become live components.
 * A longer outer fence, a quoted example, and an unfinished streaming fence stay source. */
export function splitReplySegments(content: string, allowApps = false): ReplySegment[] {
  if (!/^[ \t]{0,3}(?:`{3,}|~{3,})(?:agena-chart|agena-ui)\s*$/m.test(content))
    return [{ kind: 'markdown', key: 'md:0', content }]
  const lines = content.split('\n')
  const offsets = [0]
  for (const line of lines) offsets.push(offsets[offsets.length - 1]! + line.length + 1)
  const segments: ReplySegment[] = []
  let previous = 0
  const seen = new Set<string>()
  for (const token of parser.parse(content, {})) {
    if (token.type !== 'fence' || token.level !== 0 || !token.map) continue
    const language = token.info.trim()
    if (language !== 'agena-chart' && (language !== 'agena-ui' || !allowApps)) continue
    const [from, to] = token.map
    const closing = lines[to - 1]?.trim() || ''
    const marker = token.markup[0]!
    if (to - from < 2 || closing.length < token.markup.length || [...closing].some((c) => c !== marker)) continue
    const start = offsets[from]!
    const end = Math.min(offsets[to]!, content.length)
    if (start > previous)
      segments.push({ kind: 'markdown', key: `md:${previous}`, content: content.slice(previous, start) })
    try {
      const raw: unknown = JSON.parse(token.content)
      if (language === 'agena-ui') {
        const manifest = parseAppManifest(raw)
        if (seen.has(manifest.id)) throw new Error('Duplicate app id in this reply; use a unique id per app.')
        seen.add(manifest.id)
        segments.push({ kind: 'app', key: `app:${manifest.id}`, manifest })
      } else segments.push({ kind: 'chart', key: `chart:${start}`, spec: parseChartSpec(raw), source: token.content })
    } catch (error) {
      segments.push({
        kind: 'markdown',
        key: `md:${start}`,
        content: content.slice(start, end),
        error: String(error instanceof Error ? error.message : error),
      })
    }
    previous = end
  }
  if (previous < content.length || !segments.length)
    segments.push({ kind: 'markdown', key: `md:${previous}`, content: content.slice(previous) })
  return segments
}

/** Resolve within exactly one declared preview service; no URL/proxy escapes. */
export function isServicePath(path: string): boolean {
  if (
    !path.startsWith('/') ||
    path.startsWith('//') ||
    path.includes('\\') ||
    path.includes('#') ||
    /[\u0000-\u001f\u007f]/.test(path)
  )
    return false
  try {
    return path
      .split('?')[0]!
      .split('/')
      .every((segment) => {
        const decoded = decodeURIComponent(segment)
        return decoded !== '.' && decoded !== '..' && !/[\\/%\u0000-\u001f\u007f]/.test(decoded)
      })
  } catch {
    return false
  }
}

export function serviceRequestUrl(proxyBase: string, path: string): string {
  if (!/^\/api\/v1\/workbench\/preview\/s\/[A-Za-z0-9_-]+\/$/.test(proxyBase) || !isServicePath(path))
    throw new Error('Request must stay inside a declared app service.')
  return proxyBase + path.slice(1)
}

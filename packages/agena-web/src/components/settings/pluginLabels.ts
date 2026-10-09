import { settingsText as st } from '@/i18n/settingsText'

/**
 * Localized label for a plugin transport kind.
 *
 * The host reports a machine value and the panel shows a human label, so a
 * translated interface reads `Built-in`/`External process` instead of the raw
 * `static`/`stdio`. A value the catalog does not know yet still shows verbatim
 * instead of an invented label.
 */
export function pluginKindLabel(kind: string | null | undefined): string {
  const raw = String(kind ?? '').trim()
  switch (raw.toLowerCase()) {
    case 'static':
    case 'builtin':
    case 'embedded':
      return st('Built-in')
    case 'stdio':
    case 'process':
      return st('External process')
    case 'mcp':
      return st('MCP server')
    case 'wasm':
      return st('WebAssembly')
    case 'cdylib':
      return st('Native library')
    case 'http':
      return st('HTTP endpoint')
    default:
      return raw
  }
}

/** Localized label for a plugin lifecycle state; unknown values stay verbatim. */
export function pluginStateLabel(state: string | null | undefined): string {
  const raw = String(state ?? '').trim()
  switch (raw.toLowerCase()) {
    case 'running':
      return st('Running')
    case 'restarting':
      return st('Restarting')
    case 'failed':
      return st('Failed')
    case 'stopped':
      return st('Stopped')
    default:
      return raw
  }
}

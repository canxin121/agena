function extname(path: string): string {
  const base = path.split('/').pop() || path
  const idx = base.lastIndexOf('.')
  return idx >= 0 ? base.slice(idx + 1).toLowerCase() : ''
}

export function languageByPath(path?: string | null): string {
  if (!path) return 'plaintext'
  const ext = extname(path)
  if (ext === 'md' || ext === 'markdown' || ext === 'mdx') return 'markdown'
  if (ext === 'ts' || ext === 'tsx' || ext === 'mts' || ext === 'cts') return 'typescript'
  if (ext === 'js' || ext === 'jsx' || ext === 'mjs' || ext === 'cjs') return 'javascript'
  if (ext === 'json' || ext === 'jsonc' || ext === 'json5') return 'json'
  if (ext === 'css' || ext === 'scss' || ext === 'less') return ext
  if (ext === 'html' || ext === 'htm') return 'html'
  if (ext === 'py') return 'python'
  if (ext === 'rs') return 'rust'
  if (ext === 'yaml' || ext === 'yml') return 'yaml'
  if (ext === 'sql' || ext === 'psql' || ext === 'plsql') return 'sql'
  if (ext === 'sh' || ext === 'bash' || ext === 'zsh') return 'shell'
  if (ext === 'xml') return 'xml'
  if (ext === 'ini' || ext === 'conf' || ext === 'config' || ext === 'env') return 'ini'
  return 'plaintext'
}

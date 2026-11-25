import { RiCodeLine, RiFileImageLine, RiFileTextLine } from '@remixicon/vue'

export const DEFAULT_IGNORED_DIR_NAMES = new Set(['node_modules'])

export const MAX_VIEW_CHARS = 200_000

export const CODE_EXTENSIONS = new Set([
  'js',
  'jsx',
  'ts',
  'tsx',
  'mjs',
  'cjs',
  'mts',
  'cts',
  'html',
  'htm',
  'xhtml',
  'css',
  'scss',
  'sass',
  'less',
  'styl',
  'stylus',
  'vue',
  'svelte',
  'astro',
  'sh',
  'bash',
  'zsh',
  'fish',
  'ps1',
  'psm1',
  'bat',
  'cmd',
  'py',
  'pyw',
  'pyx',
  'pxd',
  'pxi',
  'rb',
  'erb',
  'rake',
  'gemspec',
  'php',
  'phtml',
  'php3',
  'php4',
  'php5',
  'phps',
  'java',
  'kt',
  'kts',
  'scala',
  'sc',
  'groovy',
  'gradle',
  'c',
  'h',
  'cpp',
  'cc',
  'cxx',
  'hpp',
  'hxx',
  'hh',
  'm',
  'mm',
  'cs',
  'fs',
  'fsx',
  'fsi',
  'go',
  'rs',
  'swift',
  'dart',
  'lua',
  'pl',
  'pm',
  'pod',
  'r',
  'R',
  'rmd',
  'jl',
  'hs',
  'lhs',
  'ex',
  'exs',
  'erl',
  'hrl',
  'clj',
  'cljs',
  'cljc',
  'edn',
  'lisp',
  'cl',
  'el',
  'scm',
  'ss',
  'rkt',
  'ml',
  'mli',
  're',
  'rei',
  'nim',
  'zig',
  'v',
  'cr',
  'main.kts',
  'sql',
  'psql',
  'plsql',
  'graphql',
  'gql',
  'sol',
  'asm',
  's',
  'S',
  'mk',
  'nix',
  'tf',
  'tfvars',
  'pp',
  'ansible',
])

export const DATA_EXTENSIONS = new Set([
  'json',
  'jsonc',
  'json5',
  'jsonl',
  'ndjson',
  'geojson',
  'yaml',
  'yml',
  'toml',
  'xml',
  'xsl',
  'xslt',
  'xsd',
  'dtd',
  'plist',
  'ini',
  'cfg',
  'conf',
  'config',
  'env',
  'properties',
  'csv',
  'tsv',
  'lock',
])

export const IMAGE_EXTENSIONS = new Set([
  'png',
  'jpg',
  'jpeg',
  'gif',
  'svg',
  'webp',
  'ico',
  'icns',
  'bmp',
  'tiff',
  'tif',
  'psd',
  'ai',
  'eps',
  'raw',
  'cr2',
  'nef',
  'heic',
  'heif',
  'avif',
  'jxl',
])

export const DOCUMENT_EXTENSIONS = new Set([
  'md',
  'mdx',
  'markdown',
  'mdown',
  'mkd',
  'txt',
  'text',
  'rtf',
  'doc',
  'docx',
  'odt',
  'pdf',
  'rst',
  'adoc',
  'asciidoc',
  'org',
  'tex',
  'latex',
  'bib',
])

export const MARKDOWN_EXTENSIONS = new Set(['md', 'mdx', 'markdown', 'mdown', 'mkd'])
const MERMAID_EXTENSIONS = new Set(['mermaid', 'mmd'])

export const PDF_EXTENSIONS = new Set(['pdf'])

export const AUDIO_EXTENSIONS = new Set(['mp3', 'wav', 'ogg', 'oga', 'm4a', 'aac', 'flac', 'opus', 'weba'])

export const VIDEO_EXTENSIONS = new Set(['mp4', 'webm', 'ogv', 'mov', 'm4v', 'mkv'])

export type PreviewMode = 'text' | 'markdown' | 'image' | 'pdf' | 'audio' | 'video'

export function extensionFromPath(path: string): string {
  const base = path.split(/[\\/]/).pop() || ''
  const idx = base.lastIndexOf('.')
  return idx >= 0 ? base.slice(idx + 1).toLowerCase() : ''
}

export function isHiddenName(name: string): boolean {
  return name.startsWith('.')
}

export function shouldIgnoreEntryName(name: string): boolean {
  return DEFAULT_IGNORED_DIR_NAMES.has(name)
}

export function shouldIgnorePath(path: string): boolean {
  const normalized = path.replace(/\\/g, '/')
  return normalized === 'node_modules' || normalized.endsWith('/node_modules') || normalized.includes('/node_modules/')
}

export function isImagePath(path: string): boolean {
  return IMAGE_EXTENSIONS.has(extensionFromPath(path))
}

export function isMermaidPath(path: string): boolean {
  return MERMAID_EXTENSIONS.has(extensionFromPath(path))
}

export function detectPreviewMode(path: string): PreviewMode {
  const ext = extensionFromPath(path)
  if (IMAGE_EXTENSIONS.has(ext)) return 'image'
  if (PDF_EXTENSIONS.has(ext)) return 'pdf'
  if (AUDIO_EXTENSIONS.has(ext)) return 'audio'
  if (VIDEO_EXTENSIONS.has(ext)) return 'video'
  if (MARKDOWN_EXTENSIONS.has(ext) || MERMAID_EXTENSIONS.has(ext)) return 'markdown'
  return 'text'
}

export function fileIconComponent(ext?: string) {
  const e = (ext || '').toLowerCase()
  if (e && CODE_EXTENSIONS.has(e)) return RiCodeLine
  if (e && DATA_EXTENSIONS.has(e)) return RiCodeLine
  if (e && IMAGE_EXTENSIONS.has(e)) return RiFileImageLine
  if (e && DOCUMENT_EXTENSIONS.has(e)) return RiFileTextLine
  return RiFileTextLine
}

export function fileIconClass(ext?: string): string {
  const e = (ext || '').toLowerCase()
  if (e && CODE_EXTENSIONS.has(e)) return 'text-blue-500'
  if (e && DATA_EXTENSIONS.has(e)) return 'text-amber-500'
  if (e && IMAGE_EXTENSIONS.has(e)) return 'text-emerald-500'
  if (e && DOCUMENT_EXTENSIONS.has(e)) return 'text-muted-foreground'
  return 'text-muted-foreground'
}

export function languageForPath(path: string): string {
  const ext = extensionFromPath(path)
  return ext || 'text'
}

export function truncateContent(content: string): string {
  if (content.length <= MAX_VIEW_CHARS) return content
  return `${content.slice(0, MAX_VIEW_CHARS)}\n\n... truncated ...`
}

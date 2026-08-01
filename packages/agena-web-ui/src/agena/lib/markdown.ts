/** Small, dependency-free Markdown renderer for activity output.
 *
 * Activity content is produced by models and tools, so it must never be
 * inserted as raw HTML. This renderer supports the Markdown constructs that
 * are useful in a transcript while escaping every source character first.
 */

export function escapeHtml(value: string): string {
  return value
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;')
}

function inlineMarkdown(value: string): string {
  let output = escapeHtml(value)
  output = output.replace(/`([^`\n]+)`/g, '<code>$1</code>')
  output = output.replace(/\*\*([^*\n]+)\*\*/g, '<strong>$1</strong>')
  output = output.replace(/__([^_\n]+)__/g, '<strong>$1</strong>')
  output = output.replace(/(?<!\*)\*([^*\n]+)\*(?!\*)/g, '<em>$1</em>')
  output = output.replace(/(?<!_)_([^_\n]+)_(?!_)/g, '<em>$1</em>')
  output = output.replace(
    /\[([^\]]+)\]\((https?:\/\/[^\s)]+|mailto:[^\s)]+)\)/g,
    '<a href="$2" target="_blank" rel="noreferrer noopener">$1</a>',
  )
  return output
}

function safeLanguage(value: string | undefined): string {
  const language = (value || '')
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9_+-]/g, '')
  return language ? ` language-${language}` : ''
}

export function renderCodeBlock(value: string, language?: string, className = 'markdown-code'): string {
  const languageClass = safeLanguage(language).trim()
  const codeClass = languageClass ? ` class="${languageClass}"` : ''
  return `<pre class="${className}"><code${codeClass}>${escapeHtml(value)}</code></pre>`
}

export function renderDiff(value: string): string {
  const lines = escapeHtml(value)
    .split('\n')
    .map((line) => {
      const className =
        line.startsWith('+++') || line.startsWith('---')
          ? 'diff-header'
          : line.startsWith('+')
            ? 'diff-added'
            : line.startsWith('-')
              ? 'diff-removed'
              : line.startsWith('@@')
                ? 'diff-hunk'
                : ''
      return className ? `<span class="${className}">${line || ' '}</span>` : line || ' '
    })
    .join('\n')
  return `<pre class="diff-output"><code class="language-diff">${lines}</code></pre>`
}

export function renderTerminal(value: string, language = 'text'): string {
  return renderCodeBlock(value, language, 'terminal-output')
}

function tableCells(value: string): string[] | null {
  if (!value.includes('|')) return null
  let normalized = value.trim()
  if (normalized.startsWith('|')) normalized = normalized.slice(1)
  if (normalized.endsWith('|')) normalized = normalized.slice(0, -1)
  const cells = normalized.split('|').map((cell) => cell.trim())
  return cells.length >= 2 ? cells : null
}

function isTableSeparator(value: string): boolean {
  const cells = tableCells(value)
  return Boolean(cells?.length && cells.every((cell) => /^:?-{3,}:?$/.test(cell)))
}

function renderTable(header: string[], rows: string[][]): string {
  const renderRow = (cells: string[], tag: 'th' | 'td') =>
    `<tr>${header.map((_, index) => `<${tag}>${inlineMarkdown(cells[index] || '')}</${tag}>`).join('')}</tr>`
  return `<div class="markdown-table-wrap"><table class="markdown-table"><thead>${renderRow(header, 'th')}</thead><tbody>${rows
    .map((row) => renderRow(row, 'td'))
    .join('')}</tbody></table></div>`
}

export function renderMarkdown(value: string): string {
  const lines = value.replaceAll('\r\n', '\n').split('\n')
  const output: string[] = []
  let paragraph: string[] = []
  let listType: 'ul' | 'ol' | null = null
  let listItems: string[] = []
  let inFence = false
  let fenceLanguage = ''
  let fenceLines: string[] = []

  const flushParagraph = () => {
    if (!paragraph.length) return
    output.push(`<p>${inlineMarkdown(paragraph.join('\n')).replaceAll('\n', '<br>')}</p>`)
    paragraph = []
  }
  const flushList = () => {
    if (!listType || !listItems.length) return
    output.push(`<${listType}>${listItems.map((item) => `<li>${item}</li>`).join('')}</${listType}>`)
    listType = null
    listItems = []
  }

  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index] ?? ''
    const fence = line.match(/^\s*```\s*([\w+.-]*)\s*$/)
    if (fence) {
      if (!inFence) {
        flushParagraph()
        flushList()
        inFence = true
        fenceLanguage = fence[1] || ''
        fenceLines = []
      } else {
        output.push(renderCodeBlock(fenceLines.join('\n'), fenceLanguage))
        inFence = false
        fenceLanguage = ''
        fenceLines = []
      }
      continue
    }
    if (inFence) {
      fenceLines.push(line)
      continue
    }

    const headerCells = tableCells(line)
    const nextLine = lines[index + 1] ?? ''
    if (headerCells && index + 1 < lines.length && isTableSeparator(nextLine)) {
      flushParagraph()
      flushList()
      const rows: string[][] = []
      index += 2
      while (index < lines.length) {
        const rowLine = lines[index] ?? ''
        const row = tableCells(rowLine)
        if (!row || !rowLine.trim()) {
          index -= 1
          break
        }
        rows.push(row)
        index += 1
      }
      output.push(renderTable(headerCells, rows))
      continue
    }

    const heading = line.match(/^\s*(#{1,6})\s+(.+?)\s*#*\s*$/)
    if (heading) {
      flushParagraph()
      flushList()
      const level = heading[1]?.length ?? 1
      const headingText = heading[2] ?? ''
      output.push(`<h${level}>${inlineMarkdown(headingText)}</h${level}>`)
      continue
    }
    if (/^\s*(?:---+|\*\*\*+|___+)\s*$/.test(line)) {
      flushParagraph()
      flushList()
      output.push('<hr>')
      continue
    }
    const unordered = line.match(/^\s*[-*+]\s+(.+)$/)
    const ordered = line.match(/^\s*\d+[.)]\s+(.+)$/)
    if (unordered || ordered) {
      flushParagraph()
      const nextType = unordered ? 'ul' : 'ol'
      if (listType && listType !== nextType) flushList()
      listType = nextType
      listItems.push(inlineMarkdown((unordered || ordered)?.[1] ?? ''))
      continue
    }
    if (/^\s*>\s?/.test(line)) {
      flushParagraph()
      flushList()
      output.push(`<blockquote>${inlineMarkdown(line.replace(/^\s*>\s?/, ''))}</blockquote>`)
      continue
    }
    if (!line.trim()) {
      flushParagraph()
      flushList()
      continue
    }
    paragraph.push(line)
  }

  if (inFence) output.push(renderCodeBlock(fenceLines.join('\n'), fenceLanguage))
  flushParagraph()
  flushList()
  return output.join('')
}

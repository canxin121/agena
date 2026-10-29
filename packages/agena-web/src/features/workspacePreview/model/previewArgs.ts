/** Preserve argv byte-for-byte. Arguments are data passed to exec, never a shell program. */
export function copyPreviewArgs(value: unknown): string[] {
  if (!Array.isArray(value) || value.some((arg) => typeof arg !== 'string')) {
    throw new Error('Preview arguments must be a JSON array of strings.')
  }
  return [...value]
}

/** Accept an exact JSON string[] or familiar quoting, without shell expansion. */
export function parsePreviewArgs(input: string): string[] {
  const text = input.trim()
  if (!text) return []
  if (text.startsWith('[')) {
    try {
      return copyPreviewArgs(JSON.parse(text))
    } catch {
      throw new Error('Invalid arguments. Use a JSON array of strings, for example ["--name", "My app"].')
    }
  }

  const args: string[] = []
  let arg = ''
  let started = false
  let quote: 'single' | 'double' | null = null
  for (let i = 0; i < text.length; i++) {
    const char = text[i]!
    if (quote === 'single') {
      if (char === "'") quote = null
      else arg += char
      continue
    }
    if (char === '\\') {
      const next = text[i + 1]
      if (next === undefined) throw new Error('Invalid arguments: a trailing backslash must be escaped or quoted.')
      // In double quotes, keep Windows paths and other literal backslashes.
      if (quote === 'double' && !['"', '\\', '$', '`', '\n'].includes(next)) {
        arg += char
        continue
      }
      if (next !== '\n') arg += next
      i++
      started = true
      continue
    }
    if (quote === 'double') {
      if (char === '"') quote = null
      else arg += char
      continue
    }
    if (char === "'" || char === '"') {
      quote = char === "'" ? 'single' : 'double'
      started = true
    } else if (/\s/.test(char)) {
      if (started) args.push(arg)
      arg = ''
      started = false
    } else {
      arg += char
      started = true
    }
  }
  if (quote) throw new Error('Invalid arguments: close the quoted argument or use a JSON array of strings.')
  if (started) args.push(arg)
  return args
}

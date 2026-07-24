export function normalizeSseBuffer(buffer: string): string {
  return buffer.replace(/\r\n/g, '\n').replace(/\r/g, '\n')
}

export function parseSseEventBlock(block: string): {
  event: string
  id: string
  data: string
} {
  let event = 'message'
  let id = ''
  const data: string[] = []

  for (const rawLine of block.split('\n')) {
    if (!rawLine || rawLine.startsWith(':')) continue

    const separator = rawLine.indexOf(':')
    const field = separator >= 0 ? rawLine.slice(0, separator) : rawLine
    const value = separator >= 0 ? rawLine.slice(separator + 1).replace(/^ /, '') : ''

    switch (field) {
      case 'event':
        event = value || 'message'
        break
      case 'id':
        id = value
        break
      case 'data':
        data.push(value)
        break
      default:
        break
    }
  }

  return {
    event,
    id,
    data: data.join('\n'),
  }
}

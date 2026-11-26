/** The upper half of the safe integer range is reserved for live ACP views. */
export function isAcpSessionId(id: string | number | null | undefined): boolean {
  if (typeof id === 'string' && !/^\d+$/.test(id)) return false
  const value = Number(id)
  return Number.isSafeInteger(value) && value >= 2 ** 52
}

export function sessionApiPath(id: string | number): string {
  return `/api/v1/${isAcpSessionId(id) ? 'acp/' : ''}sessions/${encodeURIComponent(String(id))}`
}

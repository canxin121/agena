export function normalizeOptionSearch(query: string): string {
  return query.trim().toLowerCase()
}

export function optionSearchText(...fields: Array<string | undefined>): string {
  return fields.join(' ').toLowerCase()
}

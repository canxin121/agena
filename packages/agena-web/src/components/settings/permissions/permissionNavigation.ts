export const PERMISSION_ACCESS_VIEW = 'access'
export const PERMISSION_ACCESS_SECTIONS = ['filesystem', 'network', 'tools'] as const
export type PermissionAccessSection = (typeof PERMISSION_ACCESS_SECTIONS)[number]

export function permissionAccessSection(value: unknown): PermissionAccessSection | undefined {
  const normalized = String(value || '')
    .trim()
    .toLowerCase()
  return PERMISSION_ACCESS_SECTIONS.find((section) => section === normalized)
}

export function isPermissionAccessView(value: unknown): boolean {
  return (
    String(value || '')
      .trim()
      .toLowerCase() === PERMISSION_ACCESS_VIEW || Boolean(permissionAccessSection(value))
  )
}

export function permissionAccessHash(section: unknown): string {
  const resolved = permissionAccessSection(section)
  return resolved ? `#permission-${resolved}` : ''
}

export function permissionSectionFromHash(hash: unknown): PermissionAccessSection | undefined {
  const value = String(hash || '')
  return value.startsWith('#permission-') ? permissionAccessSection(value.slice('#permission-'.length)) : undefined
}

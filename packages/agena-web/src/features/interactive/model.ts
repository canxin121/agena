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

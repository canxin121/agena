import type { JsonObject, JsonValue } from '@/types/json'

export type JsonSchemaObject = JsonObject

export function isJsonRecord(value: unknown): value is JsonObject {
  return Boolean(value && typeof value === 'object' && !Array.isArray(value))
}

export function cloneJson<T extends JsonValue>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
}

export function stableJson(value: JsonValue): string {
  if (Array.isArray(value)) return `[${value.map((item) => stableJson(item)).join(',')}]`
  if (isJsonRecord(value)) {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${stableJson(value[key])}`)
      .join(',')}}`
  }
  return JSON.stringify(value)
}

export function mergeSchemaOverlay(target: JsonValue, overlay: JsonValue): JsonValue {
  if (!isJsonRecord(target) || !isJsonRecord(overlay)) return cloneJson(overlay)
  const next: JsonObject = cloneJson(target)
  for (const [key, value] of Object.entries(overlay)) {
    next[key] = key in next ? mergeSchemaOverlay(next[key], value) : cloneJson(value)
  }
  return next
}

export function localizedPluginSchema(
  schema: JsonValue,
  overlays: Record<string, JsonValue> | null | undefined,
  locale: string,
): JsonValue {
  if (!isJsonRecord(schema)) return schema
  const normalized = String(locale || '').trim()
  const language = normalized.split('-')[0] || ''
  const overlay = overlays?.[normalized] ?? (language ? overlays?.[language] : undefined)
  return overlay === undefined ? cloneJson(schema) : mergeSchemaOverlay(schema, overlay)
}

function refTarget(root: JsonValue, refValue: string): JsonValue | null {
  if (!refValue.startsWith('#/')) return null
  let current: JsonValue = root
  for (const rawSegment of refValue.slice(2).split('/')) {
    if (!isJsonRecord(current)) return null
    const segment = rawSegment.replaceAll('~1', '/').replaceAll('~0', '~')
    if (!(segment in current)) return null
    current = current[segment]
  }
  return current
}

export function resolveJsonSchema(schema: JsonValue, root: JsonValue): JsonValue {
  if (!isJsonRecord(schema)) return schema
  const refValue = typeof schema.$ref === 'string' ? schema.$ref : ''
  if (!refValue) return schema
  const target = refTarget(root, refValue)
  if (!target) return schema
  const rest: JsonObject = { ...schema }
  delete rest.$ref
  return mergeSchemaOverlay(target, rest)
}

function mergeAllOf(schema: JsonValue, root: JsonValue): JsonValue {
  const resolved = resolveJsonSchema(schema, root)
  if (!isJsonRecord(resolved) || !Array.isArray(resolved.allOf)) return resolved
  let merged: JsonValue = { ...resolved }
  if (isJsonRecord(merged)) delete merged.allOf
  for (const branch of resolved.allOf) merged = mergeSchemaOverlay(merged, normalizeJsonSchema(branch, root))
  return merged
}

export function normalizeJsonSchema(schema: JsonValue, root: JsonValue): JsonValue {
  return mergeAllOf(resolveJsonSchema(schema, root), root)
}

export function schemaType(schema: JsonValue, root: JsonValue): string {
  const normalized = normalizeJsonSchema(schema, root)
  if (!isJsonRecord(normalized)) return ''
  const rawType = normalized.type
  if (typeof rawType === 'string') return rawType
  if (Array.isArray(rawType)) {
    const nonNull = rawType.find((value) => value !== 'null')
    return typeof nonNull === 'string' ? nonNull : 'null'
  }
  if (isJsonRecord(normalized.properties)) return 'object'
  if (normalized.items !== undefined) return 'array'
  if (normalized.const !== undefined) return typeof normalized.const
  if (Array.isArray(normalized.enum) && normalized.enum.length > 0) {
    const first = normalized.enum.find((value) => value !== null)
    if (Array.isArray(first)) return 'array'
    if (isJsonRecord(first)) return 'object'
    return first === null ? 'null' : typeof first
  }
  return ''
}

export function schemaTitle(schema: JsonValue, fallback = ''): string {
  return isJsonRecord(schema) && typeof schema.title === 'string' && schema.title.trim()
    ? schema.title.trim()
    : fallback
}

export function schemaDescription(schema: JsonValue): string {
  return isJsonRecord(schema) && typeof schema.description === 'string' ? schema.description.trim() : ''
}

export function schemaProperties(schema: JsonValue, root: JsonValue): Array<[string, JsonValue]> {
  const normalized = normalizeJsonSchema(schema, root)
  if (!isJsonRecord(normalized) || !isJsonRecord(normalized.properties)) return []
  return Object.entries(normalized.properties)
}

export function schemaRequired(schema: JsonValue, root: JsonValue): Set<string> {
  const normalized = normalizeJsonSchema(schema, root)
  if (!isJsonRecord(normalized) || !Array.isArray(normalized.required)) return new Set()
  return new Set(normalized.required.filter((value): value is string => typeof value === 'string'))
}

export function schemaBranches(schema: JsonValue, root: JsonValue): JsonValue[] {
  const normalized = normalizeJsonSchema(schema, root)
  if (!isJsonRecord(normalized)) return []
  const branches = Array.isArray(normalized.oneOf)
    ? normalized.oneOf
    : Array.isArray(normalized.anyOf)
      ? normalized.anyOf
      : []
  return branches.map((branch) => normalizeJsonSchema(branch, root))
}

export function schemaEnumOptions(
  schema: JsonValue,
  root: JsonValue,
): Array<{ value: string; label: string; description?: string; raw: JsonValue }> {
  const normalized = normalizeJsonSchema(schema, root)
  if (!isJsonRecord(normalized)) return []
  if (Array.isArray(normalized.enum)) {
    return normalized.enum.map((raw) => ({ value: stableJson(raw), label: String(raw), raw }))
  }
  const branches = schemaBranches(normalized, root)
  const options = branches
    .filter((branch) => isJsonRecord(branch) && branch.const !== undefined)
    .map((branch) => {
      const raw = (branch as JsonObject).const
      return {
        value: stableJson(raw),
        label: schemaTitle(branch, String(raw)),
        description: schemaDescription(branch) || undefined,
        raw,
      }
    })
  return options
}

function minimumNumber(schema: JsonObject, integer: boolean): number {
  const minimum = typeof schema.minimum === 'number' ? schema.minimum : 0
  const exclusive = typeof schema.exclusiveMinimum === 'number' ? schema.exclusiveMinimum : null
  const multiple = typeof schema.multipleOf === 'number' && schema.multipleOf > 0 ? schema.multipleOf : integer ? 1 : 1
  let value = exclusive === null ? Math.max(0, minimum) : Math.max(0, exclusive + multiple)
  value = Math.ceil(value / multiple) * multiple
  return integer ? Math.ceil(value) : value
}

export function defaultValueForSchema(schema: JsonValue, root: JsonValue = schema): JsonValue {
  const normalized = normalizeJsonSchema(schema, root)
  if (!isJsonRecord(normalized)) return null
  if (normalized.default !== undefined) return cloneJson(normalized.default)
  if (normalized.const !== undefined) return cloneJson(normalized.const)
  const types = Array.isArray(normalized.type) ? normalized.type : []
  if (types.includes('null')) return null
  if (Array.isArray(normalized.enum) && normalized.enum.length > 0) return cloneJson(normalized.enum[0])
  const branches = schemaBranches(normalized, root)
  if (branches.length > 0) return defaultValueForSchema(branches[0], root)

  switch (schemaType(normalized, root)) {
    case 'object': {
      const required = schemaRequired(normalized, root)
      const object: JsonObject = {}
      for (const [key, childSchema] of schemaProperties(normalized, root)) {
        const child = normalizeJsonSchema(childSchema, root)
        const prefersPresence =
          required.has(key) ||
          (isJsonRecord(child) &&
            (child.default !== undefined || child.const !== undefined || schemaRequired(child, root).size > 0))
        if (prefersPresence) object[key] = defaultValueForSchema(child, root)
      }
      return object
    }
    case 'array':
      return []
    case 'string': {
      const minimum = typeof normalized.minLength === 'number' ? Math.max(0, Math.floor(normalized.minLength)) : 0
      return minimum > 0 ? 'x'.repeat(minimum) : ''
    }
    case 'integer':
      return minimumNumber(normalized, true)
    case 'number':
      return minimumNumber(normalized, false)
    case 'boolean':
      return false
    case 'null':
      return null
    default:
      return null
  }
}

export function mergeConfigOverride(target: JsonValue, overrideValue: JsonValue): JsonValue {
  if (!isJsonRecord(target) || !isJsonRecord(overrideValue)) return cloneJson(overrideValue)
  const next: JsonObject = cloneJson(target)
  for (const [key, value] of Object.entries(overrideValue)) {
    next[key] = key in next ? mergeConfigOverride(next[key], value) : cloneJson(value)
  }
  return next
}

export function materializeConfigValue(schema: JsonValue | null | undefined, value: JsonValue): JsonValue {
  if (!schema || !isJsonRecord(schema)) return cloneJson(value)
  const defaults = defaultValueForSchema(schema, schema)
  return value === null || value === undefined ? defaults : mergeConfigOverride(defaults, value)
}

export function deriveConfigOverride(defaultValue: JsonValue, effectiveValue: JsonValue): JsonValue | undefined {
  if (stableJson(defaultValue) === stableJson(effectiveValue)) return undefined
  if (isJsonRecord(defaultValue) && isJsonRecord(effectiveValue)) {
    const output: JsonObject = {}
    const keys = new Set([...Object.keys(defaultValue), ...Object.keys(effectiveValue)])
    for (const key of keys) {
      // Match the TUI workbench's merge-patch semantics. A key present in the
      // materialized default but absent from an advanced/raw draft must be
      // represented as null rather than silently reappearing after save.
      const childDefault = key in defaultValue ? defaultValue[key] : null
      const childEffective = key in effectiveValue ? effectiveValue[key] : null
      const child = deriveConfigOverride(childDefault, childEffective)
      if (child !== undefined) output[key] = child
    }
    return Object.keys(output).length > 0 ? output : undefined
  }
  return cloneJson(effectiveValue)
}

function schemaHasDiscriminatorConstraint(schema: JsonValue, root: JsonValue): boolean {
  const normalized = normalizeJsonSchema(schema, root)
  return Boolean(
    isJsonRecord(normalized) &&
    (normalized.const !== undefined ||
      (Array.isArray(normalized.enum) && normalized.enum.length > 0) ||
      schemaBranches(normalized, root).length > 0),
  )
}

export function schemaMatchesValue(schema: JsonValue, value: JsonValue, root: JsonValue): boolean {
  const normalized = normalizeJsonSchema(schema, root)
  if (!isJsonRecord(normalized)) return true
  if (normalized.const !== undefined) return stableJson(normalized.const) === stableJson(value)
  if (Array.isArray(normalized.enum)) return normalized.enum.some((entry) => stableJson(entry) === stableJson(value))
  const kind = schemaType(normalized, root)
  if (kind === 'null') return value === null
  if (kind === 'array') return Array.isArray(value)
  if (kind === 'object') {
    if (!isJsonRecord(value)) return false
    const required = schemaRequired(normalized, root)
    if ([...required].some((key) => !Object.prototype.hasOwnProperty.call(value, key))) return false
    for (const [key, childSchema] of schemaProperties(normalized, root)) {
      const constrained = schemaHasDiscriminatorConstraint(childSchema, root)
      if (!Object.prototype.hasOwnProperty.call(value, key)) {
        if (required.has(key) || constrained) return false
        continue
      }
      if (constrained && !schemaMatchesValue(childSchema, value[key], root)) return false
    }
    return true
  }
  if (kind === 'integer') return typeof value === 'number' && Number.isInteger(value)
  if (kind === 'number') return typeof value === 'number'
  return !kind || typeof value === kind
}

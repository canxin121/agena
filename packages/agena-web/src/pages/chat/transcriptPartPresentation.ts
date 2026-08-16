import type { TranscriptDisplayPart } from '@/components/chat/messageList.types'
import type { JsonValue } from '@/types/json'

export type JsonRecord = Record<string, JsonValue>

export type PartStatusPresentation = {
  icon: string
  label: string
  tone: 'pending' | 'success' | 'warning' | 'danger' | 'muted'
  spinning: boolean
  terminal: boolean
}

export type OperationDisplaySection = { title: string; text: string }
export type OperationPermissionPresentation = {
  requestId: string
  status: string
  action: string
  reason: string
  explanation: string
  replyReason: string
  provenance: string
}

export type OperationPresentation = {
  title: string
  summary: string
  toolName: string
  input: JsonValue | null
  inputMarkdown: string
  error: string
  humanMarkdown: string
  modelOutput: string
  stdout: string
  structured: JsonValue | null
  displaySections: OperationDisplaySection[]
  blocks: JsonRecord[]
  attachments: AttachmentPresentation[]
  metadata: JsonRecord
  durationMs: number | null
  userInputs: InteractionPresentation[]
  permissions: OperationPermissionPresentation[]
}

export type AttachmentPresentation = {
  key: string
  label: string
  mime: string
  url: string
  path: string
  sizeBytes: number | null
}

export type SkillPresentation = {
  name: string
  description: string
  instructions: string
  source: string
  contentHash: string
}

export type InteractionOptionPresentation = { label: string; description: string }
export type InteractionQuestionPresentation = {
  header: string
  question: string
  multiple: boolean
  allowCustom: boolean
  options: InteractionOptionPresentation[]
}

export type InteractionPresentation = {
  requestId: string
  title: string
  bodyMarkdown: string
  kind: string
  pending: boolean
  reply: JsonValue | null
  questions: InteractionQuestionPresentation[]
}

export type ErrorPresentation = {
  message: string
  code: string
  category: string
  responsibility: string
  impact: string
  recovery: string
  retry: string
  correlationId: string
  details: JsonRecord
}

export function isJsonRecord(value: unknown): value is JsonRecord {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

export function jsonRecord(value: unknown): JsonRecord {
  return isJsonRecord(value) ? value : {}
}

export function jsonArray(value: unknown): JsonValue[] {
  return Array.isArray(value) ? value : []
}

export function stringValue(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

export function firstString(source: JsonRecord, keys: readonly string[]): string {
  for (const key of keys) {
    const candidate = stringValue(source[key])
    if (candidate) return candidate
  }
  return ''
}

export function attentionRequestId(payload: JsonValue | null | undefined): string {
  const root = jsonRecord(payload)
  const properties = jsonRecord(root.properties)
  const request = jsonRecord(root.request)
  return (
    firstString(properties, ['id', 'request_id']) ||
    firstString(root, ['request_id', 'id']) ||
    firstString(request, ['request_id', 'id'])
  )
}

function numericValue(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

export function prettyJson(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value ?? '')
  }
}

export function partStatusPresentation(statusInput: string): PartStatusPresentation {
  const status = String(statusInput || 'pending')
    .trim()
    .toLowerCase()
  if (status === 'in_progress' || status === 'running') {
    return { icon: '⠋', label: 'running', tone: 'pending', spinning: true, terminal: false }
  }
  if (status === 'completed') {
    return { icon: '●', label: 'completed', tone: 'success', spinning: false, terminal: true }
  }
  if (status === 'policy_denied') {
    return { icon: '⊘', label: 'policy denied', tone: 'warning', spinning: false, terminal: true }
  }
  if (status === 'user_declined') {
    return { icon: '–', label: 'declined', tone: 'muted', spinning: false, terminal: true }
  }
  if (status === 'capability_unavailable' || status === 'tool_unavailable') {
    return { icon: '◇', label: 'unavailable', tone: 'warning', spinning: false, terminal: true }
  }
  if (status === 'failed' || status === 'error') {
    return { icon: '×', label: 'failed', tone: 'danger', spinning: false, terminal: true }
  }
  if (status === 'cancelled' || status === 'canceled') {
    return { icon: '–', label: 'cancelled', tone: 'muted', spinning: false, terminal: true }
  }
  return { icon: '○', label: 'pending', tone: 'pending', spinning: false, terminal: false }
}

/** Decode the API StructuredValue/StructuredObject envelope into ordinary JSON. */
export function decodeStructuredValue(value: JsonValue): JsonValue {
  if (Array.isArray(value)) return value.map((item) => decodeStructuredValue(item))
  if (!isJsonRecord(value)) return value

  const kind = stringValue(value.kind)
  if (kind === 'null') return null
  if (kind === 'text' || kind === 'number') return value.value ?? ''
  if (kind === 'integer' || kind === 'boolean') return value.value ?? null
  if (kind === 'array') return jsonArray(value.items).map((item) => decodeStructuredValue(item))
  if (kind === 'object' || Array.isArray(value.fields)) {
    const output: JsonRecord = {}
    for (const rawField of jsonArray(value.fields)) {
      const field = jsonRecord(rawField)
      const name = stringValue(field.name)
      if (!name) continue
      output[name] = decodeStructuredValue(field.value ?? null)
    }
    if (Object.keys(output).length || kind === 'object' || Array.isArray(value.fields)) return output
  }

  const output: JsonRecord = {}
  for (const [key, nested] of Object.entries(value)) output[key] = decodeStructuredValue(nested)
  return output
}

function inlineValue(value: JsonValue): string {
  if (value === null) return '`null`'
  if (typeof value === 'string') return value.includes('\n') ? value : `\`${value}\``
  if (typeof value === 'number' || typeof value === 'boolean') return `\`${String(value)}\``
  return ''
}

function markdownLines(value: JsonValue, depth: number): string[] {
  const indent = '  '.repeat(depth)
  if (Array.isArray(value)) {
    return value.flatMap((item) => {
      const inline = inlineValue(item)
      if (inline) return [`${indent}- ${inline}`]
      return [`${indent}-`, ...markdownLines(item, depth + 1)]
    })
  }
  if (isJsonRecord(value)) {
    return Object.entries(value).flatMap(([key, nested]) => {
      const inline = inlineValue(nested)
      if (inline) return [`${indent}- **${key}**: ${inline}`]
      return [`${indent}- **${key}**`, ...markdownLines(nested, depth + 1)]
    })
  }
  const inline = inlineValue(value)
  return inline ? [`${indent}- ${inline}`] : []
}

export function structuredValueMarkdown(value: JsonValue): string {
  return markdownLines(decodeStructuredValue(value), 0).join('\n')
}

function problemMessage(value: JsonValue): string {
  const problem = jsonRecord(value)
  const user = jsonRecord(problem.user)
  return firstString(user, ['fallback']) || firstString(problem, ['message'])
}

function operationFailureMessage(value: JsonValue): string {
  const error = jsonRecord(value)
  return problemMessage(error.failure) || problemMessage(error.problem) || firstString(error, ['message', 'detail'])
}

function normalizedBlockText(value: string): string {
  return value.replace(/\r\n/g, '\n').trim()
}

function operationBlockText(block: JsonRecord): string {
  const kind = firstString(block, ['type', 'kind'])
  if (kind === 'text' || kind === 'markdown' || kind === 'log') return firstString(block, ['text', 'markdown'])
  return ''
}

function splitOperationStdout(blocks: JsonRecord[]): { blocks: JsonRecord[]; stdout: string[] } {
  const outputBlocks: JsonRecord[] = []
  const stdout: string[] = []

  for (const block of blocks) {
    const kind = firstString(block, ['type', 'kind']).toLowerCase()
    if (kind === 'log' && firstString(block, ['stream']).toLowerCase() === 'stdout') {
      const text = firstString(block, ['text', 'markdown', 'content'])
      if (text) stdout.push(text)
      continue
    }

    if (kind === 'command') {
      const text = firstString(block, ['stdout'])
      if (text) stdout.push(text)
      const remainder: JsonRecord = { ...block }
      delete remainder.stdout
      const hasVisibleRemainder = Boolean(
        firstString(remainder, ['command', 'cwd', 'stderr']) || typeof remainder.exit_code === 'number',
      )
      if (hasVisibleRemainder) outputBlocks.push(remainder)
      continue
    }

    outputBlocks.push(block)
  }

  return { blocks: outputBlocks, stdout }
}

function operationStdout(
  content: JsonRecord,
  operation: JsonRecord,
  result: JsonRecord,
  blockStdout: string[],
): { text: string; normalized: Set<string> } {
  const candidates = [
    ...blockStdout,
    firstString(content, ['stdout']),
    firstString(operation, ['stdout']),
    firstString(result, ['stdout']),
    firstString(jsonRecord(operation.raw), ['stdout']),
    firstString(jsonRecord(result.raw), ['stdout']),
  ]
  const normalized = new Set<string>()
  const output: string[] = []
  for (const candidate of candidates) {
    const key = normalizedBlockText(candidate)
    if (!key || normalized.has(key)) continue
    normalized.add(key)
    output.push(candidate.trim())
  }
  const text = output.join('\n\n')
  if (text) normalized.add(normalizedBlockText(text))
  return { text, normalized }
}

function operationBlocks(
  operation: JsonRecord,
  result: JsonRecord,
  structured: JsonValue | null,
  toolName: string,
  modelText: string,
  humanMarkdown: string,
): JsonRecord[] {
  const merged = [...jsonArray(operation.blocks), ...jsonArray(result.content)]
    .map(jsonRecord)
    .filter((item) => Object.keys(item).length > 0)
    .filter(
      (item, index, values) => values.findIndex((candidate) => prettyJson(candidate) === prettyJson(item)) === index,
    )

  const structuredRecord = jsonRecord(structured)
  const structuredResults = jsonArray(structuredRecord.results)
  const searchTool = /(^|[._-])search$/i.test(toolName) || /web[._-]?search/i.test(toolName)
  const hasSearchBlock = merged.some((block) => firstString(block, ['type', 'kind']) === 'search_results')
  if (searchTool && structuredResults.length && !hasSearchBlock) {
    merged.unshift({
      type: 'search_results',
      query: firstString(structuredRecord, ['query']),
      results: structuredResults,
    })
  }

  const hasSemanticSearch = merged.some((block) => firstString(block, ['type', 'kind']) === 'search_results')
  const normalizedPrimary = normalizedBlockText(humanMarkdown || modelText)
  return merged.filter((block) => {
    const kind = firstString(block, ['type', 'kind'])
    if (hasSemanticSearch && kind === 'json' && prettyJson(block.value) === prettyJson(structured)) return false
    const text = normalizedBlockText(operationBlockText(block))
    if (hasSemanticSearch && text && text === normalizedPrimary) return false
    if (humanMarkdown && kind === 'markdown' && text === normalizedBlockText(humanMarkdown)) return false
    return true
  })
}

function attachmentFromRecord(value: JsonValue, index: number): AttachmentPresentation | null {
  const item = jsonRecord(value)
  if (!Object.keys(item).length) return null
  const source = jsonRecord(item.source)
  const mime = firstString(item, ['mime'])
  const path = firstString(source, ['path']) || firstString(item, ['path'])
  const directUrl = firstString(source, ['data_url', 'url']) || firstString(item, ['data_url', 'url'])
  const base64 = firstString(source, ['base64', 'data']) || firstString(item, ['base64'])
  const url = directUrl || (base64 && mime ? `data:${mime};base64,${base64}` : '') || path
  const label = firstString(item, ['title', 'filename', 'name']) || path || mime || `attachment-${index + 1}`
  return {
    key: firstString(item, ['sha256']) || url || `${label}:${index}`,
    label,
    mime,
    url,
    path,
    sizeBytes: numericValue(item.size_bytes),
  }
}

export function operationPresentation(part: TranscriptDisplayPart): OperationPresentation {
  const content = jsonRecord(part.source.agenaContent)
  const operation = jsonRecord(content.operation)
  const invocation = jsonRecord(operation.invocation)
  const result = jsonRecord(operation.result)
  const display = jsonRecord(result.display)
  const human = jsonRecord(result.human)
  const modelPreview = jsonRecord(result.model_preview)
  const modelOutput = jsonRecord(operation.model_output)
  const canonicalInput = jsonRecord(content.input)
  const encodedInput = Object.keys(canonicalInput).length ? canonicalInput : jsonRecord(invocation.input)
  const input = Object.keys(encodedInput).length ? decodeStructuredValue(encodedInput) : null
  const toolName =
    firstString(content, ['name', 'tool']) || firstString(invocation, ['name']) || stringValue(part.source.tool)
  const rawHumanMarkdown = firstString(human, ['markdown', 'summary'])
  const rawModelOutput = firstString(modelPreview, ['text']) || firstString(modelOutput, ['text'])
  const structured = result.structured ?? operation.structured ?? null
  const projectedBlocks = operationBlocks(operation, result, structured, toolName, rawModelOutput, rawHumanMarkdown)
  const splitBlocks = splitOperationStdout(projectedBlocks)
  const blocks = splitBlocks.blocks
  const stdout = operationStdout(content, operation, result, splitBlocks.stdout)
  const humanMarkdown = stdout.normalized.has(normalizedBlockText(rawHumanMarkdown)) ? '' : rawHumanMarkdown
  const modelOutputDuplicatedByBlock =
    blocks.some((block) => normalizedBlockText(operationBlockText(block)) === normalizedBlockText(rawModelOutput)) ||
    ((/(^|[._-])search$/i.test(toolName) || /web[._-]?search/i.test(toolName)) &&
      blocks.some((block) => firstString(block, ['type', 'kind']) === 'search_results'))
  const modelOutputText =
    modelOutputDuplicatedByBlock || stdout.normalized.has(normalizedBlockText(rawModelOutput)) ? '' : rawModelOutput

  const displaySections = jsonArray(display.sections)
    .map((item) => {
      const section = jsonRecord(item)
      const title = firstString(section, ['title'])
      const text = firstString(section, ['text'])
      return title && text ? { title, text } : null
    })
    .filter((item): item is OperationDisplaySection => Boolean(item))

  const attachments = [
    ...jsonArray(operation.attachments),
    ...jsonArray(modelOutput.attachments),
    ...jsonArray(result.attachments),
  ]
    .map(attachmentFromRecord)
    .filter((item): item is AttachmentPresentation => Boolean(item))
    .filter((item, index, list) => list.findIndex((candidate) => candidate.key === item.key) === index)

  const startMs = numericValue(jsonRecord(operation.lifecycle).start_ms) ?? part.source.time?.start ?? null
  const endMs = numericValue(jsonRecord(operation.lifecycle).end_ms) ?? part.source.time?.end ?? null
  const userInput = jsonRecord(operation.user_input)
  const userInputs = jsonArray(userInput.requests)
    .map((raw) => {
      const record = jsonRecord(raw)
      const request = jsonRecord(record.request)
      if (!Object.keys(request).length) return null
      return interactionFromRequest(request, record.reply ?? null, part.title)
    })
    .filter((item): item is InteractionPresentation => Boolean(item))
  const authorization = jsonRecord(operation.authorization)
  const permissions = jsonArray(authorization.permissions).map((raw) => {
    const permission = jsonRecord(raw)
    const request = jsonRecord(permission.request)
    const reply = jsonRecord(permission.reply)
    const action = jsonRecord(request.action)
    const actionKind = firstString(action, ['kind'])
    const actionLabel =
      actionKind === 'path_access'
        ? [firstString(action, ['access_kind']), firstString(action, ['target_path'])].filter(Boolean).join(' ')
        : actionKind === 'network_access'
          ? `network ${firstString(action, ['target', 'host'])}`.trim()
          : [firstString(action, ['tool_name', 'name']), firstString(action, ['qualifier'])]
              .filter(Boolean)
              .join(' · ') ||
            actionKind ||
            'permission'
    const replyKind = firstString(reply, ['kind'])
    const status =
      replyKind === 'allow_once'
        ? 'Allowed once'
        : replyKind === 'allow_always'
          ? 'Allowed persistently'
          : replyKind === 'deny_once'
            ? 'Denied once'
            : replyKind === 'deny_always'
              ? 'Denied persistently'
              : 'Awaiting user approval'
    return {
      requestId: firstString(request, ['request_id']),
      status,
      action: actionLabel,
      reason: firstString(request, ['reason']),
      explanation: firstString(request, ['explanation']),
      replyReason: firstString(reply, ['reason']),
      provenance: [firstString(request, ['source']), firstString(request, ['scope'])].filter(Boolean).join(' · '),
    }
  })

  return {
    title: firstString(operation, ['title']) || firstString(display, ['title']) || part.title,
    summary: firstString(operation, ['summary']) || firstString(display, ['summary']) || part.summary,
    toolName,
    input,
    inputMarkdown: input === null ? '' : structuredValueMarkdown(input),
    error:
      firstString(content, ['error']) ||
      operationFailureMessage(result.error ?? null) ||
      operationFailureMessage(operation.error ?? null),
    humanMarkdown,
    modelOutput: modelOutputText,
    stdout: stdout.text,
    structured,
    displaySections,
    blocks,
    attachments,
    metadata: {
      ...jsonRecord(operation.metadata),
      ...jsonRecord(result.metadata),
      ...jsonRecord(content.metadata),
    },
    durationMs: startMs !== null && endMs !== null && endMs >= startMs ? endMs - startMs : null,
    userInputs,
    permissions,
  }
}

export function partInteractionRequestIds(part: TranscriptDisplayPart): string[] {
  if (part.kind === 'interaction') {
    const id = interactionPresentation(part).requestId
    return id ? [id] : []
  }
  if (part.kind !== 'operation') return []
  const operation = operationPresentation(part)
  return [...operation.userInputs.map((item) => item.requestId), ...operation.permissions.map((item) => item.requestId)]
    .filter(Boolean)
    .filter((id, index, values) => values.indexOf(id) === index)
}

export function attachmentPresentations(part: TranscriptDisplayPart): AttachmentPresentation[] {
  const content = jsonRecord(part.source.agenaContent)
  const attachments = jsonArray(content.attachments)
  const projected = attachments
    .map(attachmentFromRecord)
    .filter((item): item is AttachmentPresentation => Boolean(item))
  if (projected.length) return projected

  const mime = stringValue(part.source.mime) || firstString(content, ['mime'])
  const path = stringValue(part.source.serverPath) || firstString(content, ['path'])
  const url = stringValue(part.source.url) || path
  const label =
    stringValue(part.source.filename) || firstString(content, ['name', 'title']) || path || mime || 'attachment'
  return [{ key: url || label, label, mime, url, path, sizeBytes: null }]
}

export function skillPresentations(part: TranscriptDisplayPart): SkillPresentation[] {
  const content = jsonRecord(part.source.agenaContent)
  const values = jsonArray(content.skills)
  const sourceValues = values.length ? values : [content]
  return sourceValues
    .map((value) => {
      const skill = jsonRecord(value)
      const name = firstString(skill, ['name', 'skill'])
      if (!name) return null
      return {
        name,
        description: firstString(skill, ['description']),
        instructions: firstString(skill, ['instructions']),
        source: firstString(skill, ['source']),
        contentHash: firstString(skill, ['content_hash']),
      }
    })
    .filter((item): item is SkillPresentation => Boolean(item))
}

function interactionQuestion(value: JsonValue): InteractionQuestionPresentation | null {
  const question = jsonRecord(value)
  const label = firstString(question, ['question', 'title'])
  if (!label) return null
  return {
    header: firstString(question, ['header']),
    question: label,
    multiple: question.multiple === true,
    allowCustom: question.allow_custom === true,
    options: jsonArray(question.options)
      .map((raw) => {
        const option = jsonRecord(raw)
        const label = firstString(option, ['label', 'title', 'value'])
        return label ? { label, description: firstString(option, ['description']) } : null
      })
      .filter((item): item is InteractionOptionPresentation => Boolean(item)),
  }
}

function interactionFromRequest(
  request: JsonRecord,
  reply: JsonValue | null,
  fallbackTitle: string,
): InteractionPresentation {
  return {
    requestId: firstString(request, ['request_id']),
    title: firstString(request, ['title']) || fallbackTitle,
    bodyMarkdown: firstString(request, ['body_markdown']),
    kind: firstString(request, ['kind']),
    pending: reply === null,
    reply,
    questions: jsonArray(request.questions)
      .map(interactionQuestion)
      .filter((item): item is InteractionQuestionPresentation => Boolean(item)),
  }
}

export function interactionPresentation(part: TranscriptDisplayPart): InteractionPresentation {
  const content = jsonRecord(part.source.agenaContent)
  const request = jsonRecord(content.request)
  const extra = jsonRecord(content.extra)
  const extraRequest = jsonRecord(extra.request)
  const resolvedRequest = Object.keys(request).length ? request : extraRequest
  const reply = content.reply ?? content.response ?? extra.reply ?? null
  const questionValues = jsonArray(resolvedRequest.questions).length
    ? jsonArray(resolvedRequest.questions)
    : jsonArray(content.options)
  const projected = interactionFromRequest(resolvedRequest, reply, firstString(content, ['prompt']) || part.title)
  return {
    ...projected,
    requestId: projected.requestId || firstString(content, ['request_id']),
    kind: projected.kind || firstString(content, ['kind', 'type']),
    questions: questionValues
      .map(interactionQuestion)
      .filter((item): item is InteractionQuestionPresentation => Boolean(item)),
  }
}

export function errorPresentation(part: TranscriptDisplayPart): ErrorPresentation {
  const content = jsonRecord(part.source.agenaContent)
  const problem = jsonRecord(content.problem)
  const user = jsonRecord(problem.user)
  return {
    message:
      firstString(user, ['fallback']) ||
      firstString(problem, ['message']) ||
      firstString(content, ['message']) ||
      part.summary,
    code: firstString(problem, ['code']),
    category: firstString(problem, ['category']),
    responsibility: firstString(problem, ['responsibility']),
    impact: firstString(problem, ['impact']),
    recovery: firstString(problem, ['recovery']),
    retry: firstString(problem, ['retry']),
    correlationId: firstString(problem, ['correlation_id', 'id']),
    details: problem,
  }
}
